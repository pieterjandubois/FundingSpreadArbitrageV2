use crate::strategy::types::{SimulatedOrder, OrderSide, OrderStatus};
use crate::strategy::execution_backend::ExecutionBackend;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};

/// Configuration for repricing behavior
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepricingConfig {
    pub reprice_threshold_bps: f64,      // Default: 5.0
    pub max_reprices: u32,               // Default: 5
    pub reprice_interval_ms: u64,        // Default: 100
    pub total_timeout_seconds: u64,      // Default: 3
    pub spread_collapse_threshold_bps: f64,  // Default: 50.0
    pub execution_mode: ExecutionMode,   // ultra_fast, balanced, safe
}

/// Execution mode determines depth check strategy
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ExecutionMode {
    UltraFast,  // No pre-flight depth checks, 0ms added latency
    Balanced,   // Parallel depth checks, ~10ms added latency
    Safe,       // Sequential depth checks, ~50ms added latency
}

impl RepricingConfig {
    pub fn ultra_fast() -> Self {
        Self {
            reprice_threshold_bps: 5.0,
            max_reprices: 5,
            reprice_interval_ms: 100,
            total_timeout_seconds: 3,
            spread_collapse_threshold_bps: 50.0,
            execution_mode: ExecutionMode::UltraFast,
        }
    }
    
    pub fn balanced() -> Self {
        Self {
            reprice_threshold_bps: 5.0,
            max_reprices: 5,
            reprice_interval_ms: 100,
            total_timeout_seconds: 3,
            spread_collapse_threshold_bps: 50.0,
            execution_mode: ExecutionMode::Balanced,
        }
    }
    
    pub fn safe() -> Self {
        Self {
            reprice_threshold_bps: 5.0,
            max_reprices: 5,
            reprice_interval_ms: 100,
            total_timeout_seconds: 3,
            spread_collapse_threshold_bps: 50.0,
            execution_mode: ExecutionMode::Safe,
        }
    }
    
    /// Select execution mode based on opportunity confidence score
    /// - confidence >= 90%: UltraFast (no depth checks)
    /// - confidence >= 75%: Balanced (parallel depth checks)
    /// - confidence < 75%: Safe (sequential depth checks)
    pub fn from_confidence(confidence_score: f64) -> Self {
        if confidence_score >= 90.0 {
            Self::ultra_fast()
        } else if confidence_score >= 75.0 {
            Self::balanced()
        } else {
            Self::safe()
        }
    }
}

/// Tracks a single repricing event
#[derive(Clone, Debug)]
pub struct RepricingEvent {
    pub timestamp: u64,
    pub old_price: f64,
    pub new_price: f64,
    pub reason: String,
    pub elapsed_ms: u128,
    pub exchange: String,
    pub side: OrderSide,
}

/// Tracks repricing metrics for a trade
#[derive(Clone, Debug)]
pub struct RepricingMetrics {
    pub reprice_count: u32,
    pub reprice_total_time_ms: u128,
    pub initial_price: f64,
    pub final_price: f64,
    pub price_improvement_bps: f64,
    pub repricing_events: Vec<RepricingEvent>,
    pub max_reprices_reached: bool,
}

impl RepricingMetrics {
    pub fn new(initial_price: f64) -> Self {
        Self {
            reprice_count: 0,
            reprice_total_time_ms: 0,
            initial_price,
            final_price: initial_price,
            price_improvement_bps: 0.0,
            repricing_events: Vec::new(),
            max_reprices_reached: false,
        }
    }
    
    pub fn finalize(&mut self) {
        self.price_improvement_bps = 
            ((self.final_price - self.initial_price) / self.initial_price) * 10000.0;
    }
}

/// Price chaser module for active repricing of limit orders
pub struct PriceChaser {
    backend: Arc<dyn ExecutionBackend>,
    config: RepricingConfig,
}

impl PriceChaser {
    pub fn new(backend: Arc<dyn ExecutionBackend>, config: RepricingConfig) -> Self {
        Self { backend, config }
    }
    
    /// Check if order needs repricing based on current market price
    /// Returns true if price deviation exceeds threshold (default 5 bps)
    pub fn should_reprice(&self, order_price: f64, current_best_price: f64) -> bool {
        let price_deviation = ((current_best_price - order_price).abs() / order_price) * 10000.0;
        price_deviation > self.config.reprice_threshold_bps
    }
    
    /// Reprice an order by cancelling and placing new order at current best price
    /// Returns the new order and updates metrics
    pub async fn reprice_order(
        &self,
        order: &SimulatedOrder,
        new_price: f64,
        metrics: &mut RepricingMetrics,
    ) -> Result<SimulatedOrder, String> {
        let start = Instant::now();
        
        // Cancel existing order
        self.backend.cancel_order(&order.exchange, &order.id).await
            .map_err(|e| format!("Failed to cancel order: {}", e))?;
        
        // Create new order at new price
        let mut new_order = order.clone();
        new_order.price = new_price;
        new_order.id = String::new();  // Will be assigned by exchange
        new_order.created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let placed_order = self.backend.place_order(new_order).await
            .map_err(|e| format!("Failed to place repriced order: {}", e))?;
        
        // Record repricing event
        let elapsed_ms = start.elapsed().as_millis();
        let event = RepricingEvent {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            old_price: order.price,
            new_price,
            reason: "price_moved_away".to_string(),
            elapsed_ms,
            exchange: order.exchange.clone(),
            side: order.side,
        };
        
        metrics.reprice_count += 1;
        metrics.reprice_total_time_ms += elapsed_ms;
        metrics.final_price = new_price;
        metrics.repricing_events.push(event);
        
        Ok(placed_order)
    }
    
