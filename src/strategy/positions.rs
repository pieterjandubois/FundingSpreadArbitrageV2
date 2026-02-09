use crate::strategy::types::PaperTrade;

pub struct PositionManager;

impl PositionManager {
    /// Calculate unrealized P&L for a position
    /// 
    /// Formula: (current_long_price - entry_long_price) * position_size - (current_short_price - entry_short_price) * position_size
    /// 
    /// This accounts for:
    /// - Long leg profit: when current_long_price > entry_long_price
    /// - Short leg profit: when current_short_price < entry_short_price
    /// - Fees and slippage are accounted for in the position_size calculation
    pub fn calculate_unrealized_pnl(
        entry_long_price: f64,
        entry_short_price: f64,
        current_long_price: f64,
        current_short_price: f64,
        position_size: f64,
    ) -> f64 {
        // Long leg P&L: profit when price goes up
        let long_pnl = (current_long_price - entry_long_price) / entry_long_price * position_size;
        
        // Short leg P&L: profit when price goes down
        let short_pnl = (entry_short_price - current_short_price) / entry_short_price * position_size;
        
        long_pnl + short_pnl
    }

    /// Check if any exit conditions are met
    /// 
    /// Exit conditions (in priority order):
    /// 1. Profit target: unrealized_pnl >= projected_profit * 0.9 (90% of projected profit)
    /// 2. Loss limit: unrealized_pnl <= -projected_profit * 0.3 (loss exceeds 30% of projected profit)
    /// 3. Funding convergence: current_funding_delta < 0.005% (0.00005)
    /// 4. Spread widening: current_spread > entry_spread + 50 bps
    /// 5. Stop loss: current_spread > entry_spread + 100 bps
    /// 
    /// Returns: Some(exit_reason) if any condition is met, None otherwise
    pub fn check_exit_conditions(
        trade: &PaperTrade,
        current_funding_delta: f64,
        current_spread_bps: f64,
        unrealized_pnl: f64,
    ) -> Option<String> {
        // Profit target: 90% of projected profit
        if unrealized_pnl >= trade.projected_profit_usd * 0.9 {
            return Some("profit_target".to_string());
        }

        // Loss limit: loss exceeds 30% of projected profit
        if unrealized_pnl <= -trade.projected_profit_usd * 0.3 {
            return Some("loss_limit".to_string());
        }

        // Funding rate convergence: delta < 0.005% (0.00005)
        if current_funding_delta.abs() < 0.00005 {
            return Some("funding_convergence".to_string());
        }

        // Spread widening: entry_spread + 50 bps
        if current_spread_bps > trade.entry_spread_bps + 50.0 {
            return Some("spread_widening".to_string());
        }

        // Stop loss: entry_spread + 100 bps
        if current_spread_bps > trade.entry_spread_bps + 100.0 {
            return Some("stop_loss".to_string());
        }

        None
    }

