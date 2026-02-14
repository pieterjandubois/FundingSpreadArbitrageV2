// Integration test for Binance best bid query
// This test requires Binance testnet credentials to be set in .env

use arbitrage2::strategy::testnet::binance_demo::BinanceDemoClient;
use arbitrage2::strategy::testnet_config::ExchangeCredentials;
use std::env;

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_get_best_bid_integration() {
    // Load credentials from environment
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    // Test with BTCUSDT - a highly liquid pair
    let symbol = "BTCUSDT";

    let result = client.get_best_bid(symbol).await;

    assert!(result.is_ok(), "Failed to fetch best bid: {:?}", result.err());

    let best_bid = result.unwrap();

    // Verify best bid is positive
    assert!(best_bid > 0.0, "Best bid should be positive");

    // Verify best bid is reasonable for BTC (between $1,000 and $1,000,000)
    assert!(best_bid > 1000.0, "Best bid should be > $1,000");
    assert!(best_bid < 1_000_000.0, "Best bid should be < $1,000,000");

    println!("✅ Best bid test passed!");
    println!("   Symbol: {}", symbol);
    println!("   Best bid: ${:.2}", best_bid);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_get_best_bid_matches_order_book() {
    // Verify that get_best_bid returns the same value as the first bid in order book depth
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "ETHUSDT";

    // Get best bid
    let best_bid_result = client.get_best_bid(symbol).await;
    assert!(best_bid_result.is_ok(), "Failed to fetch best bid");
    let best_bid = best_bid_result.unwrap();

    // Get order book depth
    let depth_result = client.get_order_book_depth(symbol, 5).await;
    assert!(depth_result.is_ok(), "Failed to fetch order book depth");
    let depth = depth_result.unwrap();

    assert!(!depth.bids.is_empty(), "Order book should have bids");

    // The best bid should match the first bid in the order book
    // Allow for small timing differences (within 0.1%)
    let order_book_best_bid = depth.bids[0].price;
    let diff_pct = ((best_bid - order_book_best_bid).abs() / order_book_best_bid) * 100.0;

    assert!(
        diff_pct < 0.1,
        "Best bid ({}) should match order book best bid ({}) within 0.1%, got {:.4}% difference",
        best_bid,
        order_book_best_bid,
        diff_pct
    );

    println!("✅ Best bid matches order book!");
    println!("   Symbol: {}", symbol);
    println!("   Best bid (direct): ${:.2}", best_bid);
    println!("   Best bid (order book): ${:.2}", order_book_best_bid);
    println!("   Difference: {:.4}%", diff_pct);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_get_best_bid_multiple_symbols() {
    // Test with multiple symbols to ensure it works across different pairs
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbols = ["BTCUSDT", "ETHUSDT", "BNBUSDT"];

    for symbol in symbols {
        let result = client.get_best_bid(symbol).await;
        assert!(result.is_ok(), "Failed to fetch best bid for {}", symbol);
        
        let best_bid = result.unwrap();
        assert!(best_bid > 0.0, "Best bid for {} should be positive", symbol);
        
        println!("✅ {} best bid: ${:.2}", symbol, best_bid);
    }
}
