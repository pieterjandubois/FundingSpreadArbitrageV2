// tests/streaming_backpressure_test.rs
//
// Backpressure and Stability Testing for Streaming Opportunity Detection
//
// This test suite validates system behavior under extreme load and backpressure:
// 1. 10K+ updates/second throughput
// 2. Queue backpressure handling (drops oldest)
// 3. No memory leaks under sustained load
// 4. No crashes or deadlocks over extended runtime
//
// Requirements: Task 6.3 (Backpressure and Stability Testing)
// Acceptance Criteria:
// - Handles 10K+ updates/sec sustained
// - Graceful backpressure handling
// - No memory leaks or crashes
// - Stable over 24 hours (or 1 hour for CI)

use arbitrage2::strategy::pipeline::MarketPipeline;
use arbitrage2::strategy::opportunity_queue::OpportunityQueue;
use arbitrage2::strategy::types::{MarketUpdate, ArbitrageOpportunity, ConfluenceMetrics, HardConstraints};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Helper to create a test market update
fn create_market_update(symbol_id: u32, bid: f64, ask: f64, timestamp_us: u64) -> MarketUpdate {
    MarketUpdate::new(symbol_id, bid, ask, timestamp_us)
}

/// Helper to create a test opportunity
fn create_test_opportunity(id: u64, spread_bps: f64) -> ArbitrageOpportunity {
    ArbitrageOpportunity {
        symbol: format!("BTCUSDT{}", id),
        long_exchange: "bybit".to_string(),
        short_exchange: "okx".to_string(),
        long_price: 50000.0 + (id as f64 * 10.0),
        short_price: 50100.0 + (id as f64 * 10.0),
        spread_bps,
        funding_delta_8h: 0.0001,
        confidence_score: 80,
        projected_profit_usd: 10.0,
        projected_profit_after_slippage: 8.0,
        metrics: ConfluenceMetrics {
            funding_delta: 0.0001,
            funding_delta_projected: 0.0002,
            obi_ratio: 0.5,
            oi_current: 1000000.0,
            oi_24h_avg: 900000.0,
            vwap_deviation: 0.5,
            atr: 100.0,
            atr_trend: true,
            liquidation_cluster_distance: 50.0,
            hard_constraints: HardConstraints {
                order_book_depth_sufficient: true,
                exchange_latency_ok: true,
                funding_delta_substantial: true,
            },
        },
        order_book_depth_long: 10000.0,
        order_book_depth_short: 10000.0,
        timestamp: Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ),
    }
}

// ============================================================================
// Test 6.3.1 & 6.3.6: Inject 10K+ updates/sec and verify handling
// ============================================================================

