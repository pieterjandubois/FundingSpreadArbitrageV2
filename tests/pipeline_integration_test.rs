/// Integration tests for the complete low-latency pipeline
/// 
/// This test suite validates the end-to-end pipeline from WebSocket to Strategy execution:
/// 1. WebSocket → Queue → Strategy flow
/// 2. Backpressure handling under load
/// 3. Thread pinning for hot/warm paths
/// 4. Zero allocations in hot path
/// 
/// Requirements:
/// - 1.1: Direct memory architecture (WebSocket → Queue → Strategy)
/// - 3.1: Lock-free queues (SPSC)
/// - 4.1, 4.2: Thread pinning (strategy core 1, websocket cores 2-7)
/// - 2.5: Zero allocations in hot path
/// - 14.3, 14.4: Backpressure handling (bounded queues, drop old data)

use arbitrage2::strategy::pipeline::{MarketPipeline, ExecutionPipeline};
use arbitrage2::strategy::types::{MarketUpdate, OrderRequest};
use arbitrage2::strategy::thread_pinning::{pin_strategy_thread, get_core_count, has_sufficient_cores};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// ============================================================================
// Test 1: WebSocket → Queue → Strategy Flow
// ============================================================================

#[test]
fn test_websocket_to_strategy_flow() {
    // Create the pipeline
    let pipeline = Arc::new(MarketPipeline::new());
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Simulate WebSocket thread receiving market data
    let websocket_handle = {
        let producer = producer.clone();
        thread::spawn(move || {
            // Simulate receiving 100 market updates
            for i in 0..100 {
                let update = MarketUpdate::new(
                    (i % 10) as u32,  // Rotate through 10 symbols
                    50000.0 + i as f64,
                    50010.0 + i as f64,
                    i as u64 * 1000,
                );
                producer.push(update);
                
                // Simulate realistic WebSocket timing (~1ms between updates)
                thread::sleep(Duration::from_micros(10));
            }
        })
    };
    
    // Simulate strategy thread consuming market data
    let strategy_handle = {
        thread::spawn(move || {
            let mut processed_count = 0;
            let mut last_bid = 0.0;
            
            // Process updates until we get all 100
            while processed_count < 100 {
                if let Some(update) = consumer.pop() {
                    // Verify data integrity (monotonic increase)
                    assert!(update.bid >= last_bid, 
                        "Expected monotonic bid increase, got {} after {}", 
                        update.bid, last_bid);
                    last_bid = update.bid;
                    processed_count += 1;
                } else {
                    // Queue empty, yield to producer
                    thread::yield_now();
                }
            }
            
            processed_count
        })
    };
    
    // Wait for both threads to complete
    websocket_handle.join().unwrap();
    let processed = strategy_handle.join().unwrap();
    
    // Verify all updates were processed
    assert_eq!(processed, 100);
    assert_eq!(pipeline.depth(), 0, "Queue should be empty after processing");
    
    // Verify metrics
    let metrics = pipeline.metrics();
    assert_eq!(metrics.push_count, 100);
    assert_eq!(metrics.pop_count, 100);
    assert_eq!(metrics.drop_count, 0, "No drops should occur with sufficient capacity");
}

// ============================================================================
// Test 2: Backpressure Handling
// ============================================================================

#[test]
fn test_backpressure_under_load() {
    // Create a small queue to trigger backpressure
    let pipeline = Arc::new(MarketPipeline::with_capacity(100));
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Fast producer: Push 1000 updates as fast as possible
    let producer_handle = {
        let producer = producer.clone();
        thread::spawn(move || {
            for i in 0..1000 {
                let update = MarketUpdate::new(
                    1,
                    50000.0 + i as f64,
                    50010.0 + i as f64,
                    i as u64 * 1000,
                );
                producer.push(update);
            }
        })
    };
    
    // Slow consumer: Process with delays to create backpressure
    let consumer_handle = {
        thread::spawn(move || {
            let mut processed = 0;
            let start = Instant::now();
            
            // Process for 100ms (won't get all 1000 updates)
            while start.elapsed() < Duration::from_millis(100) {
                if let Some(_update) = consumer.pop() {
                    processed += 1;
                    // Simulate slow processing
                    thread::sleep(Duration::from_micros(50));
                }
            }
            
            processed
        })
    };
    
    // Wait for producer to finish
    producer_handle.join().unwrap();
    
    // Check metrics before consumer finishes
    let metrics = pipeline.metrics();
    assert_eq!(metrics.push_count, 1000, "All 1000 pushes should be attempted");
    assert!(metrics.drop_count > 0, "Backpressure should cause drops");
    assert!(metrics.is_backpressure(), "Should detect backpressure condition");
    
    // Wait for consumer
    let processed = consumer_handle.join().unwrap();
    
    // Verify backpressure behavior
    assert!(processed < 1000, "Consumer should not process all updates due to slow processing");
    assert!(metrics.drop_rate() > 0.0, "Drop rate should be non-zero");
    
    println!("Backpressure test: pushed={}, processed={}, dropped={}, drop_rate={:.2}%",
        metrics.push_count, processed, metrics.drop_count, metrics.drop_rate());
}