    /// Get the best price for an order based on its side
    /// For long orders: use best bid (we want to buy at the best available price)
    /// For short orders: use best ask (we want to sell at the best available price)
    pub async fn get_best_price_for_order(&self, order: &SimulatedOrder) -> Result<f64, String> {
        match order.side {
            OrderSide::Long => {
                self.backend.get_best_bid(&order.exchange, &order.symbol).await
                    .map_err(|e| format!("Failed to get best bid: {}", e))
            }
            OrderSide::Short => {
                self.backend.get_best_ask(&order.exchange, &order.symbol).await
                    .map_err(|e| format!("Failed to get best ask: {}", e))
            }
        }
    }
    
    /// Monitor and reprice an order until filled or timeout
    /// This is a standalone function for testing, but in practice repricing
    /// is integrated into the atomic execution polling loop
    #[allow(dead_code)]
    pub async fn chase_until_filled(
        &self,
        mut order: SimulatedOrder,
        timeout: Duration,
    ) -> Result<SimulatedOrder, String> {
        let start = Instant::now();
        let mut metrics = RepricingMetrics::new(order.price);
        
        while start.elapsed() < timeout {
            // Check if order filled
            let status = self.backend.get_order_status(&order.exchange, &order.id).await
                .map_err(|e| format!("Failed to get order status: {}", e))?;
            
            if status == OrderStatus::Filled {
                metrics.finalize();
                eprintln!("[PRICE CHASER] Order filled after {} reprices", metrics.reprice_count);
                return Ok(order);
            }
            
            // Check if max reprices reached
            if metrics.reprice_count >= self.config.max_reprices {
                metrics.max_reprices_reached = true;
                return Err(format!("Max reprices ({}) reached", self.config.max_reprices));
            }
            
            // Get current best price
            let best_price = self.get_best_price_for_order(&order).await?;
            
            // Check if repricing needed
            if self.should_reprice(order.price, best_price) {
                eprintln!("[PRICE CHASER] Repricing order {} from {:.4} to {:.4}", 
                    order.id, order.price, best_price);
                
                order = self.reprice_order(&order, best_price, &mut metrics).await?;
            }
            
            // Wait before next check
            tokio::time::sleep(Duration::from_millis(self.config.reprice_interval_ms)).await;
        }
        
        Err("Timeout reached without fill".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_should_reprice_threshold() {
        let config = RepricingConfig::balanced();
        let testnet_config = crate::strategy::testnet_config::TestnetConfig {
            bybit: None,
            binance: None,
            okx: None,
            kucoin: None,
            bitget: None,
            single_exchange_mode: false,
            primary_exchange: "bybit".to_string(),
        };
        let chaser = PriceChaser {
            backend: Arc::new(crate::strategy::testnet_backend::TestnetBackend::new(testnet_config)),
            config,
        };
        
        // Price deviation of 4 bps - should NOT reprice
        let order_price = 100.0;
        let current_price = 100.04; // 4 bps
        assert!(!chaser.should_reprice(order_price, current_price));
        
        // Price deviation of 6 bps - should reprice
        let current_price = 100.06; // 6 bps
        assert!(chaser.should_reprice(order_price, current_price));
        
        // Price deviation of exactly 5 bps - should NOT reprice (threshold is >5)
        let current_price = 100.05; // 5 bps
        assert!(!chaser.should_reprice(order_price, current_price));
    }
    
    #[test]
    fn test_repricing_config_presets() {
        let ultra_fast = RepricingConfig::ultra_fast();
        assert_eq!(ultra_fast.execution_mode, ExecutionMode::UltraFast);
        assert_eq!(ultra_fast.max_reprices, 5);
        
        let balanced = RepricingConfig::balanced();
        assert_eq!(balanced.execution_mode, ExecutionMode::Balanced);
        
        let safe = RepricingConfig::safe();
        assert_eq!(safe.execution_mode, ExecutionMode::Safe);
    }
    
    #[test]
    fn test_from_confidence_mode_selection() {
        // High confidence (>= 90%) -> UltraFast
        let config = RepricingConfig::from_confidence(95.0);
        assert_eq!(config.execution_mode, ExecutionMode::UltraFast);
        
        let config = RepricingConfig::from_confidence(90.0);
        assert_eq!(config.execution_mode, ExecutionMode::UltraFast);
        
        // Medium confidence (75-90%) -> Balanced
        let config = RepricingConfig::from_confidence(85.0);
        assert_eq!(config.execution_mode, ExecutionMode::Balanced);
        
        let config = RepricingConfig::from_confidence(75.0);
        assert_eq!(config.execution_mode, ExecutionMode::Balanced);
        
        // Low confidence (< 75%) -> Safe
        let config = RepricingConfig::from_confidence(70.0);
        assert_eq!(config.execution_mode, ExecutionMode::Safe);
        
        let config = RepricingConfig::from_confidence(50.0);
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
    }
    
    #[test]
    fn test_repricing_metrics_finalize() {
        let mut metrics = RepricingMetrics::new(100.0);
        metrics.final_price = 100.10;
        metrics.finalize();
        
        // Price improvement: (100.10 - 100.0) / 100.0 * 10000 = 10 bps
        assert!((metrics.price_improvement_bps - 10.0).abs() < 0.01);
    }
}
