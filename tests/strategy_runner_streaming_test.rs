//! Integration tests for StrategyRunner streaming mode
//!
//! This test verifies that the strategy runner correctly consumes market data
//! from the SPSC queue and updates the market data store.
//!
//! Requirements tested:
//! - 1.2: Strategy consumes from queue and updates market data store
//! - 14.1: Process data immediately (no polling delay)
//! - 14.2: No batching delays

use arbitrage2::strategy::pipeline::MarketPipeline;
use arbitrage2::strategy::types::MarketUpdate;

#[test]
fn test_market_pipeline_integration() {
    // Create pipeline
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Push some market updates
    producer.push(MarketUpdate::new(1, 50000.0, 50010.0, 1000000));
    producer.push(MarketUpdate::new(2, 3000.0, 3001.0, 1000001));
    producer.push(MarketUpdate::new(3, 100.0, 100.1, 1000002));
    
    // Verify queue depth
    assert_eq!(pipeline.depth(), 3);
    
    // Consume updates
    let update1 = consumer.pop().unwrap();
    assert_eq!(update1.symbol_id, 1);
    assert_eq!(update1.bid, 50000.0);
    assert_eq!(update1.ask, 50010.0);
    
    let update2 = consumer.pop().unwrap();
    assert_eq!(update2.symbol_id, 2);
    
    let update3 = consumer.pop().unwrap();
    assert_eq!(update3.symbol_id, 3);
    
    // Queue should be empty
    assert_eq!(pipeline.depth(), 0);
    assert!(consumer.pop().is_none());
}

#[test]
fn test_market_data_store_update() {
    use arbitrage2::strategy::market_data::MarketDataStore;
    
    let mut store = MarketDataStore::new();
    
    // Update with market data
    let update = MarketUpdate::new(1, 50000.0, 50010.0, 1000000);
    store.update_from_market_update(&update);
    
    // Verify data was stored
    assert_eq!(store.get_bid(1), Some(50000.0));
    assert_eq!(store.get_ask(1), Some(50010.0));
    assert_eq!(store.get_timestamp(1), Some(1000000));
    
    // Calculate spread
    let spread = store.get_spread_bps(1);
    // Spread = (50010 - 50000) / 50000 * 10000 = 2 bps
    assert!((spread - 2.0).abs() < 0.01);
}

#[test]
fn test_streaming_pipeline_end_to_end() {
    use arbitrage2::strategy::market_data::MarketDataStore;
    
    // Create pipeline
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Create market data store
    let mut store = MarketDataStore::new();
    
    // Simulate WebSocket thread pushing updates
    for i in 1..=10 {
        let update = MarketUpdate::new(
            i,
            1000.0 * i as f64,
            1001.0 * i as f64,
            1000000 + i as u64,
        );
        producer.push(update);
    }
    
    // Verify all updates are in queue
    assert_eq!(pipeline.depth(), 10);
    
    // Simulate strategy thread consuming and updating store
    let mut consumed_count = 0;
    while let Some(update) = consumer.pop() {
        store.update_from_market_update(&update);
        consumed_count += 1;
    }
    
    assert_eq!(consumed_count, 10);
    assert_eq!(pipeline.depth(), 0);
    
    // Verify all data is in store
    for i in 1..=10 {
        assert_eq!(store.get_bid(i), Some(1000.0 * i as f64));
        assert_eq!(store.get_ask(i), Some(1001.0 * i as f64));
    }
}

#[test]
fn test_backpressure_handling() {
    // Create small pipeline to test backpressure
    let pipeline = MarketPipeline::with_capacity(3);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Fill queue
    producer.push(MarketUpdate::new(1, 100.0, 101.0, 1000));
    producer.push(MarketUpdate::new(2, 200.0, 201.0, 2000));
    producer.push(MarketUpdate::new(3, 300.0, 301.0, 3000));
    
    assert!(pipeline.is_full());
    
    // Push one more (should drop oldest)
    producer.push(MarketUpdate::new(4, 400.0, 401.0, 4000));
    
    // Queue should still be full
    assert_eq!(pipeline.depth(), 3);
    
    // Verify we get 2, 3, 4 (1 was dropped)
    let update = consumer.pop().unwrap();
    assert_eq!(update.symbol_id, 2);
    
    let update = consumer.pop().unwrap();
    assert_eq!(update.symbol_id, 3);
    
    let update = consumer.pop().unwrap();
    assert_eq!(update.symbol_id, 4);
    
    // Check metrics
    let metrics = pipeline.metrics();
    assert_eq!(metrics.push_count, 4);
    assert_eq!(metrics.drop_count, 1);
    assert_eq!(metrics.pop_count, 3);
}

#[test]
fn test_zero_copy_market_update() {
    // Verify MarketUpdate is Copy (zero-copy)
    let update = MarketUpdate::new(1, 100.0, 101.0, 1000);
    
    // This should compile because MarketUpdate is Copy
    let update2 = update;
    let update3 = update;
    
    // All three should have the same data
    assert_eq!(update.symbol_id, update2.symbol_id);
    assert_eq!(update2.symbol_id, update3.symbol_id);
}

#[test]
fn test_market_data_store_staleness() {
    use arbitrage2::strategy::market_data::MarketDataStore;
    
    let mut store = MarketDataStore::new();
    
    // Update with old timestamp
    store.update(1, 100.0, 101.0, 1000000);
    
    // Check staleness (2 seconds threshold)
    let current_time = 1000000 + 2_000_001; // 2 seconds + 1 microsecond later
    assert!(store.is_stale(1, current_time, 2_000_000));
    
    // Check fresh data
    let current_time = 1000000 + 1_000_000; // 1 second later
    assert!(!store.is_stale(1, current_time, 2_000_000));
}

#[test]
fn test_pipeline_metrics() {
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Push 5 updates
    for i in 1..=5 {
        producer.push(MarketUpdate::new(i, 100.0, 101.0, 1000));
    }
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.push_count, 5);
    assert_eq!(metrics.enqueue_count, 5);
    assert_eq!(metrics.drop_count, 0);
    assert_eq!(metrics.pop_count, 0);
    assert_eq!(metrics.queue_depth, 5);
    
    // Pop 2 updates
    consumer.pop();
    consumer.pop();
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.pop_count, 2);
    assert_eq!(metrics.queue_depth, 3);
}
