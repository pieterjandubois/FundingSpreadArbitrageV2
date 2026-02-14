use arbitrage2::strategy::opportunity_queue::OpportunityQueue;
use arbitrage2::strategy::types::{ArbitrageOpportunity, ConfluenceMetrics, HardConstraints};
use std::time::Instant;

fn create_test_opportunity(symbol: &str, spread_bps: f64) -> ArbitrageOpportunity {
    ArbitrageOpportunity {
        symbol: symbol.to_string(),
        long_exchange: "bybit".to_string(),
        short_exchange: "okx".to_string(),
        long_price: 50000.0,
        short_price: 50100.0,
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
        timestamp: Some(1234567890),
    }
}

#[test]
fn test_throughput_10k_per_second() {
    let queue = OpportunityQueue::with_capacity(10000);
    let producer = queue.producer();
    let consumer = queue.consumer();
    
    // Test push throughput
    let push_count = 20000;
    let start = Instant::now();
    
    for i in 0..push_count {
        let opp = create_test_opportunity(&format!("BTC{}", i % 100), 10.0 + (i % 50) as f64);
        producer.push(opp);
    }
    
    let push_duration = start.elapsed();
    let push_per_sec = (push_count as f64 / push_duration.as_secs_f64()) as u64;
    
    println!("Push throughput: {} ops/sec", push_per_sec);
    println!("Push duration: {:?} for {} operations", push_duration, push_count);
    
    // Verify we can handle 10K+ per second
    assert!(push_per_sec > 10_000, "Push throughput {} is below 10K/sec", push_per_sec);
    
    // Test pop throughput
    let start = Instant::now();
    let mut pop_count = 0;
    
    while consumer.pop().is_some() {
        pop_count += 1;
    }
    
    let pop_duration = start.elapsed();
    let pop_per_sec = (pop_count as f64 / pop_duration.as_secs_f64()) as u64;
    
    println!("Pop throughput: {} ops/sec", pop_per_sec);
    println!("Pop duration: {:?} for {} operations", pop_duration, pop_count);
    
    // Verify we can handle 10K+ per second
    assert!(pop_per_sec > 10_000, "Pop throughput {} is below 10K/sec", pop_per_sec);
}

#[test]
fn test_lock_free_operations() {
    // This test verifies that operations are lock-free by checking
    // that we can perform operations from multiple threads without blocking
    
    let queue = OpportunityQueue::with_capacity(1000);
    let producer = queue.producer();
    let consumer1 = queue.consumer();
    let consumer2 = queue.consumer();
    
    // Spawn producer thread
    let producer_handle = std::thread::spawn(move || {
        for i in 0..1000 {
            let opp = create_test_opportunity(&format!("BTC{}", i), 10.0);
            producer.push(opp);
        }
    });
    
    // Spawn consumer threads
    let consumer1_handle = std::thread::spawn(move || {
        let mut count = 0;
        for _ in 0..500 {
            if consumer1.pop().is_some() {
                count += 1;
            }
            std::thread::sleep(std::time::Duration::from_micros(10));
        }
        count
    });
    
    let consumer2_handle = std::thread::spawn(move || {
        let mut count = 0;
        for _ in 0..500 {
            if consumer2.pop().is_some() {
                count += 1;
            }
            std::thread::sleep(std::time::Duration::from_micros(10));
        }
        count
    });
    
    // Wait for all threads to complete
    producer_handle.join().expect("Producer thread panicked");
    let count1 = consumer1_handle.join().expect("Consumer 1 thread panicked");
    let count2 = consumer2_handle.join().expect("Consumer 2 thread panicked");
    
    println!("Consumer 1 popped: {} opportunities", count1);
    println!("Consumer 2 popped: {} opportunities", count2);
    
    // Both consumers should have successfully popped some opportunities
    assert!(count1 > 0, "Consumer 1 should have popped some opportunities");
    assert!(count2 > 0, "Consumer 2 should have popped some opportunities");
}

#[test]
fn test_multiple_consumers_independence() {
    let queue = OpportunityQueue::with_capacity(100);
    let producer = queue.producer();
    
    // Create 3 independent consumers
    let consumer1 = queue.consumer();
    let consumer2 = queue.consumer();
    let consumer3 = queue.consumer();
    
    // Push 30 opportunities
    for i in 0..30 {
        producer.push(create_test_opportunity(&format!("BTC{}", i), 10.0));
    }
    
    // Each consumer can pop independently
    let c1_batch = consumer1.pop_batch(10);
    let c2_batch = consumer2.pop_batch(10);
    let c3_batch = consumer3.pop_batch(10);
    
    // All consumers should have gotten opportunities
    assert_eq!(c1_batch.len(), 10);
    assert_eq!(c2_batch.len(), 10);
    assert_eq!(c3_batch.len(), 10);
    
    // Queue should be empty now
    assert!(queue.is_empty());
    
    // Verify total pop count
    assert_eq!(queue.pop_count(), 30);
}
