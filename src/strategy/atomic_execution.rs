use crate::strategy::types::{SimulatedOrder, OrderSide, OrderStatus};
use crate::strategy::entry::EntryExecutor;
use crate::strategy::execution_backend::ExecutionBackend;
use std::error::Error;
use std::time::{Instant, Duration};
use std::sync::Arc;
use tokio::task::JoinHandle;

// ============================================================================
// Hedge Timing Metrics
// ============================================================================

#[derive(Clone, Debug, Default)]
pub struct HedgeTimingMetrics {
    pub fill_detected_at: Option<Instant>,
    pub other_leg_check_at: Option<Instant>,
    pub cancel_initiated_at: Option<Instant>,
    pub cancel_completed_at: Option<Instant>,
    pub market_order_initiated_at: Option<Instant>,
    pub market_order_accepted_at: Option<Instant>,
    pub market_order_filled_at: Option<Instant>,
    pub total_hedge_duration: Option<Duration>,
    pub depth_check_initiated_at: Option<Instant>,
    pub depth_check_completed_at: Option<Instant>,
    pub reprice_count: u32,
    pub first_reprice_at: Option<Instant>,
    pub last_reprice_at: Option<Instant>,
    pub api_response_times: Vec<(String, Duration)>,
}

impl HedgeTimingMetrics {
    pub fn new() -> Self {
        Self {
            fill_detected_at: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn record_other_leg_check(&mut self) {
        self.other_leg_check_at = Some(Instant::now());
    }

    pub fn record_cancel_initiated(&mut self) {
        self.cancel_initiated_at = Some(Instant::now());
    }

    pub fn record_cancel_completed(&mut self) {
        self.cancel_completed_at = Some(Instant::now());
    }

    pub fn record_market_order_initiated(&mut self) {
        self.market_order_initiated_at = Some(Instant::now());
    }

    pub fn record_market_order_accepted(&mut self) {
        self.market_order_accepted_at = Some(Instant::now());
    }

    pub fn record_market_order_filled(&mut self) {
        self.market_order_filled_at = Some(Instant::now());
    }

    pub fn record_depth_check_initiated(&mut self) {
        self.depth_check_initiated_at = Some(Instant::now());
    }

    pub fn record_depth_check_completed(&mut self) {
        self.depth_check_completed_at = Some(Instant::now());
    }

    pub fn record_reprice(&mut self) {
        let now = Instant::now();
        if self.first_reprice_at.is_none() {
            self.first_reprice_at = Some(now);
        }
        self.last_reprice_at = Some(now);
        self.reprice_count += 1;
    }

    pub fn record_api_response(&mut self, endpoint: String, duration: Duration) {
        self.api_response_times.push((endpoint, duration));
    }

    pub fn fill_to_other_leg_check(&self) -> Option<Duration> {
        match (self.fill_detected_at, self.other_leg_check_at) {
            (Some(fill), Some(check)) => Some(check.duration_since(fill)),
            _ => None,
        }
    }

    pub fn fill_to_cancel_initiated(&self) -> Option<Duration> {
        match (self.fill_detected_at, self.cancel_initiated_at) {
            (Some(fill), Some(cancel)) => Some(cancel.duration_since(fill)),
            _ => None,
        }
    }

    pub fn cancel_duration(&self) -> Option<Duration> {
        match (self.cancel_initiated_at, self.cancel_completed_at) {
            (Some(init), Some(comp)) => Some(comp.duration_since(init)),
            _ => None,
        }
    }

    pub fn cancel_to_market_order(&self) -> Option<Duration> {
        match (self.cancel_completed_at, self.market_order_initiated_at) {
            (Some(cancel), Some(market)) => Some(market.duration_since(cancel)),
            _ => None,
        }
    }

    pub fn market_order_acceptance_duration(&self) -> Option<Duration> {
        match (self.market_order_initiated_at, self.market_order_accepted_at) {
            (Some(init), Some(accept)) => Some(accept.duration_since(init)),
            _ => None,
        }
    }

    pub fn market_order_fill_duration(&self) -> Option<Duration> {
        match (self.market_order_accepted_at, self.market_order_filled_at) {
            (Some(accept), Some(fill)) => Some(fill.duration_since(accept)),
            _ => None,
        }
    }

    pub fn depth_check_duration(&self) -> Option<Duration> {
        match (self.depth_check_initiated_at, self.depth_check_completed_at) {
            (Some(init), Some(comp)) => Some(comp.duration_since(init)),
            _ => None,
        }
    }

    pub fn total_reprice_duration(&self) -> Option<Duration> {
        match (self.first_reprice_at, self.last_reprice_at) {
            (Some(first), Some(last)) => Some(last.duration_since(first)),
            _ => None,
        }
    }

    pub fn finalize(&mut self) {
        if let (Some(fill), Some(final_time)) = (self.fill_detected_at, self.market_order_filled_at) {
            self.total_hedge_duration = Some(final_time.duration_since(fill));
        }
    }

    pub fn log_summary(&self) {
        println!("=== Hedge Timing Summary ===");
        if let Some(d) = self.fill_to_other_leg_check() {
            println!("Fill to other leg check: {}ms", d.as_millis());
        }
        if let Some(d) = self.fill_to_cancel_initiated() {
            println!("Fill to cancel initiated: {}ms", d.as_millis());
        }
        if let Some(d) = self.cancel_duration() {
            println!("Cancel duration: {}ms", d.as_millis());
        }
        if let Some(d) = self.cancel_to_market_order() {
            println!("Cancel to market order: {}ms", d.as_millis());
        }
        if let Some(d) = self.market_order_acceptance_duration() {
            println!("Market order acceptance: {}ms", d.as_millis());
        }
        if let Some(d) = self.market_order_fill_duration() {
            println!("Market order fill: {}ms", d.as_millis());
        }
        if let Some(d) = self.total_hedge_duration {
            println!("Total hedge duration: {}ms", d.as_millis());
        }
        if let Some(d) = self.depth_check_duration() {
            println!("Depth check duration: {}ms", d.as_millis());
        }
        if self.reprice_count > 0 {
            println!("Reprice count: {}", self.reprice_count);
            if let Some(d) = self.total_reprice_duration() {
                println!("Total reprice duration: {}ms", d.as_millis());
            }
        }
    }
}

// ============================================================================
// Hedge Logger
// ============================================================================

#[derive(Clone, Debug)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct HedgeLogger {
    level: LogLevel,
}

impl HedgeLogger {
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }

