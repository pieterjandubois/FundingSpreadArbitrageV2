use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::DynError;
use crate::utils;
use crate::strategy::pipeline::MarketProducer;
use crate::strategy::types::{MarketUpdate, symbol_to_id};

const BINANCE_USDM_BASE_URL: &str = "https://fapi.binance.com";
const BINANCE_USDM_WS_BASE_URL: &str = "wss://fstream.binance.com/ws";

// BINANCE OFFICIAL LIMITS (https://developers.binance.info/docs/derivatives/usds-margined-futures/websocket-market-streams/Connect):
// - Maximum 1024 streams per connection
// - Maximum 10 incoming messages per second per connection (FROM CLIENT: pings, pongs, subscribe/unsubscribe)
// - Maximum 300 connection attempts per 5 minutes per IP ⚠️ CRITICAL
// - 24-hour connection lifetime (auto-disconnect)
// - Binance sends PING frames every 3 minutes, client must PONG within 10 minutes
// - NO LIMIT on outgoing messages FROM SERVER (markPrice can send 100+ msg/sec)
//
// CRITICAL FINDING: Subscribe-after-connect approach fails after ~200 streams
// Both workers disconnect at exactly 200 streams during subscription
// HYPOTHESIS: Undocumented limit on subscription messages or subscribed streams per session
//
// SOLUTION: Use combined stream URL (original approach) with moderate stream count
// - 539 symbols × 2 streams = 1078 total streams
// - Use 200 streams per connection (100 symbols × 2 streams)
// - This requires 6 workers total
// - Combined stream URL: wss://fstream.binance.com/stream?streams=stream1/stream2/...
// 
// Configuration: All workers enabled to cover all symbols
// 539 symbols × 2 streams = 1078 streams total
// Using 100 streams per worker = 11 workers needed
const STREAMS_PER_CONNECTION: usize = 100;
const MAX_WORKERS: usize = 999; // No limit - spawn as many workers as needed

pub struct BinanceUsdmConnector;

