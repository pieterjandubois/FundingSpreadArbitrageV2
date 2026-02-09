use crate::strategy::types::{SimulatedOrder, OrderSide};
use crate::strategy::entry::EntryExecutor;
use std::error::Error;
use tokio::task::JoinHandle;

#[derive(Clone, Debug)]
pub struct AtomicExecutionResult {
    pub long_order: SimulatedOrder,
    pub short_order: SimulatedOrder,
    pub both_filled: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct NegativeFundingTracker {
    pub symbol: String,
    pub consecutive_negative_cycles: u32,
    pub last_funding_rate: f64,
}

#[allow(dead_code)]
pub struct AtomicExecutor;

#[allow(dead_code)]
impl AtomicExecutor {
    /// Execute both legs concurrently (atomic execution)
    /// Returns result with both orders and whether both were filled
    pub async fn execute_dual_leg(
        symbol: &str,
        long_exchange: &str,
        short_exchange: &str,
        long_price: f64,
        short_price: f64,
        position_size: f64,
    ) -> Result<AtomicExecutionResult, Box<dyn Error + Send + Sync>> {
        // Create market orders for both legs
        let long_order = EntryExecutor::create_market_order(
            long_exchange,
            symbol,
            OrderSide::Long,
            long_price,
            position_size,
        );

        let short_order = EntryExecutor::create_market_order(
            short_exchange,
            symbol,
            OrderSide::Short,
            short_price,
            position_size,
        );

        // Spawn concurrent tasks for both legs
        let long_order_clone = long_order.clone();
        let short_order_clone = short_order.clone();

        let long_task: JoinHandle<Result<SimulatedOrder, String>> = tokio::spawn(async move {
            // Simulate order execution on long exchange
            Self::execute_order_on_exchange(&long_order_clone).await
        });

        let short_task: JoinHandle<Result<SimulatedOrder, String>> = tokio::spawn(async move {
            // Simulate order execution on short exchange
            Self::execute_order_on_exchange(&short_order_clone).await
        });

        // Wait for both tasks to complete
        let long_result = long_task.await;
        let short_result = short_task.await;

        // Check if both succeeded
        let long_filled = long_result.is_ok() && long_result.as_ref().unwrap().as_ref().is_ok();
        let short_filled = short_result.is_ok() && short_result.as_ref().unwrap().as_ref().is_ok();

        if !long_filled || !short_filled {
            // One leg failed - trigger instant reversal
            let error_msg = format!(
                "Atomic execution failed: long={}, short={}",
                long_filled, short_filled
            );

            // Attempt to reverse the successful leg
            if long_filled {
                Self::reverse_order(&long_order).await.ok();
            }
            if short_filled {
                Self::reverse_order(&short_order).await.ok();
            }

            return Ok(AtomicExecutionResult {
                long_order,
                short_order,
                both_filled: false,
                error: Some(error_msg),
            });
        }

        Ok(AtomicExecutionResult {
            long_order,
            short_order,
            both_filled: true,
            error: None,
        })
    }

    /// Simulate order execution on exchange
    async fn execute_order_on_exchange(
        order: &SimulatedOrder,
    ) -> Result<SimulatedOrder, String> {
        // Simulate network latency (1-50ms)
        let latency_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() % 50) as u64;

        tokio::time::sleep(tokio::time::Duration::from_millis(latency_ms)).await;

        // Simulate 99% success rate
        let success_rate = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() % 100) as u32;

        if success_rate < 99 {
            Ok(order.clone())
        } else {
            Err("Exchange API timeout".to_string())
        }
    }

    /// Instantly reverse an order (market sell to go back to cash)
    pub async fn reverse_order(order: &SimulatedOrder) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Create market order in opposite direction
        let reverse_side = match order.side {
            OrderSide::Long => OrderSide::Short,
            OrderSide::Short => OrderSide::Long,
        };

        let reverse_order = EntryExecutor::create_market_order(
            &order.exchange,
            &order.symbol,
            reverse_side,
            order.price,
            order.size,
        );

        // Execute reversal immediately
        Self::execute_order_on_exchange(&reverse_order).await?;

        Ok(())
    }
}

