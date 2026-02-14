use arbitrage2::strategy::atomic_execution::OrderStatusChecker;
use arbitrage2::strategy::execution_backend::ExecutionBackend;
use arbitrage2::strategy::types::{SimulatedOrder, OrderStatus, OrderStatusInfo};
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::HashMap;
use std::time::Duration;

// Mock backend for testing
struct MockBackend {
    call_count: Arc<AtomicU32>,
    should_fail: bool,
}

#[async_trait::async_trait]
impl ExecutionBackend for MockBackend {
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
        Ok(OrderStatus::Filled)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        
        if self.should_fail {
            Err("Network timeout".into())
        } else {
            Ok(OrderStatusInfo::new(OrderStatus::Filled, 1.5, 1.5))
        }
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(1000.0)
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        Ok(HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        _exchange: &str,
        _symbol: &str,
        _levels: usize,
    ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }

    async fn get_best_bid(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }
    
    async fn get_best_ask(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }
    
    fn backend_name(&self) -> &str {
        "mock"
    }
    
    async fn get_quantity_step(&self, _exchange: &str, _symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(0.001)
    }
}

#[tokio::test]
async fn test_order_status_checker_creation() {
    let backend = Arc::new(MockBackend {
        call_count: Arc::new(AtomicU32::new(0)),
        should_fail: false,
    }) as Arc<dyn ExecutionBackend>;
    
    let _checker = OrderStatusChecker::new(backend);
    
    // Successfully created the checker
    // Cache TTL is internal implementation detail (50ms)
}

#[tokio::test]
async fn test_order_status_checker_get_status_success() {
    let backend = Arc::new(MockBackend {
        call_count: Arc::new(AtomicU32::new(0)),
        should_fail: false,
    }) as Arc<dyn ExecutionBackend>;
    
    let checker = OrderStatusChecker::new(backend);
    
    let result = checker.get_status_with_retry("binance", "order123", "BTCUSDT", 2).await;
    assert!(result.is_ok());
    
    let status_info = result.unwrap();
    assert_eq!(status_info.filled_quantity, 1.5);
    assert_eq!(status_info.total_quantity, 1.5);
}

#[tokio::test]
async fn test_order_status_checker_caching() {
    let call_count = Arc::new(AtomicU32::new(0));
    let backend = Arc::new(MockBackend {
        call_count: call_count.clone(),
        should_fail: false,
    }) as Arc<dyn ExecutionBackend>;
    
    let checker = OrderStatusChecker::new(backend);
    
    // First call should hit the backend
    let _ = checker.get_status_with_retry("binance", "order123", "BTCUSDT", 2).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    
    // Second call within 50ms should use cache
    let _ = checker.get_status_with_retry("binance", "order123", "BTCUSDT", 2).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1); // Still 1, cache was used
    
    // Wait for cache to expire
    tokio::time::sleep(Duration::from_millis(60)).await;
    
    // Third call should hit the backend again
    let _ = checker.get_status_with_retry("binance", "order123", "BTCUSDT", 2).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_order_status_checker_logs_retry_attempts() {
    let call_count = Arc::new(AtomicU32::new(0));
    let backend = Arc::new(MockBackend {
        call_count: call_count.clone(),
        should_fail: true,
    }) as Arc<dyn ExecutionBackend>;
    
    let checker = OrderStatusChecker::new(backend);
    
    // Should fail after all retries
    let result = checker.get_status_with_retry("binance", "order123", "BTCUSDT", 2).await;
    assert!(result.is_err());
    
    // Should have tried 3 times (initial + 2 retries)
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
    
    // Error message should mention retries
    let error_msg = result.unwrap_err();
    assert!(error_msg.contains("Failed to get order status after 2 retries"));
}

// Tests for verify_cancellation method

use arbitrage2::strategy::atomic_execution::CancellationResult;

// Mock backend that simulates successful cancellation
struct MockCancelSuccessBackend;

