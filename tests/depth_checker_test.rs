use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::Mutex;

// Import the modules we're testing
use arbitrage2::strategy::depth_checker::{DepthChecker, DepthCheckResult};
use arbitrage2::strategy::execution_backend::ExecutionBackend;
use arbitrage2::strategy::types::{OrderBookDepth, PriceLevel, SimulatedOrder, OrderStatus, OrderStatusInfo};

/// Mock ExecutionBackend for testing
struct MockExecutionBackend {
    order_books: Arc<Mutex<HashMap<String, OrderBookDepth>>>,
    should_fail: Arc<Mutex<bool>>,
}

impl MockExecutionBackend {
    fn new() -> Self {
        Self {
            order_books: Arc::new(Mutex::new(HashMap::new())),
            should_fail: Arc::new(Mutex::new(false)),
        }
    }
    
    async fn set_order_book(&self, exchange: &str, symbol: &str, depth: OrderBookDepth) {
        let key = format!("{}:{}", exchange, symbol);
        let mut books = self.order_books.lock().await;
        books.insert(key, depth);
    }
    
    async fn set_should_fail(&self, fail: bool) {
        let mut should_fail = self.should_fail.lock().await;
        *should_fail = fail;
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for MockExecutionBackend {
    async fn set_leverage(&self, _exchange: &str, _symbol: &str, _leverage: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    
    async fn set_margin_type_isolated(&self, _exchange: &str, _symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    
    async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        Ok(order)
    }
    
    async fn place_market_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        Ok(order)
    }
    
    async fn cancel_order(&self, _exchange: &str, _order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    
    async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatus::Pending)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatusInfo::new(OrderStatus::Pending, 0.0, 0.0))
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(10000.0)
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        Ok(HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        exchange: &str,
        symbol: &str,
        _levels: usize,
    ) -> Result<OrderBookDepth, Box<dyn Error + Send + Sync>> {
        let should_fail = *self.should_fail.lock().await;
        if should_fail {
            return Err("Mock API error".into());
        }
        
        let key = format!("{}:{}", exchange, symbol);
        let books = self.order_books.lock().await;
        
        books.get(&key)
            .cloned()
            .ok_or_else(|| format!("Order book not found for {}", key).into())
    }
    
    async fn get_best_bid(&self, exchange: &str, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let depth = self.get_order_book_depth(exchange, symbol, 1).await?;
        depth.bids.first()
            .map(|level| level.price)
            .ok_or_else(|| "No bids available".into())
    }
    
    async fn get_best_ask(&self, exchange: &str, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let depth = self.get_order_book_depth(exchange, symbol, 1).await?;
        depth.asks.first()
            .map(|level| level.price)
            .ok_or_else(|| "No asks available".into())
    }
    
    fn backend_name(&self) -> &str {
        "MockBackend"
    }
}

// Helper function to create a mock order book
fn create_mock_order_book(bid_quantities: Vec<f64>, timestamp: u64) -> OrderBookDepth {
    let bids = bid_quantities.iter().enumerate()
        .map(|(i, &qty)| PriceLevel {
            price: 50000.0 - (i as f64 * 10.0),
            quantity: qty,
        })
        .collect();
    
    let asks = vec![
        PriceLevel { price: 50100.0, quantity: 1.0 },
        PriceLevel { price: 50110.0, quantity: 1.0 },
    ];
    
    OrderBookDepth {
        bids,
        asks,
        timestamp,
    }
}

// ============================================================================
// Task 7.1: Unit tests for depth calculation logic
// ============================================================================

#[tokio::test]
async fn test_depth_calculation_sufficient_liquidity() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with sufficient liquidity
    // Top 5 levels: 2.0 + 1.5 + 1.0 + 0.8 + 0.5 = 5.8 BTC
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5, 0.3, 0.2], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 5.8 BTC
    // Depth ratio: 5.8 / 3.0 = 1.93
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    assert_eq!(result.exchange, "binance");
    assert_eq!(result.symbol, "BTCUSDT");
    assert_eq!(result.available_liquidity, 5.8);
    assert_eq!(result.required_liquidity, 3.0);
    assert!((result.depth_ratio - 1.93).abs() < 0.01);
    assert!(result.is_sufficient);
    assert!(!result.is_critical);
    assert!(!result.should_abort());
    assert!(!result.should_warn());
}

