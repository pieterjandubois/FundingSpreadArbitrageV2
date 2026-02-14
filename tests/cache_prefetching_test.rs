// Test for CPU cache prefetching hints
// Requirement: 5.3 (CPU cache prefetching hints)

use arbitrage2::strategy::market_data::MarketDataStore;

#[test]
fn test_prefetch_symbol() {
    let mut store = MarketDataStore::new();
    
    // Populate with some test data
    for i in 0..10 {
        store.update(i, 50000.0 + i as f64, 50010.0 + i as f64, 1000000);
    }
    
    // Test prefetching (should not panic or cause errors)
    for i in 0..10 {
        if i + 1 < 10 {
            store.prefetch_symbol(i + 1);
        }
        
        // Access current symbol
        let bid = store.get_bid(i).unwrap();
        let ask = store.get_ask(i).unwrap();
        
        assert_eq!(bid, 50000.0 + i as f64);
        assert_eq!(ask, 50010.0 + i as f64);
    }
}

#[test]
fn test_iter_spreads_with_prefetch() {
    let mut store = MarketDataStore::new();
    
    // Populate with test data
    for i in 0..100 {
        store.update(i, 50000.0 + i as f64, 50010.0 + i as f64, 1000000);
    }
    
    // Iterate with built-in prefetching
    let mut count = 0;
    for (symbol_id, spread_bps) in store.iter_spreads() {
        assert!(spread_bps > 0.0);
        count += 1;
    }
    
    // Should iterate over all 100 symbols
    assert_eq!(count, 100);
}

#[test]
fn test_prefetch_out_of_bounds() {
    let store = MarketDataStore::new();
    
    // Prefetching out of bounds should not panic
    store.prefetch_symbol(300); // Beyond MAX_SYMBOLS (256)
    
    // Should be a no-op, no crash
}

#[test]
fn test_sequential_access_pattern() {
    let mut store = MarketDataStore::new();
    
    // Populate with 256 symbols (full capacity)
    for i in 0..256 {
        store.update(i as u32, 50000.0 + i as f64, 50010.0 + i as f64, 1000000);
    }
    
    // Sequential access with prefetching
    let mut sum = 0.0;
    for i in 0..256 {
        // Prefetch 8 ahead (1 cache line)
        if i + 8 < 256 {
            store.prefetch_symbol((i + 8) as u32);
        }
        
        if let Some(bid) = store.get_bid(i as u32) {
            sum += bid;
        }
        if let Some(ask) = store.get_ask(i as u32) {
            sum += ask;
        }
    }
    
    // Verify we processed all data
    assert!(sum > 0.0);
}

#[test]
fn test_random_access_pattern_with_prefetch() {
    let mut store = MarketDataStore::new();
    
    // Populate with test data
    for i in 0..100 {
        store.update(i, 50000.0 + i as f64, 50010.0 + i as f64, 1000000);
    }
    
    // Random access pattern
    let access_pattern = vec![5, 23, 7, 89, 12, 45, 67, 3, 91, 34];
    
    let mut sum = 0.0;
    for i in 0..access_pattern.len() {
        // Prefetch next in pattern
        if i + 1 < access_pattern.len() {
            store.prefetch_symbol(access_pattern[i + 1]);
        }
        
        let symbol_id = access_pattern[i];
        if let Some(bid) = store.get_bid(symbol_id) {
            sum += bid;
        }
    }
    
    assert!(sum > 0.0);
}
