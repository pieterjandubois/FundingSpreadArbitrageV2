use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use crate::strategy::execution_backend::ExecutionBackend;
use crate::strategy::types::OrderBookDepth;

/// Result of a depth check operation
#[derive(Clone, Debug)]
pub struct DepthCheckResult {
    pub exchange: String,
    pub symbol: String,
    pub available_liquidity: f64,  // Sum of top 5 levels
    pub required_liquidity: f64,   // hedge_quantity * 1.5
    pub depth_ratio: f64,          // available / required
    pub is_sufficient: bool,       // depth_ratio >= 1.0
    pub is_critical: bool,         // depth_ratio < 0.73 (1.1x / 1.5x)
    pub check_duration_ms: u128,
    pub timestamp: u64,
}

impl DepthCheckResult {
    /// Returns true if the trade should be aborted due to critically low depth
    pub fn should_abort(&self) -> bool {
        self.is_critical
    }
    
    /// Returns true if a warning should be logged (low but not critical depth)
    pub fn should_warn(&self) -> bool {
        !self.is_sufficient && !self.is_critical
    }
}

/// Pre-flight depth checker for market hedge orders
pub struct DepthChecker {
    backend: Arc<dyn ExecutionBackend>,
    cache: Arc<Mutex<HashMap<String, (OrderBookDepth, Instant)>>>,
    cache_ttl: Duration,
}

impl DepthChecker {
    /// Create a new DepthChecker with the given execution backend
    pub fn new(backend: Arc<dyn ExecutionBackend>) -> Self {
        Self {
            backend,
            cache: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl: Duration::from_millis(100),
        }
    }
    
    /// Check if sufficient depth exists for market hedge order
    /// 
    /// This method queries the order book depth and calculates if there is sufficient
    /// liquidity to execute a market hedge order without excessive slippage.
    /// 
    /// # Arguments
    /// * `exchange` - The exchange to check (e.g., "binance", "bybit")
    /// * `symbol` - The trading symbol (e.g., "BTCUSDT")
    /// * `hedge_quantity` - The quantity to hedge
    /// 
    /// # Returns
    /// * `Ok(DepthCheckResult)` - Depth check result with liquidity analysis
    /// * `Err(String)` - Error message if depth check fails
    pub async fn check_depth_for_hedge(
        &self,
        exchange: &str,
        symbol: &str,
        hedge_quantity: f64,
    ) -> Result<DepthCheckResult, String> {
        let start = Instant::now();
        
        // Check cache first
        let cache_key = format!("{}:{}", exchange, symbol);
        {
            let cache = self.cache.lock().await;
            if let Some((depth, timestamp)) = cache.get(&cache_key) {
                if timestamp.elapsed() < self.cache_ttl {
                    return Ok(self.calculate_depth_result(
                        exchange, symbol, depth, hedge_quantity, start.elapsed().as_millis()
                    ));
                }
            }
        }
        
        // Query order book depth (top 10 levels)
        let depth = self.backend.get_order_book_depth(exchange, symbol, 10).await
            .map_err(|e| format!("Failed to get order book depth: {}", e))?;
        
        // Update cache
        {
            let mut cache = self.cache.lock().await;
            cache.insert(cache_key, (depth.clone(), Instant::now()));
        }
        
        Ok(self.calculate_depth_result(
            exchange, symbol, &depth, hedge_quantity, start.elapsed().as_millis()
        ))
    }
    
    /// Calculate depth check result from order book data
    fn calculate_depth_result(
        &self,
        exchange: &str,
        symbol: &str,
        depth: &OrderBookDepth,
        hedge_quantity: f64,
        duration_ms: u128,
    ) -> DepthCheckResult {
        // Sum top 5 levels of bids (for selling/shorting)
        let available_liquidity = depth.bids.iter()
            .take(5)
            .map(|level| level.quantity)
            .sum::<f64>();
        
        // Required liquidity with 50% safety buffer
        let required_liquidity = hedge_quantity * 1.5;
        
        // Calculate depth ratio
        let depth_ratio = if required_liquidity > 0.0 {
            available_liquidity / required_liquidity
        } else {
            0.0
        };
        
        DepthCheckResult {
            exchange: exchange.to_string(),
            symbol: symbol.to_string(),
            available_liquidity,
            required_liquidity,
            depth_ratio,
            is_sufficient: depth_ratio >= 1.0,
            is_critical: depth_ratio < 0.73,  // 1.1x / 1.5x = 0.733...
            check_duration_ms: duration_ms,
            timestamp: depth.timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depth_check_result_should_abort() {
        let result = DepthCheckResult {
            exchange: "binance".to_string(),
            symbol: "BTCUSDT".to_string(),
            available_liquidity: 1.0,
            required_liquidity: 1.5,
            depth_ratio: 0.67,
            is_sufficient: false,
            is_critical: true,
            check_duration_ms: 10,
            timestamp: 1234567890,
        };
        
        assert!(result.should_abort());
        assert!(!result.should_warn());
    }
    
    #[test]
    fn test_depth_check_result_should_warn() {
        let result = DepthCheckResult {
            exchange: "binance".to_string(),
            symbol: "BTCUSDT".to_string(),
            available_liquidity: 1.2,
            required_liquidity: 1.5,
            depth_ratio: 0.8,
            is_sufficient: false,
            is_critical: false,
            check_duration_ms: 10,
            timestamp: 1234567890,
        };
        
        assert!(!result.should_abort());
        assert!(result.should_warn());
    }
    
    #[test]
    fn test_depth_check_result_sufficient() {
        let result = DepthCheckResult {
            exchange: "binance".to_string(),
            symbol: "BTCUSDT".to_string(),
            available_liquidity: 2.0,
            required_liquidity: 1.5,
            depth_ratio: 1.33,
            is_sufficient: true,
            is_critical: false,
            check_duration_ms: 10,
            timestamp: 1234567890,
        };
        
        assert!(!result.should_abort());
        assert!(!result.should_warn());
    }
}