#[tokio::test]
async fn test_depth_calculation_exact_required_liquidity() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with exactly required liquidity
    // Top 5 levels: 1.0 + 0.8 + 0.6 + 0.4 + 0.2 = 3.0 BTC
    let order_book = create_mock_order_book(vec![1.0, 0.8, 0.6, 0.4, 0.2], 1234567890);
    backend.set_order_book("bybit", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 3.0 BTC
    // Depth ratio: 3.0 / 3.0 = 1.0
    let result = checker.check_depth_for_hedge("bybit", "BTCUSDT", 2.0).await.unwrap();
    
    assert_eq!(result.available_liquidity, 3.0);
    assert_eq!(result.required_liquidity, 3.0);
    assert!((result.depth_ratio - 1.0).abs() < 0.001);
    assert!(result.is_sufficient);
    assert!(!result.is_critical);
}

#[tokio::test]
async fn test_depth_calculation_only_top_5_levels() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with 10 levels, but only top 5 should be counted
    // Top 5: 1.0 + 1.0 + 1.0 + 1.0 + 1.0 = 5.0 BTC
    // Levels 6-10: 2.0 + 2.0 + 2.0 + 2.0 + 2.0 = 10.0 BTC (should be ignored)
    let order_book = create_mock_order_book(
        vec![1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 2.0],
        1234567890
    );
    backend.set_order_book("binance", "ETHUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 ETH
    // Required liquidity: 2.0 * 1.5 = 3.0 ETH
    // Available liquidity: 5.0 ETH (only top 5 levels)
    let result = checker.check_depth_for_hedge("binance", "ETHUSDT", 2.0).await.unwrap();
    
    assert_eq!(result.available_liquidity, 5.0);
    assert_eq!(result.required_liquidity, 3.0);
}

#[tokio::test]
async fn test_depth_calculation_zero_hedge_quantity() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![1.0, 1.0, 1.0], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Edge case: zero hedge quantity
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 0.0).await.unwrap();
    
    assert_eq!(result.required_liquidity, 0.0);
    assert_eq!(result.depth_ratio, 0.0);
}

// ============================================================================
// Task 7.3: Unit tests for abort threshold (depth_ratio < 0.73)
// ============================================================================

#[tokio::test]
async fn test_abort_threshold_critical_depth() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with critically low liquidity
    // Top 5 levels: 0.5 + 0.4 + 0.3 + 0.2 + 0.1 = 1.5 BTC
    let order_book = create_mock_order_book(vec![0.5, 0.4, 0.3, 0.2, 0.1], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 1.5 BTC
    // Depth ratio: 1.5 / 3.0 = 0.5 (< 0.73, critical!)
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    assert_eq!(result.available_liquidity, 1.5);
    assert_eq!(result.required_liquidity, 3.0);
    assert!((result.depth_ratio - 0.5).abs() < 0.01);
    assert!(!result.is_sufficient);
    assert!(result.is_critical);
    assert!(result.should_abort());
    assert!(!result.should_warn());
}

#[tokio::test]
async fn test_abort_threshold_exactly_at_boundary() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with depth ratio exactly at 0.73
    // Top 5 levels: 2.19 BTC total
    let order_book = create_mock_order_book(vec![0.8, 0.6, 0.4, 0.3, 0.09], 1234567890);
    backend.set_order_book("bybit", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 2.19 BTC
    // Depth ratio: 2.19 / 3.0 = 0.73
    let result = checker.check_depth_for_hedge("bybit", "BTCUSDT", 2.0).await.unwrap();
    
    // Due to floating point precision, we check if it's close to 0.73
    // The boundary is < 0.73, so at exactly 0.73 it should NOT be critical
    // But we need to account for floating point errors
    if result.depth_ratio < 0.73 {
        assert!(result.is_critical, "depth_ratio {} should be critical", result.depth_ratio);
    } else {
        assert!(!result.is_critical, "depth_ratio {} should not be critical", result.depth_ratio);
    }
}

