// Integration test for Binance best ask query
// This test requires Binance testnet credentials to be set in .env

use arbitrage2::strategy::testnet::binance_demo::BinanceDemoClient;
use arbitrage2::strategy::testnet_config::ExchangeCredentials;
use std::env;

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_get_best_ask_integration() {
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

    let result = client.get_best_ask(symbol).await;

    assert!(result.is_ok(), "Failed to fetch best ask: {:?}", result.err());

    let best_ask = result.unwrap();

    // Verify best ask is positive
    assert!(best_ask > 0.0, "Best ask should be positive");

    // Verify best ask is reasonable for BTC (between $1,000 and $1,000,000)
    assert!(best_ask > 1000.0, "Best ask should be > $1,000");
    assert!(best_ask < 1_000_000.0, "Best ask should be < $1,000,000");

    println!("✅ Best ask test passed!");
    println!("   Symbol: {}", symbol);
    println!("   Best ask: ${:.2}", best_ask);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_get_best_ask_matches_order_book() {
    // Verify that get_best_ask returns the same value as the first ask in order book depth
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "ETHUSDT";

    // Get best ask
    let best_ask_result = client.get_best_ask(symbol).await;
    assert!(best_ask_result.is_ok(), "Failed to fetch best ask");
    let best_ask = best_ask_result.unwrap();

    // Get order book depth
    let depth_result = client.get_order_book_depth(symbol, 5).await;
    assert!(depth_result.is_ok(), "Failed to fetch order book depth");
    let depth = depth_result.unwrap();

    assert!(!depth.asks.is_empty(), "Order book should have asks");

    // The best ask should match the first ask in the order book
    // Allow for small timing differences (within 0.1%)
    let order_book_best_ask = depth.asks[0].price;
    let diff_pct = ((best_ask - order_book_best_ask).abs() / order_book_best_ask) * 100.0;

    assert!(
        diff_pct < 0.1,
        "Best ask ({}) should match order book best ask ({}) within 0.1%, got {:.4}% difference",
        best_ask,
        order_book_best_ask,
        diff_pct
    );

    println!("✅ Best ask matches order book!");
    println!("   Symbol: {}", symbol);
    println!("   Best ask (direct): ${:.2}", best_ask);
    println!("   Best ask (order book): ${:.2}", order_book_best_ask);
    println!("   Difference: {:.4}%", diff_pct);
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_get_best_ask_multiple_symbols() {
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
        let result = client.get_best_ask(symbol).await;
        assert!(result.is_ok(), "Failed to fetch best ask for {}", symbol);
        
        let best_ask = result.unwrap();
        assert!(best_ask > 0.0, "Best ask for {} should be positive", symbol);
        
        println!("✅ {} best ask: ${:.2}", symbol, best_ask);
    }
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_spread_sanity_check() {
    // Verify that best ask is always greater than best bid (positive spread)
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    let symbol = "BTCUSDT";

    // Get both best bid and best ask
    let best_bid_result = client.get_best_bid(symbol).await;
    let best_ask_result = client.get_best_ask(symbol).await;

    assert!(best_bid_result.is_ok(), "Failed to fetch best bid");
    assert!(best_ask_result.is_ok(), "Failed to fetch best ask");

    let best_bid = best_bid_result.unwrap();
    let best_ask = best_ask_result.unwrap();

    // Verify spread is positive
    assert!(
        best_ask > best_bid,
        "Best ask ({}) should be greater than best bid ({})",
        best_ask,
        best_bid
    );

    let spread = best_ask - best_bid;
    let spread_bps = (spread / best_bid) * 10000.0;

    // Verify spread is reasonable (< 1% for BTC)
    assert!(
        spread_bps < 100.0,
        "Spread should be < 100 bps (1%), got {:.2} bps",
        spread_bps
    );

    println!("✅ Spread sanity check passed!");
    println!("   Symbol: {}", symbol);
    println!("   Best bid: ${:.2}", best_bid);
    println!("   Best ask: ${:.2}", best_ask);
    println!("   Spread: ${:.2} ({:.2} bps)", spread, spread_bps);
}
