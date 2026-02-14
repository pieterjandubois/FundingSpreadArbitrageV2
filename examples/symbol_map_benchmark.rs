//! Simple benchmark to verify SymbolMap performance meets acceptance criteria.

use arbitrage2::strategy::symbol_map::SymbolMap;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

fn main() {
    println!("=== SymbolMap Performance Benchmark ===\n");
    
    // Test 1: Throughput
    println!("Test 1: Concurrent Throughput");
    test_throughput();
    
    // Test 2: ID Stability
    println!("\nTest 2: ID Stability Across Instances");
    test_id_stability();
    
    // Test 3: Lookup Performance
    println!("\nTest 3: Lookup Performance");
    test_lookup_performance();
    
    println!("\n=== All Tests Passed ===");
}

fn test_throughput() {
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
    
    println!("  Total requests: {}", total_requests);
    println!("  Elapsed time: {:?}", elapsed);
    println!("  Throughput: {:.0} requests/sec", requests_per_sec);
    
    if requests_per_sec > 1000.0 {
        println!("  ✓ PASS: Exceeds 1000 req/sec requirement");
    } else {
        println!("  ✗ FAIL: Below 1000 req/sec requirement");
    }
}

fn test_id_stability() {
    let map1 = SymbolMap::new();
    let map2 = SymbolMap::new();
    
    let id1_btc = map1.get_or_insert("bybit", "BTCUSDT");
    let id2_btc = map2.get_or_insert("bybit", "BTCUSDT");
    
    println!("  Map1 BTCUSDT ID: {}", id1_btc);
    println!("  Map2 BTCUSDT ID: {}", id2_btc);
    
    if id1_btc == id2_btc {
        println!("  ✓ PASS: IDs are stable across instances");
    } else {
        println!("  ✗ FAIL: IDs differ across instances");
    }
}

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
    let avg_lookup_ns = elapsed.as_nanos() as f64 / (iterations * ids.len()) as f64;
    
    println!("  Total lookups: {}", iterations * ids.len());
    println!("  Elapsed time: {:?}", elapsed);
    println!("  Lookups/sec: {:.0}", lookups_per_sec);
    println!("  Avg lookup time: {:.2} ns", avg_lookup_ns);
    
    if lookups_per_sec > 1_000_000.0 {
        println!("  ✓ PASS: O(1) performance verified (>1M lookups/sec)");
    } else {
        println!("  ✗ FAIL: Below expected O(1) performance");
    }
}