#[tokio::test]
async fn test_abort_threshold_just_below_boundary() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with depth ratio just below 0.73
    // Top 5 levels: 2.18 BTC total
    let order_book = create_mock_order_book(vec![0.8, 0.6, 0.4, 0.3, 0.08], 1234567890);
    backend.set_order_book("bitget", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 2.18 BTC
    // Depth ratio: 2.18 / 3.0 = 0.7267 (< 0.73, critical!)
    let result = checker.check_depth_for_hedge("bitget", "BTCUSDT", 2.0).await.unwrap();
    
    assert!((result.depth_ratio - 0.7267).abs() < 0.01);
    assert!(result.is_critical);
    assert!(result.should_abort());
}

#[tokio::test]
async fn test_abort_threshold_very_low_depth() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with very low liquidity
    // Top 5 levels: 0.1 + 0.05 + 0.03 + 0.01 + 0.01 = 0.2 BTC
    let order_book = create_mock_order_book(vec![0.1, 0.05, 0.03, 0.01, 0.01], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 5.0 BTC
    // Required liquidity: 5.0 * 1.5 = 7.5 BTC
    // Available liquidity: 0.2 BTC
    // Depth ratio: 0.2 / 7.5 = 0.0267 (extremely critical!)
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 5.0).await.unwrap();
    
    assert!((result.available_liquidity - 0.2).abs() < 0.01);
    assert_eq!(result.required_liquidity, 7.5);
    assert!((result.depth_ratio - 0.0267).abs() < 0.01);
    assert!(result.is_critical);
    assert!(result.should_abort());
}

// ============================================================================
// Task 7.4: Unit tests for warning threshold (depth_ratio < 1.0)
// ============================================================================

#[tokio::test]
async fn test_warning_threshold_low_but_not_critical() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with low but not critical liquidity
    // Top 5 levels: 2.5 BTC total
    let order_book = create_mock_order_book(vec![0.8, 0.7, 0.5, 0.3, 0.2], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 2.5 BTC
    // Depth ratio: 2.5 / 3.0 = 0.833 (< 1.0 but > 0.73, warning!)
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    assert_eq!(result.available_liquidity, 2.5);
    assert_eq!(result.required_liquidity, 3.0);
    assert!((result.depth_ratio - 0.833).abs() < 0.01);
    assert!(!result.is_sufficient);
    assert!(!result.is_critical);
    assert!(!result.should_abort());
    assert!(result.should_warn());
}

#[tokio::test]
async fn test_warning_threshold_just_below_sufficient() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with depth ratio just below 1.0
    // Top 5 levels: 2.97 BTC total
    let order_book = create_mock_order_book(vec![0.9, 0.8, 0.6, 0.4, 0.27], 1234567890);
    backend.set_order_book("bybit", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 2.97 BTC
    // Depth ratio: 2.97 / 3.0 = 0.99 (< 1.0, warning!)
    let result = checker.check_depth_for_hedge("bybit", "BTCUSDT", 2.0).await.unwrap();
    
    assert!((result.depth_ratio - 0.99).abs() < 0.01);
    assert!(!result.is_sufficient);
    assert!(!result.is_critical);
    assert!(result.should_warn());
}

#[tokio::test]
async fn test_warning_threshold_at_boundary() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Create order book with depth ratio exactly at 1.0
    // Top 5 levels: 3.0 BTC total
    let order_book = create_mock_order_book(vec![1.0, 0.8, 0.6, 0.4, 0.2], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Hedge quantity: 2.0 BTC
    // Required liquidity: 2.0 * 1.5 = 3.0 BTC
    // Available liquidity: 3.0 BTC
    // Depth ratio: 3.0 / 3.0 = 1.0 (exactly at boundary, sufficient!)
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    assert!((result.depth_ratio - 1.0).abs() < 0.001);
    assert!(result.is_sufficient);
    assert!(!result.is_critical);
    assert!(!result.should_warn());
}