#[async_trait::async_trait]
impl ExecutionBackend for MockCancelSuccessBackend {
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
        Ok(OrderStatus::Filled)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatusInfo::new(OrderStatus::Filled, 1.5, 1.5))
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(1000.0)
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        Ok(HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        _exchange: &str,
        _symbol: &str,
        _levels: usize,
    ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }

    async fn get_best_bid(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }
    
    fn backend_name(&self) -> &str {
        "mock_cancel_success"
    }
}

// Mock backend that simulates Binance "already filled" error
struct MockBinanceAlreadyFilledBackend;

#[async_trait::async_trait]
impl ExecutionBackend for MockBinanceAlreadyFilledBackend {
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
        Err("Failed to cancel order: UNKNOWN_ORDER".into())
    }
    
    async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatus::Filled)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatusInfo::new(OrderStatus::Filled, 2.5, 2.5))
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(1000.0)
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        Ok(HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        _exchange: &str,
        _symbol: &str,
        _levels: usize,
    ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }

    async fn get_best_bid(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }
    
    fn backend_name(&self) -> &str {
        "mock_binance_already_filled"
    }
}

// Mock backend that simulates Bybit "already filled" error
struct MockBybitAlreadyFilledBackend;

#[async_trait::async_trait]
impl ExecutionBackend for MockBybitAlreadyFilledBackend {
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
        Err("Failed to cancel order: 110017".into())
    }
    
    async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatus::Filled)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatusInfo::new(OrderStatus::Filled, 3.5, 3.5))
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(1000.0)
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        Ok(HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        _exchange: &str,
        _symbol: &str,
        _levels: usize,
    ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }

    async fn get_best_bid(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }
    
    fn backend_name(&self) -> &str {
        "mock_bybit_already_filled"
    }
}

// Mock backend that simulates generic cancellation failure
struct MockCancelFailureBackend;

#[async_trait::async_trait]
impl ExecutionBackend for MockCancelFailureBackend {
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
        Err("Network timeout during cancellation".into())
    }
    
    async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatus::Pending)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatusInfo::new(OrderStatus::Pending, 0.0, 1.5))
    }
    
    async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Ok(1000.0)
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        Ok(HashMap::new())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        _exchange: &str,
        _symbol: &str,
        _levels: usize,
    ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }

    async fn get_best_bid(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        Err("Not implemented in mock".into())
    }
    
    fn backend_name(&self) -> &str {
        "mock_cancel_failure"
    }
}

#[tokio::test]
async fn test_verify_cancellation_success() {
    let backend = Arc::new(MockCancelSuccessBackend) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    
    let result = checker.verify_cancellation("binance", "order123", "BTCUSDT").await;
    assert!(result.is_ok());
    
    match result.unwrap() {
        CancellationResult::Cancelled => {
            // Expected result
        }
        _ => panic!("Expected Cancelled result"),
    }
}

#[tokio::test]
async fn test_verify_cancellation_binance_already_filled() {
    let backend = Arc::new(MockBinanceAlreadyFilledBackend) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    
    let result = checker.verify_cancellation("binance", "order123", "BTCUSDT").await;
    assert!(result.is_ok());
    
    match result.unwrap() {
        CancellationResult::AlreadyFilled(qty) => {
            assert_eq!(qty, 2.5);
        }
        _ => panic!("Expected AlreadyFilled result"),
    }
}

#[tokio::test]
async fn test_verify_cancellation_bybit_already_filled() {
    let backend = Arc::new(MockBybitAlreadyFilledBackend) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    
    let result = checker.verify_cancellation("bybit", "order456", "BTCUSDT").await;
    assert!(result.is_ok());
    
    match result.unwrap() {
        CancellationResult::AlreadyFilled(qty) => {
            assert_eq!(qty, 3.5);
        }
        _ => panic!("Expected AlreadyFilled result"),
    }
}

#[tokio::test]
async fn test_verify_cancellation_generic_failure() {
    let backend = Arc::new(MockCancelFailureBackend) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    
    let result = checker.verify_cancellation("binance", "order789", "BTCUSDT").await;
    assert!(result.is_ok());
    
    match result.unwrap() {
        CancellationResult::Failed(msg) => {
            assert!(msg.contains("Network timeout"));
        }
        _ => panic!("Expected Failed result"),
    }
}

