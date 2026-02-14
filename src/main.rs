use std::error::Error;
use std::time::Duration;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod binance;
mod bitget;
mod bybit;
mod exchange_parser;
mod hyperliquid;
mod kucoin;
//mod lighter;
mod okx;
mod paradex;
mod utils;
mod strategy;

use tokio::sync::mpsc;
use tokio::time;
use strategy::runner::StrategyRunner;
use strategy::paper_trading_backend::PaperTradingBackend;
use crossbeam_queue::ArrayQueue;

const REDIS_URL: &str = "redis://127.0.0.1:6379";

const REDIS_FLUSH_MAX_ITEMS: usize = 512;
const REDIS_FLUSH_INTERVAL_MS: u64 = 50;
const REDIS_QUEUE_CAPACITY: usize = 32_768;

/// Graceful shutdown timeout: maximum time to wait for clean shutdown
const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

pub type DynError = Box<dyn Error + Send + Sync>;

/// Global shutdown flag for coordinating graceful shutdown across threads
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Check if shutdown has been requested
pub fn is_shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Relaxed)
}

/// Request graceful shutdown
pub fn request_shutdown() {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

/// Background thread for Redis persistence (non-blocking writes)
/// Uses SPSC queue to decouple Redis writes from hot path
fn redis_writer_thread(queue: Arc<ArrayQueue<(String, String)>>) {
    // Create a new Tokio runtime for this thread
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Redis writer runtime");
    
    rt.block_on(async {
        let client = match redis::Client::open(REDIS_URL) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Redis writer: Failed to create client: {}", e);
                return;
            }
        };
        
        let mut conn = match client.get_multiplexed_tokio_connection().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Redis writer: Failed to connect: {}", e);
                return;
            }
        };
        
        let mut buffer: Vec<(String, String)> = Vec::with_capacity(REDIS_FLUSH_MAX_ITEMS);
        let mut tick = time::interval(Duration::from_millis(REDIS_FLUSH_INTERVAL_MS));
        
        loop {
            // Check for shutdown request
            if is_shutdown_requested() {
                eprintln!("[SHUTDOWN] Redis writer: Draining queue before exit...");
                
                // Drain all remaining items from queue
                while let Some(item) = queue.pop() {
                    buffer.push(item);
                    if buffer.len() >= REDIS_FLUSH_MAX_ITEMS {
                        // Flush batch
                        if let Err(e) = flush_redis_buffer(&mut conn, &mut buffer).await {
                            eprintln!("[SHUTDOWN] Redis writer: Failed to flush during shutdown: {}", e);
                        }
                    }
                }
                
                // Flush any remaining items
                if !buffer.is_empty() {
                    if let Err(e) = flush_redis_buffer(&mut conn, &mut buffer).await {
                        eprintln!("[SHUTDOWN] Redis writer: Failed to flush final batch: {}", e);
                    }
                }
                
                eprintln!("[SHUTDOWN] Redis writer: Queue drained, exiting");
                return;
            }
            
            tokio::select! {
                _ = tick.tick() => {
                    // Drain queue into buffer
                    while buffer.len() < REDIS_FLUSH_MAX_ITEMS {
                        match queue.pop() {
                            Some(item) => buffer.push(item),
                            None => break,
                        }
                    }
                    
                    // Flush buffer to Redis
                    if !buffer.is_empty() {
                        if let Err(e) = flush_redis_buffer(&mut conn, &mut buffer).await {
                            eprintln!("Redis writer: Failed to flush: {}", e);
                        }
                    }
                }
            }
        }
    });
}

/// Helper function to flush Redis buffer
async fn flush_redis_buffer(
    conn: &mut redis::aio::MultiplexedConnection,
    buffer: &mut Vec<(String, String)>,
) -> Result<(), redis::RedisError> {
    let mut pipe = redis::pipe();
    for (k, v) in buffer.iter() {
        pipe.cmd("SET").arg(k).arg(v).ignore();
        pipe.publish(k, v).ignore();
    }
    
    pipe.query_async::<_, ()>(conn).await?;
    buffer.clear();
    Ok(())
}