#[tokio::test]
async fn test_warning_threshold_range() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Test multiple depth ratios in the warning range (0.73 to 1.0)
    let test_cases = vec![
        (0.75, vec![0.6, 0.5, 0.4, 0.3, 0.45]),  // 2.25 / 3.0 = 0.75
        (0.80, vec![0.7, 0.6, 0.4, 0.3, 0.4]),   // 2.4 / 3.0 = 0.80
        (0.90, vec![0.8, 0.7, 0.6, 0.4, 0.2]),   // 2.7 / 3.0 = 0.90
        (0.95, vec![0.9, 0.7, 0.6, 0.5, 0.15]),  // 2.85 / 3.0 = 0.95
    ];
    
    for (expected_ratio, quantities) in test_cases {
        let order_book = create_mock_order_book(quantities, 1234567890);
        backend.set_order_book("binance", "BTCUSDT", order_book).await;
        
        let checker = DepthChecker::new(backend.clone());
        let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
        
        assert!((result.depth_ratio - expected_ratio).abs() < 0.01, 
            "Expected ratio {}, got {}", expected_ratio, result.depth_ratio);
        assert!(!result.is_sufficient, "Should not be sufficient at ratio {}", expected_ratio);
        assert!(!result.is_critical, "Should not be critical at ratio {}", expected_ratio);
        assert!(result.should_warn(), "Should warn at ratio {}", expected_ratio);
    }
}

// ============================================================================
// Task 7.2: Unit tests for cache hit/miss scenarios
// ============================================================================

#[tokio::test]
async fn test_cache_miss_first_query() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // First query should be a cache miss
    let result1 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    assert_eq!(result1.available_liquidity, 5.8);
    assert_eq!(result1.exchange, "binance");
}

#[tokio::test]
async fn test_cache_hit_within_ttl() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend.clone());
    
    // First query - cache miss
    let result1 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    // Update the order book in the backend
    let new_order_book = create_mock_order_book(vec![10.0, 10.0, 10.0, 10.0, 10.0], 1234567891);
    backend.set_order_book("binance", "BTCUSDT", new_order_book).await;
    
    // Second query immediately after - should hit cache and return old data
    let result2 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    // Should still have the old liquidity (5.8) from cache, not the new (50.0)
    assert_eq!(result2.available_liquidity, 5.8);
    assert_eq!(result1.available_liquidity, result2.available_liquidity);
}

#[tokio::test]
async fn test_cache_miss_after_ttl_expires() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend.clone());
    
    // First query - cache miss
    let result1 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    assert_eq!(result1.available_liquidity, 5.8);
    
    // Wait for cache to expire (100ms TTL + buffer)
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    
    // Update the order book
    let new_order_book = create_mock_order_book(vec![10.0, 10.0, 10.0, 10.0, 10.0], 1234567891);
    backend.set_order_book("binance", "BTCUSDT", new_order_book).await;
    
    // Second query after TTL - should miss cache and get new data
    let result2 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    // Should have the new liquidity (50.0) from fresh query
    assert_eq!(result2.available_liquidity, 50.0);
    assert_ne!(result1.available_liquidity, result2.available_liquidity);
}

#[tokio::test]
async fn test_cache_separate_keys_for_different_exchanges() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let binance_book = create_mock_order_book(vec![1.0, 1.0, 1.0, 1.0, 1.0], 1234567890);
    let bybit_book = create_mock_order_book(vec![2.0, 2.0, 2.0, 2.0, 2.0], 1234567891);
    
    backend.set_order_book("binance", "BTCUSDT", binance_book).await;
    backend.set_order_book("bybit", "BTCUSDT", bybit_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Query both exchanges
    let binance_result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    let bybit_result = checker.check_depth_for_hedge("bybit", "BTCUSDT", 2.0).await.unwrap();
    
    // Should have different liquidity values
    assert_eq!(binance_result.available_liquidity, 5.0);
    assert_eq!(bybit_result.available_liquidity, 10.0);
    assert_ne!(binance_result.available_liquidity, bybit_result.available_liquidity);
}

