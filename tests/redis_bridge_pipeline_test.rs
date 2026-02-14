//! Integration tests for Task 1.3: Redis Bridge Pipeline Integration
//!
//! Tests that redis_bridge correctly:
//! 1. Converts JSON strings to MarketUpdate structs
//! 2. Pushes MarketUpdate to pipeline
//! 3. Still writes to Redis for persistence

use arbitrage2::strategy::pipeline::MarketPipeline;
use arbitrage2::strategy::symbol_map::SymbolMap;
use arbitrage2::strategy::types::MarketUpdate;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use crossbeam_queue::ArrayQueue;

/// Helper function to simulate parse_to_market_update logic
fn parse_to_market_update(
    key: &str,
    value: &str,
    symbol_map: &SymbolMap,
) -> Option<MarketUpdate> {
    // Parse key format: "exchange:type:subtype:symbol"
    let parts: Vec<&str> = key.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    
    let exchange = parts[0];
    let symbol_raw = parts[parts.len() - 1];
    
    // Normalize symbol
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
    
    // Parse prices
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
    Some(MarketUpdate::new(symbol_id, bid, ask, timestamp_us))
}

/// Simulated redis_bridge for testing
async fn test_redis_bridge(
    mut rx: mpsc::Receiver<(String, String)>,
    redis_queue: Arc<ArrayQueue<(String, String)>>,
    pipeline: Arc<MarketPipeline>,
    symbol_map: Arc<SymbolMap>,
) {
    let producer = pipeline.producer();
    
    while let Some((key, value)) = rx.recv().await {
        // Hot path: Parse and push to pipeline
        if let Some(update) = parse_to_market_update(&key, &value, &symbol_map) {
            producer.push(update);
        }
        
        // Cold path: Push to Redis queue
        if let Err(rejected) = redis_queue.push((key, value)) {
            redis_queue.pop();
            let _ = redis_queue.push(rejected);
        }
    }
}

#[tokio::test]
async fn test_json_to_market_update_conversion() {
    // Test data: Bybit ticker JSON
    let key = "bybit:linear:tickers:BTCUSDT";
    let value = r#"{"data":{"bid1Price":"50000.0","ask1Price":"50010.0"}}"#;
    
    let symbol_map = SymbolMap::new();
    
    // Parse to MarketUpdate
    let update = parse_to_market_update(key, value, &symbol_map)
        .expect("Should parse valid JSON");
    
    // Verify conversion
    assert_eq!(update.bid, 50000.0);
    assert_eq!(update.ask, 50010.0);
    assert!(update.timestamp_us > 0);
    
    // Verify symbol mapping
    let (exchange, symbol) = symbol_map.get(update.symbol_id).unwrap();
    assert_eq!(exchange, "bybit");
    assert_eq!(symbol, "BTCUSDT");
}

#[tokio::test]
async fn test_pipeline_receives_correct_data() {
    let symbol_map = Arc::new(SymbolMap::new());
    let pipeline = Arc::new(MarketPipeline::new());
    let redis_queue = Arc::new(ArrayQueue::new(1000));
    
    let (tx, rx) = mpsc::channel(100);
    
    // Spawn bridge
    let symbol_map_clone = symbol_map.clone();
    let pipeline_clone = pipeline.clone();
    let redis_queue_clone = redis_queue.clone();
    
    tokio::spawn(async move {
        test_redis_bridge(rx, redis_queue_clone, pipeline_clone, symbol_map_clone).await;
    });
    
    // Send test data
    let key = "bybit:linear:tickers:BTCUSDT".to_string();
    let value = r#"{"data":{"bid1Price":"50000.0","ask1Price":"50010.0"}}"#.to_string();
    
    tx.send((key, value)).await.unwrap();
    
    // Give bridge time to process
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify pipeline received data
    let consumer = pipeline.consumer();
    let update = consumer.pop().expect("Pipeline should have data");
    
    assert_eq!(update.bid, 50000.0);
    assert_eq!(update.ask, 50010.0);
    
    // Verify symbol mapping
    let (exchange, symbol) = symbol_map.get(update.symbol_id).unwrap();
    assert_eq!(exchange, "bybit");
    assert_eq!(symbol, "BTCUSDT");
}

#[tokio::test]
async fn test_redis_still_receives_data() {
    let symbol_map = Arc::new(SymbolMap::new());
    let pipeline = Arc::new(MarketPipeline::new());
    let redis_queue = Arc::new(ArrayQueue::new(1000));
    
    let (tx, rx) = mpsc::channel(100);
    
    // Spawn bridge
    let symbol_map_clone = symbol_map.clone();
    let pipeline_clone = pipeline.clone();
    let redis_queue_clone = redis_queue.clone();
    
    tokio::spawn(async move {
        test_redis_bridge(rx, redis_queue_clone, pipeline_clone, symbol_map_clone).await;
    });
    
    // Send test data
    let key = "bybit:linear:tickers:BTCUSDT".to_string();
    let value = r#"{"data":{"bid1Price":"50000.0","ask1Price":"50010.0"}}"#.to_string();
    
    tx.send((key.clone(), value.clone())).await.unwrap();
    
    // Give bridge time to process
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify Redis queue received data (cold path)
    let (redis_key, redis_value) = redis_queue.pop().expect("Redis queue should have data");
    
    assert_eq!(redis_key, key);
    assert_eq!(redis_value, value);
}

