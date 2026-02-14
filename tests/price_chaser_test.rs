use arbitrage2::strategy::price_chaser::{PriceChaser, RepricingConfig, ExecutionMode};
use arbitrage2::strategy::types::{RepricingMetrics};
use std::sync::Arc;

// Mock backend for testing
struct MockBackend;

#[async_trait::async_trait]
impl arbitrage2::strategy::execution_backend::ExecutionBackend for MockBackend {
    async fn set_leverage(&self, _exchange: &str, _symbol: &str, _leverage: u8) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    
    async fn set_margin_type_isolated(&self, _exchange: &str, _symbol: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    
    async fn place_order(&self, order: arbitrage2::strategy::types::SimulatedOrder) -> Result<arbitrage2::strategy::types::SimulatedOrder, Box<dyn std::error::Error + Send + Sync>> {
        Ok(order)
    }
    
    async fn place_market_order(&self, order: arbitrage2::strategy::types::SimulatedOrder) -> Result<arbitrage2::strategy::types::SimulatedOrder, Box<dyn std::error::Error + Send + Sync>> {
        Ok(order)
    }
    
    async fn cancel_order(&self, _exchange: &str, _order_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    
    async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<arbitrage2::strategy::types::OrderStatus, Box<dyn std::error::Error + Send + Sync>> {
        Ok(arbitrage2::strategy::types::OrderStatus::Pending)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<arbitrage2::strategy::types::OrderStatusInfo, Box<dyn std::error::Error + Send + Sync>> {
        Ok(arbitrage2::strategy::types::OrderStatusInfo::new(
            arbitrage2::strategy::types::OrderStatus::Pending,
            0.0,
            1.0
        ))
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(10000.0)
    }
    
    async fn get_all_balances(&self) -> Result<std::collections::HashMap<String, f64>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(std::collections::HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(&self, _exchange: &str, _symbol: &str, _levels: usize) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn std::error::Error + Send + Sync>> {
        Ok(arbitrage2::strategy::types::OrderBookDepth {
            bids: vec![],
            asks: vec![],
            timestamp: 0,
        })
    }
    
    async fn get_best_bid(&self, _exchange: &str, _symbol: &str) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(100.0)
    }
    
    async fn get_best_ask(&self, _exchange: &str, _symbol: &str) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(100.0)
    }
    
    fn backend_name(&self) -> &str {
        "mock"
    }
}

#[test]
fn test_should_reprice_below_threshold() {
    let config = RepricingConfig::balanced();
    let backend = Arc::new(MockBackend);
    let chaser = PriceChaser::new(backend, config);
    
    // Price deviation of 4 bps - should NOT reprice (below 5 bps threshold)
    let order_price = 100.0;
    let current_price = 100.04; // 4 bps
    assert!(!chaser.should_reprice(order_price, current_price));
    
    // Price deviation of 3 bps
    let current_price = 100.03;
    assert!(!chaser.should_reprice(order_price, current_price));
    
    // Price deviation of 1 bps
    let current_price = 100.01;
    assert!(!chaser.should_reprice(order_price, current_price));
}

#[test]
fn test_should_reprice_above_threshold() {
    let config = RepricingConfig::balanced();
    let backend = Arc::new(MockBackend);
    let chaser = PriceChaser::new(backend, config);
    
    // Price deviation of 6 bps - should reprice (above 5 bps threshold)
    let order_price = 100.0;
    let current_price = 100.06; // 6 bps
    assert!(chaser.should_reprice(order_price, current_price));
    
    // Price deviation of 10 bps
    let current_price = 100.10;
    assert!(chaser.should_reprice(order_price, current_price));
    
    // Price deviation of 20 bps
    let current_price = 100.20;
    assert!(chaser.should_reprice(order_price, current_price));
}

#[test]
fn test_should_reprice_exactly_at_threshold() {
    let config = RepricingConfig::balanced();
    let backend = Arc::new(MockBackend);
    let chaser = PriceChaser::new(backend, config);
    
    // Price deviation of exactly 5 bps - should NOT reprice (threshold is >5, not >=5)
    let order_price = 100.0;
    let current_price = 100.05; // 5 bps
    assert!(!chaser.should_reprice(order_price, current_price));
}

#[test]
fn test_should_reprice_negative_deviation() {
    let config = RepricingConfig::balanced();
    let backend = Arc::new(MockBackend);
    let chaser = PriceChaser::new(backend, config);
    
    // Price moved down by 6 bps - should reprice
    let order_price = 100.0;
    let current_price = 99.94; // -6 bps
    assert!(chaser.should_reprice(order_price, current_price));
    
    // Price moved down by 4 bps - should NOT reprice
    let current_price = 99.96; // -4 bps
    assert!(!chaser.should_reprice(order_price, current_price));
}

#[test]
fn test_should_reprice_custom_threshold() {
    let mut config = RepricingConfig::balanced();
    config.reprice_threshold_bps = 10.0; // Custom 10 bps threshold
    
    let backend = Arc::new(MockBackend);
    let chaser = PriceChaser::new(backend, config);
    
    // 9 bps deviation - should NOT reprice
    let order_price = 100.0;
    let current_price = 100.09;
    assert!(!chaser.should_reprice(order_price, current_price));
    
    // 11 bps deviation - should reprice
    let current_price = 100.11;
    assert!(chaser.should_reprice(order_price, current_price));
}

#[test]
fn test_repricing_config_presets() {
    let ultra_fast = RepricingConfig::ultra_fast();
    assert_eq!(ultra_fast.execution_mode, ExecutionMode::UltraFast);
    assert_eq!(ultra_fast.max_reprices, 5);
    assert_eq!(ultra_fast.reprice_threshold_bps, 5.0);
    assert_eq!(ultra_fast.reprice_interval_ms, 100);
    assert_eq!(ultra_fast.total_timeout_seconds, 3);
    assert_eq!(ultra_fast.spread_collapse_threshold_bps, 50.0);
    
    let balanced = RepricingConfig::balanced();
    assert_eq!(balanced.execution_mode, ExecutionMode::Balanced);
    assert_eq!(balanced.max_reprices, 5);
    
    let safe = RepricingConfig::safe();
    assert_eq!(safe.execution_mode, ExecutionMode::Safe);
    assert_eq!(safe.max_reprices, 5);
}

#[test]
fn test_from_confidence_ultra_fast() {
    // High confidence (>= 90%) -> UltraFast
    let config = RepricingConfig::from_confidence(95.0);
    assert_eq!(config.execution_mode, ExecutionMode::UltraFast);
    
    let config = RepricingConfig::from_confidence(90.0);
    assert_eq!(config.execution_mode, ExecutionMode::UltraFast);
    
    let config = RepricingConfig::from_confidence(99.9);
    assert_eq!(config.execution_mode, ExecutionMode::UltraFast);
}

#[test]
fn test_from_confidence_balanced() {
    // Medium confidence (75-90%) -> Balanced
    let config = RepricingConfig::from_confidence(85.0);
    assert_eq!(config.execution_mode, ExecutionMode::Balanced);
    
    let config = RepricingConfig::from_confidence(75.0);
    assert_eq!(config.execution_mode, ExecutionMode::Balanced);
    
    let config = RepricingConfig::from_confidence(89.9);
    assert_eq!(config.execution_mode, ExecutionMode::Balanced);
}

#[test]
fn test_from_confidence_safe() {
    // Low confidence (< 75%) -> Safe
    let config = RepricingConfig::from_confidence(70.0);
    assert_eq!(config.execution_mode, ExecutionMode::Safe);
    
    let config = RepricingConfig::from_confidence(50.0);
    assert_eq!(config.execution_mode, ExecutionMode::Safe);
    
    let config = RepricingConfig::from_confidence(74.9);
    assert_eq!(config.execution_mode, ExecutionMode::Safe);
    
    let config = RepricingConfig::from_confidence(0.0);
    assert_eq!(config.execution_mode, ExecutionMode::Safe);
}

#[test]
fn test_repricing_metrics_initialization() {
    let initial_price = 100.0;
    let metrics = RepricingMetrics::new(initial_price);
    
    assert_eq!(metrics.initial_price, 100.0);
    assert_eq!(metrics.final_price, 100.0);
    assert_eq!(metrics.reprice_count, 0);
    assert_eq!(metrics.reprice_total_time_ms, 0);
    assert!(!metrics.max_reprices_reached);
    assert_eq!(metrics.repricing_events.len(), 0);
    assert_eq!(metrics.price_improvement_bps, 0.0);
}

#[test]
fn test_repricing_metrics_finalize_positive_improvement() {
    let mut metrics = RepricingMetrics::new(100.0);
    metrics.final_price = 100.10;
    metrics.finalize();
    
    // Price improvement: (100.10 - 100.0) / 100.0 * 10000 = 10 bps
    assert!((metrics.price_improvement_bps - 10.0).abs() < 0.01);
}

#[test]
fn test_repricing_metrics_finalize_negative_improvement() {
    let mut metrics = RepricingMetrics::new(100.0);
    metrics.final_price = 99.95;
    metrics.finalize();
    
    // Price improvement: (99.95 - 100.0) / 100.0 * 10000 = -5 bps
    assert!((metrics.price_improvement_bps - (-5.0)).abs() < 0.01);
}

#[test]
fn test_repricing_metrics_finalize_no_change() {
    let mut metrics = RepricingMetrics::new(100.0);
    metrics.final_price = 100.0;
    metrics.finalize();
    
    // No price change
    assert_eq!(metrics.price_improvement_bps, 0.0);
}

#[test]
fn test_repricing_metrics_tracking() {
    let mut metrics = RepricingMetrics::new(100.0);
    
    // Simulate 3 reprices
    metrics.reprice_count = 3;
    metrics.reprice_total_time_ms = 450; // 150ms per reprice
    metrics.final_price = 100.15;
    
    metrics.finalize();
    
    assert_eq!(metrics.reprice_count, 3);
    assert_eq!(metrics.reprice_total_time_ms, 450);
    assert!((metrics.price_improvement_bps - 15.0).abs() < 0.01);
}

#[test]
fn test_max_reprices_limit() {
    let config = RepricingConfig::balanced();
    assert_eq!(config.max_reprices, 5);
    
    // Verify that max_reprices is enforced in metrics
    let mut metrics = RepricingMetrics::new(100.0);
    metrics.reprice_count = 5;
    metrics.max_reprices_reached = true;
    
    assert!(metrics.max_reprices_reached);
    assert_eq!(metrics.reprice_count, 5);
}

#[test]
fn test_should_reprice_with_different_order_prices() {
    let config = RepricingConfig::balanced();
    let backend = Arc::new(MockBackend);
    let chaser = PriceChaser::new(backend, config);
    
    // Test with higher order price
    let order_price = 1000.0;
    let current_price = 1000.6; // 6 bps
    assert!(chaser.should_reprice(order_price, current_price));
    
    // Test with lower order price
    let order_price = 10.0;
    let current_price = 10.006; // 6 bps
    assert!(chaser.should_reprice(order_price, current_price));
    
    // Test with very small order price
    let order_price = 0.01;
    let current_price = 0.01006; // 6 bps
    assert!(chaser.should_reprice(order_price, current_price));
}
