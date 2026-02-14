use arbitrage2::strategy::types::{MarketUpdate, symbol_to_id, id_to_symbol};
use zerocopy::{AsBytes, FromBytes};

#[test]
fn test_market_update_creation() {
    let update = MarketUpdate::new(1, 50000.0, 50100.0, 1234567890);
    
    assert_eq!(update.symbol_id, 1);
    assert_eq!(update.bid, 50000.0);
    assert_eq!(update.ask, 50100.0);
    assert_eq!(update.timestamp_us, 1234567890);
}

#[test]
fn test_market_update_spread_calculation() {
    let update = MarketUpdate::new(1, 50000.0, 50100.0, 1234567890);
    
    // Spread = ((50100 - 50000) / 50000) * 10000 = 20 bps
    let spread = update.spread_bps();
    assert!((spread - 20.0).abs() < 0.01, "Expected spread ~20 bps, got {}", spread);
}

#[test]
fn test_market_update_mid_price() {
    let update = MarketUpdate::new(1, 50000.0, 50100.0, 1234567890);
    
    let mid = update.mid_price();
    assert_eq!(mid, 50050.0);
}

#[test]
fn test_market_update_size() {
    // Verify struct is exactly 64 bytes (cache line aligned)
    assert_eq!(std::mem::size_of::<MarketUpdate>(), 64);
}

#[test]
fn test_market_update_alignment() {
    // Verify struct is aligned to 64 bytes
    assert_eq!(std::mem::align_of::<MarketUpdate>(), 64);
}

#[test]
fn test_market_update_zero_copy_as_bytes() {
    let update = MarketUpdate::new(1, 50000.0, 50100.0, 1234567890);
    
    // Convert to bytes
    let bytes = update.as_bytes();
    assert_eq!(bytes.len(), 64);
    
    // Verify we can read the bytes back
    let reconstructed = MarketUpdate::read_from(bytes).expect("Failed to read from bytes");
    assert_eq!(reconstructed.symbol_id, update.symbol_id);
    assert_eq!(reconstructed.bid, update.bid);
    assert_eq!(reconstructed.ask, update.ask);
    assert_eq!(reconstructed.timestamp_us, update.timestamp_us);
}

#[test]
fn test_market_update_zero_copy_from_bytes() {
    // Create a byte buffer
    let mut buffer = [0u8; 64];
    
    // Write a MarketUpdate to it
    let original = MarketUpdate::new(2, 3000.0, 3010.0, 9876543210);
    buffer.copy_from_slice(original.as_bytes());
    
    // Read it back without copying
    let reconstructed = MarketUpdate::read_from(&buffer[..]).expect("Failed to read from bytes");
    
    assert_eq!(reconstructed.symbol_id, 2);
    assert_eq!(reconstructed.bid, 3000.0);
    assert_eq!(reconstructed.ask, 3010.0);
    assert_eq!(reconstructed.timestamp_us, 9876543210);
}

#[test]
fn test_symbol_to_id_mapping() {
    assert_eq!(symbol_to_id("BTCUSDT"), Some(1));
    assert_eq!(symbol_to_id("ETHUSDT"), Some(2));
    assert_eq!(symbol_to_id("SOLUSDT"), Some(3));
    assert_eq!(symbol_to_id("UNKNOWN"), None);
}

#[test]
fn test_id_to_symbol_mapping() {
    assert_eq!(id_to_symbol(1), Some("BTCUSDT"));
    assert_eq!(id_to_symbol(2), Some("ETHUSDT"));
    assert_eq!(id_to_symbol(3), Some("SOLUSDT"));
    assert_eq!(id_to_symbol(999), None);
}

#[test]
fn test_symbol_roundtrip() {
    let symbols = vec!["BTCUSDT", "ETHUSDT", "SOLUSDT"];
    
    for symbol in symbols {
        let id = symbol_to_id(symbol).expect("Symbol should have ID");
        let recovered = id_to_symbol(id).expect("ID should have symbol");
        assert_eq!(symbol, recovered);
    }
}

#[test]
fn test_market_update_copy_trait() {
    let update1 = MarketUpdate::new(1, 50000.0, 50100.0, 1234567890);
    let update2 = update1; // Copy, not move
    
    // Both should be usable
    assert_eq!(update1.symbol_id, update2.symbol_id);
    assert_eq!(update1.bid, update2.bid);
}

#[test]
fn test_market_update_no_heap_allocation() {
    // MarketUpdate should be stack-allocated (Copy trait)
    let update = MarketUpdate::new(1, 50000.0, 50100.0, 1234567890);
    
    // This should compile and work without heap allocation
    let stack_array = [update; 10];
    assert_eq!(stack_array.len(), 10);
    assert_eq!(stack_array[0].symbol_id, 1);
}

#[test]
fn test_market_update_realistic_scenario() {
    // Simulate receiving market data for BTC
    let btc_id = symbol_to_id("BTCUSDT").expect("BTC should be mapped");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;
    
    let update = MarketUpdate::new(btc_id, 50000.0, 50100.0, timestamp);
    
    // Verify we can calculate spread
    let spread = update.spread_bps();
    assert!(spread > 0.0);
    
    // Verify we can get symbol back
    let symbol = id_to_symbol(update.symbol_id).expect("Should get symbol");
    assert_eq!(symbol, "BTCUSDT");
}