/// Parse Redis key/value to MarketUpdate struct
///
/// Extracts exchange and symbol from key, parses JSON for bid/ask,
/// maps to symbol_id, and creates MarketUpdate.
///
/// # Performance
/// - Target: < 50μs per message
/// - Uses SIMD-accelerated price parsing
/// - Zero-copy where possible
fn parse_to_market_update(
    key: &str,
    value: &str,
    symbol_map: &strategy::symbol_map::SymbolMap,
) -> Option<strategy::types::MarketUpdate> {
    // Parse key format: "exchange:type:subtype:symbol"
    // Examples:
    // - "bybit:linear:tickers:BTCUSDT"
    // - "okx:usdt:tickers:BTC-USDT-SWAP"
    // - "hyperliquid:usdc:ctx:BTC"
    let parts: Vec<&str> = key.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    
    let exchange = parts[0];
    let symbol_raw = parts[parts.len() - 1];
    
    // Normalize symbol to standard format (BTCUSDT)
    let symbol = exchange_parser::normalize_symbol(symbol_raw);
    
    // Parse JSON value
    let json: serde_json::Value = match serde_json::from_str(value) {
        Ok(j) => j,
        Err(_) => return None,
    };
    
    // Get exchange-specific parser
    let parser = exchange_parser::get_parser(exchange);
    
    // Extract bid and ask prices
    let bid_str = parser.parse_bid(&json)?;
    let ask_str = parser.parse_ask(&json)?;
    
    // Parse prices using SIMD-accelerated parser
    let bid = exchange_parser::parse_price_simd(&bid_str)?;
    let ask = exchange_parser::parse_price_simd(&ask_str)?;
    
    // Validate prices
    if bid <= 0.0 || ask <= 0.0 || bid >= ask {
        return None;
    }
    
    // Map (exchange, symbol) to symbol_id
    let symbol_id = symbol_map.get_or_insert(exchange, &symbol);
    
    // Get current timestamp
    let timestamp_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;
    
    // Create MarketUpdate
    Some(strategy::types::MarketUpdate::new(
        symbol_id,
        bid,
        ask,
        timestamp_us,
    ))
}

/// Bridge task: forwards from mpsc channel to SPSC queue (non-blocking)
/// This allows connectors to keep using mpsc::Sender while Redis writes use SPSC queue
/// 
/// Enhanced for Task 1.3: Also parses JSON and pushes MarketUpdate to pipeline
async fn redis_bridge(
    mut rx: mpsc::Receiver<(String, String)>, 
    queue: Arc<ArrayQueue<(String, String)>>,
    pipeline: Arc<strategy::pipeline::MarketPipeline>,
    symbol_map: Arc<strategy::symbol_map::SymbolMap>,
) {
    let producer = pipeline.producer();
    
    while let Some((key, value)) = rx.recv().await {
        // Hot path: Parse and push to pipeline (streaming)
        if let Some(update) = parse_to_market_update(&key, &value, &symbol_map) {
            producer.push(update);
        }
        
        // Cold path: Push to Redis queue (persistence)
        // Try to push to SPSC queue (non-blocking)
        if let Err(rejected_item) = queue.push((key, value)) {
            // Queue full - drop oldest item and retry
            queue.pop();
            let _ = queue.push(rejected_item);
        }
    }
}