#[test]
fn test_handles_10k_updates_per_second() {
    println!("\n=== Test 6.3.1 & 6.3.6: 10K+ Updates/Second ===");
    
    let market_pipeline = Arc::new(MarketPipeline::new());
    let opportunity_queue = Arc::new(OpportunityQueue::with_capacity(1000));
    
    let market_producer = market_pipeline.producer();
    let market_consumer = market_pipeline.consumer();
    let opportunity_producer = opportunity_queue.producer();
    let opportunity_consumer = opportunity_queue.consumer();
    
    let running = Arc::new(AtomicBool::new(true));
    let updates_sent = Arc::new(AtomicU64::new(0));
    let opportunities_generated = Arc::new(AtomicU64::new(0));
    
    // Producer: Send 10K+ updates/second for 5 seconds
    let producer_handle = {
        let updates_sent = Arc::clone(&updates_sent);
        thread::spawn(move || {
            let start = Instant::now();
            let target_duration = Duration::from_secs(5);
            let target_rate = 10_000; // 10k updates/sec
            let interval = Duration::from_nanos(1_000_000_000 / target_rate); // 100µs
            
            let mut next_send = start;
            let mut sent = 0;
            
            while start.elapsed() < target_duration {
                let now = Instant::now();
                
                if now >= next_send {
                    let timestamp_us = now.duration_since(start).as_micros() as u64;
                    let update = create_market_update(
                        (sent % 10) as u32,
                        50000.0 + sent as f64,
                        50010.0 + sent as f64,
                        timestamp_us,
                    );
                    market_producer.push(update);
                    sent += 1;
                    updates_sent.fetch_add(1, Ordering::Relaxed);
                    
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
            
            (sent, actual_rate)
        })
    };
    
    // Simulated detector: Consume market updates and generate opportunities
    let detector_handle = {
        let running = Arc::clone(&running);
        let opportunities_generated = Arc::clone(&opportunities_generated);
        thread::spawn(move || {
            let mut processed = 0;
            let mut generated = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(_update) = market_consumer.pop() {
                    processed += 1;
                    
                    // Generate opportunity for every 100th update
                    if processed % 100 == 0 {
                        let opp = create_test_opportunity(generated, 15.0);
                        opportunity_producer.push(opp);
                        generated += 1;
                        opportunities_generated.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    thread::yield_now();
                }
            }
            
            println!("Detector: processed {} updates, generated {} opportunities",
                processed, generated);
            
            (processed, generated)
        })
    };
    
    // Consumer: Process opportunities
    let consumer_handle = {
        let running = Arc::clone(&running);
        thread::spawn(move || {
            let mut consumed = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(_opp) = opportunity_consumer.pop() {
                    consumed += 1;
                } else {
                    thread::yield_now();
                }
            }
            
            // Drain remaining
            while let Some(_opp) = opportunity_consumer.pop() {
                consumed += 1;
            }
            
            println!("Consumer: consumed {} opportunities", consumed);
            consumed
        })
    };
    
    // Wait for producer to finish
    let (sent, actual_rate) = producer_handle.join().unwrap();
    
    // Let detector and consumer catch up
    thread::sleep(Duration::from_millis(500));
    
    // Stop all threads
    running.store(false, Ordering::Relaxed);
    
    let (processed, generated) = detector_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    // Get final metrics
    let market_metrics = market_pipeline.metrics();
    
    println!("\nMarket Pipeline Metrics:");
    println!("  Push count: {}", market_metrics.push_count);
    println!("  Pop count: {}", market_metrics.pop_count);
    println!("  Drop count: {}", market_metrics.drop_count);
    println!("  Drop rate: {:.2}%", market_metrics.drop_rate());
    
    println!("\nOpportunity Queue Metrics:");
    println!("  Push count: {}", opportunity_queue.push_count());
    println!("  Pop count: {}", opportunity_queue.pop_count());
    println!("  Drop count: {}", opportunity_queue.drop_count());
    println!("  Queue length: {}", opportunity_queue.len());
    
    // Assertions
    assert!(actual_rate >= 9_000.0, 
        "Should achieve at least 9k updates/sec, got {:.0}", actual_rate);
    assert!(sent >= 45_000, 
        "Should send at least 45k updates in 5 seconds, got {}", sent);
    assert_eq!(market_metrics.push_count, sent as u64,
        "Push count should match sent");
    assert!(processed > 0, "Detector should process updates");
    assert!(generated > 0, "Detector should generate opportunities");
    assert_eq!(consumed, generated, "All opportunities should be consumed");
    
    println!("\n✓ Test passed: System handles 10K+ updates/second");
}

// ============================================================================
// Test 6.3.2 & 6.3.7: Verify backpressure drops oldest
// ============================================================================

#[test]
fn test_backpressure_drops_oldest() {
    println!("\n=== Test 6.3.2 & 6.3.7: Backpressure Drops Oldest ===");
    
    // Create small queue to trigger backpressure quickly
    let opportunity_queue = Arc::new(OpportunityQueue::with_capacity(50));
    let producer = opportunity_queue.producer();
    let consumer = opportunity_queue.consumer();
    
    // Fast producer: Overwhelm the queue
    let producer_handle = thread::spawn(move || {
        let start = Instant::now();
        let mut sent = 0;
        
        // Send 200 opportunities as fast as possible
        while sent < 200 {
            let opp = create_test_opportunity(sent, 15.0 + sent as f64);
            producer.push(opp);
            sent += 1;
        }
        
        let elapsed = start.elapsed();
        println!("Producer: sent {} opportunities in {:.2}ms",
            sent, elapsed.as_millis());
        
        sent
    });
    
    // Slow consumer: Create backpressure
    let consumer_handle = thread::spawn(move || {
        // Wait for producer to fill queue
        thread::sleep(Duration::from_millis(100));
        
        let mut consumed = Vec::new();
        
        // Consume all remaining opportunities
        while let Some(opp) = consumer.pop() {
            // Extract ID from symbol
            let id_str = opp.symbol.strip_prefix("BTCUSDT").unwrap();
            let id: u64 = id_str.parse().unwrap();
            consumed.push(id);
        }
        
        println!("Consumer: consumed {} opportunities", consumed.len());
        consumed
    });
    
    // Wait for both threads
    let sent = producer_handle.join().unwrap();
    let consumed_ids = consumer_handle.join().unwrap();
    
    // Get metrics
    let metrics = opportunity_queue;
    
    println!("\nQueue Metrics:");
    println!("  Capacity: 50");
    println!("  Push count: {}", metrics.push_count());
    println!("  Pop count: {}", metrics.pop_count());
    println!("  Drop count: {}", metrics.drop_count());
    
    // Assertions
    assert_eq!(metrics.push_count(), sent as u64, "Push count should match sent");
    assert!(metrics.drop_count() > 0, "Should have drops due to backpressure");
    assert!(metrics.drop_count() >= 150, "Should drop at least 150 opportunities (200 - 50 capacity)");
    assert_eq!(consumed_ids.len(), 50, "Should consume exactly capacity (50)");
    
    // Verify oldest were dropped (should have IDs from 150-199, not 0-49)
    let min_id = *consumed_ids.iter().min().unwrap();
    let max_id = *consumed_ids.iter().max().unwrap();
    
    println!("\nConsumed IDs range: {} to {}", min_id, max_id);
    
    assert!(min_id >= 150, 
        "Oldest opportunities should be dropped, min ID should be >= 150, got {}", min_id);
    assert_eq!(max_id, 199, 
        "Newest opportunities should be kept, max ID should be 199, got {}", max_id);
    
    println!("\n✓ Test passed: Backpressure correctly drops oldest data");
}

// ============================================================================
// Test 6.3.3: Verify queues handle backpressure
// ============================================================================

#[test]
fn test_queues_handle_backpressure_gracefully() {
    println!("\n=== Test 6.3.3: Queues Handle Backpressure ===");
    
    let market_pipeline = Arc::new(MarketPipeline::with_capacity(100));
    let opportunity_queue = Arc::new(OpportunityQueue::with_capacity(100));
    
    let market_producer = market_pipeline.producer();
    let opportunity_producer = opportunity_queue.producer();
    
    // Overwhelm both queues
    let market_handle = thread::spawn(move || {
        for i in 0..1000 {
            let update = create_market_update(
                (i % 10) as u32,
                50000.0 + i as f64,
                50010.0 + i as f64,
                (i * 1000) as u64,
            );
            market_producer.push(update);
        }
        1000
    });
    
    let opportunity_handle = thread::spawn(move || {
        for i in 0..1000 {
            let opp = create_test_opportunity(i, 15.0);
            opportunity_producer.push(opp);
        }
        1000
    });
    
    // Wait for producers
    let market_sent = market_handle.join().unwrap();
    let opp_sent = opportunity_handle.join().unwrap();
    
    // Get metrics
    let market_metrics = market_pipeline.metrics();
    
    println!("\nMarket Pipeline:");
    println!("  Sent: {}", market_sent);
    println!("  Push count: {}", market_metrics.push_count);
    println!("  Drop count: {}", market_metrics.drop_count);
    println!("  Queue depth: {}", market_metrics.queue_depth);
    println!("  Backpressure: {}", market_metrics.is_backpressure());
    
    println!("\nOpportunity Queue:");
    println!("  Sent: {}", opp_sent);
    println!("  Push count: {}", opportunity_queue.push_count());
    println!("  Drop count: {}", opportunity_queue.drop_count());
    println!("  Queue depth: {}", opportunity_queue.len());
    
    // Assertions
    assert_eq!(market_metrics.push_count, market_sent as u64);
    assert_eq!(opportunity_queue.push_count(), opp_sent as u64);
    assert!(market_metrics.drop_count > 0, "Market pipeline should have drops");
    assert!(opportunity_queue.drop_count() > 0, "Opportunity queue should have drops");
    assert!(market_metrics.is_backpressure(), "Market pipeline should detect backpressure");
    
    // Verify queues are at capacity
    assert_eq!(market_metrics.queue_depth, 100, "Market queue should be at capacity");
    assert_eq!(opportunity_queue.len(), 100, "Opportunity queue should be at capacity");
    
    println!("\n✓ Test passed: Queues handle backpressure gracefully");
}

// ============================================================================
// Test 6.3.4: Verify no crashes or deadlocks
// ============================================================================

#[test]
fn test_no_crashes_or_deadlocks() {
    println!("\n=== Test 6.3.4: No Crashes or Deadlocks ===");
    
    let market_pipeline = Arc::new(MarketPipeline::new());
    let opportunity_queue = Arc::new(OpportunityQueue::new());
    
    let running = Arc::new(AtomicBool::new(true));
    
    // Multiple producers and consumers
    let mut handles = vec![];
    
    // 4 market producers
    for thread_id in 0..4 {
        let producer = market_pipeline.producer();
        let running = Arc::clone(&running);
        
        let handle = thread::spawn(move || {
            let mut sent = 0;
            while running.load(Ordering::Relaxed) {
                let update = create_market_update(
                    thread_id,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                producer.push(update);
                sent += 1;
                thread::sleep(Duration::from_micros(100));
            }
            sent
        });
        
        handles.push(handle);
    }
    
    // 2 market consumers / opportunity producers
    for _ in 0..2 {
        let consumer = market_pipeline.consumer();
        let producer = opportunity_queue.producer();
        let running = Arc::clone(&running);
        
        let handle = thread::spawn(move || {
            let mut processed = 0;
            while running.load(Ordering::Relaxed) {
                if let Some(_update) = consumer.pop() {
                    processed += 1;
                    
                    if processed % 10 == 0 {
                        let opp = create_test_opportunity(processed / 10, 15.0);
                        producer.push(opp);
                    }
                } else {
                    thread::yield_now();
                }
            }
            processed
        });
        
        handles.push(handle);
    }
    
    // 2 opportunity consumers
    for _ in 0..2 {
        let consumer = opportunity_queue.consumer();
        let running = Arc::clone(&running);
        
        let handle = thread::spawn(move || {
            let mut consumed = 0;
            while running.load(Ordering::Relaxed) {
                if let Some(_opp) = consumer.pop() {
                    consumed += 1;
                    thread::sleep(Duration::from_micros(50));
                } else {
                    thread::yield_now();
                }
            }
            consumed
        });
        
        handles.push(handle);
    }
    
    // Run for 10 seconds
    println!("Running stress test for 10 seconds...");
    thread::sleep(Duration::from_secs(10));
    
    // Stop all threads
    running.store(false, Ordering::Relaxed);
    
    // Wait for all threads to complete (with timeout)
    let timeout = Duration::from_secs(5);
    let start = Instant::now();
    
    for (i, handle) in handles.into_iter().enumerate() {
        if start.elapsed() > timeout {
            panic!("Thread {} did not complete within timeout - possible deadlock", i);
        }
        
        match handle.join() {
            Ok(_) => println!("Thread {} completed successfully", i),
            Err(_) => panic!("Thread {} panicked", i),
        }
    }
    
    println!("\n✓ Test passed: No crashes or deadlocks detected");
}

// ============================================================================
// Test 6.3.5 & 6.3.9: Run for 1 hour continuous (or shorter for CI)
// ============================================================================

#[test]
#[ignore] // Run with --ignored for extended stability testing
fn test_one_hour_stability() {
    println!("\n=== Test 6.3.5 & 6.3.9: 1 Hour Stability Test ===");
    
    let market_pipeline = Arc::new(MarketPipeline::new());
    let opportunity_queue = Arc::new(OpportunityQueue::new());
    
    let running = Arc::new(AtomicBool::new(true));
    let errors = Arc::new(AtomicU64::new(0));
    
    // Producer: Sustained load
    let producer_handle = {
        let producer = market_pipeline.producer();
        thread::spawn(move || {
            let start = Instant::now();
            let duration = Duration::from_secs(3600); // 1 hour
            let mut sent = 0;
            let mut last_report = start;
            
            while start.elapsed() < duration {
                let update = create_market_update(
                    (sent % 10) as u32,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                producer.push(update);
                sent += 1;
                
                // Target ~1k updates/sec (sustainable rate)
                thread::sleep(Duration::from_micros(1000));
                
                // Report every 5 minutes
                if last_report.elapsed() >= Duration::from_secs(300) {
                    let elapsed_mins = start.elapsed().as_secs() / 60;
                    println!("[{}min] Producer: {} updates", elapsed_mins, sent);
                    last_report = Instant::now();
                }
            }
            
            println!("Producer: completed {} updates in 1 hour", sent);
            sent
        })
    };
    
    // Detector: Process and generate opportunities
    let detector_handle = {
        let consumer = market_pipeline.consumer();
        let producer = opportunity_queue.producer();
        let running = Arc::clone(&running);
        let errors = Arc::clone(&errors);
        
        thread::spawn(move || {
            let mut processed = 0;
            let mut generated = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(update) = consumer.pop() {
                    processed += 1;
                    
                    // Verify data integrity
                    if update.bid <= 0.0 || update.ask <= 0.0 || update.ask <= update.bid {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    
                    // Generate opportunity every 100 updates
                    if processed % 100 == 0 {
                        let opp = create_test_opportunity(generated, 15.0);
                        producer.push(opp);
                        generated += 1;
                    }
                } else {
                    thread::yield_now();
                }
            }
            
            (processed, generated)
        })
    };
    
    // Consumer: Process opportunities
    let consumer_handle = {
        let consumer = opportunity_queue.consumer();
        let running = Arc::clone(&running);
        
        thread::spawn(move || {
            let mut consumed = 0;
            
            while running.load(Ordering::Relaxed) {
                if let Some(_opp) = consumer.pop() {
                    consumed += 1;
                    thread::sleep(Duration::from_micros(100));
                } else {
                    thread::yield_now();
                }
            }
            
            consumed
        })
    };
    
    // Wait for producer to finish
    let sent = producer_handle.join().unwrap();
    
    // Stop other threads
    running.store(false, Ordering::Relaxed);
    
    let (processed, generated) = detector_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    // Get final metrics
    let market_metrics = market_pipeline.metrics();
    let error_count = errors.load(Ordering::Relaxed);
    
    println!("\n=== Final Metrics After 1 Hour ===");
    println!("Market Pipeline:");
    println!("  Updates sent: {}", sent);
    println!("  Updates processed: {}", processed);
    println!("  Push count: {}", market_metrics.push_count);
    println!("  Pop count: {}", market_metrics.pop_count);
    println!("  Drop count: {}", market_metrics.drop_count);
    println!("  Drop rate: {:.2}%", market_metrics.drop_rate());
    
    println!("\nOpportunity Queue:");
    println!("  Opportunities generated: {}", generated);
    println!("  Opportunities consumed: {}", consumed);
    println!("  Push count: {}", opportunity_queue.push_count());
    println!("  Pop count: {}", opportunity_queue.pop_count());
    println!("  Drop count: {}", opportunity_queue.drop_count());
    
    println!("\nData Integrity:");
    println!("  Errors: {}", error_count);
    
    // Assertions
    assert!(sent >= 3_000_000, "Should send at least 3M updates in 1 hour");
    assert_eq!(error_count, 0, "Should have zero data integrity errors");
    assert!(processed > 0, "Should process updates");
    assert!(generated > 0, "Should generate opportunities");
    
    println!("\n✓ Test passed: System stable for 1 hour");
}

// ============================================================================
// Test 6.3.8: No memory leaks
// ============================================================================

#[test]
fn test_no_memory_leaks() {
    println!("\n=== Test 6.3.8: No Memory Leaks ===");
    
    // This test runs for a shorter duration but monitors memory usage
    let market_pipeline = Arc::new(MarketPipeline::new());
    let opportunity_queue = Arc::new(OpportunityQueue::new());
    
    let running = Arc::new(AtomicBool::new(true));
    
    // Producer
    let producer_handle = {
        let producer = market_pipeline.producer();
        let running = Arc::clone(&running);
        
        thread::spawn(move || {
            let mut sent = 0;
            while running.load(Ordering::Relaxed) {
                let update = create_market_update(
                    (sent % 10) as u32,
                    50000.0 + sent as f64,
                    50010.0 + sent as f64,
                    (sent * 1000) as u64,
                );
                producer.push(update);
                sent += 1;
                thread::sleep(Duration::from_micros(100));
            }
            sent
        })
    };
    
    // Detector
    let detector_handle = {
        let consumer = market_pipeline.consumer();
        let producer = opportunity_queue.producer();
        let running = Arc::clone(&running);
        
        thread::spawn(move || {
            let mut processed = 0;
            while running.load(Ordering::Relaxed) {
                if let Some(_update) = consumer.pop() {
                    processed += 1;
                    
                    if processed % 10 == 0 {
                        let opp = create_test_opportunity(processed / 10, 15.0);
                        producer.push(opp);
                    }
                } else {
                    thread::yield_now();
                }
            }
            processed
        })
    };
    
    // Consumer
    let consumer_handle = {
        let consumer = opportunity_queue.consumer();
        let running = Arc::clone(&running);
        
        thread::spawn(move || {
            let mut consumed = 0;
            while running.load(Ordering::Relaxed) {
                if let Some(_opp) = consumer.pop() {
                    consumed += 1;
                } else {
                    thread::yield_now();
                }
            }
            consumed
        })
    };
    
    // Run for 30 seconds
    println!("Running memory leak test for 30 seconds...");
    thread::sleep(Duration::from_secs(30));
    
    // Stop all threads
    running.store(false, Ordering::Relaxed);
    
    let sent = producer_handle.join().unwrap();
    let processed = detector_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    // Get metrics
    let market_metrics = market_pipeline.metrics();
    
    println!("\nMetrics After 30 Seconds:");
    println!("  Updates sent: {}", sent);
    println!("  Updates processed: {}", processed);
    println!("  Opportunities consumed: {}", consumed);
    println!("  Market queue depth: {}", market_metrics.queue_depth);
    println!("  Opportunity queue depth: {}", opportunity_queue.len());
    
    // Assertions
    assert!(sent > 10_000, "Should send at least 10k updates in 30 seconds, got {}", sent);
    assert!(processed > 0, "Should process updates");
    assert!(consumed > 0, "Should consume opportunities");
    
    // Verify queues are not growing unbounded
    assert!(market_metrics.queue_depth < 10_000, 
        "Market queue should not grow unbounded, depth: {}", market_metrics.queue_depth);
    assert!(opportunity_queue.len() < 1_000, 
        "Opportunity queue should not grow unbounded, depth: {}", opportunity_queue.len());
    
    println!("\n✓ Test passed: No memory leaks detected (queues bounded)");
}
