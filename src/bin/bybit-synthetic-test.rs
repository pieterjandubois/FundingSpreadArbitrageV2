// Bybit Synthetic Test Mode
// 
// This binary tests the arbitrage strategy by:
// 1. Connecting to REAL exchange WebSocket feeds (Binance, OKX, KuCoin, Bitget, etc.)
// 2. Using REAL market data to detect cross-exchange arbitrage opportunities
// 3. Using the dashboard qualification logic (OpportunityScanner) to validate opportunities
// 4. Executing BOTH legs of each trade on Bybit demo (single-exchange mode)
//
// This allows testing the complete strategy logic (detection, qualification, execution)
// without needing access to multiple exchange demo environments.

use std::error::Error;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arbitrage2::strategy::runner::StrategyRunner;
use arbitrage2::strategy::testnet_backend::TestnetBackend;
use arbitrage2::strategy::testnet_config::TestnetConfig;
use arbitrage2::strategy::execution_backend::ExecutionBackend;
use arbitrage2::{bitget, bybit, kucoin, okx, hyperliquid, paradex};

use tokio::sync::mpsc;
use tokio::time;
use crossbeam_queue::ArrayQueue;

const REDIS_URL: &str = "redis://127.0.0.1:6379";
const REDIS_FLUSH_MAX_ITEMS: usize = 512;
const REDIS_FLUSH_INTERVAL_MS: u64 = 50;
const REDIS_QUEUE_CAPACITY: usize = 32_768;
const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

pub type DynError = Box<dyn Error + Send + Sync>;

/// Global shutdown flag
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn is_shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Relaxed)
}