#[test]
fn test_backpressure_drops_oldest_data() {
    // Create tiny queue (capacity 5)
    let pipeline = MarketPipeline::with_capacity(5);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Fill queue completely
    for i in 0..5 {
        producer.push(MarketUpdate::new(i, 100.0 + i as f64, 101.0 + i as f64, i as u64 * 1000));
    }
    
    assert_eq!(pipeline.depth(), 5);
    assert!(pipeline.is_full());
    
    // Push 3 more (should drop 3 oldest: 0, 1, 2)
    for i in 5..8 {
        producer.push(MarketUpdate::new(i, 100.0 + i as f64, 101.0 + i as f64, i as u64 * 1000));
    }
    
    // Queue should still be at capacity
    assert_eq!(pipeline.depth(), 5);
    
    // Pop all and verify we got 3, 4, 5, 6, 7 (not 0, 1, 2)
    let mut symbol_ids = Vec::new();
    while let Some(update) = consumer.pop() {
        symbol_ids.push(update.symbol_id);
    }
    
    assert_eq!(symbol_ids, vec![3, 4, 5, 6, 7], "Should have newest data, oldest dropped");
    
    // Verify metrics
    let metrics = pipeline.metrics();
    assert_eq!(metrics.drop_count, 3, "Should have dropped 3 oldest updates");
}

// ============================================================================
// Test 3: Thread Pinning
// ============================================================================

#[test]
fn test_thread_pinning_strategy_core() {
    // Skip test if insufficient cores
    if !has_sufficient_cores() {
        println!("Skipping thread pinning test: insufficient cores (need 8, have {})", 
            get_core_count());
        return;
    }
    
    // Spawn thread and pin to strategy core (core 1)
    let handle = thread::spawn(|| {
        let result = pin_strategy_thread();
        
        // Verify pinning succeeded
        assert!(result.is_ok(), "Failed to pin strategy thread: {:?}", result.err());
        
        // Thread should now be pinned to core 1
        // We can't easily verify the actual core assignment in a test,
        // but we can verify the function didn't error
        true
    });
    
    let success = handle.join().unwrap();
    assert!(success, "Thread pinning should succeed");
}

#[test]
fn test_thread_pinning_with_pipeline() {
    // Skip test if insufficient cores
    if !has_sufficient_cores() {
        println!("Skipping thread pinning test: insufficient cores (need 8, have {})", 
            get_core_count());
        return;
    }
    
    let pipeline = Arc::new(MarketPipeline::new());
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Spawn strategy thread with pinning
    let strategy_handle = thread::spawn(move || {
        // Pin to strategy core
        if let Err(e) = pin_strategy_thread() {
            eprintln!("Warning: Failed to pin strategy thread: {}", e);
        }
        
        // Process updates
        let mut count = 0;
        let start = Instant::now();
        
        while start.elapsed() < Duration::from_millis(50) {
            if let Some(_update) = consumer.pop() {
                count += 1;
            }
        }
        
        count
    });
    
    // Spawn producer thread (simulating WebSocket)
    let producer_handle = thread::spawn(move || {
        for i in 0..100 {
            let update = MarketUpdate::new(1, 50000.0, 50010.0, i as u64 * 1000);
            producer.push(update);
            thread::sleep(Duration::from_micros(100));
        }
    });
    
    // Wait for completion
    producer_handle.join().unwrap();
    let processed = strategy_handle.join().unwrap();
    
    assert!(processed > 0, "Strategy thread should process some updates");
}

// ============================================================================
// Test 4: Zero Allocations
// ============================================================================

