/// Stress Tests for Low-Latency Pipeline
///
/// This test suite validates system behavior under extreme load conditions:
/// 1. 10,000 updates/second throughput
/// 2. Queue overflow handling (backpressure)
/// 3. Thread starvation scenarios
/// 4. System stability under sustained load
///
/// Requirement 14.4: WHEN the system is overloaded, THE system SHALL drop old data, not block

use arbitrage2::strategy::pipeline::{MarketPipeline, ExecutionPipeline};
use arbitrage2::strategy::types::{MarketUpdate, OrderRequest};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// ============================================================================
// Test 1: 10,000 Updates/Second Throughput
// ============================================================================

#[test]
fn test_10k_updates_per_second_throughput() {
    println!("\n=== Test 1: 10,000 Updates/Second Throughput ===");
    
    let pipeline = Arc::new(MarketPipeline::new());
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    let running = Arc::new(AtomicBool::new(true));
    
    // Producer: Send 10,000 updates/second for 5 seconds
    let producer_handle = {
        thread::spawn(move || {
            let start = Instant::now();
            let target_duration = Duration::from_secs(5);
            let target_updates = 50_000; // 10k/sec * 5 sec
            let interval = Duration::from_nanos(100_000); // 100µs = 10k/sec
            
            let mut next_send = start;
            let mut sent = 0;
            
            while start.elapsed() < target_duration && sent < target_updates {
                let now = Instant::now();
                
                if now >= next_send {
                    let update = MarketUpdate::new(
                        (sent % 10) as u32,
                        50000.0 + sent as f64,
                        50010.0 + sent as f64,
                        now.duration_since(start).as_micros() as u64,
                    );
                    producer.push(update);
                    sent += 1;
                    
                    next_send += interval;
                } else {
                    // Spin wait for precise timing
                    std::hint::spin_loop();
                }
            }
            
            let elapsed = start.elapsed();
            let actual_rate = sent as f64 / elapsed.as_secs_f64();
            
            println!("Producer: sent {} updates in {:.2}s ({:.0} updates/sec)",
                sent, elapsed.as_secs_f64(), actual_rate);
            
            actual_rate
        })
    };
    
    // Consumer: Process as fast as possible
    let consumer_handle = {
        let running = Arc::clone(&running);
        thread::spawn(move || {
            let start = Instant::now();
            let mut received = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(_update) = consumer.pop() {
                    received += 1;
                } else {
                    // Yield to avoid busy-waiting when queue is empty
                    thread::yield_now();
                }
            }
            
            // Drain remaining updates
            while let Some(_update) = consumer.pop() {
                received += 1;
            }
            
            let elapsed = start.elapsed();
            let throughput = received as f64 / elapsed.as_secs_f64();
            
            println!("Consumer: received {} updates in {:.2}s ({:.0} updates/sec)",
                received, elapsed.as_secs_f64(), throughput);
            
            (received, throughput)
        })
    };
    
    // Wait for producer to finish
    let producer_throughput = producer_handle.join().unwrap();
    
    // Stop consumer
    running.store(false, Ordering::Relaxed);
    let (received, consumer_throughput) = consumer_handle.join().unwrap();
    
    // Get final metrics
    let metrics = pipeline.metrics();
    
    println!("\nMetrics:");
    println!("  Push count: {}", metrics.push_count);
    println!("  Pop count: {}", metrics.pop_count);
    println!("  Drop count: {}", metrics.drop_count);
    println!("  Drop rate: {:.2}%", metrics.drop_rate());
    println!("  Final queue depth: {}", metrics.queue_depth);
    
    // Assertions
    assert!(producer_throughput >= 9_000.0, 
        "Producer should achieve at least 9k updates/sec, got {:.0}", producer_throughput);
    assert!(consumer_throughput >= 9_000.0, 
        "Consumer should achieve at least 9k updates/sec, got {:.0}", consumer_throughput);
    assert_eq!(received, metrics.push_count - metrics.drop_count,
        "Received count should match push_count - drop_count");
    
    println!("\n✓ Test passed: System handles 10k+ updates/second");
}

// ============================================================================
// Test 2: Queue Overflow Handling (Backpressure)
// ============================================================================

