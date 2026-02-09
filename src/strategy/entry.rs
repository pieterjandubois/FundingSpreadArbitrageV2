use crate::strategy::types::{
    SimulatedOrder, OrderSide, OrderType, OrderStatus, ArbitrageOpportunity, 
    PaperTrade, QueuePosition, TradeStatus
};
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct EntryExecutor;

/// Identifies which leg of a trade is "harder" to fill (has less liquidity).
/// 
/// The "harder leg" is the one that should be filled first, as it's more likely 
/// to have slippage or not fill at all. This helps with atomic execution - we 
/// place the harder leg first, and if it fills, we know we can likely fill the 
/// easier leg.
/// 
/// Exchange liquidity tiers:
/// - Tier 1 (High liquidity): Binance, Bybit, OKX, Deribit
/// - Tier 2 (Medium liquidity): Bitget, KuCoin, Gate.io
/// - Tier 3 (Lower liquidity): Hyperliquid, Paradex, Lighter
/// 
/// Returns: "long" or "short" indicating which leg is harder to fill
pub fn identify_harder_leg(long_exchange: &str, short_exchange: &str) -> String {
    let long_tier = get_exchange_tier(long_exchange);
    let short_tier = get_exchange_tier(short_exchange);
    
    // The harder leg is on the exchange with lower tier (higher tier number = lower liquidity)
    if long_tier > short_tier {
        "long".to_string()
    } else if short_tier > long_tier {
        "short".to_string()
    } else {
        // If same tier, use alphabetical order for deterministic behavior
        // The exchange that comes first alphabetically is the "harder" leg
        if long_exchange.to_lowercase() < short_exchange.to_lowercase() {
            "long".to_string()
        } else if long_exchange.to_lowercase() > short_exchange.to_lowercase() {
            "short".to_string()
        } else {
            // If both exchanges are identical, arbitrarily choose "long"
            "long".to_string()
        }
    }
}

/// Returns the liquidity tier of an exchange (lower number = higher liquidity).
/// 
/// Tier 1 (highest liquidity): 1
/// Tier 2 (medium liquidity): 2
/// Tier 3 (lower liquidity): 3
fn get_exchange_tier(exchange: &str) -> u8 {
    match exchange.to_lowercase().as_str() {
        // Tier 1: High liquidity
        "binance" => 1,
        "bybit" => 1,
        "okx" => 1,
        "deribit" => 1,
        
        // Tier 2: Medium liquidity
        "bitget" => 2,
        "kucoin" => 2,
        "gate" | "gateio" | "gate.io" => 2,
        
        // Tier 3: Lower liquidity
        "hyperliquid" => 3,
        "paradex" => 3,
        "lighter" => 3,
        
        // Default to Tier 3 for unknown exchanges (conservative approach)
        _ => 3,
    }
}

impl EntryExecutor {
    pub fn calculate_position_size(
        spread_bps: f64,
        available_capital: f64,
        fees_bps: f64,
        funding_cost_bps: f64,
    ) -> f64 {
        // Formula: base_size = (spread_bps - fees - funding_cost) / spread_bps * available_capital
        let numerator = spread_bps - fees_bps - funding_cost_bps;
        
        // If net profit is non-positive, return 0 (no profitable trade)
        if numerator <= 0.0 {
            return 0.0;
        }
        
        // If spread is 0 or negative, return 0 (invalid spread)
        if spread_bps <= 0.0 {
            return 0.0;
        }

        let base_size = (numerator / spread_bps) * available_capital;
        
        // Cap at 50% of available capital per trade
        let capped_size = base_size.min(available_capital * 0.5);
        
        // Adaptive minimum position size: $10 or 1% of available capital, whichever is larger
        // This prevents the minimum from exceeding available capital when capital is low
        let adaptive_minimum = (10.0_f64).max(available_capital * 0.01);
        capped_size.max(adaptive_minimum)
    }

    pub fn calculate_slippage(position_size: f64, order_book_depth: f64) -> f64 {
        let base_slippage = 0.0002;
        let depth_ratio = position_size / order_book_depth;
        let additional_slippage = depth_ratio * 0.0003;
        (base_slippage + additional_slippage).min(0.0005)
    }

    #[allow(dead_code)]
    pub fn create_market_order(
        exchange: &str,
        symbol: &str,
        side: OrderSide,
        price: f64,
        size: f64,
    ) -> SimulatedOrder {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        SimulatedOrder {
            id: Uuid::new_v4().to_string(),
            exchange: exchange.to_string(),
            symbol: symbol.to_string(),
            side,
            order_type: OrderType::Market,
            price,
            size,
            queue_position: None,
            created_at: now,
            filled_at: None,
            fill_price: None,
            status: OrderStatus::Pending,
        }
    }

    /// Create a limit order with queue position tracking
    fn create_limit_order(
        exchange: &str,
        symbol: &str,
        side: OrderSide,
        price: f64,
        size: f64,
        resting_depth: f64,
    ) -> SimulatedOrder {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let queue_position = QueuePosition {
            price,
            cumulative_volume_at_price: 0.0,
            resting_depth_at_entry: resting_depth,
            fill_threshold_pct: 0.20,  // 20% of resting depth
            is_filled: false,
        };

        SimulatedOrder {
            id: Uuid::new_v4().to_string(),
            exchange: exchange.to_string(),
            symbol: symbol.to_string(),
            side,
            order_type: OrderType::Limit,
            price,
            size,
            queue_position: Some(queue_position),
            created_at: now,
            filled_at: None,
            fill_price: None,
            status: OrderStatus::Pending,
        }
    }