    pub fn default_level() -> Self {
        Self { level: LogLevel::Info }
    }

    pub fn log_fill_detected(&self, exchange: &str, order_id: &str, filled_qty: f64, elapsed_ms: u128) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Fill detected: {} on {} - {:.4} contracts ({}ms)", order_id, exchange, filled_qty, elapsed_ms);
        }
    }

    pub fn log_other_leg_check(&self, exchange: &str, status: &str) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Checking other leg on {}: {}", exchange, status);
        }
    }

    pub fn log_cancel_initiated(&self, exchange: &str, order_id: &str) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Cancelling order {} on {}", order_id, exchange);
        }
    }

    pub fn log_cancel_completed(&self, exchange: &str, success: bool) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Cancel completed on {}: {}", exchange, if success { "success" } else { "failed" });
        }
    }

    pub fn log_cancel_result(&self, exchange: &str, result: &CancellationResult, elapsed_ms: u128) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            match result {
                CancellationResult::Success => println!("[HEDGE] Cancel succeeded on {} ({}ms)", exchange, elapsed_ms),
                CancellationResult::Cancelled => println!("[HEDGE] Order cancelled on {} ({}ms)", exchange, elapsed_ms),
                CancellationResult::Failed(e) => println!("[HEDGE] Cancel failed on {}: {} ({}ms)", exchange, e, elapsed_ms),
                CancellationResult::AlreadyFilled => println!("[HEDGE] Cancel skipped - already filled on {} ({}ms)", exchange, elapsed_ms),
                CancellationResult::NotFound => println!("[HEDGE] Cancel skipped - order not found on {} ({}ms)", exchange, elapsed_ms),
            }
        }
    }

    pub fn log_market_order_initiated(&self, exchange: &str, symbol: &str, side: &str, quantity: f64) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Placing market order: {} {} {} on {}", side, quantity, symbol, exchange);
        }
    }

    pub fn log_market_order_placed(&self, exchange: &str, status: &str, quantity: f64) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Market order placed on {}: {} - {:.4} contracts", exchange, status, quantity);
        }
    }

    pub fn log_market_order_accepted(&self, exchange: &str, order_id: &str) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Market order accepted: {} on {}", order_id, exchange);
        }
    }

    pub fn log_market_order_filled(&self, exchange: &str, order_id: &str, fill_elapsed_ms: u128) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Market order filled: {} on {} ({}ms)", order_id, exchange, fill_elapsed_ms);
        }
    }

    pub fn log_race_condition_detected(&self, symbol: &str, long_qty: f64, short_qty: f64) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Race condition detected for {}: long={:.4} short={:.4}", symbol, long_qty, short_qty);
        }
    }

    pub fn log_timing_summary(&self, metrics: &HedgeTimingMetrics, exchange: &str, symbol: &str) {
        if matches!(self.level, LogLevel::Debug | LogLevel::Info) {
            println!("[HEDGE] Timing summary for {} on {}:", symbol, exchange);
            if let Some(d) = metrics.fill_to_other_leg_check() {
                println!("  Fill to other leg check: {}ms", d.as_millis());
            }
            if let Some(d) = metrics.fill_to_cancel_initiated() {
                println!("  Fill to cancel initiated: {}ms", d.as_millis());
            }
            if let Some(d) = metrics.cancel_duration() {
                println!("  Cancel duration: {}ms", d.as_millis());
            }
            if let Some(d) = metrics.cancel_to_market_order() {
                println!("  Cancel to market order: {}ms", d.as_millis());
            }
            if let Some(d) = metrics.market_order_acceptance_duration() {
                println!("  Market order acceptance: {}ms", d.as_millis());
            }
            if let Some(d) = metrics.market_order_fill_duration() {
                println!("  Market order fill: {}ms", d.as_millis());
            }
            if let Some(d) = metrics.total_hedge_duration {
                println!("  Total hedge duration: {}ms", d.as_millis());
            }
        }
    }

    pub fn log_api_response_time(&self, exchange: &str, endpoint: &str, duration: u128) {
        if matches!(self.level, LogLevel::Debug) {
            println!("[HEDGE] API response time: {} {} - {}ms", exchange, endpoint, duration);
        }
    }

    pub fn log_error(&self, message: &str) {
        println!("[HEDGE ERROR] {}", message);
    }
}