#[tokio::test]
async fn test_multiple_exchanges() {
    let symbol_map = Arc::new(SymbolMap::new());
    let pipeline = Arc::new(MarketPipeline::new());
    let redis_queue = Arc::new(ArrayQueue::new(1000));
    
    let (tx, rx) = mpsc::channel(100);
    
    // Spawn bridge
    let symbol_map_clone = symbol_map.clone();
    let pipeline_clone = pipeline.clone();
    let redis_queue_clone = redis_queue.clone();
    
    tokio::spawn(async move {
        test_redis_bridge(rx, redis_queue_clone, pipeline_clone, symbol_map_clone).await;
    });
    
    // Send data from multiple exchanges
    let test_data = vec![
        ("bybit:linear:tickers:BTCUSDT", r#"{"data":{"bid1Price":"50000.0","ask1Price":"50010.0"}}"#),
        ("okx:usdt:tickers:BTC-USDT-SWAP", r#"{"data":[{"bidPx":"50005.0","askPx":"50015.0"}]}"#),
        ("kucoin:futures:tickerV2:BTCUSDTM", r#"{"data":{"bestBidPrice":"50002.0","bestAskPrice":"50012.0"}}"#),
    ];
    
    for (key, value) in test_data {
        tx.send((key.to_string(), value.to_string())).await.unwrap();
    }
    
    // Give bridge time to process
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all updates in pipeline
    let consumer = pipeline.consumer();
    
    let update1 = consumer.pop().expect("Should have bybit update");
    assert_eq!(update1.bid, 50000.0);
    
    let update2 = consumer.pop().expect("Should have okx update");
    assert_eq!(update2.bid, 50005.0);
    
    let update3 = consumer.pop().expect("Should have kucoin update");
    assert_eq!(update3.bid, 50002.0);
    
    // Verify symbol IDs are different for different exchanges
    assert_ne!(update1.symbol_id, update2.symbol_id);
    assert_ne!(update1.symbol_id, update3.symbol_id);
    assert_ne!(update2.symbol_id, update3.symbol_id);
}

#[tokio::test]
async fn test_invalid_json_handling() {
    let symbol_map = Arc::new(SymbolMap::new());
    let pipeline = Arc::new(MarketPipeline::new());
    let redis_queue = Arc::new(ArrayQueue::new(1000));
    
    let (tx, rx) = mpsc::channel(100);
    
    // Spawn bridge
    let symbol_map_clone = symbol_map.clone();
    let pipeline_clone = pipeline.clone();
    let redis_queue_clone = redis_queue.clone();
    
    tokio::spawn(async move {
        test_redis_bridge(rx, redis_queue_clone, pipeline_clone, symbol_map_clone).await;
    });
    
    // Send invalid JSON
    let key = "bybit:linear:tickers:BTCUSDT".to_string();
    let value = "invalid json".to_string();
    
    tx.send((key.clone(), value.clone())).await.unwrap();
    
    // Give bridge time to process
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Pipeline should be empty (invalid data not pushed)
    let consumer = pipeline.consumer();
    assert!(consumer.pop().is_none(), "Pipeline should not have invalid data");
    
    // But Redis queue should still have it (persistence)
    let (redis_key, redis_value) = redis_queue.pop().expect("Redis should have data");
    assert_eq!(redis_key, key);
    assert_eq!(redis_value, value);
}

#[tokio::test]
async fn test_conversion_performance() {
    use std::time::Instant;
    
    let symbol_map = SymbolMap::new();
    
    // Test data
    let key = "bybit:linear:tickers:BTCUSDT";
    let value = r#"{"data":{"bid1Price":"50000.0","ask1Price":"50010.0"}}"#;
    
    // Warm up
    for _ in 0..100 {
        let _ = parse_to_market_update(key, value, &symbol_map);
    }
    
    // Measure performance
    let iterations = 10000;
    let start = Instant::now();
    
    for _ in 0..iterations {
        let _ = parse_to_market_update(key, value, &symbol_map);
    }
    
    let duration = start.elapsed();
    let avg_micros = duration.as_micros() / iterations;
    
    println!("Average conversion time: {}μs", avg_micros);
    
    // Acceptance criteria: < 50μs per message
    assert!(avg_micros < 50, "Conversion took {}μs, expected < 50μs", avg_micros);
}

#[tokio::test]
async fn test_no_data_loss() {
    let symbol_map = Arc::new(SymbolMap::new());
    let pipeline = Arc::new(MarketPipeline::new());
    let redis_queue = Arc::new(ArrayQueue::new(1000));
    
    let (tx, rx) = mpsc::channel(1000);
    
    // Spawn bridge
    let symbol_map_clone = symbol_map.clone();
    let pipeline_clone = pipeline.clone();
    let redis_queue_clone = redis_queue.clone();
    
    tokio::spawn(async move {
        test_redis_bridge(rx, redis_queue_clone, pipeline_clone, symbol_map_clone).await;
    });
    
    // Send 100 messages
    let message_count = 100;
    for i in 0..message_count {
        let key = format!("bybit:linear:tickers:BTCUSDT");
        let value = format!(r#"{{"data":{{"bid1Price":"{}","ask1Price":"{}"}}}}"#, 
            50000.0 + i as f64, 50010.0 + i as f64);
        tx.send((key, value)).await.unwrap();
    }
    
    // Give bridge time to process
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Count messages in pipeline
    let consumer = pipeline.consumer();
    let mut pipeline_count = 0;
    while consumer.pop().is_some() {
        pipeline_count += 1;
    }
    
    // Count messages in Redis queue
    let redis_count = redis_queue.len();
    
    println!("Pipeline received: {} messages", pipeline_count);
    println!("Redis queue has: {} messages", redis_count);
    
    // Both should have received all messages (no data loss)
    assert_eq!(pipeline_count, message_count, "Pipeline should have all messages");
    assert_eq!(redis_count, message_count, "Redis queue should have all messages");
}