impl BinanceUsdmConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let url = format!("{}/fapi/v1/time", BINANCE_USDM_BASE_URL);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            eprintln!("Binance Futures connection check failed: {}", response.status());
            std::process::exit(1);
        }
        println!("Binance Futures connection check OK");
        Ok(())
    }

    pub async fn run(
        client: &reqwest::Client, 
        tx: mpsc::Sender<(String, String)>,
        market_producer: Option<MarketProducer>,
    ) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let symbols = fetch_valid_usdt_perp_symbols(client).await?;
        println!("Valid USDT PERPETUAL symbols (TRADING): {}", symbols.len());

        let streams = build_streams(&symbols);
        let batches = utils::chunk_vec(&streams, STREAMS_PER_CONNECTION);
        let worker_count = batches.len().min(MAX_WORKERS);
        println!("Starting Binance websocket workers: {} (limited to {} for testing)", batches.len(), worker_count);

        for (worker_id, batch) in batches.into_iter().take(MAX_WORKERS).enumerate() {
            let tx_clone = tx.clone();
            let producer_clone = market_producer.clone();
            tokio::spawn(async move {
                // Stagger worker startup to avoid connection burst
                // 6 workers × 2000ms = 12 seconds total startup time
                tokio::time::sleep(std::time::Duration::from_millis(worker_id as u64 * 2000)).await;
                run_ws_worker(worker_id, Arc::new(batch), tx_clone, producer_clone).await;
            });
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ExchangeInfo {
    symbols: Vec<ExchangeSymbol>,
}

#[derive(Debug, Deserialize)]
struct ExchangeSymbol {
    symbol: String,
    status: String,
    #[serde(rename = "quoteAsset")]
    quote_asset: String,
    #[serde(rename = "contractType")]
    contract_type: String,
}

async fn fetch_valid_usdt_perp_symbols(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let url = format!("{}/fapi/v1/exchangeInfo", BINANCE_USDM_BASE_URL);
    let exchange_info = client.get(url).send().await?.json::<ExchangeInfo>().await?;

    let mut symbols: Vec<String> = exchange_info
        .symbols
        .into_iter()
        .filter(|s| s.status == "TRADING")
        .filter(|s| s.quote_asset == "USDT")
        .filter(|s| s.contract_type == "PERPETUAL")
        .filter(|s| is_plain_usdt_symbol(&s.symbol))
        .map(|s| s.symbol)
        .collect();

    symbols.sort();
    Ok(symbols)
}

fn build_streams(symbols: &[String]) -> Vec<String> {
    let mut streams = Vec::with_capacity(symbols.len() * 2);
    for s in symbols {
        let sym = s.to_lowercase();
        streams.push(format!("{}@markPrice@1s", sym));
        streams.push(format!("{}@bookTicker", sym));
    }
    streams
}

fn redis_key_for(stream_name: &str, symbol: &str) -> Option<String> {
    if stream_name.ends_with("@bookTicker") {
        return Some(format!("binance:usdm:book:{}", symbol));
    }
    if stream_name.contains("@markPrice") {
        return Some(format!("binance:usdm:mark:{}", symbol));
    }
    None
}

async fn run_ws_batch(
    worker_id: usize, 
    streams: &[String], 
    tx: mpsc::Sender<(String, String)>,
    market_producer: Option<MarketProducer>,
) -> Result<(), DynError> {
    // Use combined stream URL (not subscribe-after-connect)
    // Format: wss://fstream.binance.com/stream?streams=stream1/stream2/stream3
    let streams_param = streams.join("/");
    let url = format!("{}?streams={}", BINANCE_USDM_WS_BASE_URL.replace("/ws", "/stream"), streams_param);
    
    let connect_time = std::time::Instant::now();
    
    // Create websocket connection with proper request
    // Binance feedback: "If the initiation is good, there should be no problem"
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request()?;
    
    // Add standard websocket headers
    request.headers_mut().insert("User-Agent", "Mozilla/5.0".parse().unwrap());
    request.headers_mut().insert("Connection", "Upgrade".parse().unwrap());
    request.headers_mut().insert("Upgrade", "websocket".parse().unwrap());
    
    println!("[{}] Binance ws[{}] connecting to {} streams...", utils::ts_hm(), worker_id, streams.len());
    
    let (ws, response) = tokio_tungstenite::connect_async(request).await?;
    
    println!("[{}] Binance ws[{}] connected with {} streams (HTTP {}) - headers: {:?}", 
        utils::ts_hm(), worker_id, streams.len(), response.status(), response.headers());
    
    let (mut write, mut read) = ws.split();

    let mut first_data_logged = false;
    let mut message_count = 0u64;
    let mut last_message_time = std::time::Instant::now();
    let mut ping_count = 0u32;
    let mut pong_count = 0u32;

    let result = async {
        loop {
            let msg = match read.next().await {
                Some(m) => {
                    match m {
                        Ok(msg) => msg,
                        Err(e) => {
                            // Capture detailed error information
                            let duration = connect_time.elapsed();
                            println!("[{}] Binance ws[{}] read error after {:.1}s: {:?} | kind: {:?}", 
                                utils::ts_hm(), worker_id, duration.as_secs_f64(), e, 
                                std::error::Error::source(&e));
                            return Err(e.into());
                        }
                    }
                }
                None => {
                    let duration = connect_time.elapsed();
                    println!("[{}] Binance ws[{}] stream ended (None) after {:.1}s ({} msgs, {} pings, {} pongs)", 
                        utils::ts_hm(), worker_id, duration.as_secs_f64(), message_count, ping_count, pong_count);
                    break;
                }
            };

            // Handle Ping/Pong frames (CRITICAL: Binance sends pings every 3 minutes)
            let bytes = match msg {
                tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                    ping_count += 1;
                    println!("[{}] Binance ws[{}] received PING #{} (after {:.1}s)", 
                        utils::ts_hm(), worker_id, ping_count, connect_time.elapsed().as_secs_f64());
                    // Respond to Binance's ping with pong
                    if write.send(tokio_tungstenite::tungstenite::Message::Pong(payload)).await.is_err() {
                        println!("[{}] Binance ws[{}] failed to send PONG response", utils::ts_hm(), worker_id);
                        break;
                    }
                    pong_count += 1; // Successfully sent pong
                    println!("[{}] Binance ws[{}] sent PONG #{}", utils::ts_hm(), worker_id, pong_count);
                    continue;
                }
                tokio_tungstenite::tungstenite::Message::Pong(_) => {
                    // Received pong response (we don't send pings, so this shouldn't happen)
                    continue;
                }
                tokio_tungstenite::tungstenite::Message::Close(frame) => {
                    let duration = connect_time.elapsed();
                    let reason = frame.as_ref().map(|f| format!("code={}, reason={}", f.code, f.reason)).unwrap_or_else(|| "no reason".to_string());
                    println!("[{}] Binance ws[{}] received CLOSE frame after {:.1}s: {} ({} msgs, {} pings, {} pongs)", 
                        utils::ts_hm(), worker_id, duration.as_secs_f64(), reason, message_count, ping_count, pong_count);
                    break;
                }
                // Zero-copy WebSocket message handling (Requirement 8.1, 8.3, 8.4)
                // Work directly with bytes, avoiding String allocation
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    // Convert String to bytes (unavoidable with tungstenite's API)
                    text.into_bytes()
                }
                tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                    // Binary messages can be used directly
                    bytes
                }
                _ => {
                    // Skip other message types (Frame)
                    continue;
                }
            };

            message_count += 1;
            last_message_time = std::time::Instant::now();

            // SIMD-accelerated JSON parsing (Requirement 8.2)
            // Parse directly from bytes without intermediate String allocation
            let mut bytes_mut = bytes;
            let v: serde_json::Value = match simd_json::serde::from_slice(&mut bytes_mut) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Combined stream format: data is wrapped in stream/data
            let stream_name = match v.get("stream").and_then(|s| s.as_str()) {
                Some(s) => s,
                None => continue,
            };
            let data = match v.get("data") {
                Some(d) => d,
                None => continue,
            };
            let symbol = match data.get("s").and_then(|s| s.as_str()) {
                Some(s) => s,
                None => continue,
            };

            if !first_data_logged {
                first_data_logged = true;
                println!("[{}] Binance ws[{}] first data message received", utils::ts_hm(), worker_id);
            }

            // HOT PATH: Push to queue if this is bookTicker data and we have a producer
            if stream_name.ends_with("@bookTicker") {
                if let Some(ref producer) = market_producer {
                    if let (Some(bid), Some(ask)) = (
                        data.get("b").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()),
                        data.get("a").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()),
                    ) {
                        // Map symbol to ID
                        if let Some(symbol_id) = symbol_to_id(symbol) {
                            let timestamp_us = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_micros() as u64;
                            
                            let update = MarketUpdate::new(symbol_id, bid, ask, timestamp_us);
                            producer.push(update);
                        }
                    }
                }
            }

            let key = match redis_key_for(stream_name, symbol) {
                Some(k) => k,
                None => continue,
            };

            // COLD PATH: Still write to Redis for monitoring/dashboard
            let payload = serde_json::to_string(data)?;
            if tx.try_send((key, payload)).is_err() {
                continue;
            }
        }
        Ok::<(), DynError>(())
    }.await;

    // Always log session statistics, even on error
    let duration = connect_time.elapsed();
    let idle_time = last_message_time.elapsed();
    let msg_rate = if duration.as_secs_f64() > 0.0 {
        message_count as f64 / duration.as_secs_f64()
    } else {
        0.0
    };
    
    match &result {
        Ok(()) => {
            println!("[{}] Binance ws[{}] session ended cleanly: duration={:.1}s, messages={}, msg_rate={:.1}/s, idle_for={:.1}s", 
                utils::ts_hm(), worker_id, duration.as_secs_f64(), message_count, msg_rate, idle_time.as_secs_f64());
        }
        Err(e) => {
            println!("[{}] Binance ws[{}] session ended with error: {} | duration={:.1}s, messages={}, msg_rate={:.1}/s, idle_for={:.1}s, pings={}, pongs={}", 
                utils::ts_hm(), worker_id, e, duration.as_secs_f64(), message_count, msg_rate, idle_time.as_secs_f64(), ping_count, pong_count);
        }
    }

    result
}