pub fn request_shutdown() {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

/// Background thread for Redis persistence
fn redis_writer_thread(queue: Arc<ArrayQueue<(String, String)>>) {
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
            if is_shutdown_requested() {
                eprintln!("[SHUTDOWN] Redis writer: Draining queue before exit...");
                
                while let Some(item) = queue.pop() {
                    buffer.push(item);
                    if buffer.len() >= REDIS_FLUSH_MAX_ITEMS {
                        if let Err(e) = flush_redis_buffer(&mut conn, &mut buffer).await {
                            eprintln!("[SHUTDOWN] Redis writer: Failed to flush: {}", e);
                        }
                    }
                }
                
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
                    while buffer.len() < REDIS_FLUSH_MAX_ITEMS {
                        match queue.pop() {
                            Some(item) => buffer.push(item),
                            None => break,
                        }
                    }
                    
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
    symbol_map: &arbitrage2::strategy::symbol_map::SymbolMap,
) -> Option<arbitrage2::strategy::types::MarketUpdate> {
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
    let symbol = arbitrage2::exchange_parser::normalize_symbol(symbol_raw);
    
    // Parse JSON value
    let json: serde_json::Value = match serde_json::from_str(value) {
        Ok(j) => j,
        Err(_) => return None,
    };
    
    // Get exchange-specific parser
    let parser = arbitrage2::exchange_parser::get_parser(exchange);
    
    // Extract bid and ask prices
    let bid_str = parser.parse_bid(&json)?;
    let ask_str = parser.parse_ask(&json)?;
    
    // Parse prices using SIMD-accelerated parser
    let bid = arbitrage2::exchange_parser::parse_price_simd(&bid_str)?;
    let ask = arbitrage2::exchange_parser::parse_price_simd(&ask_str)?;
    
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
    Some(arbitrage2::strategy::types::MarketUpdate::new(
        symbol_id,
        bid,
        ask,
        timestamp_us,
    ))
}

async fn redis_bridge(
    mut rx: mpsc::Receiver<(String, String)>, 
    queue: Arc<ArrayQueue<(String, String)>>,
    pipeline: Arc<arbitrage2::strategy::pipeline::MarketPipeline>,
    symbol_map: Arc<arbitrage2::strategy::symbol_map::SymbolMap>,
) {
    let producer = pipeline.producer();
    
    while let Some((key, value)) = rx.recv().await {
        // Hot path: Parse and push to pipeline (streaming)
        if let Some(update) = parse_to_market_update(&key, &value, &symbol_map) {
            producer.push(update);
        }
        
        // Cold path: Push to Redis queue (persistence)
        if let Err(rejected_item) = queue.push((key, value)) {
            queue.pop();
            let _ = queue.push(rejected_item);
        }
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
    println!("=== Bybit Synthetic Test Mode ===");
    println!("Starting initialization...\n");
    
    // Load environment variables
    dotenv::dotenv().ok();
    
    // Print thread pinning info
    arbitrage2::strategy::thread_pinning::print_core_assignment_info();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    // Create SPSC queue for Redis writes
    let redis_queue = Arc::new(ArrayQueue::new(REDIS_QUEUE_CAPACITY));
    
    // Task 5.1.1: Create SymbolMap instance
    let symbol_map = Arc::new(arbitrage2::strategy::symbol_map::SymbolMap::new());
    println!("[SYMBOL-MAP] Created with {} pre-allocated symbols", symbol_map.len());
    
    // Task 5.1.2: Create MarketPipeline instance
    let market_pipeline = Arc::new(arbitrage2::strategy::pipeline::MarketPipeline::new());
    println!("[PIPELINE] Created market pipeline (capacity: {})", market_pipeline.capacity());
    
    // Task 5.1.3: Create OpportunityQueue instance
    let opportunity_queue = Arc::new(arbitrage2::strategy::opportunity_queue::OpportunityQueue::new());
    println!("[OPPORTUNITY-QUEUE] Created opportunity queue (capacity: 1024)");
    
    // Task 5.1.4: Get consumers and producers
    let market_consumer = market_pipeline.consumer();
    let market_consumer_strategy = market_pipeline.consumer();  // Second consumer for strategy
    let opportunity_producer = opportunity_queue.producer();
    let opportunity_consumer_strategy = opportunity_queue.consumer();
    println!("[STREAMING] Created consumers and producers for streaming architecture");
    
    // Spawn Redis writer thread
    let redis_queue_clone = redis_queue.clone();
    let redis_writer_handle = std::thread::Builder::new()
        .name("redis-writer".to_string())
        .spawn(move || {
            redis_writer_thread(redis_queue_clone);
        })?;
    
    println!("[REDIS] Writer thread spawned (background persistence)");

    // Create mpsc channel for connectors
    let (tx, rx) = mpsc::channel::<(String, String)>(32_768);
    
    // Task 5.1.5: Pass SymbolMap and Pipeline to redis_bridge
    // Spawn bridge task with pipeline and symbol map
    let redis_queue_bridge = redis_queue.clone();
    let market_pipeline_bridge = market_pipeline.clone();
    let symbol_map_bridge = symbol_map.clone();
    let bridge_handle = tokio::spawn(async move {
        redis_bridge(rx, redis_queue_bridge, market_pipeline_bridge, symbol_map_bridge).await;
    });

    println!("[CONNECTORS] Starting exchange WebSocket connectors...");
    
    // Start exchange connectors (REAL market data)
    spawn_connector!(client, tx, bybit::BybitLinearConnector);
    spawn_connector!(client, tx, bitget::BitgetUsdtFuturesConnector);
    spawn_connector!(client, tx, kucoin::KucoinFuturesConnector);
    spawn_connector!(client, tx, okx::OkxUsdtSwapConnector);
    spawn_connector!(client, tx, hyperliquid::HyperliquidPerpsConnector);
    spawn_connector!(client, tx, paradex::ParadexPerpsConnector);
    
    println!("[CONNECTORS] ✅ All connectors started\n");

    drop(tx);

    // Task 5.1.6: Create OpportunityDetector with consumers/producers
    // Task 5.1.7: Spawn detector task
    println!("\n[OPPORTUNITY-DETECTOR] Initializing opportunity detection service...");
    let mut detector = arbitrage2::strategy::opportunity_detector::OpportunityDetector::new(
        market_consumer,
        symbol_map.clone(),
        opportunity_producer,
    );
    
    let detector_handle = tokio::spawn(async move {
        detector.run().await;
    });
    
    println!("[OPPORTUNITY-DETECTOR] ✅ Detector service started\n");

    // Load testnet configuration
    println!("[CONFIG] Loading testnet configuration...");
    let testnet_config = TestnetConfig::from_env()?;
    
    if testnet_config.single_exchange_mode {
        println!("[CONFIG] ✅ Single-exchange mode ENABLED");
        println!("[CONFIG] Primary exchange: {}", testnet_config.primary_exchange);
    } else {
        println!("[CONFIG] Single-exchange mode disabled (normal routing)");
    }
    
    // Create TestnetBackend with single-exchange mode
    let backend = Arc::new(TestnetBackend::new(testnet_config));
    
    // Sync server time for Bybit demo
    println!("\n[BACKEND] Synchronizing with Bybit demo...");
    backend.sync_server_time().await?;
    println!("[BACKEND] ✅ Time synchronized\n");

    // Start strategy runner
    println!("[STRATEGY] Initializing strategy runner...");
    let redis_client = redis::Client::open(REDIS_URL)?;
    let redis_conn = redis_client.get_multiplexed_tokio_connection().await?;
    
    // Get actual balance from Bybit demo to use as starting capital
    let actual_balance = match backend.get_available_balance("bybit").await {
        Ok(balance) => {
            println!("[STRATEGY] Bybit demo balance: ${:.2}", balance);
            balance
        }
        Err(e) => {
            eprintln!("[STRATEGY] ⚠️  Failed to fetch Bybit balance: {}, using default $20,000", e);
            20000.0
        }
    };
    
    let mut strategy_runner = StrategyRunner::new(
        redis_conn,
        actual_balance,  // Use actual balance from exchange
        backend,
        Some("synthetic".to_string()),  // Redis prefix
        None,  // All exchanges allowed
        symbol_map.clone(),  // Pass the dynamic symbol map
    ).await?;
    
    // Task 5.1.8: Pass OpportunityConsumer to StrategyRunner
    strategy_runner.set_opportunity_consumer(opportunity_consumer_strategy);
    strategy_runner.set_market_consumer(market_consumer_strategy);  // Also pass market consumer
    
    println!("[STRATEGY] ✅ Strategy runner initialized with ${:.2} capital", actual_balance);
    println!("[STRATEGY] ✅ OpportunityConsumer connected to streaming queue");
    println!("[STRATEGY] ✅ MarketConsumer connected for price data");
    println!("[STRATEGY] Starting streaming mode (consuming opportunities from queue)...\n");
    
    let strategy_handle = tokio::spawn(async move {
        if let Err(e) = strategy_runner.run_scanning_loop().await {
            eprintln!("Strategy runner error: {}", e);
        }
    });

    println!("=== System Running ===");
    println!("Press Ctrl+C to stop...\n");

    // Set up signal handlers
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        
        tokio::select! {
            _ = sigterm.recv() => {
                println!("\n[SHUTDOWN] Received SIGTERM");
            }
            _ = sigint.recv() => {
                println!("\n[SHUTDOWN] Received SIGINT (Ctrl+C)");
            }
        }
    }
    
    #[cfg(windows)]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[SHUTDOWN] Received Ctrl+C");
            }
        }
    }

    // Request shutdown
    request_shutdown();
    println!("[SHUTDOWN] Initiating graceful shutdown...");
    
    // Stop detector first
    detector_handle.abort();
    println!("[SHUTDOWN] Detector service stopped");
    
    // Wait for components to shut down
    let shutdown_result = tokio::time::timeout(
        Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
        async {
            let _ = strategy_handle.await;
            let _ = bridge_handle.await;
            let _ = redis_writer_handle.join();
            Ok::<(), DynError>(())
        }
    ).await;
    
    match shutdown_result {
        Ok(Ok(())) => {
            println!("[SHUTDOWN] ✅ Graceful shutdown completed");
            Ok(())
        }
        Ok(Err(e)) => {
            eprintln!("[SHUTDOWN] Completed with errors: {}", e);
            Err(e)
        }
        Err(_) => {
            eprintln!("[SHUTDOWN] Timed out after {}s", SHUTDOWN_TIMEOUT_SECS);
            Err("Shutdown timeout".into())
        }
    }
}