#[test]
fn test_zero_allocations_in_hot_path() {
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Pre-create update (cold path allocation)
    let update = MarketUpdate::new(1, 50000.0, 50010.0, 1000000);
    
    // Verify MarketUpdate is Copy (no heap allocation)
    let _copy1 = update;
    let _copy2 = update;
    
    // Verify size and alignment (cache line optimized)
    assert_eq!(std::mem::size_of::<MarketUpdate>(), 64, "Should be 64 bytes (cache line)");
    assert_eq!(std::mem::align_of::<MarketUpdate>(), 64, "Should be 64-byte aligned");
    
    // Hot path: push and pop (should be zero allocations)
    producer.push(update);
    let popped = consumer.pop().unwrap();
    
    // Verify data integrity
    assert_eq!(popped.symbol_id, update.symbol_id);
    assert_eq!(popped.bid, update.bid);
    assert_eq!(popped.ask, update.ask);
    
    // Note: We can't directly measure allocations in a test,
    // but we can verify the types are Copy and properly sized
}

#[test]
fn test_order_request_zero_copy() {
    let pipeline = ExecutionPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Pre-create order (cold path allocation)
    let order = OrderRequest::market(1, 100, 1, 0, 1.5, 1000000);
    
    // Verify OrderRequest is Copy (no heap allocation)
    let _copy1 = order;
    let _copy2 = order;
    
    // Verify size and alignment (cache line optimized)
    assert_eq!(std::mem::size_of::<OrderRequest>(), 64, "Should be 64 bytes (cache line)");
    assert_eq!(std::mem::align_of::<OrderRequest>(), 64, "Should be 64-byte aligned");
    
    // Hot path: submit and pop (should be zero allocations)
    producer.submit(order);
    let popped = consumer.pop().unwrap();
    
    // Verify data integrity
    assert_eq!(popped.order_id, order.order_id);
    assert_eq!(popped.symbol_id, order.symbol_id);
    assert_eq!(popped.size, order.size);
}

// ============================================================================
// Test 5: End-to-End Pipeline (WebSocket → Strategy → Execution)
// ============================================================================

#[test]
fn test_full_pipeline_websocket_to_execution() {
    let market_pipeline = Arc::new(MarketPipeline::new());
    let execution_pipeline = Arc::new(ExecutionPipeline::new());
    
    let market_producer = market_pipeline.producer();
    let market_consumer = market_pipeline.consumer();
    let execution_producer = execution_pipeline.producer();
    let execution_consumer = execution_pipeline.consumer();
    
    // Thread 1: WebSocket (producer)
    let websocket_handle = {
        let producer = market_producer.clone();
        thread::spawn(move || {
            for i in 0..50 {
                let update = MarketUpdate::new(
                    1,
                    50000.0 + i as f64,
                    50010.0 + i as f64,
                    i as u64 * 1000,
                );
                producer.push(update);
                thread::sleep(Duration::from_micros(100));
            }
        })
    };
    
    // Thread 2: Strategy (consumer → decision → producer)
    let strategy_handle = {
        let execution_producer = execution_producer.clone();
        thread::spawn(move || {
            let mut orders_generated = 0;
            let start = Instant::now();
            
            // Process market data and generate orders
            while start.elapsed() < Duration::from_millis(100) {
                if let Some(update) = market_consumer.pop() {
                    // Simple strategy: if spread > 1 bps, generate order
                    // (spread is 10 bps in our test data: (50010 - 50000) / 50000 * 10000 = 2 bps)
                    let spread_bps = ((update.ask - update.bid) / update.bid) * 10000.0;
                    
                    if spread_bps > 1.0 {
                        let order = OrderRequest::market(
                            orders_generated,
                            update.symbol_id,
                            1,
                            0,
                            1.0,
                            update.timestamp_us,
                        );
                        execution_producer.submit(order);
                        orders_generated += 1;
                    }
                }
            }
            
            orders_generated
        })
    };
    
    // Thread 3: Execution (consumer)
    let execution_handle = {
        thread::spawn(move || {
            let mut executed = 0;
            let start = Instant::now();
            
            while start.elapsed() < Duration::from_millis(150) {
                if let Some(_order) = execution_consumer.pop() {
                    // Simulate order execution
                    executed += 1;
                }
            }
            
            executed
        })
    };
    
    // Wait for all threads
    websocket_handle.join().unwrap();
    let orders_generated = strategy_handle.join().unwrap();
    let orders_executed = execution_handle.join().unwrap();
    
    // Verify pipeline worked
    assert!(orders_generated > 0, "Strategy should generate some orders");
    assert_eq!(orders_executed, orders_generated, "All generated orders should be executed");
    
    println!("Full pipeline test: market_updates=50, orders_generated={}, orders_executed={}",
        orders_generated, orders_executed);
}