async fn run_ws_worker(
    worker_id: usize, 
    streams: Arc<Vec<String>>, 
    tx: mpsc::Sender<(String, String)>,
    market_producer: Option<MarketProducer>,
) {
    // Pin WebSocket thread to cores 2-7 for optimal cache performance
    // Requirement: 4.2 (Pin WebSocket threads to cores 2-7)
    if let Err(e) = crate::strategy::thread_pinning::pin_websocket_thread(worker_id) {
        eprintln!("[THREAD-PIN] Warning: Failed to pin Binance worker {}: {}", worker_id, e);
        eprintln!("[THREAD-PIN] Continuing without thread pinning (performance may be degraded)");
    }
    
    let mut backoff_ms: u64 = 0;
    let mut connection_attempts = 0u32;
    let mut last_reset = std::time::Instant::now();
    
    loop {
        // Track connection attempts per 5-minute window (Binance limit: 300 per 5 min per IP)
        if last_reset.elapsed().as_secs() >= 300 {
            connection_attempts = 0;
            last_reset = std::time::Instant::now();
            println!("[{}] Binance ws[{}] connection attempt counter reset", utils::ts_hm(), worker_id);
        }
        
        connection_attempts += 1;
        
        // If we're approaching the limit (with 11 workers, each can do ~27 attempts per 5 min)
        // Add aggressive delays after multiple failures
        if connection_attempts > 10 {
            let penalty_ms = (connection_attempts as u64 - 10) * 5000; // 5s per attempt over 10
            println!("[{}] Binance ws[{}] connection attempt #{}, adding {}ms penalty", 
                utils::ts_hm(), worker_id, connection_attempts, penalty_ms);
            tokio::time::sleep(std::time::Duration::from_millis(penalty_ms)).await;
        }
        
        let res = run_ws_batch(worker_id, &streams[..], tx.clone(), market_producer.clone()).await;
        match &res {
            Ok(()) => {
                println!("[{}] Binance ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id);
                // Clean disconnect - reset backoff but keep connection attempt tracking
                backoff_ms = 0;
            }
            Err(e) => {
                println!("[{}] Binance ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e);
                // Error disconnect - use exponential backoff
                utils::apply_backoff(&mut backoff_ms).await;
            }
        }
        
        // Always add minimum 2-second delay between reconnection attempts
        // This spreads out reconnections across 11 workers (11 × 2s = 22s stagger)
        tokio::time::sleep(std::time::Duration::from_millis(2000.max(backoff_ms))).await;
    }
}

fn is_plain_usdt_symbol(symbol: &str) -> bool {
    if !symbol.is_ascii() {
        return false;
    }
    if !symbol.ends_with("USDT") {
        return false;
    }

    symbol
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'Z'))
}
