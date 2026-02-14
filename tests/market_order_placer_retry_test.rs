use arbitrage2::strategy::atomic_execution::{MarketOrderPlacer, HedgeTimingMetrics};
use arbitrage2::strategy::execution_backend::ExecutionBackend;
use arbitrage2::strategy::types::{SimulatedOrder, OrderSide, OrderStatus, OrderType, OrderStatusInfo};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::HashMap;
use std::error::Error;
use std::time::Instant;

struct MockBackendSuccessFirstAttempt;

#[async_trait::async_trait]
impl ExecutionBackend for MockBackendSuccessFirstAttempt {
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
async fn test_place_with_retry_success_first_attempt() {
    let backend = Arc::new(MockBackendSuccessFirstAttempt) as Arc<dyn ExecutionBackend>;
    let placer = MarketOrderPlacer::new(backend);
    
    let order = SimulatedOrder {
        id: "retry_test_1".to_string(),
        exchange: "binance".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Long,
        order_type: OrderType::Market,
        price: 50000.0,
        size: 1.5,
        queue_position: None,
        created_at: 0,
        filled_at: None,
        fill_price: None,
        status: OrderStatus::Pending,
    };
    
    let mut metrics = HedgeTimingMetrics::new();
    let result = placer.place_with_retry(order.clone(), 1.5, 3, &mut metrics).await;
    
    assert!(result.is_ok());
    let placed_order = result.unwrap();
    assert_eq!(placed_order.size, 1.5);
}

struct MockBackendSuccessAfterRetries {
    attempt_count: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl ExecutionBackend for MockBackendSuccessAfterRetries {
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
        let count = self.attempt_count.fetch_add(1, Ordering::SeqCst);
        if count < 2 {
            // Fail first 2 attempts
            Err("Temporary network error".into())
        } else {
            // Succeed on 3rd attempt
            Ok(order)
        }
    }
    
    async fn cancel_order(&self, _exchange: &str, _order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
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
async fn test_place_with_retry_success_after_retries() {
    let backend = Arc::new(MockBackendSuccessAfterRetries {
        attempt_count: Arc::new(AtomicU32::new(0)),
    }) as Arc<dyn ExecutionBackend>;
    let placer = MarketOrderPlacer::new(backend);
    
    let order = SimulatedOrder {
        id: "retry_test_2".to_string(),
        exchange: "binance".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Long,
        order_type: OrderType::Market,
        price: 50000.0,
        size: 1.0,
        queue_position: None,
        created_at: 0,
        filled_at: None,
        fill_price: None,
        status: OrderStatus::Pending,
    };
    
    let mut metrics = HedgeTimingMetrics::new();
    let result = placer.place_with_retry(order.clone(), 1.0, 3, &mut metrics).await;
    
    assert!(result.is_ok());
}

struct MockBackendAllFail;

#[async_trait::async_trait]
impl ExecutionBackend for MockBackendAllFail {
    async fn set_leverage(&self, _exchange: &str, _symbol: &str, _leverage: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    
    async fn set_margin_type_isolated(&self, _exchange: &str, _symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    
    async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        Ok(order)
    }
    
    async fn place_market_order(&self, _order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        Err("Insufficient balance".into())
    }
    
    async fn cancel_order(&self, _exchange: &str, _order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    
    async fn get_order_status(&self, _exchange: &str, _order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatus::Filled)
    }
    
    async fn get_order_status_detailed(&self, _exchange: &str, _order_id: &str, _symbol: &str) -> Result<OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        Ok(OrderStatusInfo::new(OrderStatus::Pending, 0.0, 1.0))
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
async fn test_place_with_retry_all_attempts_fail() {
    let backend = Arc::new(MockBackendAllFail) as Arc<dyn ExecutionBackend>;
    let placer = MarketOrderPlacer::new(backend);
    
    let order = SimulatedOrder {
        id: "retry_test_3".to_string(),
        exchange: "binance".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Long,
        order_type: OrderType::Market,
        price: 50000.0,
        size: 1.0,
        queue_position: None,
        created_at: 0,
        filled_at: None,
        fill_price: None,
        status: OrderStatus::Pending,
    };
    
    let mut metrics = HedgeTimingMetrics::new();
    let result = placer.place_with_retry(order.clone(), 1.0, 3, &mut metrics).await;
    
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to place market order after 3 attempts"));
}

#[tokio::test]
async fn test_place_with_retry_exponential_backoff_timing() {
    // This test verifies that exponential backoff delays are applied between retries
    // We use a backend that always fails to ensure all retries are attempted
    let backend = Arc::new(MockBackendAllFail) as Arc<dyn ExecutionBackend>;
    let placer = MarketOrderPlacer::new(backend);
    
    let order = SimulatedOrder {
        id: "retry_test_4".to_string(),
        exchange: "binance".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Long,
        order_type: OrderType::Market,
        price: 50000.0,
        size: 1.0,
        queue_position: None,
        created_at: 0,
        filled_at: None,
        fill_price: None,
        status: OrderStatus::Pending,
    };
    
    let mut metrics = HedgeTimingMetrics::new();
    let start = Instant::now();
    let result = placer.place_with_retry(order.clone(), 1.0, 3, &mut metrics).await;
    let elapsed = start.elapsed();
    
    // Should fail after all retries
    assert!(result.is_err());
    
    // Should take at least 100ms + 200ms = 300ms for 2 retries (3 total attempts)
    // Allow some variance for async scheduling
    assert!(elapsed.as_millis() >= 280, "Expected at least 280ms for exponential backoff, got {}ms", elapsed.as_millis());
}
