use std::error::Error;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::strategy::execution_backend::ExecutionBackend;
use crate::strategy::types::{SimulatedOrder, OrderStatus};
use uuid::Uuid;

/// Paper trading backend - simulates order execution without real money
pub struct PaperTradingBackend {
    /// Simulated available balances per exchange
    balances: Arc<RwLock<HashMap<String, f64>>>,
    /// Simulated open orders
    orders: Arc<RwLock<HashMap<String, SimulatedOrder>>>,
}

impl PaperTradingBackend {
    pub fn new(initial_balances: HashMap<String, f64>) -> Self {
        Self {
            balances: Arc::new(RwLock::new(initial_balances)),
            orders: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for PaperTradingBackend {
    async fn set_leverage(&self, _exchange: &str, _symbol: &str, _leverage: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Paper trading doesn't need leverage setting
        Ok(())
    }

    async fn set_margin_type_isolated(&self, _exchange: &str, _symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Paper trading doesn't need margin type setting
        Ok(())
    }

    async fn place_order(&self, mut order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        let mut orders = self.orders.write().await;
        
        // Generate order ID if not present
        if order.id.is_empty() {
            order.id = Uuid::new_v4().to_string();
        }
        
        // Simulate immediate fill for paper trading
        order.status = OrderStatus::Filled;
        order.filled_at = Some(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs());
        order.fill_price = Some(order.price);
        
        orders.insert(order.id.clone(), order.clone());
        
        Ok(order)
    }

    async fn place_market_order(&self, mut order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        // For paper trading, market orders behave the same as limit orders (immediate fill)
        let mut orders = self.orders.write().await;
        
        // Generate order ID if not present
        if order.id.is_empty() {
            order.id = Uuid::new_v4().to_string();
        }
        
        // Simulate immediate fill for paper trading
        order.status = OrderStatus::Filled;
        order.filled_at = Some(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs());
        order.fill_price = Some(order.price);
        
        orders.insert(order.id.clone(), order.clone());
        
        Ok(order)
    }
    
    async fn cancel_order(&self, _exchange: &str, order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut orders = self.orders.write().await;
        orders.remove(order_id);
        Ok(())
    }
    
    async fn get_order_status(&self, _exchange: &str, order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        let orders = self.orders.read().await;
        match orders.get(order_id) {
            Some(order) => Ok(order.status),
            None => Ok(OrderStatus::Cancelled),
        }
    }

    async fn get_order_status_detailed(&self, _exchange: &str, order_id: &str, _symbol: &str) -> Result<crate::strategy::types::OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        use crate::strategy::types::OrderStatusInfo;
        
        let orders = self.orders.read().await;
        match orders.get(order_id) {
            Some(order) => {
                // Paper trading always fills completely
                Ok(OrderStatusInfo::new(order.status, order.size, order.size))
            }
            None => {
                // Order not found - return cancelled with 0 fill
                Ok(OrderStatusInfo::new(OrderStatus::Cancelled, 0.0, 0.0))
            }
        }
    }
    
    async fn get_available_balance(&self, exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let balances = self.balances.read().await;
        match balances.get(exchange) {
            Some(&balance) => Ok(balance),
            None => Err("Exchange not found".into()),
        }
    }
    
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        let balances = self.balances.read().await;
        Ok(balances.clone())
    }
    
    async fn is_symbol_tradeable(&self, _exchange: &str, _symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        // Paper trading accepts all symbols
        Ok(true)
    }
    
    async fn get_order_book_depth(
        &self,
        _exchange: &str,
        _symbol: &str,
        _levels: usize,
    ) -> Result<crate::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        // TODO: Implement order book depth query for paper trading
        Err("get_order_book_depth not yet implemented for paper trading".into())
    }

    async fn get_best_bid(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // TODO: Implement best bid query for paper trading
        Err("get_best_bid not yet implemented for paper trading".into())
    }

    async fn get_best_ask(
        &self,
        _exchange: &str,
        _symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // TODO: Implement best ask query for paper trading
        Err("get_best_ask not yet implemented for paper trading".into())
    }

    async fn get_quantity_step(&self, _exchange: &str, _symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // Paper trading uses default step of 0.1
        Ok(0.1)
    }
    
    fn backend_name(&self) -> &str {
        "PaperTrading"
    }
}