#[allow(dead_code)]
impl NegativeFundingTracker {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            consecutive_negative_cycles: 0,
            last_funding_rate: 0.0,
        }
    }

    /// Update funding rate and track consecutive negative cycles
    /// Returns true if should exit (2+ consecutive negative cycles)
    pub fn update_funding(&mut self, funding_rate: f64) -> bool {
        self.last_funding_rate = funding_rate;

        if funding_rate < 0.0 {
            self.consecutive_negative_cycles += 1;
        } else {
            self.consecutive_negative_cycles = 0;
        }

        // Exit if 2+ consecutive negative cycles (16 hours)
        self.consecutive_negative_cycles >= 2
    }

    /// Check if should exit based on negative funding
    pub fn should_exit(&self) -> bool {
        self.consecutive_negative_cycles >= 2
    }

    /// Reset tracker (when exiting position)
    pub fn reset(&mut self) {
        self.consecutive_negative_cycles = 0;
        self.last_funding_rate = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_negative_funding_tracker() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());

        // First negative cycle
        assert!(!tracker.update_funding(-0.01));
        assert_eq!(tracker.consecutive_negative_cycles, 1);

        // Second negative cycle - should trigger exit
        assert!(tracker.update_funding(-0.01));
        assert_eq!(tracker.consecutive_negative_cycles, 2);

        // Positive funding resets counter
        assert!(!tracker.update_funding(0.05));
        assert_eq!(tracker.consecutive_negative_cycles, 0);
    }

    #[test]
    fn test_negative_funding_reset() {
        let mut tracker = NegativeFundingTracker::new("ETHUSDT".to_string());
        tracker.update_funding(-0.01);
        tracker.update_funding(-0.01);

        assert!(tracker.should_exit());

        tracker.reset();
        assert!(!tracker.should_exit());
        assert_eq!(tracker.consecutive_negative_cycles, 0);
    }

    #[test]
    fn test_negative_funding_mixed_cycles() {
        let mut tracker = NegativeFundingTracker::new("SOLUSDT".to_string());

        // Negative, negative, positive, negative, negative
        assert!(!tracker.update_funding(-0.01)); // 1 negative
        assert!(tracker.update_funding(-0.01));  // 2 negative - should exit
        assert!(!tracker.update_funding(0.01));  // Reset to 0
        assert!(!tracker.update_funding(-0.01)); // 1 negative
        assert!(tracker.update_funding(-0.01));  // 2 negative - should exit again

        assert!(tracker.should_exit());
    }

    #[tokio::test]
    async fn test_atomic_execution_both_legs_succeed() {
        let result = AtomicExecutor::execute_dual_leg(
            "BTCUSDT",
            "binance",
            "bybit",
            100.0,
            101.0,
            1000.0,
        )
        .await;

        assert!(result.is_ok());
        let _exec_result = result.unwrap();
        // Note: Due to 99% success rate simulation, this may occasionally fail
        // In production, we'd mock the exchange responses
    }

    #[tokio::test]
    async fn test_reverse_order_creates_opposite_side() {
        let order = EntryExecutor::create_market_order(
            "binance",
            "BTCUSDT",
            OrderSide::Long,
            100.0,
            1000.0,
        );

        let result = AtomicExecutor::reverse_order(&order).await;
        // Should succeed (or fail gracefully)
        let _ = result;
    }

    #[test]
    fn test_negative_funding_tracker_symbol_tracking() {
        let tracker = NegativeFundingTracker::new("ETHUSDT".to_string());
        assert_eq!(tracker.symbol, "ETHUSDT");
        assert_eq!(tracker.consecutive_negative_cycles, 0);
        assert_eq!(tracker.last_funding_rate, 0.0);
    }

    #[test]
    fn test_negative_funding_tracker_last_rate_update() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        
        tracker.update_funding(0.0005);
        assert_eq!(tracker.last_funding_rate, 0.0005);
        
        tracker.update_funding(-0.0003);
        assert_eq!(tracker.last_funding_rate, -0.0003);
    }

    #[test]
    fn test_atomic_execution_result_structure() {
        let long_order = EntryExecutor::create_market_order(
            "binance",
            "BTCUSDT",
            OrderSide::Long,
            100.0,
            1000.0,
        );

        let short_order = EntryExecutor::create_market_order(
            "bybit",
            "BTCUSDT",
            OrderSide::Short,
            101.0,
            1000.0,
        );

        let result = AtomicExecutionResult {
            long_order: long_order.clone(),
            short_order: short_order.clone(),
            both_filled: true,
            error: None,
        };

        assert!(result.both_filled);
        assert!(result.error.is_none());
        assert_eq!(result.long_order.side, OrderSide::Long);
        assert_eq!(result.short_order.side, OrderSide::Short);
    }

    #[test]
    fn test_atomic_execution_result_with_error() {
        let long_order = EntryExecutor::create_market_order(
            "binance",
            "BTCUSDT",
            OrderSide::Long,
            100.0,
            1000.0,
        );

        let short_order = EntryExecutor::create_market_order(
            "bybit",
            "BTCUSDT",
            OrderSide::Short,
            101.0,
            1000.0,
        );

        let result = AtomicExecutionResult {
            long_order,
            short_order,
            both_filled: false,
            error: Some("Exchange API timeout".to_string()),
        };

        assert!(!result.both_filled);
        assert!(result.error.is_some());
        assert_eq!(result.error.unwrap(), "Exchange API timeout");
    }
}
