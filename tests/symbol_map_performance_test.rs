//! Performance tests for SymbolMap to verify acceptance criteria.
//!
//! Acceptance Criteria:
//! - SymbolMap handles 1000+ concurrent requests/sec
//! - IDs are stable across restarts (same symbol = same ID)
//! - O(1) lookup performance

use arbitrage2::strategy::symbol_map::SymbolMap;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

#[test]
fn test_throughput_1000_requests_per_sec() {
    let map = Arc::new(SymbolMap::new());
    let num_threads = 10;
    let requests_per_thread = 1000;
    let total_requests = num_threads * requests_per_thread;
    
    let start = Instant::now();
    let mut handles = vec![];
    
    for thread_id in 0..num_threads {
        let map_clone = Arc::clone(&map);
        let handle = thread::spawn(move || {
            for i in 0..requests_per_thread {
                let symbol = format!("SYMBOL{}", (thread_id * requests_per_thread + i) % 100);
                let _id = map_clone.get_or_insert("exchange", &symbol);
            }
        });
        handles.push(handle);
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    let elapsed = start.elapsed();
    let requests_per_sec = total_requests as f64 / elapsed.as_secs_f64();
    
    println!("Throughput: {:.0} requests/sec", requests_per_sec);
    println!("Total requests: {}", total_requests);
    println!("Elapsed time: {:?}", elapsed);
    
    // Verify we exceed 1000 requests/sec
    assert!(requests_per_sec > 1000.0, 
        "Expected >1000 req/sec, got {:.0}", requests_per_sec);
}

#[test]
fn test_id_stability() {
    // Create two separate SymbolMap instances
    let map1 = SymbolMap::new();
    let map2 = SymbolMap::new();
    
    // Both should pre-allocate the same common symbols
    let id1_btc = map1.get_or_insert("bybit", "BTCUSDT");
    let id2_btc = map2.get_or_insert("bybit", "BTCUSDT");
    
    // IDs should be the same because of pre-allocation
    assert_eq!(id1_btc, id2_btc, 
        "Pre-allocated symbols should have stable IDs across instances");
    
    let id1_eth = map1.get_or_insert("bybit", "ETHUSDT");
    let id2_eth = map2.get_or_insert("bybit", "ETHUSDT");
    
    assert_eq!(id1_eth, id2_eth,
        "Pre-allocated symbols should have stable IDs across instances");
}

#[test]
fn test_lookup_performance() {
    let map = SymbolMap::new();
    
    // Insert 1000 symbols
    let mut ids = vec![];
    for i in 0..1000 {
        let symbol = format!("SYMBOL{}", i);
        let id = map.get_or_insert("exchange", &symbol);
        ids.push(id);
    }
    
    // Measure lookup performance
    let iterations = 100_000;
    let start = Instant::now();
    
    for _ in 0..iterations {
        for &id in &ids {
            let _ = map.get(id);
        }
    }
    
    let elapsed = start.elapsed();
    let lookups_per_sec = (iterations * ids.len()) as f64 / elapsed.as_secs_f64();
    
    println!("Lookup performance: {:.0} lookups/sec", lookups_per_sec);
    println!("Average lookup time: {:.2} ns", 
        elapsed.as_nanos() as f64 / (iterations * ids.len()) as f64);
    
    // O(1) performance should give us millions of lookups per second
    assert!(lookups_per_sec > 1_000_000.0,
        "Expected >1M lookups/sec for O(1) performance, got {:.0}", lookups_per_sec);
}

#[test]
fn test_concurrent_mixed_operations() {
    let map = Arc::new(SymbolMap::new());
    let num_threads = 20;
    let operations_per_thread = 500;
    
    let start = Instant::now();
    let mut handles = vec![];
    
    for thread_id in 0..num_threads {
        let map_clone = Arc::clone(&map);
        let handle = thread::spawn(move || {
            for i in 0..operations_per_thread {
                // Mix of inserts and lookups
                let symbol = format!("SYMBOL{}", i % 50);
                let id = map_clone.get_or_insert("exchange", &symbol);
                
                // Verify reverse lookup
                let result = map_clone.get(id);
                assert!(result.is_some());
                
                let (exchange, retrieved_symbol) = result.unwrap();
                assert_eq!(exchange, "exchange");
                assert_eq!(retrieved_symbol, symbol);
            }
        });
        handles.push(handle);
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    let elapsed = start.elapsed();
    let total_ops = num_threads * operations_per_thread * 2; // insert + lookup
    let ops_per_sec = total_ops as f64 / elapsed.as_secs_f64();
    
    println!("Mixed operations: {:.0} ops/sec", ops_per_sec);
    println!("Total operations: {}", total_ops);
    println!("Elapsed time: {:?}", elapsed);
    
    // Should handle well over 1000 ops/sec
    assert!(ops_per_sec > 10_000.0,
        "Expected >10K ops/sec, got {:.0}", ops_per_sec);
}

#[test]
fn test_memory_efficiency() {
    let map = SymbolMap::new();
    
    // Insert 100 symbols
    for i in 0..100 {
        let symbol = format!("SYMBOL{}", i);
        map.get_or_insert("exchange", &symbol);
    }
    
    // Verify all symbols are accessible
    assert!(map.len() >= 100);
    
    // Verify bidirectional mapping works for all
    for i in 0..100 {
        let symbol = format!("SYMBOL{}", i);
        let id = map.get_or_insert("exchange", &symbol);
        let (exchange, retrieved_symbol) = map.get(id).unwrap();
        assert_eq!(exchange, "exchange");
        assert_eq!(retrieved_symbol, symbol);
    }
}
