use std::error::Error;
use std::collections::HashMap;
use crate::strategy::types::{SimulatedOrder, OrderStatus, OrderBookDepth};

/// Trait for different execution backends (paper trading, testnet, live)
#[async_trait::async_trait]
pub trait ExecutionBackend: Send + Sync {
    /// Set leverage for a symbol on an exchange (must be called before placing orders)
    async fn set_leverage(&self, exchange: &str, symbol: &str, leverage: u8) -> Result<(), Box<dyn Error + Send + Sync>>;
    
    /// Set margin type to ISOLATED for a symbol on an exchange (must be called before placing orders)
    async fn set_margin_type_isolated(&self, exchange: &str, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
    
    /// Place a limit order on the exchange
    async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>>;
    
    /// Place a market order on the exchange (for immediate hedging)
    async fn place_market_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>>;
    
    /// Cancel an existing order
    async fn cancel_order(&self, exchange: &str, order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
    
    /// Get the status of an order
    async fn get_order_status(&self, exchange: &str, order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>>;
    
    /// Get detailed order status including filled quantity (for partial fill handling)
    async fn get_order_status_detailed(&self, exchange: &str, order_id: &str, symbol: &str) -> Result<crate::strategy::types::OrderStatusInfo, Box<dyn Error + Send + Sync>>;
    
    /// Get current available balance for an exchange
    async fn get_available_balance(&self, exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>>;
    
    /// Get balances from all configured exchanges
    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>>;
    
    /// Check if a symbol is tradeable on an exchange
    async fn is_symbol_tradeable(&self, exchange: &str, symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>>;
    
    /// Get order book depth (top N levels) for a symbol on an exchange
    async fn get_order_book_depth(
        &self,
        exchange: &str,
        symbol: &str,
        levels: usize,
    ) -> Result<OrderBookDepth, Box<dyn Error + Send + Sync>>;
    
    /// Get the best bid price for a symbol on an exchange
    async fn get_best_bid(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>>;
    
    /// Get the best ask price for a symbol on an exchange
    async fn get_best_ask(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>>;
    
    /// Get the name of this backend (for logging)
    fn backend_name(&self) -> &str;
    
    /// Get the quantity rounding step for a symbol on an exchange
    /// Returns the minimum quantity increment (e.g., 0.1, 0.01, 1.0)
    async fn get_quantity_step(&self, exchange: &str, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>>;
}