    /// Simulate order fill based on queue position
    /// Returns true if order should fill (cumulative volume >= 20% of resting depth)
    fn simulate_order_fill(order: &mut SimulatedOrder, cumulative_volume: f64) -> bool {
        if let Some(ref mut queue_pos) = order.queue_position {
            queue_pos.cumulative_volume_at_price = cumulative_volume;
            
            let fill_threshold = queue_pos.resting_depth_at_entry * queue_pos.fill_threshold_pct;
            if cumulative_volume >= fill_threshold {
                queue_pos.is_filled = true;
                order.status = OrderStatus::Filled;
                order.fill_price = Some(queue_pos.price);
                order.filled_at = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                );
                return true;
            }
        }
        false
    }

    /// Execute atomic dual-leg entry with 500ms timeout per leg
    /// 
    /// This function implements the atomic execution logic:
    /// 1. Place limit order on harder leg
    /// 2. Wait up to 500ms for fill
    /// 3. If filled: place limit order on easier leg
    /// 4. Wait up to 500ms for fill
    /// 5. If both filled: create PaperTrade with status=Active
    /// 6. If one doesn't fill: cancel other and reject trade
    /// 7. Deduct capital from available pool
    /// 8. Add to active_trades list
    pub fn execute_atomic_entry(
        opportunity: &ArbitrageOpportunity,
        available_capital: f64,
        position_size: f64,
    ) -> Result<PaperTrade, String> {
        // Validate inputs
        if position_size <= 0.0 {
            return Err("Position size must be positive".to_string());
        }

        if position_size > available_capital {
            return Err(format!(
                "Position size ${:.2} exceeds available capital ${:.2}",
                position_size, available_capital
            ));
        }

        // Identify harder leg
        let harder_leg = identify_harder_leg(&opportunity.long_exchange, &opportunity.short_exchange);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Create limit orders for both legs
        let mut long_order = Self::create_limit_order(
            &opportunity.long_exchange,
            &opportunity.symbol,
            OrderSide::Long,
            opportunity.long_price,
            position_size,
            opportunity.order_book_depth_long,
        );

        let mut short_order = Self::create_limit_order(
            &opportunity.short_exchange,
            &opportunity.symbol,
            OrderSide::Short,
            opportunity.short_price,
            position_size,
            opportunity.order_book_depth_short,
        );

        // Step 1: Place harder leg first
        let harder_leg_is_long = harder_leg == "long";
        let (harder_order, easier_order) = if harder_leg_is_long {
            (&mut long_order, &mut short_order)
        } else {
            (&mut short_order, &mut long_order)
        };

        // Step 2: Simulate 500ms timeout for harder leg
        // For simulation: assume 25% of resting depth trades in 500ms
        let harder_cumulative_volume = opportunity.order_book_depth_long * 0.25;
        let harder_filled = Self::simulate_order_fill(harder_order, harder_cumulative_volume);

        if !harder_filled {
            return Err(format!(
                "Harder leg ({:?}) failed to fill on {} within 500ms",
                harder_order.side, harder_order.exchange
            ));
        }

        // Step 3: Place easier leg
        // Step 4: Simulate 500ms timeout for easier leg
        // For simulation: assume 30% of resting depth trades in 500ms (easier to fill)
        let easier_cumulative_volume = if easier_order.side == OrderSide::Long {
            opportunity.order_book_depth_long * 0.30
        } else {
            opportunity.order_book_depth_short * 0.30
        };

        let easier_filled = Self::simulate_order_fill(easier_order, easier_cumulative_volume);

        if !easier_filled {
            // Cancel harder leg
            harder_order.status = OrderStatus::Cancelled;
            return Err(format!(
                "Easier leg ({:?}) failed to fill on {} within 500ms. Harder leg cancelled.",
                easier_order.side, easier_order.exchange
            ));
        }

        // Step 5: Both legs filled - create PaperTrade with status=Active
        let trade_id = Uuid::new_v4().to_string();
        
        // Calculate entry spread in basis points
        let entry_spread_bps = ((opportunity.short_price - opportunity.long_price) / opportunity.long_price) * 10000.0;

        let paper_trade = PaperTrade {
            id: trade_id,
            symbol: opportunity.symbol.clone(),
            long_exchange: opportunity.long_exchange.clone(),
            short_exchange: opportunity.short_exchange.clone(),
            entry_time: now,
            entry_long_price: opportunity.long_price,
            entry_short_price: opportunity.short_price,
            entry_spread_bps,
            position_size_usd: position_size,
            funding_delta_entry: opportunity.funding_delta_8h,
            projected_profit_usd: opportunity.projected_profit_after_slippage,
            actual_profit_usd: 0.0,
            status: TradeStatus::Active,
            exit_reason: None,
            exit_spread_bps: None,
            exit_time: None,
            long_order,
            short_order,
            leg_out_event: None,
        };

        Ok(paper_trade)
    }
}

