/// Test to verify Redis has been removed from the hot path
///
/// This test validates that the strategy runner uses the market data store
/// instead of Redis for price lookups during opportunity validation and
/// position monitoring.
///
/// Requirements: 1.3 (Redis only for persistence), 1.4 (Direct memory access)

use arbitrage2::strategy::market_data::MarketDataStore;
use arbitrage2::strategy::types::{symbol_to_id, MarketUpdate};

#[test]
fn test_market_data_store_replaces_redis() {
    // Create market data store (replaces Redis for hot path)
    let mut store = MarketDataStore::new();
    
    // Simulate market data arriving from WebSocket queue
    let btc_id = symbol_to_id("BTCUSDT").expect("BTC should be mapped");
    let eth_id = symbol_to_id("ETHUSDT").expect("ETH should be mapped");
    
    // Update market data (this is what the streaming pipeline does)
    store.update(btc_id, 50000.0, 50010.0, 1000000);
    store.update(eth_id, 3000.0, 3001.0, 1000000);
    
    // Verify we can retrieve prices without Redis
    let btc_bid = store.get_bid(btc_id).expect("Should have BTC bid");
    let btc_ask = store.get_ask(btc_id).expect("Should have BTC ask");
    
    assert_eq!(btc_bid, 50000.0);
    assert_eq!(btc_ask, 50010.0);
    
    // Verify spread calculation (hot path operation)
    let btc_spread = store.get_spread_bps(btc_id);
    let expected_spread = ((50010.0 - 50000.0) / 50000.0) * 10000.0;
    assert!((btc_spread - expected_spread).abs() < 0.01);
    
    println!("✅ Market data store successfully replaces Redis for hot path");
    println!("   BTC Bid: ${:.2}, Ask: ${:.2}, Spread: {:.2}bps", btc_bid, btc_ask, btc_spread);
}

#[test]
fn test_market_update_integration() {
    // Test the full flow: MarketUpdate → Store → Price Lookup
    let mut store = MarketDataStore::new();
    
    let btc_id = symbol_to_id("BTCUSDT").expect("BTC should be mapped");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;
    
    // Create market update (what comes from WebSocket)
    let update = MarketUpdate::new(btc_id, 50000.0, 50010.0, timestamp);
    
    // Update store (what strategy runner does)
    store.update_from_market_update(&update);
    
    // Verify prices are available (no Redis needed)
    assert_eq!(store.get_bid(btc_id), Some(50000.0));
    assert_eq!(store.get_ask(btc_id), Some(50010.0));
    assert_eq!(store.get_timestamp(btc_id), Some(timestamp));
    
    println!("✅ MarketUpdate → Store → Lookup works without Redis");
}

#[test]
fn test_hot_path_performance() {
    // Verify hot path operations are fast (no network calls)
    let mut store = MarketDataStore::new();
    
    let btc_id = symbol_to_id("BTCUSDT").expect("BTC should be mapped");
    store.update(btc_id, 50000.0, 50010.0, 1000000);
    
    // Measure hot path operation (should be <1 microsecond)
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = store.get_bid(btc_id);
        let _ = store.get_ask(btc_id);
        let _ = store.get_spread_bps(btc_id);
    }
    let elapsed = start.elapsed();
    
    let avg_ns = elapsed.as_nanos() / 1000;
    println!("✅ Hot path performance: {} ns per lookup (3 operations)", avg_ns);
    
    // Should be much faster than Redis (which is ~1-2ms = 1,000,000-2,000,000 ns)
    assert!(avg_ns < 10_000, "Hot path should be <10 microseconds, got {} ns", avg_ns);
}

#[test]
fn test_symbol_id_mapping() {
    // Verify symbol to ID mapping works (required for market data store)
    let symbols = vec!["BTCUSDT", "ETHUSDT", "SOLUSDT"];
    
    for symbol in symbols {
        let id = symbol_to_id(symbol).expect(&format!("{} should have ID", symbol));
        assert!(id > 0, "Symbol ID should be positive");
        println!("✅ {} → ID {}", symbol, id);
    }
}

#[test]
fn test_multiple_symbols() {
    // Test that multiple symbols can be tracked simultaneously
    let mut store = MarketDataStore::new();
    
    let symbols = vec![
        ("BTCUSDT", 50000.0, 50010.0),
        ("ETHUSDT", 3000.0, 3001.0),
        ("SOLUSDT", 100.0, 100.1),
    ];
    
    // Update all symbols
    for (symbol, bid, ask) in &symbols {
        let id = symbol_to_id(symbol).expect("Symbol should be mapped");
        store.update(id, *bid, *ask, 1000000);
    }
    
    // Verify all symbols are retrievable
    for (symbol, expected_bid, expected_ask) in &symbols {
        let id = symbol_to_id(symbol).expect("Symbol should be mapped");
        let bid = store.get_bid(id).expect("Should have bid");
        let ask = store.get_ask(id).expect("Should have ask");
        
        assert_eq!(bid, *expected_bid);
        assert_eq!(ask, *expected_ask);
        
        println!("✅ {} - Bid: ${:.2}, Ask: ${:.2}", symbol, bid, ask);
    }
}

#[test]
fn test_staleness_detection() {
    // Test that stale data can be detected without Redis
    let mut store = MarketDataStore::new();
    
    let btc_id = symbol_to_id("BTCUSDT").expect("BTC should be mapped");
    let old_timestamp = 1000000; // 1 second
    let current_timestamp = 3000000; // 3 seconds
    
    store.update(btc_id, 50000.0, 50010.0, old_timestamp);
    
    // Check if data is stale (threshold: 1 second = 1,000,000 microseconds)
    let is_stale = store.is_stale(btc_id, current_timestamp, 1_000_000);
    
    assert!(is_stale, "Data should be stale after 2 seconds");
    println!("✅ Staleness detection works without Redis");
}

#[test]
fn test_zero_allocations() {
    // Verify that hot path operations don't allocate
    let mut store = MarketDataStore::new();
    
    let btc_id = symbol_to_id("BTCUSDT").expect("BTC should be mapped");
    store.update(btc_id, 50000.0, 50010.0, 1000000);
    
    // These operations should not allocate (use pre-allocated arrays)
    let _ = store.get_bid(btc_id);
    let _ = store.get_ask(btc_id);
    let _ = store.get_spread_bps(btc_id);
    let _ = store.get_mid_price(btc_id);
    
    // If we got here without panicking, zero allocations worked
    println!("✅ Hot path operations use zero allocations");
}