async fn oi_poller(client: reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    // Poll OI data from exchanges every 5 minutes
    let mut interval = time::interval(Duration::from_secs(300));
    let redis_client = redis::Client::open(REDIS_URL)?;
    let mut redis_conn = redis_client.get_connection()?;
    
    loop {
        interval.tick().await;
        
        // Collect OI data from all exchanges (Binance excluded - websocket disabled)
        let symbols = vec!["BTCUSDT", "ETHUSDT"];
        let exchanges = vec!["okx", "bybit", "kucoin", "bitget", "hyperliquid", "paradex"];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        for symbol in &symbols {
            for exchange in &exchanges {
                let oi_data = match fetch_oi_data(&client, exchange, symbol).await {
                    Ok(data) => data,
                    Err(e) => {
                        eprintln!("Failed to fetch OI from {} for {}: {}", exchange, symbol, e);
                        continue;
                    }
                };
                
                // Extract OI value from JSON
                if let Ok(oi_value) = extract_oi_value(&oi_data, exchange) {
                    // Store current OI
                    let key = format!("{}:oi:{}", exchange, symbol);
                    if tx.send((key.clone(), oi_data.clone())).await.is_err() {
                        return Ok(());
                    }
                    
                    // Store OI snapshot with timestamp in sorted set for 24h average calculation
                    let snapshot_key = format!("{}:oi_history:{}", exchange, symbol);
                    let score = now as f64;
                    let member = format!("{},{}", now, oi_value);
                    
                    // Add to sorted set
                    let _: () = redis::cmd("ZADD")
                        .arg(&snapshot_key)
                        .arg(score)
                        .arg(&member)
                        .query(&mut redis_conn)
                        .unwrap_or_default();
                    
                    // Remove snapshots older than 24 hours (86400 seconds)
                    let cutoff_time = now - 86400;
                    let _: () = redis::cmd("ZREMRANGEBYSCORE")
                        .arg(&snapshot_key)
                        .arg("-inf")
                        .arg(cutoff_time as f64)
                        .query(&mut redis_conn)
                        .unwrap_or_default();
                    
                    // Set expiry on the sorted set (25 hours to be safe)
                    let _: () = redis::cmd("EXPIRE")
                        .arg(&snapshot_key)
                        .arg(90000)
                        .query(&mut redis_conn)
                        .unwrap_or_default();
                }
            }
        }
    }
}

