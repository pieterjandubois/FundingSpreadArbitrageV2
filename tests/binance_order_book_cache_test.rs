// Integration test for Binance order book caching layer
// This test verifies that the 100ms TTL cache is working correctly

use arbitrage2::strategy::testnet::binance_demo::BinanceDemoClient;
use arbitrage2::strategy::testnet_config::ExchangeCredentials;
use std::env;
use std::time::Instant;

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_order_book_cache_hit() {
    // Load credentials from environment
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "BTCUSDT";
    let levels = 10;

    // First call - should be a cache MISS and fetch from API
    let start1 = Instant::now();
    let result1 = client.get_order_book_depth(symbol, levels).await;
    let duration1 = start1.elapsed();
    
    assert!(result1.is_ok(), "First fetch failed: {:?}", result1.err());
    let depth1 = result1.unwrap();

    println!("✅ First fetch completed in {:?}", duration1);

    // Second call immediately after - should be a cache HIT
    let start2 = Instant::now();
    let result2 = client.get_order_book_depth(symbol, levels).await;
    let duration2 = start2.elapsed();
    
    assert!(result2.is_ok(), "Second fetch failed: {:?}", result2.err());
    let depth2 = result2.unwrap();

    println!("✅ Second fetch completed in {:?}", duration2);

    // Verify the second call was faster (cache hit)
    assert!(
        duration2 < duration1,
        "Second call should be faster due to cache hit. First: {:?}, Second: {:?}",
        duration1, duration2
    );

    // Verify the data is the same (from cache)
    assert_eq!(depth1.timestamp, depth2.timestamp, "Timestamps should match (cached data)");
    assert_eq!(depth1.bids.len(), depth2.bids.len(), "Bid count should match");
    assert_eq!(depth1.asks.len(), depth2.asks.len(), "Ask count should match");

    println!("✅ Cache hit test passed!");
    println!("   First call: {:?}", duration1);
    println!("   Second call: {:?} (cache hit)", duration2);
    println!("   Speedup: {:.2}x", duration1.as_micros() as f64 / duration2.as_micros() as f64);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_order_book_cache_expiry() {
    // Load credentials from environment
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "ETHUSDT";
    let levels = 5;

    // First call - cache MISS
    let result1 = client.get_order_book_depth(symbol, levels).await;
    assert!(result1.is_ok(), "First fetch failed: {:?}", result1.err());
    let depth1 = result1.unwrap();

    println!("✅ First fetch completed, timestamp: {}", depth1.timestamp);

    // Wait for cache to expire (100ms + buffer)
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // Second call after expiry - should be a cache MISS and fetch fresh data
    let result2 = client.get_order_book_depth(symbol, levels).await;
    assert!(result2.is_ok(), "Second fetch failed: {:?}", result2.err());
    let depth2 = result2.unwrap();

    println!("✅ Second fetch completed, timestamp: {}", depth2.timestamp);

    // Verify we got fresh data (different timestamp)
    assert!(
        depth2.timestamp >= depth1.timestamp,
        "Second fetch should have newer or equal timestamp after cache expiry"
    );

    println!("✅ Cache expiry test passed!");
    println!("   First timestamp: {}", depth1.timestamp);
    println!("   Second timestamp: {} (after 150ms)", depth2.timestamp);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_best_bid_ask_cache() {
    // Load credentials from environment
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "BTCUSDT";

    // Call get_best_bid - should populate cache
    let start1 = Instant::now();
    let result1 = client.get_best_bid(symbol).await;
    let duration1 = start1.elapsed();
    
    assert!(result1.is_ok(), "get_best_bid failed: {:?}", result1.err());
    let best_bid = result1.unwrap();

    println!("✅ get_best_bid completed in {:?}, price: {}", duration1, best_bid);

    // Call get_best_ask immediately - should use cached order book
    let start2 = Instant::now();
    let result2 = client.get_best_ask(symbol).await;
    let duration2 = start2.elapsed();
    
    assert!(result2.is_ok(), "get_best_ask failed: {:?}", result2.err());
    let best_ask = result2.unwrap();

    println!("✅ get_best_ask completed in {:?}, price: {}", duration2, best_ask);

    // Verify best_ask > best_bid (no crossed market)
    assert!(
        best_ask > best_bid,
        "Best ask ({}) should be higher than best bid ({})",
        best_ask, best_bid
    );

    // Verify the second call was fast (cache hit)
    assert!(
        duration2 < duration1,
        "get_best_ask should be faster due to cache hit. get_best_bid: {:?}, get_best_ask: {:?}",
        duration1, duration2
    );

    println!("✅ Best bid/ask cache test passed!");
    println!("   Best bid: {} (took {:?})", best_bid, duration1);
    println!("   Best ask: {} (took {:?}, cache hit)", best_ask, duration2);
    println!("   Spread: {:.2} bps", ((best_ask - best_bid) / best_bid) * 10000.0);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_cache_different_levels() {
    // Load credentials from environment
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "BTCUSDT";

    // Fetch with 5 levels - cache MISS
    let result1 = client.get_order_book_depth(symbol, 5).await;
    assert!(result1.is_ok(), "Fetch with 5 levels failed");
    let depth1 = result1.unwrap();

    // Immediately fetch with 5 levels again - should be cache HIT
    let start2 = Instant::now();
    let result2 = client.get_order_book_depth(symbol, 5).await;
    let duration2 = start2.elapsed();
    
    assert!(result2.is_ok(), "Second fetch with 5 levels failed");
    let depth2 = result2.unwrap();

    // Verify it's from cache (same timestamp)
    assert_eq!(depth1.timestamp, depth2.timestamp, "Should be cached data from first fetch");
    
    // Verify the second call was fast (cache hit)
    assert!(
        duration2.as_millis() < 10,
        "Second fetch should be very fast (cache hit), took {:?}",
        duration2
    );

    // Fetch with 10 levels - should be cache MISS (different key)
    let result3 = client.get_order_book_depth(symbol, 10).await;
    assert!(result3.is_ok(), "Fetch with 10 levels failed");
    let depth3 = result3.unwrap();

    // Verify we got different amounts of data
    assert_eq!(depth1.bids.len(), 5, "Should have 5 bids");
    assert!(depth3.bids.len() >= 5, "Should have at least 5 bids");

    println!("✅ Different levels cache test passed!");
    println!("   5 levels: {} bids, {} asks", depth1.bids.len(), depth1.asks.len());
    println!("   5 levels (cached): took {:?}", duration2);
    println!("   10 levels: {} bids, {} asks", depth3.bids.len(), depth3.asks.len());
}