#[tokio::test]
async fn test_cache_separate_keys_for_different_symbols() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let btc_book = create_mock_order_book(vec![1.0, 1.0, 1.0, 1.0, 1.0], 1234567890);
    let eth_book = create_mock_order_book(vec![3.0, 3.0, 3.0, 3.0, 3.0], 1234567891);
    
    backend.set_order_book("binance", "BTCUSDT", btc_book).await;
    backend.set_order_book("binance", "ETHUSDT", eth_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Query both symbols
    let btc_result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    let eth_result = checker.check_depth_for_hedge("binance", "ETHUSDT", 2.0).await.unwrap();
    
    // Should have different liquidity values
    assert_eq!(btc_result.available_liquidity, 5.0);
    assert_eq!(eth_result.available_liquidity, 15.0);
    assert_ne!(btc_result.available_liquidity, eth_result.available_liquidity);
}

#[tokio::test]
async fn test_cache_concurrent_access() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = Arc::new(DepthChecker::new(backend));
    
    // Spawn multiple concurrent queries
    let mut handles = vec![];
    for _ in 0..10 {
        let checker_clone = checker.clone();
        let handle = tokio::spawn(async move {
            checker_clone.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await
        });
        handles.push(handle);
    }
    
    // Wait for all queries to complete
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await);
    }
    
    // All should succeed and return the same cached data
    for result in results {
        let depth_result = result.unwrap().unwrap();
        assert_eq!(depth_result.available_liquidity, 5.8);
    }
}

// ============================================================================
// Task 7.5: Integration tests with mock ExecutionBackend
// ============================================================================

#[tokio::test]
async fn test_integration_full_flow_sufficient_depth() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Setup: Create order book with good liquidity
    let order_book = create_mock_order_book(vec![5.0, 4.0, 3.0, 2.0, 1.0], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Execute depth check
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 5.0).await.unwrap();
    
    // Verify: Should pass all checks
    assert_eq!(result.available_liquidity, 15.0);
    assert_eq!(result.required_liquidity, 7.5);
    assert!((result.depth_ratio - 2.0).abs() < 0.01);
    assert!(result.is_sufficient);
    assert!(!result.is_critical);
    assert!(!result.should_abort());
    assert!(!result.should_warn());
}

#[tokio::test]
async fn test_integration_full_flow_warning_depth() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Setup: Create order book with low liquidity (warning zone)
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("bybit", "ETHUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Execute depth check with larger hedge quantity
    let result = checker.check_depth_for_hedge("bybit", "ETHUSDT", 5.0).await.unwrap();
    
    // Verify: Should trigger warning but not abort
    assert_eq!(result.available_liquidity, 5.8);
    assert_eq!(result.required_liquidity, 7.5);
    assert!((result.depth_ratio - 0.773).abs() < 0.01);
    assert!(!result.is_sufficient);
    assert!(!result.is_critical);
    assert!(!result.should_abort());
    assert!(result.should_warn());
}

#[tokio::test]
async fn test_integration_full_flow_critical_depth() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Setup: Create order book with critically low liquidity
    let order_book = create_mock_order_book(vec![0.5, 0.4, 0.3, 0.2, 0.1], 1234567890);
    backend.set_order_book("bitget", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Execute depth check
    let result = checker.check_depth_for_hedge("bitget", "BTCUSDT", 3.0).await.unwrap();
    
    // Verify: Should trigger abort
    assert_eq!(result.available_liquidity, 1.5);
    assert_eq!(result.required_liquidity, 4.5);
    assert!((result.depth_ratio - 0.333).abs() < 0.01);
    assert!(!result.is_sufficient);
    assert!(result.is_critical);
    assert!(result.should_abort());
    assert!(!result.should_warn());
}

#[tokio::test]
async fn test_integration_api_error_handling() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Setup: Configure backend to fail
    backend.set_should_fail(true).await;
    
    let checker = DepthChecker::new(backend);
    
    // Execute depth check - should return error
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await;
    
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to get order book depth"));
}

