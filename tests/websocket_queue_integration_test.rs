/// Integration test for WebSocket → Queue pipeline
/// 
/// This test verifies that:
/// 1. MarketProducer can push MarketUpdate to the queue
/// 2. MarketConsumer can pop MarketUpdate from the queue
/// 3. The pipeline handles backpressure correctly
/// 
/// Requirements: 1.1 (Direct memory architecture), 3.1 (Lock-free queues), 8.1 (Zero-copy parsing)

use arbitrage2::strategy::pipeline::MarketPipeline;
use arbitrage2::strategy::types::MarketUpdate;

#[test]
fn test_websocket_to_queue_flow() {
    // Create pipeline
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Simulate WebSocket receiving market data
    let update = MarketUpdate::new(
        1,          // symbol_id (BTCUSDT)
        50000.0,    // bid
        50010.0,    // ask
        1000000,    // timestamp_us
    );
    
    // Push to queue (hot path)
    producer.push(update);
    
    // Verify queue has data
    assert_eq!(pipeline.depth(), 1);
    
    // Pop from queue (strategy thread)
    let popped = consumer.pop().unwrap();
    
    // Verify data integrity
    assert_eq!(popped.symbol_id, 1);
    assert_eq!(popped.bid, 50000.0);
    assert_eq!(popped.ask, 50010.0);
    assert_eq!(popped.timestamp_us, 1000000);
    
    // Verify queue is empty
    assert_eq!(pipeline.depth(), 0);
}

#[test]
fn test_high_throughput_simulation() {
    // Simulate high-frequency market data updates
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Push 1000 updates (simulating 1000 updates/second)
    for i in 0..1000 {
        let update = MarketUpdate::new(
            (i % 10) as u32,  // Rotate through 10 symbols
            50000.0 + i as f64,
            50010.0 + i as f64,
            i as u64 * 1000,
        );
        producer.push(update);
    }
    
    // Verify all updates are in queue
    assert_eq!(pipeline.depth(), 1000);
    
    // Pop all updates
    let mut count = 0;
    while consumer.pop().is_some() {
        count += 1;
    }
    
    assert_eq!(count, 1000);
    assert_eq!(pipeline.depth(), 0);
}

#[test]
fn test_backpressure_handling() {
    // Create small queue to test backpressure
    let pipeline = MarketPipeline::with_capacity(10);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Fill queue beyond capacity
    for i in 0..20 {
        let update = MarketUpdate::new(
            i as u32,
            50000.0,
            50010.0,
            i as u64 * 1000,
        );
        producer.push(update);
    }
    
    // Queue should be at capacity (oldest dropped)
    assert_eq!(pipeline.depth(), 10);
    
    // Verify we get the NEWEST data (10-19, not 0-9)
    let first = consumer.pop().unwrap();
    assert_eq!(first.symbol_id, 10); // First item should be symbol_id 10 (oldest was dropped)
    
    // Verify metrics show drops
    let metrics = pipeline.metrics();
    assert_eq!(metrics.push_count, 20);
    assert_eq!(metrics.drop_count, 10);
    assert_eq!(metrics.enqueue_count, 20);
}

#[test]
fn test_zero_copy_performance() {
    // Verify MarketUpdate is Copy (zero-copy)
    let update = MarketUpdate::new(1, 50000.0, 50010.0, 1000000);
    
    // This should compile (Copy trait)
    let _copy1 = update;
    let _copy2 = update;
    
    // Verify size is 64 bytes (cache line aligned)
    assert_eq!(std::mem::size_of::<MarketUpdate>(), 64);
    
    // Verify alignment is 64 bytes
    assert_eq!(std::mem::align_of::<MarketUpdate>(), 64);
}

#[test]
fn test_spread_calculation() {
    let update = MarketUpdate::new(1, 100.0, 100.1, 1000000);
    
    // Spread = (100.1 - 100.0) / 100.0 * 10000 = 10 bps
    let spread = update.spread_bps();
    assert!((spread - 10.0).abs() < 0.01);
}

#[test]
fn test_mid_price_calculation() {
    let update = MarketUpdate::new(1, 100.0, 100.2, 1000000);
    
    // Mid = (100.0 + 100.2) / 2 = 100.1
    let mid = update.mid_price();
    assert!((mid - 100.1).abs() < 0.01);
}

#[test]
fn test_concurrent_producer_consumer() {
    use std::sync::Arc;
    use std::thread;
    
    let pipeline = Arc::new(MarketPipeline::new());
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Spawn producer thread
    let producer_handle = thread::spawn(move || {
        for i in 0..1000 {
            let update = MarketUpdate::new(
                1,
                50000.0 + i as f64,
                50010.0 + i as f64,
                i as u64 * 1000,
            );
            producer.push(update);
        }
    });
    
    // Spawn consumer thread
    let consumer_handle = thread::spawn(move || {
        let mut count = 0;
        let mut last_bid = 0.0;
        
        // Keep consuming until we get all 1000 updates
        while count < 1000 {
            if let Some(update) = consumer.pop() {
                // Verify monotonic increase (no reordering)
                assert!(update.bid >= last_bid);
                last_bid = update.bid;
                count += 1;
            } else {
                // Queue empty, yield to producer
                thread::yield_now();
            }
        }
        
        count
    });
    
    // Wait for both threads
    producer_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    assert_eq!(consumed, 1000);
}