#[tokio::test]
async fn test_verify_cancellation_binance_error_codes() {
    // Test various Binance error messages
    struct MockBinanceErrorBackend {
        error_msg: String,
    }
    
    #[async_trait::async_trait]
    impl ExecutionBackend for MockBinanceErrorBackend {
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
            Err(self.error_msg.clone().into())
        }
        
        async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
            Ok(OrderStatus::Filled)
        }
        
        async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
            Ok(OrderStatusInfo::new(OrderStatus::Filled, 1.0, 1.0))
        }
        
        async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
            Ok(1000.0)
        }
        
        async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
            Ok(HashMap::new())
        }
        
        async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
            Ok(true)
        }
        
        async fn get_order_book_depth(
            &self,
            _exchange: &str,
            _symbol: &str,
            _levels: usize,
        ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
            Err("Not implemented in mock".into())
        }

        async fn get_best_bid(
            &self,
            _exchange: &str,
            _symbol: &str,
        ) -> Result<f64, Box<dyn Error + Send + Sync>> {
            Err("Not implemented in mock".into())
        }
        
        fn backend_name(&self) -> &str {
            "mock_binance_error"
        }
    }
    
    // Test "Unknown order sent" error
    let backend = Arc::new(MockBinanceErrorBackend {
        error_msg: "Unknown order sent".to_string(),
    }) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    let result = checker.verify_cancellation("binance", "order1", "BTCUSDT").await;
    assert!(matches!(result, Ok(CancellationResult::AlreadyFilled(_))));
    
    // Test "Order does not exist" error
    let backend = Arc::new(MockBinanceErrorBackend {
        error_msg: "Order does not exist".to_string(),
    }) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    let result = checker.verify_cancellation("binance", "order2", "BTCUSDT").await;
    assert!(matches!(result, Ok(CancellationResult::AlreadyFilled(_))));
}

#[tokio::test]
async fn test_verify_cancellation_bybit_error_codes() {
    // Test various Bybit error codes
    struct MockBybitErrorBackend {
        error_msg: String,
    }
    
    #[async_trait::async_trait]
    impl ExecutionBackend for MockBybitErrorBackend {
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
            Err(self.error_msg.clone().into())
        }
        
        async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
            Ok(OrderStatus::Filled)
        }
        
        async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
            Ok(OrderStatusInfo::new(OrderStatus::Filled, 1.0, 1.0))
        }
        
        async fn get_available_balance(&self, _exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
            Ok(1000.0)
        }
        
        async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
            Ok(HashMap::new())
        }
        
        async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
            Ok(true)
        }
        
        async fn get_order_book_depth(
            &self,
            _exchange: &str,
            _symbol: &str,
            _levels: usize,
        ) -> Result<arbitrage2::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
            Err("Not implemented in mock".into())
        }

        async fn get_best_bid(
            &self,
            _exchange: &str,
            _symbol: &str,
        ) -> Result<f64, Box<dyn Error + Send + Sync>> {
            Err("Not implemented in mock".into())
        }
        
        fn backend_name(&self) -> &str {
            "mock_bybit_error"
        }
    }
    
    // Test error code 110001 (order not found)
    let backend = Arc::new(MockBybitErrorBackend {
        error_msg: "Error 110001: order not found".to_string(),
    }) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    let result = checker.verify_cancellation("bybit", "order1", "BTCUSDT").await;
    assert!(matches!(result, Ok(CancellationResult::AlreadyFilled(_))));
    
    // Test "Order not exists" error
    let backend = Arc::new(MockBybitErrorBackend {
        error_msg: "Order not exists".to_string(),
    }) as Arc<dyn ExecutionBackend>;
    let checker = OrderStatusChecker::new(backend);
    let result = checker.verify_cancellation("bybit", "order2", "BTCUSDT").await;
    assert!(matches!(result, Ok(CancellationResult::AlreadyFilled(_))));
}