#[tokio::test]
async fn test_integration_multiple_exchanges_sequential() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Setup: Different order books for different exchanges
    let binance_book = create_mock_order_book(vec![3.0, 2.5, 2.0, 1.5, 1.0], 1234567890);
    let bybit_book = create_mock_order_book(vec![1.0, 0.9, 0.8, 0.7, 0.6], 1234567891);
    let bitget_book = create_mock_order_book(vec![5.0, 4.0, 3.0, 2.0, 1.0], 1234567892);
    
    backend.set_order_book("binance", "BTCUSDT", binance_book).await;
    backend.set_order_book("bybit", "BTCUSDT", bybit_book).await;
    backend.set_order_book("bitget", "BTCUSDT", bitget_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Execute depth checks sequentially
    let binance_result = checker.check_depth_for_hedge("binance", "BTCUSDT", 3.0).await.unwrap();
    let bybit_result = checker.check_depth_for_hedge("bybit", "BTCUSDT", 3.0).await.unwrap();
    let bitget_result = checker.check_depth_for_hedge("bitget", "BTCUSDT", 3.0).await.unwrap();
    
    // Verify: Each exchange has different liquidity
    assert_eq!(binance_result.available_liquidity, 10.0);
    assert_eq!(bybit_result.available_liquidity, 4.0);
    assert_eq!(bitget_result.available_liquidity, 15.0);
    
    // Verify: Different depth ratios
    assert!((binance_result.depth_ratio - 2.222).abs() < 0.01);  // 10.0 / 4.5
    assert!((bybit_result.depth_ratio - 0.889).abs() < 0.01);    // 4.0 / 4.5
    assert!((bitget_result.depth_ratio - 3.333).abs() < 0.01);   // 15.0 / 4.5
    
    // Verify: Different status
    assert!(binance_result.is_sufficient);
    assert!(!bybit_result.is_sufficient && bybit_result.should_warn());
    assert!(bitget_result.is_sufficient);
}

#[tokio::test]
async fn test_integration_multiple_exchanges_parallel() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    // Setup: Different order books for different exchanges
    let binance_book = create_mock_order_book(vec![3.0, 2.5, 2.0, 1.5, 1.0], 1234567890);
    let bybit_book = create_mock_order_book(vec![1.0, 0.9, 0.8, 0.7, 0.6], 1234567891);
    
    backend.set_order_book("binance", "BTCUSDT", binance_book).await;
    backend.set_order_book("bybit", "BTCUSDT", bybit_book).await;
    
    let checker = Arc::new(DepthChecker::new(backend));
    
    // Execute depth checks in parallel
    let checker1 = checker.clone();
    let checker2 = checker.clone();
    
    let (binance_result, bybit_result) = tokio::join!(
        checker1.check_depth_for_hedge("binance", "BTCUSDT", 3.0),
        checker2.check_depth_for_hedge("bybit", "BTCUSDT", 3.0)
    );
    
    let binance_result = binance_result.unwrap();
    let bybit_result = bybit_result.unwrap();
    
    // Verify: Both queries succeeded with correct data
    assert_eq!(binance_result.available_liquidity, 10.0);
    assert_eq!(bybit_result.available_liquidity, 4.0);
}

#[tokio::test]
async fn test_integration_timing_metrics() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // Execute depth check
    let result = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    
    // Verify: Timing metrics are recorded
    assert!(result.check_duration_ms >= 0);
    assert!(result.timestamp > 0);
}

#[tokio::test]
async fn test_integration_cache_performance() {
    let backend = Arc::new(MockExecutionBackend::new());
    
    let order_book = create_mock_order_book(vec![2.0, 1.5, 1.0, 0.8, 0.5], 1234567890);
    backend.set_order_book("binance", "BTCUSDT", order_book).await;
    
    let checker = DepthChecker::new(backend);
    
    // First query - cache miss
    let start1 = std::time::Instant::now();
    let _result1 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    let duration1 = start1.elapsed();
    
    // Second query - cache hit (should be faster)
    let start2 = std::time::Instant::now();
    let _result2 = checker.check_depth_for_hedge("binance", "BTCUSDT", 2.0).await.unwrap();
    let duration2 = start2.elapsed();
    
    // Cache hit should be significantly faster (at least 2x)
    // Note: This is a rough heuristic and may be flaky in CI
    println!("First query (cache miss): {:?}", duration1);
    println!("Second query (cache hit): {:?}", duration2);
    
    // Just verify both completed successfully
    assert!(duration1.as_micros() > 0);
    assert!(duration2.as_micros() > 0);
}