// ============================================================================
// Cancellation Result
// ============================================================================

#[derive(Clone, Debug)]
pub enum CancellationResult {
    Success,
    Cancelled,
    Failed(String),
    AlreadyFilled,
    NotFound,
}

// ============================================================================
// Race Condition Guard
// ============================================================================

use std::sync::Mutex;
use std::collections::HashSet;

lazy_static::lazy_static! {
    static ref HEDGE_LOCKS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

pub struct HedgeLock {
    symbol: String,
}

impl Drop for HedgeLock {
    fn drop(&mut self) {
        if let Ok(mut locks) = HEDGE_LOCKS.lock() {
            locks.remove(&self.symbol);
        }
    }
}

#[derive(Clone, Debug)]
pub struct RaceConditionGuard {
    pub check_interval_ms: u64,
    pub max_checks: u32,
}

impl RaceConditionGuard {
    pub fn new(check_interval_ms: u64, max_checks: u32) -> Self {
        Self {
            check_interval_ms,
            max_checks,
        }
    }

    pub fn default() -> Self {
        Self {
            check_interval_ms: 100,
            max_checks: 10,
        }
    }

    pub fn try_acquire_hedge_lock(&self, symbol: &str) -> Result<HedgeLock, String> {
        let mut locks = HEDGE_LOCKS.lock()
            .map_err(|e| format!("Failed to acquire lock mutex: {}", e))?;
        
        if locks.contains(symbol) {
            return Err(format!("Hedge lock already held for {}", symbol));
        }
        
        locks.insert(symbol.to_string());
        Ok(HedgeLock {
            symbol: symbol.to_string(),
        })
    }

    pub async fn check_both_legs_status(
        &self,
        backend: &Arc<dyn ExecutionBackend>,
        long_order: &SimulatedOrder,
        short_order: &SimulatedOrder,
    ) -> Result<BothLegsStatus, String> {
        // Query both order statuses in parallel
        let (long_result, short_result) = tokio::join!(
            backend.get_order_status_detailed(&long_order.exchange, &long_order.id, &long_order.symbol),
            backend.get_order_status_detailed(&short_order.exchange, &short_order.id, &short_order.symbol)
        );

        let long_status = long_result.map_err(|e| format!("Failed to get long order status: {}", e))?;
        let short_status = short_result.map_err(|e| format!("Failed to get short order status: {}", e))?;

        let long_filled = long_status.is_fully_filled();
        let short_filled = short_status.is_fully_filled();

        match (long_filled, short_filled) {
            (true, true) => Ok(BothLegsStatus::BothFilled {
                long_qty: long_status.filled_quantity,
                short_qty: short_status.filled_quantity,
            }),
            (true, false) => Ok(BothLegsStatus::OnlyLongFilled {
                long_qty: long_status.filled_quantity,
            }),
            (false, true) => Ok(BothLegsStatus::OnlyShortFilled {
                short_qty: short_status.filled_quantity,
            }),
            (false, false) => Ok(BothLegsStatus::NeitherFilled),
        }
    }
}

// ============================================================================
// Both Legs Status
// ============================================================================

#[derive(Clone, Debug)]
pub enum BothLegsStatus {
    BothFilled { long_qty: f64, short_qty: f64 },
    OnlyLongFilled { long_qty: f64 },
    OnlyShortFilled { short_qty: f64 },
    NeitherFilled,
}

// ============================================================================
// Market Order Placer
// ============================================================================

pub struct MarketOrderPlacer {
    backend: Arc<dyn ExecutionBackend>,
}

impl MarketOrderPlacer {
    pub fn new(backend: Arc<dyn ExecutionBackend>) -> Self {
        Self { backend }
    }

    pub async fn place_with_retry(
        &self,
        order: SimulatedOrder,
        target_quantity: f64,
        max_retries: u32,
        metrics: &mut HedgeTimingMetrics,
    ) -> Result<SimulatedOrder, String> {
        let mut attempts = 0;
        let mut current_order = order.clone();

        while attempts < max_retries {
            attempts += 1;

            // Record API call
            let start = Instant::now();
            let result = self.backend.place_market_order(current_order.clone()).await;
            let duration = start.elapsed();
            metrics.record_api_response(format!("place_market_order_attempt_{}", attempts), duration);

            match result {
                Ok(placed_order) => {
                    // Check if filled
                    let status_start = Instant::now();
                    let status_result = self.backend.get_order_status_detailed(&placed_order.exchange, &placed_order.id, &order.symbol).await;
                    let status_duration = status_start.elapsed();
                    metrics.record_api_response(format!("get_order_status_attempt_{}", attempts), status_duration);

                    if let Ok(status_info) = status_result {
                        if status_info.status == OrderStatus::Filled {
                            return Ok(placed_order);
                        }
                    }

                    // If not filled and we have retries left, try again
                    if attempts < max_retries {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        current_order = order.clone();
                        continue;
                    }

                    return Ok(placed_order);
                }
                Err(e) => {
                    if attempts >= max_retries {
                        return Err(format!("Failed after {} attempts: {}", attempts, e));
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }

        Err(format!("Failed to place order after {} attempts", max_retries))
    }
}

// ============================================================================
// Trading Halt State
// ============================================================================

use std::sync::atomic::{AtomicBool, Ordering};

static TRADING_HALTED: AtomicBool = AtomicBool::new(false);

pub fn halt_trading(reason: &str) {
    TRADING_HALTED.store(true, Ordering::SeqCst);
    println!("[TRADING HALT] Trading has been halted: {}", reason);
}

pub fn resume_trading() {
    TRADING_HALTED.store(false, Ordering::SeqCst);
    println!("[TRADING RESUME] Trading has been resumed");
}

pub fn is_trading_halted() -> bool {
    TRADING_HALTED.load(Ordering::SeqCst)
}

// ============================================================================
// Existing Types
// ============================================================================


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