    /// Detect if a leg-out condition has occurred
    /// 
    /// Leg-out occurs when:
    /// - One leg is filled and the other is not
    /// - AND more than 500ms has passed since entry
    /// 
    /// This indicates that one side of the trade filled but the other didn't,
    /// creating a naked position that needs to be hedged immediately.
    pub fn detect_leg_out(
        long_filled: bool,
        short_filled: bool,
        time_since_entry_ms: u64,
    ) -> bool {
        let one_leg_filled = (long_filled && !short_filled) || (!long_filled && short_filled);
        one_leg_filled && time_since_entry_ms > 500
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::types::{TradeStatus, SimulatedOrder, OrderSide, OrderType, OrderStatus};

    #[test]
    fn test_unrealized_pnl_positive() {
        // Long price goes up, short price goes down = profit
        let pnl = PositionManager::calculate_unrealized_pnl(
            100.0,  // entry_long_price
            101.0,  // entry_short_price
            101.0,  // current_long_price (up 1%)
            100.0,  // current_short_price (down 1%)
            1000.0, // position_size
        );
        assert!(pnl > 0.0, "Expected positive P&L, got {}", pnl);
    }

    #[test]
    fn test_unrealized_pnl_negative() {
        // Long price goes down, short price goes up = loss
        let pnl = PositionManager::calculate_unrealized_pnl(
            100.0,  // entry_long_price
            101.0,  // entry_short_price
            99.0,   // current_long_price (down 1%)
            102.0,  // current_short_price (up 1%)
            1000.0, // position_size
        );
        assert!(pnl < 0.0, "Expected negative P&L, got {}", pnl);
    }

    #[test]
    fn test_unrealized_pnl_zero() {
        // Prices unchanged = zero P&L
        let pnl = PositionManager::calculate_unrealized_pnl(
            100.0,
            101.0,
            100.0,
            101.0,
            1000.0,
        );
        assert!((pnl - 0.0).abs() < 0.01, "Expected zero P&L, got {}", pnl);
    }

    #[test]
    fn test_exit_condition_profit_target() {
        let trade = create_test_trade(100.0);
        
        // Unrealized P&L = 90% of projected profit
        let exit_reason = PositionManager::check_exit_conditions(
            &trade,
            0.0001,
            100.0,
            90.0, // 90% of 100.0 projected profit
        );
        
        assert_eq!(exit_reason, Some("profit_target".to_string()));
    }

    #[test]
    fn test_exit_condition_loss_limit() {
        let trade = create_test_trade(100.0);
        
        // Unrealized P&L = -30% of projected profit
        let exit_reason = PositionManager::check_exit_conditions(
            &trade,
            0.0001,
            100.0,
            -30.0, // -30% of 100.0 projected profit
        );
        
        assert_eq!(exit_reason, Some("loss_limit".to_string()));
    }

    #[test]
    fn test_exit_condition_funding_convergence() {
        let trade = create_test_trade(100.0);
        
        // Funding delta < 0.005% (0.00005)
        let exit_reason = PositionManager::check_exit_conditions(
            &trade,
            0.00001, // Less than 0.00005
            100.0,
            50.0,
        );
        
        assert_eq!(exit_reason, Some("funding_convergence".to_string()));
    }

    #[test]
    fn test_exit_condition_spread_widening() {
        let trade = create_test_trade(100.0);
        
        // Current spread > entry_spread + 50 bps
        let exit_reason = PositionManager::check_exit_conditions(
            &trade,
            0.0001,
            151.0, // entry_spread (100) + 51 bps
            50.0,
        );
        
        assert_eq!(exit_reason, Some("spread_widening".to_string()));
    }

    #[test]
    fn test_exit_condition_stop_loss() {
        let trade = create_test_trade(100.0);
        
        // Current spread > entry_spread + 100 bps
        let exit_reason = PositionManager::check_exit_conditions(
            &trade,
            0.0001,
            201.0, // entry_spread (100) + 101 bps
            50.0,
        );
        
        assert_eq!(exit_reason, Some("stop_loss".to_string()));
    }

    #[test]
    fn test_no_exit_condition() {
        let trade = create_test_trade(100.0);
        
        // No exit conditions met
        let exit_reason = PositionManager::check_exit_conditions(
            &trade,
            0.0001,
            100.0,
            50.0,
        );
        
        assert_eq!(exit_reason, None);
    }

    #[test]
    fn test_detect_leg_out_long_filled_short_not() {
        let is_leg_out = PositionManager::detect_leg_out(
            true,   // long_filled
            false,  // short_filled
            600,    // time_since_entry_ms (> 500ms)
        );
        
        assert!(is_leg_out);
    }

    #[test]
    fn test_detect_leg_out_short_filled_long_not() {
        let is_leg_out = PositionManager::detect_leg_out(
            false,  // long_filled
            true,   // short_filled
            600,    // time_since_entry_ms (> 500ms)
        );
        
        assert!(is_leg_out);
    }

    #[test]
    fn test_detect_leg_out_both_filled() {
        let is_leg_out = PositionManager::detect_leg_out(
            true,   // long_filled
            true,   // short_filled
            600,    // time_since_entry_ms (> 500ms)
        );
        
        assert!(!is_leg_out);
    }

    #[test]
    fn test_detect_leg_out_neither_filled() {
        let is_leg_out = PositionManager::detect_leg_out(
            false,  // long_filled
            false,  // short_filled
            600,    // time_since_entry_ms (> 500ms)
        );
        
        assert!(!is_leg_out);
    }

    #[test]
    fn test_detect_leg_out_timeout_not_exceeded() {
        let is_leg_out = PositionManager::detect_leg_out(
            true,   // long_filled
            false,  // short_filled
            400,    // time_since_entry_ms (< 500ms)
        );
        
        assert!(!is_leg_out);
    }

    fn create_test_trade(projected_profit: f64) -> PaperTrade {
        PaperTrade {
            id: "test".to_string(),
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            entry_time: 0,
            entry_long_price: 100.0,
            entry_short_price: 101.0,
            entry_spread_bps: 100.0,
            position_size_usd: 1000.0,
            funding_delta_entry: 0.0002,
            projected_profit_usd: projected_profit,
            actual_profit_usd: 0.0,
            status: TradeStatus::Active,
            exit_reason: None,
            exit_spread_bps: None,
            exit_time: None,
            long_order: SimulatedOrder {
                id: "long_order".to_string(),
                exchange: "binance".to_string(),
                symbol: "BTCUSDT".to_string(),
                side: OrderSide::Long,
                order_type: OrderType::Limit,
                price: 100.0,
                size: 1000.0,
                queue_position: None,
                created_at: 0,
                filled_at: Some(0),
                fill_price: Some(100.0),
                status: OrderStatus::Filled,
            },
            short_order: SimulatedOrder {
                id: "short_order".to_string(),
                exchange: "bybit".to_string(),
                symbol: "BTCUSDT".to_string(),
                side: OrderSide::Short,
                order_type: OrderType::Limit,
                price: 101.0,
                size: 1000.0,
                queue_position: None,
                created_at: 0,
                filled_at: Some(0),
                fill_price: Some(101.0),
                status: OrderStatus::Filled,
            },
            leg_out_event: None,
        }
    }
}
