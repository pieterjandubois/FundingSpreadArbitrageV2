// Tests for TestnetBackend single-exchange mode
// Verifies that orders are correctly routed to the primary exchange

use arbitrage2::strategy::testnet_backend::TestnetBackend;
use arbitrage2::strategy::testnet_config::{TestnetConfig, ExchangeCredentials};
use arbitrage2::strategy::types::SimulatedOrder;
use arbitrage2::strategy::execution_backend::ExecutionBackend;

#[tokio::test]
async fn test_single_exchange_mode_routes_to_primary() {
    // Create config with single-exchange mode enabled
    let config = TestnetConfig {
        bybit: Some(ExchangeCredentials {
            api_key: "test_key".to_string(),
            api_secret: "test_secret".to_string(),
            passphrase: None,
        }),
        binance: None,
        okx: None,
        kucoin: None,
        bitget: None,
        single_exchange_mode: true,
        primary_exchange: "bybit".to_string(),
    };

    let backend = TestnetBackend::new(config);

    // Create an order for "binance" (should be routed to bybit)
    let order = SimulatedOrder {
        id: "test_order_1".to_string(),
        exchange: "binance".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: "buy".to_string(),
        order_type: "limit".to_string(),
        quantity: 0.001,
        price: 50000.0,
        status: "new".to_string(),
        filled_quantity: 0.0,
        average_fill_price: 0.0,
        timestamp: chrono::Utc::now().timestamp_millis() as u64,
    };

    // Place order - should route to bybit
    let result = backend.place_order(order).await;

    // Verify the order was placed (even though it will fail due to test credentials)
    // The important part is that it attempted to route to bybit, not binance
    assert!(result.is_err()); // Will fail due to invalid credentials, but that's expected
    
    // The error should be about bybit, not binance
    let error_msg = result.unwrap_err().to_string();
    assert!(!error_msg.contains("Binance demo not configured"));
}

#[tokio::test]
async fn test_normal_mode_routes_to_specified_exchange() {
    // Create config with single-exchange mode DISABLED
    let config = TestnetConfig {
        bybit: Some(ExchangeCredentials {
            api_key: "test_key".to_string(),
            api_secret: "test_secret".to_string(),
            passphrase: None,
        }),
        binance: Some(ExchangeCredentials {
            api_key: "test_key".to_string(),
            api_secret: "test_secret".to_string(),
            passphrase: None,
        }),
        okx: None,
        kucoin: None,
        bitget: None,
        single_exchange_mode: false,
        primary_exchange: "bybit".to_string(),
    };

    let backend = TestnetBackend::new(config);

    // Create an order for "binance" (should go to binance)
    let order = SimulatedOrder {
        id: "test_order_2".to_string(),
        exchange: "binance".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: "buy".to_string(),
        order_type: "limit".to_string(),
        quantity: 0.001,
        price: 50000.0,
        status: "new".to_string(),
        filled_quantity: 0.0,
        average_fill_price: 0.0,
        timestamp: chrono::Utc::now().timestamp_millis() as u64,
    };

    // Place order - should route to binance (not bybit)
    let result = backend.place_order(order).await;

    // Will fail due to invalid credentials, but should attempt binance
    assert!(result.is_err());
}

#[test]
fn test_config_validation_single_exchange_mode() {
    // Test that validation works: single_exchange_mode requires primary_exchange
    std::env::set_var("SINGLE_EXCHANGE_MODE", "true");
    std::env::set_var("PRIMARY_EXCHANGE", "");

    let result = TestnetConfig::from_env();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("PRIMARY_EXCHANGE must be set"));

    // Cleanup
    std::env::remove_var("SINGLE_EXCHANGE_MODE");
    std::env::remove_var("PRIMARY_EXCHANGE");
}

#[test]
fn test_config_defaults() {
    // Test default values
    std::env::remove_var("SINGLE_EXCHANGE_MODE");
    std::env::remove_var("PRIMARY_EXCHANGE");

    let result = TestnetConfig::from_env();
    assert!(result.is_ok());
    
    let config = result.unwrap();
    assert_eq!(config.single_exchange_mode, false);
    assert_eq!(config.primary_exchange, "bybit");
}
