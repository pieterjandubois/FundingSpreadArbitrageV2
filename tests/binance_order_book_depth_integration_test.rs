// Integration test for Binance order book depth query
// This test requires Binance testnet credentials to be set in .env

use arbitrage2::strategy::testnet::binance_demo::BinanceDemoClient;
use arbitrage2::strategy::testnet_config::ExchangeCredentials;
use std::env;

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_order_book_depth_integration() {
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
    let levels = 10;

    let result = client.get_order_book_depth(symbol, levels).await;

    assert!(result.is_ok(), "Failed to fetch order book depth: {:?}", result.err());

    let depth = result.unwrap();

    // Verify we got data
    assert!(!depth.bids.is_empty(), "Bids should not be empty");
    assert!(!depth.asks.is_empty(), "Asks should not be empty");

    // Verify we got the requested number of levels (or less if not available)
    assert!(depth.bids.len() <= levels, "Bids should not exceed requested levels");
    assert!(depth.asks.len() <= levels, "Asks should not exceed requested levels");

    // Verify bids are sorted in descending order (highest price first)
    for i in 1..depth.bids.len() {
        assert!(
            depth.bids[i - 1].price >= depth.bids[i].price,
            "Bids should be sorted in descending order"
        );
    }

    // Verify asks are sorted in ascending order (lowest price first)
    for i in 1..depth.asks.len() {
        assert!(
            depth.asks[i - 1].price <= depth.asks[i].price,
            "Asks should be sorted in ascending order"
        );
    }

    // Verify best ask is higher than best bid (no crossed market)
    if !depth.bids.is_empty() && !depth.asks.is_empty() {
        assert!(
            depth.asks[0].price > depth.bids[0].price,
            "Best ask should be higher than best bid"
        );
    }

    // Verify all prices and quantities are positive
    for bid in &depth.bids {
        assert!(bid.price > 0.0, "Bid price should be positive");
        assert!(bid.quantity > 0.0, "Bid quantity should be positive");
    }

    for ask in &depth.asks {
        assert!(ask.price > 0.0, "Ask price should be positive");
        assert!(ask.quantity > 0.0, "Ask quantity should be positive");
    }

    // Verify timestamp is recent (within last minute)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    
    assert!(
        now - depth.timestamp < 60_000,
        "Timestamp should be recent (within last minute)"
    );

    println!("✅ Order book depth test passed!");
    println!("   Symbol: {}", symbol);
    println!("   Bids: {}", depth.bids.len());
    println!("   Asks: {}", depth.asks.len());
    println!("   Best bid: {}", depth.bids[0].price);
    println!("   Best ask: {}", depth.asks[0].price);
    println!("   Spread: {:.2} bps", 
        ((depth.asks[0].price - depth.bids[0].price) / depth.bids[0].price) * 10000.0
    );
}

#[tokio::test]
#[ignore] // Ignore by default since it requires testnet credentials
async fn test_binance_order_book_depth_different_levels() {
    // Load credentials from environment
    let api_key = env::var("BINANCE_DEMO_API_KEY").expect("BINANCE_DEMO_API_KEY not set");
    let api_secret = env::var("BINANCE_DEMO_API_SECRET").expect("BINANCE_DEMO_API_SECRET not set");

    let credentials = ExchangeCredentials {
        api_key,
        api_secret,
        passphrase: None,
    };

    let client = BinanceDemoClient::new(credentials);

    // Test with different level counts
    let symbol = "ETHUSDT";
    
    for levels in [5, 10, 20] {
        let result = client.get_order_book_depth(symbol, levels).await;
        assert!(result.is_ok(), "Failed to fetch order book depth for {} levels", levels);
        
        let depth = result.unwrap();
        assert!(depth.bids.len() <= levels, "Bids should not exceed {} levels", levels);
        assert!(depth.asks.len() <= levels, "Asks should not exceed {} levels", levels);
        
        println!("✅ Fetched {} levels: {} bids, {} asks", levels, depth.bids.len(), depth.asks.len());
    }
}

#[test]
fn test_binance_order_book_depth_unit() {
    // Unit test that doesn't require credentials
    // Just verify the data structures work correctly
    use arbitrage2::strategy::types::{OrderBookDepth, PriceLevel};

    let depth = OrderBookDepth {
        bids: vec![
            PriceLevel { price: 50000.0, quantity: 1.0 },
            PriceLevel { price: 49999.0, quantity: 2.0 },
        ],
        asks: vec![
            PriceLevel { price: 50001.0, quantity: 1.5 },
            PriceLevel { price: 50002.0, quantity: 2.5 },
        ],
        timestamp: 1234567890,
    };

    assert_eq!(depth.bids.len(), 2);
    assert_eq!(depth.asks.len(), 2);
    assert_eq!(depth.bids[0].price, 50000.0);
    assert_eq!(depth.asks[0].price, 50001.0);
}