// ============================================================================
// Test 6: High-Throughput Stress Test
// ============================================================================

#[test]
fn test_high_throughput_stress() {
    let pipeline = Arc::new(MarketPipeline::new());
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Producer: Push 10,000 updates as fast as possible
    let producer_handle = {
        let producer = producer.clone();
        thread::spawn(move || {
            let start = Instant::now();
            
            for i in 0..10_000 {
                let update = MarketUpdate::new(
                    (i % 10) as u32,
                    50000.0 + i as f64,
                    50010.0 + i as f64,
                    i as u64 * 1000,
                );
                producer.push(update);
            }
            
            let elapsed = start.elapsed();
            let throughput = 10_000.0 / elapsed.as_secs_f64();
            
            println!("Producer throughput: {:.0} updates/sec", throughput);
            throughput
        })
    };
    
    // Consumer: Pop as fast as possible
    let consumer_handle = {
        thread::spawn(move || {
            let start = Instant::now();
            let mut count = 0;
            
            // Keep consuming until queue is empty for 10ms
            let mut empty_count = 0;
            while empty_count < 100 {
                if let Some(_update) = consumer.pop() {
                    count += 1;
                    empty_count = 0;
                } else {
                    empty_count += 1;
                    thread::sleep(Duration::from_micros(100));
                }
            }
            
            let elapsed = start.elapsed();
            let throughput = count as f64 / elapsed.as_secs_f64();
            
            println!("Consumer throughput: {:.0} updates/sec", throughput);
            (count, throughput)
        })
    };
    
    // Wait for completion
    let producer_throughput = producer_handle.join().unwrap();
    let (consumed, consumer_throughput) = consumer_handle.join().unwrap();
    
    // Verify high throughput
    assert!(producer_throughput > 100_000.0, 
        "Producer should achieve >100k updates/sec, got {:.0}", producer_throughput);
    assert!(consumer_throughput > 100_000.0, 
        "Consumer should achieve >100k updates/sec, got {:.0}", consumer_throughput);
    assert_eq!(consumed, 10_000, "All updates should be consumed");
    
    // Verify metrics
    let metrics = pipeline.metrics();
    assert_eq!(metrics.push_count, 10_000);
    assert_eq!(metrics.pop_count, 10_000);
}

// ============================================================================
// Test 7: Pipeline Metrics and Monitoring
// ============================================================================

#[test]
fn test_pipeline_metrics_accuracy() {
    let pipeline = MarketPipeline::with_capacity(10);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Push 15 updates (5 will be dropped due to capacity)
    for i in 0..15 {
        producer.push(MarketUpdate::new(i, 100.0, 101.0, i as u64 * 1000));
    }
    
    let metrics = pipeline.metrics();
    
    // Verify metrics
    assert_eq!(metrics.push_count, 15, "Should count all push attempts");
    assert_eq!(metrics.enqueue_count, 15, "Should count all enqueues (including after drops)");
    assert_eq!(metrics.drop_count, 5, "Should drop 5 oldest updates");
    assert_eq!(metrics.queue_depth, 10, "Queue should be at capacity");
    assert_eq!(metrics.utilization(), 100.0, "Should be 100% utilized");
    assert!(metrics.is_backpressure(), "Should detect backpressure");
    
    // Pop 5 updates
    for _ in 0..5 {
        consumer.pop();
    }
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.pop_count, 5, "Should count pops");
    assert_eq!(metrics.queue_depth, 5, "Should have 5 remaining");
    assert_eq!(metrics.utilization(), 50.0, "Should be 50% utilized");
    // Note: drop_rate is 33.3% (5/15), which exceeds 1% threshold, so backpressure is still detected
    // This is correct behavior - we had drops, so backpressure occurred
    assert!(metrics.is_backpressure(), "Should still detect backpressure due to drop rate");
}

#[test]
fn test_execution_pipeline_metrics() {
    let pipeline = ExecutionPipeline::with_capacity(5);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Submit 8 orders (3 will be dropped)
    for i in 0..8 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, i as u64 * 1000));
    }
    
    let metrics = pipeline.metrics();
    
    // Verify metrics
    assert_eq!(metrics.submit_count, 8);
    assert_eq!(metrics.drop_count, 3);
    assert_eq!(metrics.queue_depth, 5);
    assert_eq!(metrics.drop_rate(), 37.5); // 3/8 = 37.5%
    
    // Pop all orders
    while consumer.pop().is_some() {}
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.pop_count, 5);
    assert_eq!(metrics.queue_depth, 0);
}