#[test]
fn test_queue_overflow_handling() {
    println!("\n=== Test 2: Queue Overflow Handling ===");
    
    // Create small queue to trigger overflow quickly
    let pipeline = Arc::new(MarketPipeline::with_capacity(100));
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    let running = Arc::new(AtomicBool::new(true));
    
    // Fast producer: Overwhelm the queue
    let producer_handle = {
        thread::spawn(move || {
            let start = Instant::now();
            let mut sent = 0;
            
            // Send as fast as possible for 1 second
            while start.elapsed() < Duration::from_secs(1) {
                let update = MarketUpdate::new(
                    (sent % 10) as u32,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                producer.push(update);
                sent += 1;
            }
            
            println!("Producer: sent {} updates", sent);
            sent
        })
    };
    
    // Slow consumer: Process with delays to create backpressure
    let consumer_handle = {
        let running = Arc::clone(&running);
        thread::spawn(move || {
            let start = Instant::now();
            let mut received = 0;
            
            // Process for 1.1 seconds
            while start.elapsed() < Duration::from_millis(1100) {
                if let Some(_update) = consumer.pop() {
                    received += 1;
                    // Simulate slow processing (10µs per update)
                    thread::sleep(Duration::from_micros(10));
                }
            }
            
            println!("Consumer: received {} updates", received);
            received
        })
    };
    
    // Wait for both threads
    let sent = producer_handle.join().unwrap();
    let received = consumer_handle.join().unwrap();
    
    // Get metrics
    let metrics = pipeline.metrics();
    
    println!("\nMetrics:");
    println!("  Push count: {}", metrics.push_count);
    println!("  Enqueue count: {}", metrics.enqueue_count);
    println!("  Drop count: {}", metrics.drop_count);
    println!("  Pop count: {}", metrics.pop_count);
    println!("  Drop rate: {:.2}%", metrics.drop_rate());
    println!("  Queue utilization: {:.2}%", metrics.utilization());
    
    // Assertions
    assert_eq!(metrics.push_count, sent as u64, "Push count should match sent");
    assert!(metrics.drop_count > 0, "Should have drops due to backpressure");
    assert!(metrics.is_backpressure(), "Should detect backpressure");
    assert!(received < sent, "Consumer should receive less than sent due to drops");
    
    // Verify system didn't block (producer finished quickly)
    assert!(sent > 10_000, "Producer should send many updates without blocking");
    
    println!("\n✓ Test passed: Queue overflow handled correctly (drops old data)");
}

// ============================================================================
// Test 3: Thread Starvation Scenarios
// ============================================================================

#[test]
fn test_thread_starvation_multiple_producers() {
    println!("\n=== Test 3: Thread Starvation (Multiple Producers) ===");
    
    let pipeline = Arc::new(MarketPipeline::new());
    let consumer = pipeline.consumer();
    
    let running = Arc::new(AtomicBool::new(true));
    let total_sent = Arc::new(AtomicU64::new(0));
    
    // Spawn 8 producer threads (simulating multiple WebSocket connections)
    let mut producer_handles = vec![];
    for thread_id in 0..8 {
        let producer = pipeline.producer();
        let total_sent = Arc::clone(&total_sent);
        
        let handle = thread::spawn(move || {
            let start = Instant::now();
            let mut sent = 0;
            
            // Each thread sends for 2 seconds
            while start.elapsed() < Duration::from_secs(2) {
                let update = MarketUpdate::new(
                    thread_id,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                producer.push(update);
                sent += 1;
                total_sent.fetch_add(1, Ordering::Relaxed);
                
                // Small delay to simulate realistic WebSocket rate
                thread::sleep(Duration::from_micros(50));
            }
            
            println!("Producer {}: sent {} updates", thread_id, sent);
            sent
        });
        
        producer_handles.push(handle);
    }
    
    // Single consumer thread
    let consumer_handle = {
        let running = Arc::clone(&running);
        thread::spawn(move || {
            let mut received = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(_update) = consumer.pop() {
                    received += 1;
                } else {
                    thread::yield_now();
                }
            }
            
            // Drain remaining
            while let Some(_update) = consumer.pop() {
                received += 1;
            }
            
            println!("Consumer: received {} updates", received);
            received
        })
    };
    
    // Wait for all producers
    let mut producer_totals = vec![];
    for handle in producer_handles {
        producer_totals.push(handle.join().unwrap());
    }
    
    // Stop consumer
    thread::sleep(Duration::from_millis(100)); // Let consumer catch up
    running.store(false, Ordering::Relaxed);
    let received = consumer_handle.join().unwrap();
    
    // Get metrics
    let metrics = pipeline.metrics();
    let total_sent_value = total_sent.load(Ordering::Relaxed);
    
    println!("\nMetrics:");
    println!("  Total sent (all producers): {}", total_sent_value);
    println!("  Total received: {}", received);
    println!("  Push count: {}", metrics.push_count);
    println!("  Drop count: {}", metrics.drop_count);
    println!("  Drop rate: {:.2}%", metrics.drop_rate());
    
    // Assertions
    assert_eq!(metrics.push_count, total_sent_value, "Push count should match total sent");
    assert!(received > 0, "Consumer should receive updates");
    
    // Verify no thread was completely starved (all producers sent some data)
    for (i, sent) in producer_totals.iter().enumerate() {
        assert!(*sent > 0, "Producer {} should have sent some updates", i);
    }
    
    println!("\n✓ Test passed: No thread starvation with multiple producers");
}

// ============================================================================
// Test 4: System Stability Under Sustained Load
// ============================================================================

#[test]
fn test_system_stability_sustained_load() {
    println!("\n=== Test 4: System Stability Under Sustained Load ===");
    
    let pipeline = Arc::new(MarketPipeline::new());
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    let running = Arc::new(AtomicBool::new(true));
    let errors = Arc::new(AtomicU64::new(0));
    
    // Producer: Sustained load for 10 seconds
    let producer_handle = {
        thread::spawn(move || {
            let start = Instant::now();
            let duration = Duration::from_secs(10);
            let mut sent = 0;
            let mut last_report = start;
            
            while start.elapsed() < duration {
                let update = MarketUpdate::new(
                    (sent % 10) as u32,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                
                producer.push(update);
                sent += 1;
                
                // Target ~10k updates/sec
                thread::sleep(Duration::from_micros(100));
                
                // Report progress every second
                if last_report.elapsed() >= Duration::from_secs(1) {
                    let elapsed = start.elapsed().as_secs_f64();
                    let rate = sent as f64 / elapsed;
                    println!("  [{:.1}s] Producer: {} updates ({:.0}/sec)", 
                        elapsed, sent, rate);
                    last_report = Instant::now();
                }
            }
            
            println!("Producer: completed {} updates", sent);
            sent
        })
    };
    
    // Consumer: Match producer rate
    let consumer_handle = {
        let running = Arc::clone(&running);
        let errors = Arc::clone(&errors);
        thread::spawn(move || {
            let start = Instant::now();
            let mut received = 0;
            let mut last_report = start;
            let mut last_timestamp = 0u64;
            
            while running.load(Ordering::Relaxed) {
                if let Some(update) = consumer.pop() {
                    received += 1;
                    
                    // Verify data integrity (timestamps should be monotonic)
                    if update.timestamp_us < last_timestamp {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    last_timestamp = update.timestamp_us;
                    
                    // Report progress every second
                    if last_report.elapsed() >= Duration::from_secs(1) {
                        let elapsed = start.elapsed().as_secs_f64();
                        let rate = received as f64 / elapsed;
                        println!("  [{:.1}s] Consumer: {} updates ({:.0}/sec)", 
                            elapsed, received, rate);
                        last_report = Instant::now();
                    }
                } else {
                    thread::yield_now();
                }
            }
            
            // Drain remaining
            while let Some(_update) = consumer.pop() {
                received += 1;
            }
            
            println!("Consumer: completed {} updates", received);
            received
        })
    };
    
    // Wait for producer
    let sent = producer_handle.join().unwrap();
    
    // Stop consumer
    thread::sleep(Duration::from_millis(500)); // Let consumer catch up
    running.store(false, Ordering::Relaxed);
    let received = consumer_handle.join().unwrap();
    
    // Get final metrics
    let metrics = pipeline.metrics();
    let error_count = errors.load(Ordering::Relaxed);
    
    println!("\nFinal Metrics:");
    println!("  Push count: {}", metrics.push_count);
    println!("  Pop count: {}", metrics.pop_count);
    println!("  Drop count: {}", metrics.drop_count);
    println!("  Drop rate: {:.2}%", metrics.drop_rate());
    println!("  Queue depth: {}", metrics.queue_depth);
    println!("  Data integrity errors: {}", error_count);
    
    // Assertions
    assert_eq!(metrics.push_count, sent as u64, "Push count should match sent");
    assert!(sent >= 10_000, "Should send at least 10k updates in 10 seconds");
    assert_eq!(error_count, 0, "Should have zero data integrity errors");
    assert!(metrics.queue_depth < 1000, "Queue should not be severely backed up");
    
    // Verify system remained stable (no panics, no deadlocks)
    println!("\n✓ Test passed: System stable under 10 seconds of sustained load");
}

// ============================================================================
// Test 5: Combined Stress Test (All Scenarios)
// ============================================================================

#[test]
fn test_combined_stress_all_scenarios() {
    println!("\n=== Test 5: Combined Stress Test ===");
    
    let market_pipeline = Arc::new(MarketPipeline::new());
    let execution_pipeline = Arc::new(ExecutionPipeline::new());
    
    let running = Arc::new(AtomicBool::new(true));
    let total_updates = Arc::new(AtomicU64::new(0));
    let total_orders = Arc::new(AtomicU64::new(0));
    
    // Multiple WebSocket producers (4 threads)
    let mut websocket_handles = vec![];
    for thread_id in 0..4 {
        let producer = market_pipeline.producer();
        let total_updates = Arc::clone(&total_updates);
        
        let handle = thread::spawn(move || {
            let start = Instant::now();
            let mut sent = 0;
            
            while start.elapsed() < Duration::from_secs(5) {
                let update = MarketUpdate::new(
                    thread_id,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                producer.push(update);
                sent += 1;
                total_updates.fetch_add(1, Ordering::Relaxed);
                
                thread::sleep(Duration::from_micros(50));
            }
            
            sent
        });
        
        websocket_handles.push(handle);
    }
    
    // Strategy thread (consumer + producer)
    let strategy_handle = {
        let market_consumer = market_pipeline.consumer();
        let execution_producer = execution_pipeline.producer();
        let running = Arc::clone(&running);
        let total_orders = Arc::clone(&total_orders);
        
        thread::spawn(move || {
            let mut processed = 0;
            let mut orders_generated = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(update) = market_consumer.pop() {
                    processed += 1;
                    
                    // Generate order for every 10th update
                    if processed % 10 == 0 {
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
                        total_orders.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    thread::yield_now();
                }
            }
            
            (processed, orders_generated)
        })
    };
    
    // Execution thread (consumer)
    let execution_handle = {
        let execution_consumer = execution_pipeline.consumer();
        let running = Arc::clone(&running);
        
        thread::spawn(move || {
            let mut executed = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(_order) = execution_consumer.pop() {
                    executed += 1;
                    // Simulate order execution delay
                    thread::sleep(Duration::from_micros(50));
                } else {
                    thread::yield_now();
                }
            }
            
            // Drain remaining
            while let Some(_order) = execution_consumer.pop() {
                executed += 1;
            }
            
            executed
        })
    };
    
    // Wait for all WebSocket threads
    for handle in websocket_handles {
        handle.join().unwrap();
    }
    
    // Let strategy and execution catch up
    thread::sleep(Duration::from_millis(500));
    
    // Stop all threads
    running.store(false, Ordering::Relaxed);
    
    let (processed, orders_generated) = strategy_handle.join().unwrap();
    let executed = execution_handle.join().unwrap();
    
    // Get metrics
    let market_metrics = market_pipeline.metrics();
    let execution_metrics = execution_pipeline.metrics();
    
    println!("\nMarket Pipeline Metrics:");
    println!("  Push count: {}", market_metrics.push_count);
    println!("  Pop count: {}", market_metrics.pop_count);
    println!("  Drop count: {}", market_metrics.drop_count);
    println!("  Drop rate: {:.2}%", market_metrics.drop_rate());
    
    println!("\nExecution Pipeline Metrics:");
    println!("  Submit count: {}", execution_metrics.submit_count);
    println!("  Pop count: {}", execution_metrics.pop_count);
    println!("  Drop count: {}", execution_metrics.drop_count);
    println!("  Drop rate: {:.2}%", execution_metrics.drop_rate());
    
    println!("\nProcessing Stats:");
    println!("  Updates processed: {}", processed);
    println!("  Orders generated: {}", orders_generated);
    println!("  Orders executed: {}", executed);
    
    // Assertions
    assert!(market_metrics.push_count > 10_000, "Should process many market updates");
    assert!(processed > 0, "Strategy should process updates");
    assert!(orders_generated > 0, "Strategy should generate orders");
    assert_eq!(executed, orders_generated, "All orders should be executed");
    
    println!("\n✓ Test passed: Combined stress test successful");
}