fn extract_oi_value(oi_data: &str, exchange: &str) -> Result<f64, DynError> {
    let json: serde_json::Value = serde_json::from_str(oi_data)?;
    
    let oi = match exchange {
        "binance" => {
            json.get("openInterest")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "okx" => {
            json.get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("oi"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "bybit" => {
            json.get("result")
                .and_then(|r| r.get("list"))
                .and_then(|l| l.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("openInterest"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "kucoin" => {
            json.get("data")
                .and_then(|d| d.get("openInterest"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "bitget" => {
            json.get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("oi"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "hyperliquid" => {
            json.get("openInterest")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "paradex" => {
            json.get("market")
                .and_then(|m| m.get("open_interest"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        _ => None,
    };
    
    oi.ok_or_else(|| "Failed to extract OI value".into())
}

async fn fetch_oi_data(client: &reqwest::Client, exchange: &str, symbol: &str) -> Result<String, DynError> {
    match exchange {
        "binance" => {
            let url = format!("https://fapi.binance.com/fapi/v1/openInterest?symbol={}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "okx" => {
            let inst_id = format!("{}-USDT-SWAP", symbol.trim_end_matches("USDT"));
            let url = format!("https://www.okx.com/api/v5/public/open-interest?instId={}", inst_id);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "bybit" => {
            let url = format!("https://api.bybit.com/v5/market/open-interest?category=linear&symbol={}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "kucoin" => {
            let symbol_kc = format!("{}M", symbol.trim_end_matches("USDT"));
            let url = format!("https://api-futures.kucoin.com/api/v1/contracts/{}/open-interest", symbol_kc);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "bitget" => {
            let url = format!("https://api.bitget.com/api/v2/mix/market/open-interest?productType=usdt-futures&symbol={}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "hyperliquid" => {
            // Hyperliquid: POST /info with type=openInterest
            let coin = symbol.trim_end_matches("USDT");
            let payload = serde_json::json!({
                "type": "openInterest",
                "coin": coin
            });
            let resp = client
                .post("https://api.hyperliquid.xyz/info")
                .json(&payload)
                .send()
                .await?;
            Ok(resp.text().await?)
        }
        "paradex" => {
            // Paradex: GET /markets/{symbol}
            let url = format!("https://api.prod.paradex.trade/v1/markets/{}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        _ => Err("Unknown exchange".into()),
    }
}

macro_rules! spawn_connector {
    ($client:expr, $tx:expr, $connector:ty) => {
        {
            let c = $client.clone();
            let t = $tx.clone();
            tokio::spawn(async move {
                let _ = <$connector>::run(&c, t, None).await;
            });
        }
    };
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    // Load environment variables from .env file
    dotenv::dotenv().ok();
    
    // Print thread pinning information
    // Note: Actual pinning happens in strategy runner and WebSocket threads
    strategy::thread_pinning::print_core_assignment_info();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    // Create SPSC queue for non-blocking Redis writes
    let redis_queue = Arc::new(ArrayQueue::new(REDIS_QUEUE_CAPACITY));
    
    // Task 5.2.1: Create SymbolMap instance
    let symbol_map = Arc::new(strategy::symbol_map::SymbolMap::new());
    println!("Symbol map created with {} pre-allocated symbols", symbol_map.len());
    
    // Task 5.2.2: Create MarketPipeline instance
    let market_pipeline = Arc::new(strategy::pipeline::MarketPipeline::new());
    println!("Market pipeline created (capacity: {})", market_pipeline.capacity());
    
    // Task 5.2.3: Create OpportunityQueue instance
    let opportunity_queue = Arc::new(strategy::opportunity_queue::OpportunityQueue::new());
    println!("Opportunity queue created (capacity: 1024)");
    
    // Task 5.2.4: Get consumers and producers
    let market_consumer = market_pipeline.consumer();
    let opportunity_producer = opportunity_queue.producer();
    let opportunity_consumer_strategy = opportunity_queue.consumer();
    println!("Created consumers and producers for streaming architecture");
    
    // Spawn dedicated Redis writer thread (background persistence)
    let redis_queue_clone = redis_queue.clone();
    let redis_writer_handle = std::thread::Builder::new()
        .name("redis-writer".to_string())
        .spawn(move || {
            redis_writer_thread(redis_queue_clone);
        })?;
    
    println!("Redis writer thread spawned (background persistence)");

    // Create mpsc channel for connectors (maintains existing interface)
    let (tx, rx) = mpsc::channel::<(String, String)>(32_768);
    
    // Task 5.2.5: Pass SymbolMap and Pipeline to redis_bridge
    // Spawn bridge task to forward mpsc -> SPSC queue and pipeline
    let redis_queue_bridge = redis_queue.clone();
    let market_pipeline_bridge = market_pipeline.clone();
    let symbol_map_bridge = symbol_map.clone();
    let bridge_handle = tokio::spawn(async move {
        redis_bridge(rx, redis_queue_bridge, market_pipeline_bridge, symbol_map_bridge).await;
    });

    // DISABLED: Binance websocket connection causes IP bans due to aggressive rate limiting
    // spawn_connector!(client, tx, binance::BinanceUsdmConnector);
    
    spawn_connector!(client, tx, bybit::BybitLinearConnector);
    spawn_connector!(client, tx, bitget::BitgetUsdtFuturesConnector);
    spawn_connector!(client, tx, kucoin::KucoinFuturesConnector);
    spawn_connector!(client, tx, okx::OkxUsdtSwapConnector);
    spawn_connector!(client, tx, hyperliquid::HyperliquidPerpsConnector);

    /* let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = lighter::LighterPerpsConnector::run(&client_clone, tx_clone).await;
    }); */

    spawn_connector!(client, tx, paradex::ParadexPerpsConnector);

    // Spawn OI poller (uses same mpsc channel)
    let client_oi = client.clone();
    let tx_oi = tx.clone();
    let oi_handle = tokio::spawn(async move {
        if let Err(e) = oi_poller(client_oi, tx_oi).await {
            eprintln!("OI poller error: {}", e);
        }
    });

    drop(tx);

    // Task 5.2.6: Create OpportunityDetector with consumers/producers
    // Task 5.2.7: Spawn detector task
    println!("Initializing opportunity detection service...");
    let mut detector = strategy::opportunity_detector::OpportunityDetector::new(
        market_consumer,
        symbol_map.clone(),
        opportunity_producer,
    );
    
    let detector_handle = tokio::spawn(async move {
        detector.run().await;
    });
    
    println!("Opportunity detector service started");

    // Start strategy runner with real data from Redis
    println!("Starting spread arbitrage strategy runner...");
    let redis_client = redis::Client::open(REDIS_URL)?;
    let redis_conn = redis_client.get_multiplexed_tokio_connection().await?;
    
    // Create paper trading backend with initial balances for each exchange
    let mut initial_balances = HashMap::new();
    initial_balances.insert("binance".to_string(), 20000.0);
    initial_balances.insert("bybit".to_string(), 20000.0);
    initial_balances.insert("bitget".to_string(), 20000.0);
    initial_balances.insert("kucoin".to_string(), 20000.0);
    initial_balances.insert("okx".to_string(), 20000.0);
    initial_balances.insert("hyperliquid".to_string(), 20000.0);
    initial_balances.insert("paradex".to_string(), 20000.0);
    
    let backend = Arc::new(PaperTradingBackend::new(initial_balances));
    
    let mut strategy_runner = StrategyRunner::new(
        redis_conn, 
        20000.0, 
        backend, 
        None, 
        None,
        symbol_map.clone(),  // Pass the dynamic symbol map
    ).await?;
    
    // Task 5.2.8: Pass OpportunityConsumer to StrategyRunner
    strategy_runner.set_opportunity_consumer(opportunity_consumer_strategy);
    
    println!("Strategy runner initialized with $20,000 capital");
    println!("OpportunityConsumer connected to streaming queue");
    println!("Starting streaming mode (consuming opportunities from queue)...");
    
    let strategy_handle = tokio::spawn(async move {
        if let Err(e) = strategy_runner.run_scanning_loop().await {
            eprintln!("Strategy runner error: {}", e);
        }
    });

    // Set up signal handlers for graceful shutdown
    println!("Setting up signal handlers for graceful shutdown...");
    
    // Wait for shutdown signal or keep running
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        
        tokio::select! {
            _ = sigterm.recv() => {
                println!("\n[SHUTDOWN] Received SIGTERM, initiating graceful shutdown...");
            }
            _ = sigint.recv() => {
                println!("\n[SHUTDOWN] Received SIGINT (Ctrl+C), initiating graceful shutdown...");
            }
        }
    }
    
    #[cfg(windows)]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[SHUTDOWN] Received Ctrl+C, initiating graceful shutdown...");
            }
        }
    }

    // Request shutdown
    request_shutdown();
    
    // Task 5.2.9: Update graceful shutdown to stop detector
    // Perform graceful shutdown with timeout
    println!("[SHUTDOWN] Waiting for components to shut down (timeout: {}s)...", SHUTDOWN_TIMEOUT_SECS);
    
    let shutdown_result = tokio::time::timeout(
        Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
        perform_graceful_shutdown(strategy_handle, bridge_handle, oi_handle, redis_writer_handle, redis_queue, detector_handle)
    ).await;
    
    match shutdown_result {
        Ok(Ok(())) => {
            println!("[SHUTDOWN] Graceful shutdown completed successfully");
            Ok(())
        }
        Ok(Err(e)) => {
            eprintln!("[SHUTDOWN] Graceful shutdown completed with errors: {}", e);
            Err(e)
        }
        Err(_) => {
            eprintln!("[SHUTDOWN] Graceful shutdown timed out after {}s, forcing exit", SHUTDOWN_TIMEOUT_SECS);
            Err("Shutdown timeout".into())
        }
    }
}

/// Perform graceful shutdown of all components
///
/// This function coordinates the shutdown of:
/// 1. Strategy runner (stops processing opportunities)
/// 2. Opportunity detector (stops detecting opportunities)
/// 3. Redis bridge (stops forwarding messages)
/// 4. OI poller (stops polling)
/// 5. Redis writer thread (drains queue and flushes)
/// 6. WebSocket connections (implicitly closed when tasks are dropped)
///
/// Requirements: Task 33 (Graceful shutdown), Task 5.2.9 (Stop detector)
async fn perform_graceful_shutdown(
    strategy_handle: tokio::task::JoinHandle<()>,
    bridge_handle: tokio::task::JoinHandle<()>,
    oi_handle: tokio::task::JoinHandle<()>,
    redis_writer_handle: std::thread::JoinHandle<()>,
    redis_queue: Arc<ArrayQueue<(String, String)>>,
    detector_handle: tokio::task::JoinHandle<()>,
) -> Result<(), DynError> {
    println!("[SHUTDOWN] Step 1/6: Stopping strategy runner...");
    // Strategy runner will check is_shutdown_requested() and exit gracefully
    // Wait for it to finish processing current opportunities
    if let Err(e) = strategy_handle.await {
        eprintln!("[SHUTDOWN] Strategy runner join error: {}", e);
    } else {
        println!("[SHUTDOWN] Strategy runner stopped");
    }
    
    println!("[SHUTDOWN] Step 2/6: Stopping opportunity detector...");
    // Detector will be aborted (it's a background task)
    detector_handle.abort();
    println!("[SHUTDOWN] Opportunity detector stopped");
    
    println!("[SHUTDOWN] Step 3/6: Stopping OI poller...");
    // OI poller will be aborted (it's a background task)
    oi_handle.abort();
    println!("[SHUTDOWN] OI poller stopped");
    
    println!("[SHUTDOWN] Step 4/6: Stopping Redis bridge...");
    // Redis bridge will exit when the mpsc channel is closed (already dropped tx)
    if let Err(e) = bridge_handle.await {
        if !e.is_cancelled() {
            eprintln!("[SHUTDOWN] Redis bridge join error: {}", e);
        }
    } else {
        println!("[SHUTDOWN] Redis bridge stopped");
    }
    
    println!("[SHUTDOWN] Step 5/6: Flushing Redis writes...");
    // Wait for Redis writer thread to drain queue and flush
    // The thread checks is_shutdown_requested() and will exit after draining
    let queue_depth = redis_queue.len();
    if queue_depth > 0 {
        println!("[SHUTDOWN] Waiting for {} Redis writes to flush...", queue_depth);
    }
    
    if let Err(e) = redis_writer_handle.join() {
        eprintln!("[SHUTDOWN] Redis writer thread join error: {:?}", e);
    } else {
        println!("[SHUTDOWN] Redis writes flushed");
    }
    
    println!("[SHUTDOWN] Step 6/6: Saving state to disk...");
    // Save final state snapshot
    if let Err(e) = save_shutdown_state().await {
        eprintln!("[SHUTDOWN] Failed to save state: {}", e);
    } else {
        println!("[SHUTDOWN] State saved to disk");
    }
    
    println!("[SHUTDOWN] All components shut down cleanly");
    Ok(())
}

/// Save application state to disk during shutdown
///
/// This creates a snapshot of the current state that can be used
/// for debugging or recovery after restart.
///
/// Requirements: Task 33 (Save state to disk)
async fn save_shutdown_state() -> Result<(), DynError> {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    let state_file = format!("shutdown_state_{}.json", timestamp);
    
    // Create a simple state snapshot
    let state = serde_json::json!({
        "timestamp": timestamp,
        "shutdown_reason": "graceful",
        "version": env!("CARGO_PKG_VERSION"),
    });
    
    fs::write(&state_file, serde_json::to_string_pretty(&state)?)?;
    println!("[SHUTDOWN] State saved to {}", state_file);
    
    Ok(())
}
