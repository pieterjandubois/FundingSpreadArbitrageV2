use crate::strategy::types::{
    SimulatedOrder, OrderSide, OrderType, OrderStatus, ArbitrageOpportunity, 
    PaperTrade, QueuePosition, TradeStatus
};
use crate::strategy::execution_backend::ExecutionBackend;
use crate::strategy::atomic_execution::{HedgeTimingMetrics, HedgeLogger, CancellationResult, RaceConditionGuard, BothLegsStatus};
use crate::strategy::depth_checker::DepthChecker;
use crate::strategy::price_chaser::{PriceChaser, RepricingConfig, RepricingMetrics, ExecutionMode};
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH, Duration, Instant};
use std::sync::Arc;

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
    /// Place market order with retry logic and verification (Scenarios 1 & 2 fix)
    /// 
    /// This function:
    /// 1. Places market order
    /// 2. Verifies it filled completely
    /// 3. If partial fill, places another market order for remaining quantity
    /// 4. Retries up to 3 times
    /// 5. Returns actual filled quantity
    async fn place_market_order_with_retry(
        backend: &Arc<dyn ExecutionBackend>,
        order_template: SimulatedOrder,
        target_quantity: f64,
        mut metrics: Option<&mut HedgeTimingMetrics>,
        logger: Option<&HedgeLogger>,
    ) -> Result<SimulatedOrder, String> {
        const MAX_RETRIES: u32 = 3;
        let mut total_filled = 0.0;
        let mut last_order = order_template.clone();
        
        for attempt in 1..=MAX_RETRIES {
            let remaining_qty = target_quantity - total_filled;
            
            if remaining_qty <= 0.0 {
                eprintln!("[MARKET ORDER] ✅ Fully hedged: {:.4} contracts", total_filled);
                last_order.size = total_filled;
                return Ok(last_order);
            }
            
            eprintln!("[MARKET ORDER] Attempt {}/{}: Placing market order for {:.4} contracts on {}", 
                attempt, MAX_RETRIES, remaining_qty, order_template.exchange);
            
            // Update order template with remaining quantity
            let mut current_order = order_template.clone();
            current_order.size = remaining_qty;
            
            // Place market order
            let api_start = Instant::now();
            match backend.place_market_order(current_order).await {
                Ok(filled_order) => {
                    let api_duration = api_start.elapsed();
                    if let Some(m) = metrics.as_deref_mut() {
                        m.record_api_response(
                            format!("place_market_order({})", order_template.exchange),
                            api_duration
                        );
                    }
                    if let Some(l) = logger {
                        l.log_api_response_time(&order_template.exchange, "place_market_order", api_duration.as_millis());
                    }
                    
                    last_order = filled_order.clone();
                    
                    // Fast-path verification: Query order status immediately after placement
                    // Target < 500ms for fill verification (Requirement 2.4)
                    let verification_start = Instant::now();
                    let api_start2 = Instant::now();
                    match backend.get_order_status_detailed(&filled_order.exchange, &filled_order.id, &order_template.symbol).await {
                        Ok(status_info) => {
                            let api_duration2 = api_start2.elapsed();
                            let verification_duration = verification_start.elapsed().as_millis();
                            
                            if let Some(m) = metrics.as_deref_mut() {
                                m.record_api_response(
                                    format!("get_order_status_detailed({})", order_template.exchange),
                                    api_duration2
                                );
                            }
                            if let Some(l) = logger {
                                l.log_api_response_time(&order_template.exchange, "get_order_status_detailed", api_duration2.as_millis());
                            }
                            
                            let filled_qty = status_info.filled_quantity;
                            total_filled += filled_qty;
                            
                            eprintln!("[MARKET ORDER] Filled {:.4} contracts (total: {:.4}/{:.4}) - verified in {}ms", 
                                filled_qty, total_filled, target_quantity, verification_duration);
                            
                            if total_filled >= target_quantity {
                                eprintln!("[MARKET ORDER] ✅ Fully hedged after {} attempt(s)", attempt);
                                last_order.size = total_filled;
                                return Ok(last_order);
                            } else {
                                eprintln!("[MARKET ORDER] ⚠️  Partial fill: {:.1}% - retrying for remaining {:.4}", 
                                    (total_filled / target_quantity) * 100.0, target_quantity - total_filled);
                            }
                        }
                        Err(e) => {
                            eprintln!("[MARKET ORDER] ❌ Failed to verify fill status: {}", e);
                            eprintln!("[MARKET ORDER] ⚠️  Cannot confirm hedge - will retry with exponential backoff");
                            
                            // DO NOT assume it filled - retry with backoff
                            // This prevents unhedged positions when verification fails
                            if attempt < MAX_RETRIES {
                                let backoff_ms = 100 * (1 << (attempt - 1)); // 100ms, 200ms, 400ms
                                eprintln!("[MARKET ORDER] Waiting {}ms before retry...", backoff_ms);
                                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                                continue;
                            } else {
                                return Err(format!(
                                    "Failed to verify market order fill after {} attempts: {}. Cannot confirm hedge.",
                                    MAX_RETRIES, e
                                ));
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[MARKET ORDER] ❌ Attempt {}/{} failed: {}", attempt, MAX_RETRIES, e);
                    
                    if attempt == MAX_RETRIES {
                        return Err(format!("Market order failed after {} attempts: {}", MAX_RETRIES, e));
                    }
                    
                    // Wait before retry
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        
        Err(format!("Failed to fully hedge: only filled {:.4}/{:.4} contracts", total_filled, target_quantity))
    }

    /// Emergency close position (Scenario 1 fix)
    /// 
    /// If hedge fails, immediately close the filled position to avoid naked exposure
    async fn emergency_close_position(
            backend: &Arc<dyn ExecutionBackend>,
            filled_order: &SimulatedOrder,
        ) -> Result<(), String> {
            use std::time::Instant;
            use crate::strategy::atomic_execution::{MarketOrderPlacer, HedgeTimingMetrics};

            let start_time = Instant::now();

            eprintln!("[EMERGENCY] 🚨 Closing naked position: {} contracts on {} (symbol: {})", 
                filled_order.size, filled_order.exchange, filled_order.symbol);

            // Create opposite market order to close position
            let close_side = match filled_order.side {
                OrderSide::Long => OrderSide::Short,  // Close long = sell
                OrderSide::Short => OrderSide::Long,  // Close short = buy
            };

            let close_order = SimulatedOrder {
                id: format!("emergency_close_{}", uuid::Uuid::new_v4()),
                exchange: filled_order.exchange.clone(),
                symbol: filled_order.symbol.clone(),
                side: close_side,
                order_type: OrderType::Market,
                price: filled_order.price,
                size: filled_order.size,
                queue_position: None,
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                filled_at: None,
                fill_price: None,
                status: OrderStatus::Pending,
            };

            // Use MarketOrderPlacer with retry logic (up to 10 attempts)
            let placer = MarketOrderPlacer::new(backend.clone());
            let mut metrics = HedgeTimingMetrics::new();

            // First attempt with full quantity
            match placer.place_with_retry(close_order.clone(), close_order.size, 10, &mut metrics).await {
                Ok(_) => {
                    let elapsed = start_time.elapsed();
                    eprintln!("[EMERGENCY] ✅ Position closed successfully in {}ms", elapsed.as_millis());

                    // Verify we met the 1-second timing requirement
                    if elapsed.as_millis() > 1000 {
                        eprintln!("[EMERGENCY] ⚠️  WARNING: Emergency close took {}ms (target: <1000ms)", elapsed.as_millis());
                    }

                    Ok(())
                }
                Err(e) => {
                    // Check if error is due to quantity limits
                    if e.contains("Quantity greater than max quantity") || e.contains("exceeds minimum limit") {
                        eprintln!("[EMERGENCY] ⚠️  Quantity limit hit, attempting to split order into smaller chunks");
                        
                        // Try splitting into 2 orders of half size
                        let half_size = close_order.size / 2.0;
                        eprintln!("[EMERGENCY] Attempting to close in 2 orders of {} contracts each", half_size);
                        
                        let mut close_order_half = close_order.clone();
                        close_order_half.size = half_size;
                        
                        // Try first half
                        match placer.place_with_retry(close_order_half.clone(), half_size, 5, &mut metrics).await {
                            Ok(_) => {
                                eprintln!("[EMERGENCY] ✅ First half closed ({} contracts)", half_size);
                                
                                // Try second half
                                close_order_half.id = format!("emergency_close_{}", uuid::Uuid::new_v4());
                                match placer.place_with_retry(close_order_half, half_size, 5, &mut metrics).await {
                                    Ok(_) => {
                                        let elapsed = start_time.elapsed();
                                        eprintln!("[EMERGENCY] ✅ Position fully closed in 2 orders ({}ms total)", elapsed.as_millis());
                                        return Ok(());
                                    }
                                    Err(e2) => {
                                        eprintln!("[EMERGENCY] ❌ Second half failed: {}", e2);
                                        eprintln!("[EMERGENCY] ⚠️  PARTIAL CLOSE: {} of {} contracts closed", half_size, close_order.size);
                                    }
                                }
                            }
                            Err(e1) => {
                                eprintln!("[EMERGENCY] ❌ First half failed: {}", e1);
                            }
                        }
                    }
                    
                    let elapsed = start_time.elapsed();

                    // Log critical alert with full position details
                    eprintln!("╔═══════════════════════════════════════════════════════════════╗");
                    eprintln!("║                    🚨 CRITICAL ALERT 🚨                       ║");
                    eprintln!("║           EMERGENCY CLOSE POSITION FAILED                     ║");
                    eprintln!("╠═══════════════════════════════════════════════════════════════╣");
                    eprintln!("║ Exchange:        {}                                    ", filled_order.exchange);
                    eprintln!("║ Symbol:          {}                                    ", filled_order.symbol);
                    eprintln!("║ Side:            {:?}                                  ", filled_order.side);
                    eprintln!("║ Size:            {} contracts                          ", filled_order.size);
                    eprintln!("║ Fill Price:      {}                                    ", filled_order.fill_price.unwrap_or(filled_order.price));
                    eprintln!("║ Order ID:        {}                                    ", filled_order.id);
                    eprintln!("║ Filled At:       {}                                    ", filled_order.filled_at.unwrap_or(0));
                    eprintln!("║ Close Side:      {:?}                                  ", close_side);
                    eprintln!("║ Elapsed Time:    {}ms                                  ", elapsed.as_millis());
                    eprintln!("║ Error:           {}                                    ", e);
                    eprintln!("╠═══════════════════════════════════════════════════════════════╣");
                    eprintln!("║ ACTION REQUIRED: Manual intervention needed to close position ║");
                    eprintln!("║ Naked exposure exists - immediate action required!            ║");
                    eprintln!("╚═══════════════════════════════════════════════════════════════╝");

                    // Halt all trading operations due to critical error
                    use crate::strategy::atomic_execution::halt_trading;
                    let halt_reason = format!("Emergency close failed: {} on {} ({})", filled_order.symbol, filled_order.exchange, e);
                    halt_trading(&halt_reason);

                    Err(format!("CRITICAL: Failed to emergency close position after {} attempts in {}ms: {}", 10, elapsed.as_millis(), e))
                }
            }
        }


    pub fn calculate_position_size(
        _spread_bps: f64,
        available_capital: f64,
        _fees_bps: f64,
        _funding_cost_bps: f64,
        starting_capital: f64,
    ) -> f64 {
        // Simple position sizing: 10% of starting capital
        // Cap at available capital if less than 10%
        let target_position = starting_capital * 0.10;
        let position_size = target_position.min(available_capital);
        
        // Minimum position size: $10
        if position_size < 10.0 {
            return 0.0;
        }
        
        position_size
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
    /// 
    /// NOTE: This is the PAPER TRADING version that simulates fills
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
            long_exit_order: None,  // Paper trading doesn't use exit orders
            short_exit_order: None, // Paper trading doesn't use exit orders
            stop_loss_triggered: false,
            stop_loss_long_price: 0.0,  // Not used in paper trading
            stop_loss_short_price: 0.0, // Not used in paper trading
            leg_out_event: None,
        };

        Ok(paper_trade)
    }

    /// Execute atomic dual-leg entry with REAL order placement via ExecutionBackend
    /// 
    /// PROFESSIONAL STRATEGY: Post-Only + Market Hedge
    /// 1. Place limit orders on BOTH exchanges simultaneously
    /// 2. Poll BOTH order statuses in parallel
    /// 3. As soon as ONE fills:
    ///    - If other also filled → Success!
    ///    - If other pending → Cancel it and MARKET order immediately
    /// 4. This guarantees hedge with minimal directional risk
    pub async fn execute_atomic_entry_real(
        opportunity: &ArbitrageOpportunity,
        available_capital: f64,
        position_size: f64,
        backend: Arc<dyn ExecutionBackend>,
    ) -> Result<PaperTrade, String> {
        // Check if trading is halted due to a critical error
        use crate::strategy::atomic_execution::is_trading_halted;
        if is_trading_halted() {
            return Err("Trading is halted due to a critical error. Manual intervention required.".to_string());
        }

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

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Calculate contract quantity (same on both legs for delta neutrality)
        let max_price = opportunity.long_price.max(opportunity.short_price);
        let raw_contract_quantity = position_size / max_price;

        eprintln!("[ATOMIC] Professional Strategy: Post-Only + Market Hedge");
        eprintln!("[POSITION SIZE] USD: ${:.2} | Raw contract qty: {:.4} @ max price ${:.2}", 
            position_size, raw_contract_quantity, max_price);

        // CRITICAL FIX: Pre-round quantity to BOTH exchanges' precision
        // Query both exchanges for their rounding rules
        let long_step = backend.get_quantity_step(&opportunity.long_exchange, &opportunity.symbol).await
            .unwrap_or(0.1);  // Default to 0.1 if query fails
        let short_step = backend.get_quantity_step(&opportunity.short_exchange, &opportunity.symbol).await
            .unwrap_or(0.1);  // Default to 0.1 if query fails
        
        // Round to both exchange specifications
        let long_rounded = (raw_contract_quantity / long_step).floor() * long_step;
        let short_rounded = (raw_contract_quantity / short_step).floor() * short_step;
        
        // Use the SMALLER of the two to ensure both exchanges can fill it
        let contract_quantity = long_rounded.min(short_rounded);
        
        eprintln!("[ATOMIC] Quantity rounding: long_step={} long_rounded={:.4} | short_step={} short_rounded={:.4} | using={:.4}", 
            long_step, long_rounded, short_step, short_rounded, contract_quantity);
        
        if contract_quantity <= 0.0 {
            return Err("Rounded quantity is zero or negative - position size too small for exchange minimums".to_string());
        }

        // Create limit orders for both legs
        let long_order_template = SimulatedOrder {
            id: String::new(),
            exchange: opportunity.long_exchange.clone(),
            symbol: opportunity.symbol.clone(),
            side: OrderSide::Long,
            order_type: OrderType::Limit,
            price: opportunity.long_price,
            size: contract_quantity,
            queue_position: None,
            created_at: now,
            filled_at: None,
            fill_price: None,
            status: OrderStatus::Pending,
        };

        let short_order_template = SimulatedOrder {
            id: String::new(),
            exchange: opportunity.short_exchange.clone(),
            symbol: opportunity.symbol.clone(),
            side: OrderSide::Short,
            order_type: OrderType::Limit,
            price: opportunity.short_price,
            size: contract_quantity,
            queue_position: None,
            created_at: now,
            filled_at: None,
            fill_price: None,
            status: OrderStatus::Pending,
        };

        // STEP 0: Pre-check balances on BOTH exchanges (Scenario 7 fix)
        eprintln!("[BALANCE CHECK] Verifying sufficient balance on both exchanges");
        let long_balance = backend.get_available_balance(&opportunity.long_exchange).await
            .map_err(|e| format!("Failed to get {} balance: {}", opportunity.long_exchange, e))?;
        let short_balance = backend.get_available_balance(&opportunity.short_exchange).await
            .map_err(|e| format!("Failed to get {} balance: {}", opportunity.short_exchange, e))?;
        
        // Reserve 20% extra margin for slippage on market orders
        let required_margin = position_size * 1.2;
        
        if long_balance < required_margin {
            return Err(format!(
                "Insufficient balance on {}: ${:.2} available, ${:.2} required (including 20% buffer)",
                opportunity.long_exchange, long_balance, required_margin
            ));
        }
        
        if short_balance < required_margin {
            return Err(format!(
                "Insufficient balance on {}: ${:.2} available, ${:.2} required (including 20% buffer)",
                opportunity.short_exchange, short_balance, required_margin
            ));
        }
        
        eprintln!("[BALANCE CHECK] ✅ Sufficient balance: {} ${:.2} | {} ${:.2}", 
            opportunity.long_exchange, long_balance, opportunity.short_exchange, short_balance);

        // STEP 0.5: Create RaceConditionGuard for hedge lock management
        let race_guard = RaceConditionGuard::default();

        // STEP 0.6: Initialize DepthChecker and PriceChaser modules
        // Select execution mode based on opportunity confidence score
        let config = RepricingConfig::from_confidence(opportunity.confidence_score as f64);
        let depth_checker = DepthChecker::new(backend.clone());
        let price_chaser = PriceChaser::new(backend.clone(), config.clone());
        
        eprintln!("[EXECUTION] Mode: {:?}, Confidence: {:.1}%", 
            config.execution_mode, opportunity.confidence_score);

        // STEP 1: Set leverage to 1x on BOTH exchanges
        eprintln!("[LEVERAGE] Setting 1x leverage on both exchanges");
        let _ = backend.set_leverage(&opportunity.long_exchange, &opportunity.symbol, 1).await;
        let _ = backend.set_leverage(&opportunity.short_exchange, &opportunity.symbol, 1).await;

        // STEP 1.5: Set margin type to ISOLATED on BOTH exchanges
        eprintln!("[MARGIN TYPE] Setting ISOLATED margin on both exchanges");
        let _ = backend.set_margin_type_isolated(&opportunity.long_exchange, &opportunity.symbol).await;
        let _ = backend.set_margin_type_isolated(&opportunity.short_exchange, &opportunity.symbol).await;

        // STEP 1.6: Pre-flight depth checks (mode-dependent)
        match config.execution_mode {
            ExecutionMode::UltraFast => {
                // Skip pre-flight depth checks for speed
                eprintln!("[EXECUTION] Ultra-fast mode: skipping pre-flight depth checks");
            }
            ExecutionMode::Balanced => {
                // Parallel depth checks + order placement
                eprintln!("[EXECUTION] Balanced mode: parallel depth checks");
                
                let (long_depth_result, short_depth_result) = tokio::join!(
                    depth_checker.check_depth_for_hedge(
                        &opportunity.long_exchange,
                        &opportunity.symbol,
                        contract_quantity,
                    ),
                    depth_checker.check_depth_for_hedge(
                        &opportunity.short_exchange,
                        &opportunity.symbol,
                        contract_quantity,
                    )
                );
                
                let long_depth = long_depth_result
                    .map_err(|e| format!("Long depth check failed: {}", e))?;
                let short_depth = short_depth_result
                    .map_err(|e| format!("Short depth check failed: {}", e))?;
                
                if long_depth.should_abort() || short_depth.should_abort() {
                    return Err(format!(
                        "Insufficient depth: long={:.2}x, short={:.2}x",
                        long_depth.depth_ratio, short_depth.depth_ratio
                    ));
                }
                
                if long_depth.should_warn() || short_depth.should_warn() {
                    eprintln!("[DEPTH CHECK] ⚠️  Low liquidity detected");
                }
            }
            ExecutionMode::Safe => {
                // Sequential depth checks (most thorough)
                eprintln!("[EXECUTION] Safe mode: sequential depth checks");
                
                let long_depth = depth_checker.check_depth_for_hedge(
                    &opportunity.long_exchange,
                    &opportunity.symbol,
                    contract_quantity,
                ).await
                    .map_err(|e| format!("Long depth check failed: {}", e))?;
                
                let short_depth = depth_checker.check_depth_for_hedge(
                    &opportunity.short_exchange,
                    &opportunity.symbol,
                    contract_quantity,
                ).await
                    .map_err(|e| format!("Short depth check failed: {}", e))?;
                
                if long_depth.should_abort() || short_depth.should_abort() {
                    return Err(format!(
                        "Insufficient depth: long={:.2}x, short={:.2}x",
                        long_depth.depth_ratio, short_depth.depth_ratio
                    ));
                }
                
                if long_depth.should_warn() {
                    eprintln!("[DEPTH CHECK] ⚠️  Low liquidity on {}: {:.2}x required", 
                        opportunity.long_exchange, long_depth.depth_ratio);
                }
                
                if short_depth.should_warn() {
                    eprintln!("[DEPTH CHECK] ⚠️  Low liquidity on {}: {:.2}x required", 
                        opportunity.short_exchange, short_depth.depth_ratio);
                }
            }
        }

        // STEP 2: Place limit orders on BOTH exchanges SIMULTANEOUSLY
        eprintln!("[ATOMIC] Placing limit orders on BOTH exchanges simultaneously");
        
        let long_result = backend.place_order(long_order_template.clone()).await;
        let short_result = backend.place_order(short_order_template.clone()).await;

        let mut long_order = match long_result {
            Ok(order) => {
                eprintln!("[ATOMIC] Long limit order placed: {} on {}", order.id, opportunity.long_exchange);
                order
            }
            Err(e) => {
                return Err(format!("Failed to place long limit order: {}", e));
            }
        };

        let mut short_order = match short_result {
            Ok(order) => {
                eprintln!("[ATOMIC] Short limit order placed: {} on {}", order.id, opportunity.short_exchange);
                order
            }
            Err(e) => {
                // Cancel long order if short fails
                eprintln!("[ATOMIC] Short order failed, cancelling long order");
                let _ = backend.cancel_order(&long_order.exchange, &long_order.id).await;
                return Err(format!("Failed to place short limit order: {}", e));
            }
        };

        // STEP 3: Poll BOTH orders in parallel for up to 3 seconds
        eprintln!("[ATOMIC] Polling both orders for fills (3 second timeout)");
        let timeout = Duration::from_millis(config.total_timeout_seconds * 1000);
        let start = std::time::Instant::now();
        
        let mut long_filled = false;
        let mut short_filled = false;
        let mut long_reprice_count = 0;
        let mut short_reprice_count = 0;
        let mut long_metrics = RepricingMetrics::new(long_order.price);
        let mut short_metrics = RepricingMetrics::new(short_order.price);
        let initial_spread_bps = ((opportunity.short_price - opportunity.long_price) / opportunity.long_price) * 10000.0;

        while start.elapsed() < timeout && (!long_filled || !short_filled) {
            tokio::time::sleep(Duration::from_millis(config.reprice_interval_ms)).await;
            
            // Check long order status (if not already filled)
            if !long_filled {
                let api_start = Instant::now();
                match backend.get_order_status_detailed(&long_order.exchange, &long_order.id, &opportunity.symbol).await {
                    Ok(status_info) => {
                        let api_duration = api_start.elapsed().as_millis();
                        
                        if status_info.is_fully_filled() {
                            long_filled = true;
                            long_order.status = OrderStatus::Filled;
                            long_order.filled_at = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                            long_order.fill_price = Some(long_order.price);
                            long_order.size = status_info.filled_quantity;  // Update to actual filled quantity
                            eprintln!("[ATOMIC] ✅ LONG order FILLED on {} after {}ms | Filled qty: {}", 
                                opportunity.long_exchange, start.elapsed().as_millis(), status_info.filled_quantity);
                            
                            // Initialize timing metrics at fill detection point
                            let mut metrics = HedgeTimingMetrics::new();
                            let api_duration = api_start.elapsed();
                            metrics.record_api_response(
                                format!("get_order_status_detailed({})", opportunity.long_exchange),
                                api_duration
                            );
                            let logger = HedgeLogger::default_level();
                            logger.log_fill_detected(
                                &opportunity.long_exchange,
                                &long_order.id,
                                status_info.filled_quantity,
                                start.elapsed().as_millis()
                            );
                            logger.log_api_response_time(&opportunity.long_exchange, "get_order_status_detailed", api_duration.as_millis());
                            
                            // CRITICAL FIX: Check if short also filled before hedging
                            // This prevents duplicate positions when both fill simultaneously
                            if !short_filled {
                                // Acquire hedge lock for this symbol to prevent concurrent hedges
                                let _hedge_lock = match race_guard.try_acquire_hedge_lock(&opportunity.symbol) {
                                    Ok(lock) => {
                                        eprintln!("[RACE GUARD] ✅ Acquired hedge lock for {}", opportunity.symbol);
                                        lock
                                    }
                                    Err(e) => {
                                        eprintln!("[RACE GUARD] ❌ Failed to acquire hedge lock: {}", e);
                                        return Err(format!("Failed to acquire hedge lock: {}", e));
                                    }
                                };
                                
                                // Use check_both_legs_status to detect race conditions
                                metrics.record_other_leg_check();
                                let api_start2 = Instant::now();
                                match race_guard.check_both_legs_status(&backend, &long_order, &short_order).await {
                                    Ok(BothLegsStatus::BothFilled { long_qty, short_qty }) => {
                                        let api_duration2 = api_start2.elapsed();
                                        metrics.record_api_response(
                                            "check_both_legs_status".to_string(),
                                            api_duration2
                                        );
                                        logger.log_api_response_time("BOTH", "check_both_legs_status", api_duration2.as_millis());
                                        logger.log_race_condition_detected(&opportunity.symbol, long_qty, short_qty);
                                        
                                        eprintln!("[ATOMIC] ✅ Both legs filled simultaneously - race condition detected!");
                                        
                                        // CRITICAL: Check for quantity mismatch due to different exchange rounding
                                        if (long_qty - short_qty).abs() > 0.001 {
                                            eprintln!("[ATOMIC] ⚠️  QUANTITY MISMATCH: long={} short={} diff={}", 
                                                long_qty, short_qty, (long_qty - short_qty).abs());
                                            
                                            // Determine which side has more and needs to be balanced
                                            if long_qty > short_qty {
                                                let diff = long_qty - short_qty;
                                                eprintln!("[ATOMIC] Balancing: selling {} contracts on {} to match short leg", 
                                                    diff, opportunity.long_exchange);
                                                
                                                // Place small market order to close excess long position
                                                let balance_order = SimulatedOrder {
                                                    id: String::new(),
                                                    exchange: opportunity.long_exchange.clone(),
                                                    symbol: opportunity.symbol.clone(),
                                                    side: OrderSide::Short,  // Sell to close long
                                                    order_type: OrderType::Market,
                                                    price: opportunity.long_price,
                                                    size: diff,
                                                    queue_position: None,
                                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                                    filled_at: None,
                                                    fill_price: None,
                                                    status: OrderStatus::Pending,
                                                };
                                                
                                                match backend.place_market_order(balance_order).await {
                                                    Ok(_) => {
                                                        eprintln!("[ATOMIC] ✅ Balanced long leg: sold {} contracts", diff);
                                                        long_order.size = short_qty;  // Update to match
                                                    }
                                                    Err(e) => {
                                                        eprintln!("[ATOMIC] ❌ Failed to balance long leg: {}", e);
                                                        // Continue anyway, mismatch will be visible in positions
                                                    }
                                                }
                                            } else {
                                                let diff = short_qty - long_qty;
                                                eprintln!("[ATOMIC] Balancing: buying {} contracts on {} to match long leg", 
                                                    diff, opportunity.short_exchange);
                                                
                                                // Place small market order to close excess short position
                                                let balance_order = SimulatedOrder {
                                                    id: String::new(),
                                                    exchange: opportunity.short_exchange.clone(),
                                                    symbol: opportunity.symbol.clone(),
                                                    side: OrderSide::Long,  // Buy to close short
                                                    order_type: OrderType::Market,
                                                    price: opportunity.short_price,
                                                    size: diff,
                                                    queue_position: None,
                                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                                    filled_at: None,
                                                    fill_price: None,
                                                    status: OrderStatus::Pending,
                                                };
                                                
                                                match backend.place_market_order(balance_order).await {
                                                    Ok(_) => {
                                                        eprintln!("[ATOMIC] ✅ Balanced short leg: bought {} contracts", diff);
                                                        short_order.size = long_qty;  // Update to match
                                                    }
                                                    Err(e) => {
                                                        eprintln!("[ATOMIC] ❌ Failed to balance short leg: {}", e);
                                                        // Continue anyway, mismatch will be visible in positions
                                                    }
                                                }
                                            }
                                        }
                                        
                                        short_filled = true;
                                        short_order.status = OrderStatus::Filled;
                                        short_order.filled_at = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                                        short_order.fill_price = Some(short_order.price);
                                        short_order.size = short_qty;
                                        
                                        // Log timing summary for both legs filled case
                                        metrics.finalize();
                                        logger.log_timing_summary(&metrics, "BOTH", &opportunity.symbol);
                                        continue; // Skip hedging, both filled
                                    }
                                    Ok(BothLegsStatus::OnlyLongFilled { .. }) => {
                                        let api_duration2 = api_start2.elapsed();
                                        metrics.record_api_response(
                                            "check_both_legs_status".to_string(),
                                            api_duration2
                                        );
                                        logger.log_api_response_time("BOTH", "check_both_legs_status", api_duration2.as_millis());
                                        // Only long filled, proceed with hedge - fall through to cancellation
                                    }
                                    Ok(_) => {
                                        // Unexpected status, but proceed with hedge
                                        let api_duration2 = api_start2.elapsed();
                                        metrics.record_api_response(
                                            "check_both_legs_status".to_string(),
                                            api_duration2
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("[ATOMIC] ⚠️  Failed to check both legs status: {}", e);
                                        // Proceed with hedge despite error
                                    }
                                }
                                
                                // CRITICAL: Use the ACTUAL filled quantity from the long order to ensure equal position sizes
                                let hedge_quantity = status_info.filled_quantity;
                                
                                // PRE-FLIGHT DEPTH CHECK before market hedge (always enabled for hedge)
                                // Even in ultra_fast mode, we check depth before market hedge
                                eprintln!("[DEPTH CHECK] Checking depth before short hedge");
                                let mut hedge_metrics = metrics.clone();
                                hedge_metrics.record_depth_check_initiated();
                                
                                match depth_checker.check_depth_for_hedge(
                                    &short_order.exchange,
                                    &opportunity.symbol,
                                    hedge_quantity,
                                ).await {
                                    Ok(hedge_depth) => {
                                        hedge_metrics.record_depth_check_completed();
                                        
                                        if hedge_depth.should_abort() {
                                            eprintln!("[DEPTH CHECK] ❌ Insufficient depth for short hedge: {:.2}x", 
                                                hedge_depth.depth_ratio);
                                            
                                            // Emergency close long position
                                            hedge_metrics.finalize();
                                            logger.log_timing_summary(&hedge_metrics, &opportunity.long_exchange, &opportunity.symbol);
                                            
                                            Self::emergency_close_position(&backend, &long_order).await?;
                                            return Err(format!(
                                                "Insufficient depth for short hedge ({:.2}x), long position closed",
                                                hedge_depth.depth_ratio
                                            ));
                                        }
                                        
                                        if hedge_depth.should_warn() {
                                            eprintln!("[DEPTH CHECK] ⚠️  Low liquidity for short hedge: {:.2}x", 
                                                hedge_depth.depth_ratio);
                                        } else {
                                            eprintln!("[DEPTH CHECK] ✅ Sufficient depth for short hedge: {:.2}x", 
                                                hedge_depth.depth_ratio);
                                        }
                                        
                                        metrics = hedge_metrics;
                                    }
                                    Err(e) => {
                                        eprintln!("[DEPTH CHECK] ⚠️  Depth check failed: {} (proceeding with hedge)", e);
                                        hedge_metrics.record_depth_check_completed();
                                        metrics = hedge_metrics;
                                    }
                                }
                                
                                // CRITICAL PATH OPTIMIZATION (Task 9.2): Pre-create market order template BEFORE cancellation
                                // This minimizes delay between cancellation and market order placement (target < 50ms)
                                let short_market_template = SimulatedOrder {
                                    id: String::new(),
                                    exchange: opportunity.short_exchange.clone(),
                                    symbol: opportunity.symbol.clone(),
                                    side: OrderSide::Short,
                                    order_type: OrderType::Market,
                                    price: opportunity.short_price, // Estimate
                                    size: hedge_quantity,  // Use actual filled quantity from long order
                                    queue_position: None,
                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                    filled_at: None,
                                    fill_price: None,
                                    status: OrderStatus::Pending,
                                };
                                
                                // CRITICAL PATH OPTIMIZATION: Initiate cancellation immediately after both-legs check
                                // Record timestamp and cancel in one atomic block to minimize delay
                                metrics.record_cancel_initiated();
                                let api_start_cancel = Instant::now();
                                let cancel_result = backend.cancel_order(&short_order.exchange, &short_order.id).await;
                                let cancel_api_duration = api_start_cancel.elapsed();
                                metrics.record_cancel_completed();
                                
                                // CRITICAL PATH OPTIMIZATION (Task 9.2): Place market order IMMEDIATELY after cancellation
                                // Target < 50ms from cancellation to market order placement
                                metrics.record_market_order_initiated();
                                
                                // Check cancel result for logging after market order placement
                                let cancel_error = cancel_result.as_ref().err().map(|e| e.to_string());
                                
                                // Place market order immediately - no delays between cancellation and placement
                                let market_order_result = Self::place_market_order_with_retry(
                                    &backend, 
                                    short_market_template, 
                                    hedge_quantity, 
                                    Some(&mut metrics), 
                                    Some(&logger)
                                ).await;
                                
                                // Log cancellation AFTER market order is placed (non-critical path)
                                metrics.record_api_response(
                                    format!("cancel_order({})", opportunity.short_exchange),
                                    cancel_api_duration
                                );
                                logger.log_cancel_initiated(&short_order.exchange, &short_order.id);
                                logger.log_api_response_time(&opportunity.short_exchange, "cancel_order", cancel_api_duration.as_millis());
                                let cancel_elapsed = metrics.cancel_duration().map(|d| d.as_millis()).unwrap_or(0);
                                logger.log_cancel_result(
                                    &short_order.exchange,
                                    &CancellationResult::Cancelled,
                                    cancel_elapsed
                                );
                                
                                if let Some(e) = cancel_error {
                                    eprintln!("[ATOMIC] ⚠️  Cancel warning: {} (proceeding with hedge)", e);
                                }
                                
                                eprintln!("[ATOMIC] Using long order filled quantity for hedge: {}", hedge_quantity);
                                logger.log_market_order_placed(
                                    &opportunity.short_exchange,
                                    "pending",
                                    hedge_quantity
                                );
                                
                                match market_order_result {
                                    Ok(filled_order) => {
                                        // Record timestamp after market order accepted
                                        metrics.record_market_order_accepted();
                                        
                                        short_order = filled_order;
                                        short_filled = true;
                                        
                                        // Record timestamp when market order fills
                                        metrics.record_market_order_filled();
                                        
                                        let fill_elapsed = metrics.market_order_fill_duration()
                                            .map(|d| d.as_millis()).unwrap_or(0);
                                        logger.log_market_order_filled(
                                            &opportunity.short_exchange,
                                            &short_order.id,
                                            fill_elapsed
                                        );
                                        
                                        eprintln!("[ATOMIC] ✅ SHORT MARKET order filled on {}", opportunity.short_exchange);
                                        
                                        // Log timing summary at completion
                                        metrics.finalize();
                                        logger.log_timing_summary(&metrics, &opportunity.short_exchange, &opportunity.symbol);
                                    }
                                    Err(e) => {
                                        eprintln!("[ATOMIC] ❌ CRITICAL: Short market order failed: {}", e);
                                        eprintln!("[ATOMIC] 🚨 Attempting emergency close of long position");
                                        
                                        // Log timing summary even on failure
                                        metrics.finalize();
                                        logger.log_timing_summary(&metrics, &opportunity.short_exchange, &opportunity.symbol);
                                        
                                        // Emergency close the long position
                                        if let Err(close_err) = Self::emergency_close_position(&backend, &long_order).await {
                                            eprintln!("[ATOMIC] ❌ CRITICAL: Emergency close failed: {}", close_err);
                                            return Err(format!("CRITICAL: Long filled, short hedge failed, emergency close failed: {} | {}", e, close_err));
                                        }
                                        
                                        return Err(format!("Long filled but short hedge failed (position closed): {}", e));
                                    }
                                }
                            }
                        } else if status_info.is_partially_filled() {
                            eprintln!("[ATOMIC] ⚠️  LONG order PARTIALLY filled: {:.1}% ({}/{})", 
                                status_info.fill_percentage(), status_info.filled_quantity, status_info.total_quantity);
                        } else if status_info.status == OrderStatus::Cancelled {
                            eprintln!("[ATOMIC] Long order was cancelled");
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("[ATOMIC] ⚠️  Failed to get long order status: {}", e);
                    }
                }
            }
            
            // ACTIVE PRICE CHASING: Check if long order needs repricing
            if !long_filled && long_reprice_count < config.max_reprices {
                match price_chaser.get_best_price_for_order(&long_order).await {
                    Ok(best_price) => {
                        if price_chaser.should_reprice(long_order.price, best_price) {
                            eprintln!("[PRICE CHASER] Repricing long order: {:.4} -> {:.4}", 
                                long_order.price, best_price);
                            
                            match price_chaser.reprice_order(&long_order, best_price, &mut long_metrics).await {
                                Ok(new_order) => {
                                    long_order = new_order;
                                    long_reprice_count += 1;
                                    eprintln!("[PRICE CHASER] Long order repriced ({}/{})", 
                                        long_reprice_count, config.max_reprices);
                                }
                                Err(e) => {
                                    eprintln!("[PRICE CHASER] ⚠️  Failed to reprice long order: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[PRICE CHASER] ⚠️  Failed to get best price for long: {}", e);
                    }
                }
            }
            
            // Check short order status (if not already filled)
            if !short_filled {
                match backend.get_order_status_detailed(&short_order.exchange, &short_order.id, &opportunity.symbol).await {
                    Ok(status_info) => {
                        if status_info.is_fully_filled() {
                            short_filled = true;
                            short_order.status = OrderStatus::Filled;
                            short_order.filled_at = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                            short_order.fill_price = Some(short_order.price);
                            short_order.size = status_info.filled_quantity;  // Update to actual filled quantity
                            eprintln!("[ATOMIC] ✅ SHORT order FILLED on {} after {}ms | Filled qty: {}", 
                                opportunity.short_exchange, start.elapsed().as_millis(), status_info.filled_quantity);
                            
                            // Initialize timing metrics at fill detection point
                            let mut metrics = HedgeTimingMetrics::new();
                            let logger = HedgeLogger::default_level();
                            logger.log_fill_detected(
                                &opportunity.short_exchange,
                                &short_order.id,
                                status_info.filled_quantity,
                                start.elapsed().as_millis()
                            );
                            
                            // If short filled but long hasn't, immediately market order long
                            if !long_filled {
                                // Acquire hedge lock for this symbol to prevent concurrent hedges
                                let _hedge_lock = match race_guard.try_acquire_hedge_lock(&opportunity.symbol) {
                                    Ok(lock) => {
                                        eprintln!("[RACE GUARD] ✅ Acquired hedge lock for {}", opportunity.symbol);
                                        lock
                                    }
                                    Err(e) => {
                                        eprintln!("[RACE GUARD] ❌ Failed to acquire hedge lock: {}", e);
                                        return Err(format!("Failed to acquire hedge lock: {}", e));
                                    }
                                };
                                
                                // Use check_both_legs_status to detect race conditions
                                metrics.record_other_leg_check();
                                let api_start2 = Instant::now();
                                match race_guard.check_both_legs_status(&backend, &long_order, &short_order).await {
                                    Ok(BothLegsStatus::BothFilled { long_qty, short_qty }) => {
                                        let api_duration2 = api_start2.elapsed();
                                        metrics.record_api_response(
                                            "check_both_legs_status".to_string(),
                                            api_duration2
                                        );
                                        logger.log_api_response_time("BOTH", "check_both_legs_status", api_duration2.as_millis());
                                        logger.log_race_condition_detected(&opportunity.symbol, long_qty, short_qty);
                                        
                                        eprintln!("[ATOMIC] ✅ Both legs filled simultaneously - race condition detected!");
                                        
                                        // CRITICAL: Check for quantity mismatch due to different exchange rounding
                                        if (long_qty - short_qty).abs() > 0.001 {
                                            eprintln!("[ATOMIC] ⚠️  QUANTITY MISMATCH: long={} short={} diff={}", 
                                                long_qty, short_qty, (long_qty - short_qty).abs());
                                            
                                            // Determine which side has more and needs to be balanced
                                            if long_qty > short_qty {
                                                let diff = long_qty - short_qty;
                                                eprintln!("[ATOMIC] Balancing: selling {} contracts on {} to match short leg", 
                                                    diff, opportunity.long_exchange);
                                                
                                                // Place small market order to close excess long position
                                                let balance_order = SimulatedOrder {
                                                    id: String::new(),
                                                    exchange: opportunity.long_exchange.clone(),
                                                    symbol: opportunity.symbol.clone(),
                                                    side: OrderSide::Short,  // Sell to close long
                                                    order_type: OrderType::Market,
                                                    price: opportunity.long_price,
                                                    size: diff,
                                                    queue_position: None,
                                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                                    filled_at: None,
                                                    fill_price: None,
                                                    status: OrderStatus::Pending,
                                                };
                                                
                                                match backend.place_market_order(balance_order).await {
                                                    Ok(_) => {
                                                        eprintln!("[ATOMIC] ✅ Balanced long leg: sold {} contracts", diff);
                                                        long_order.size = short_qty;  // Update to match
                                                    }
                                                    Err(e) => {
                                                        eprintln!("[ATOMIC] ❌ Failed to balance long leg: {}", e);
                                                        // Continue anyway, mismatch will be visible in positions
                                                    }
                                                }
                                            } else {
                                                let diff = short_qty - long_qty;
                                                eprintln!("[ATOMIC] Balancing: buying {} contracts on {} to match long leg", 
                                                    diff, opportunity.short_exchange);
                                                
                                                // Place small market order to close excess short position
                                                let balance_order = SimulatedOrder {
                                                    id: String::new(),
                                                    exchange: opportunity.short_exchange.clone(),
                                                    symbol: opportunity.symbol.clone(),
                                                    side: OrderSide::Long,  // Buy to close short
                                                    order_type: OrderType::Market,
                                                    price: opportunity.short_price,
                                                    size: diff,
                                                    queue_position: None,
                                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                                    filled_at: None,
                                                    fill_price: None,
                                                    status: OrderStatus::Pending,
                                                };
                                                
                                                match backend.place_market_order(balance_order).await {
                                                    Ok(_) => {
                                                        eprintln!("[ATOMIC] ✅ Balanced short leg: bought {} contracts", diff);
                                                        short_order.size = long_qty;  // Update to match
                                                    }
                                                    Err(e) => {
                                                        eprintln!("[ATOMIC] ❌ Failed to balance short leg: {}", e);
                                                        // Continue anyway, mismatch will be visible in positions
                                                    }
                                                }
                                            }
                                        }
                                        
                                        long_filled = true;
                                        long_order.status = OrderStatus::Filled;
                                        long_order.filled_at = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                                        long_order.fill_price = Some(long_order.price);
                                        long_order.size = long_qty;
                                        
                                        // Log timing summary for both legs filled case
                                        metrics.finalize();
                                        logger.log_timing_summary(&metrics, "BOTH", &opportunity.symbol);
                                        continue; // Skip hedging, both filled
                                    }
                                    Ok(BothLegsStatus::OnlyShortFilled { .. }) => {
                                        let api_duration2 = api_start2.elapsed();
                                        metrics.record_api_response(
                                            "check_both_legs_status".to_string(),
                                            api_duration2
                                        );
                                        logger.log_api_response_time("BOTH", "check_both_legs_status", api_duration2.as_millis());
                                        // Only short filled, proceed with hedge - fall through to cancellation
                                    }
                                    Ok(_) => {
                                        // Unexpected status, but proceed with hedge
                                        let api_duration2 = api_start2.elapsed();
                                        metrics.record_api_response(
                                            "check_both_legs_status".to_string(),
                                            api_duration2
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("[ATOMIC] ⚠️  Failed to check both legs status: {}", e);
                                        // Proceed with hedge despite error
                                    }
                                }
                                
                                // CRITICAL: Use the ACTUAL filled quantity from the short order to ensure equal position sizes
                                let hedge_quantity = status_info.filled_quantity;
                                
                                // PRE-FLIGHT DEPTH CHECK before market hedge (always enabled for hedge)
                                // Even in ultra_fast mode, we check depth before market hedge
                                eprintln!("[DEPTH CHECK] Checking depth before long hedge");
                                let mut hedge_metrics = metrics.clone();
                                hedge_metrics.record_depth_check_initiated();
                                
                                match depth_checker.check_depth_for_hedge(
                                    &long_order.exchange,
                                    &opportunity.symbol,
                                    hedge_quantity,
                                ).await {
                                    Ok(hedge_depth) => {
                                        hedge_metrics.record_depth_check_completed();
                                        
                                        if hedge_depth.should_abort() {
                                            eprintln!("[DEPTH CHECK] ❌ Insufficient depth for long hedge: {:.2}x", 
                                                hedge_depth.depth_ratio);
                                            
                                            // Emergency close short position
                                            hedge_metrics.finalize();
                                            logger.log_timing_summary(&hedge_metrics, &opportunity.short_exchange, &opportunity.symbol);
                                            
                                            Self::emergency_close_position(&backend, &short_order).await?;
                                            return Err(format!(
                                                "Insufficient depth for long hedge ({:.2}x), short position closed",
                                                hedge_depth.depth_ratio
                                            ));
                                        }
                                        
                                        if hedge_depth.should_warn() {
                                            eprintln!("[DEPTH CHECK] ⚠️  Low liquidity for long hedge: {:.2}x", 
                                                hedge_depth.depth_ratio);
                                        } else {
                                            eprintln!("[DEPTH CHECK] ✅ Sufficient depth for long hedge: {:.2}x", 
                                                hedge_depth.depth_ratio);
                                        }
                                        
                                        metrics = hedge_metrics;
                                    }
                                    Err(e) => {
                                        eprintln!("[DEPTH CHECK] ⚠️  Depth check failed: {} (proceeding with hedge)", e);
                                        hedge_metrics.record_depth_check_completed();
                                        metrics = hedge_metrics;
                                    }
                                }
                                
                                // CRITICAL PATH OPTIMIZATION (Task 9.2): Pre-create market order template BEFORE cancellation
                                // This minimizes delay between cancellation and market order placement (target < 50ms)
                                let long_market_template = SimulatedOrder {
                                    id: String::new(),
                                    exchange: opportunity.long_exchange.clone(),
                                    symbol: opportunity.symbol.clone(),
                                    side: OrderSide::Long,
                                    order_type: OrderType::Market,
                                    price: opportunity.long_price, // Estimate
                                    size: hedge_quantity,  // Use actual filled quantity from short order
                                    queue_position: None,
                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                    filled_at: None,
                                    fill_price: None,
                                    status: OrderStatus::Pending,
                                };
                                
                                // CRITICAL PATH OPTIMIZATION: Initiate cancellation immediately after both-legs check
                                // Record timestamp and cancel in one atomic block to minimize delay
                                metrics.record_cancel_initiated();
                                let api_start_cancel = Instant::now();
                                let cancel_result = backend.cancel_order(&long_order.exchange, &long_order.id).await;
                                let cancel_api_duration = api_start_cancel.elapsed();
                                metrics.record_cancel_completed();
                                
                                // CRITICAL PATH OPTIMIZATION (Task 9.2): Place market order IMMEDIATELY after cancellation
                                // Target < 50ms from cancellation to market order placement
                                metrics.record_market_order_initiated();
                                
                                // Check cancel result for logging after market order placement
                                let cancel_error = cancel_result.as_ref().err().map(|e| e.to_string());
                                
                                // Place market order immediately - no delays between cancellation and placement
                                let market_order_result = Self::place_market_order_with_retry(
                                    &backend, 
                                    long_market_template, 
                                    hedge_quantity, 
                                    Some(&mut metrics), 
                                    Some(&logger)
                                ).await;
                                
                                // Log cancellation AFTER market order is placed (non-critical path)
                                metrics.record_api_response(
                                    format!("cancel_order({})", opportunity.long_exchange),
                                    cancel_api_duration
                                );
                                logger.log_cancel_initiated(&long_order.exchange, &long_order.id);
                                logger.log_api_response_time(&opportunity.long_exchange, "cancel_order", cancel_api_duration.as_millis());
                                let cancel_elapsed = metrics.cancel_duration().map(|d| d.as_millis()).unwrap_or(0);
                                logger.log_cancel_result(
                                    &long_order.exchange,
                                    &CancellationResult::Cancelled,
                                    cancel_elapsed
                                );
                                
                                if let Some(e) = cancel_error {
                                    eprintln!("[ATOMIC] ⚠️  Cancel warning: {} (proceeding with hedge)", e);
                                }
                                
                                eprintln!("[ATOMIC] Using short order filled quantity for hedge: {}", hedge_quantity);
                                logger.log_market_order_placed(
                                    &opportunity.long_exchange,
                                    "pending",
                                    hedge_quantity
                                );
                                
                                match market_order_result {
                                    Ok(filled_order) => {
                                        // Record timestamp after market order accepted
                                        metrics.record_market_order_accepted();
                                        
                                        long_order = filled_order;
                                        long_filled = true;
                                        
                                        // Record timestamp when market order fills
                                        metrics.record_market_order_filled();
                                        
                                        let fill_elapsed = metrics.market_order_fill_duration()
                                            .map(|d| d.as_millis()).unwrap_or(0);
                                        logger.log_market_order_filled(
                                            &opportunity.long_exchange,
                                            &long_order.id,
                                            fill_elapsed
                                        );
                                        
                                        eprintln!("[ATOMIC] ✅ LONG MARKET order filled on {}", opportunity.long_exchange);
                                        
                                        // Log timing summary at completion
                                        metrics.finalize();
                                        logger.log_timing_summary(&metrics, &opportunity.long_exchange, &opportunity.symbol);
                                    }
                                    Err(e) => {
                                        eprintln!("[ATOMIC] ❌ CRITICAL: Long market order failed: {}", e);
                                        eprintln!("[ATOMIC] 🚨 Attempting emergency close of short position");
                                        
                                        // Log timing summary even on failure
                                        metrics.finalize();
                                        logger.log_timing_summary(&metrics, &opportunity.long_exchange, &opportunity.symbol);
                                        
                                        // Emergency close the short position
                                        if let Err(close_err) = Self::emergency_close_position(&backend, &short_order).await {
                                            eprintln!("[ATOMIC] ❌ CRITICAL: Emergency close failed: {}", close_err);
                                            return Err(format!("CRITICAL: Short filled, long hedge failed, emergency close failed: {} | {}", e, close_err));
                                        }
                                        
                                        return Err(format!("Short filled but long hedge failed (position closed): {}", e));
                                    }
                                }
                            }
                        } else if status_info.is_partially_filled() {
                            eprintln!("[ATOMIC] ⚠️  SHORT order PARTIALLY filled: {:.1}% ({}/{})", 
                                status_info.fill_percentage(), status_info.filled_quantity, status_info.total_quantity);
                        } else if status_info.status == OrderStatus::Cancelled {
                            eprintln!("[ATOMIC] Short order was cancelled");
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("[ATOMIC] ⚠️  Failed to get short order status: {}", e);
                    }
                }
            }
            
            // ACTIVE PRICE CHASING: Check if short order needs repricing
            if !short_filled && short_reprice_count < config.max_reprices {
                match price_chaser.get_best_price_for_order(&short_order).await {
                    Ok(best_price) => {
                        if price_chaser.should_reprice(short_order.price, best_price) {
                            eprintln!("[PRICE CHASER] Repricing short order: {:.4} -> {:.4}", 
                                short_order.price, best_price);
                            
                            match price_chaser.reprice_order(&short_order, best_price, &mut short_metrics).await {
                                Ok(new_order) => {
                                    short_order = new_order;
                                    short_reprice_count += 1;
                                    eprintln!("[PRICE CHASER] Short order repriced ({}/{})", 
                                        short_reprice_count, config.max_reprices);
                                }
                                Err(e) => {
                                    eprintln!("[PRICE CHASER] ⚠️  Failed to reprice short order: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[PRICE CHASER] ⚠️  Failed to get best price for short: {}", e);
                    }
                }
            }
            
            // SPREAD COLLAPSE DETECTION: Check if spread has moved too much
            if !long_filled && !short_filled {
                let current_spread_bps = ((short_order.price - long_order.price) / long_order.price) * 10000.0;
                let spread_change = (initial_spread_bps - current_spread_bps).abs();
                
                if spread_change > config.spread_collapse_threshold_bps {
                    eprintln!("[PRICE CHASER] ❌ Spread collapsed: {:.2} -> {:.2} bps (change: {:.2} bps)", 
                        initial_spread_bps, current_spread_bps, spread_change);
                    
                    // Cancel both orders
                    let _ = backend.cancel_order(&long_order.exchange, &long_order.id).await;
                    let _ = backend.cancel_order(&short_order.exchange, &short_order.id).await;
                    
                    return Err(format!(
                        "Spread collapsed during execution: {:.2} -> {:.2} bps",
                        initial_spread_bps, current_spread_bps
                    ));
                }
            }
            
            // Check if max reprices reached for either leg
            if long_reprice_count >= config.max_reprices && !long_filled {
                eprintln!("[PRICE CHASER] ❌ Max reprices reached for long order");
                let _ = backend.cancel_order(&long_order.exchange, &long_order.id).await;
                let _ = backend.cancel_order(&short_order.exchange, &short_order.id).await;
                return Err(format!("Max reprices ({}) reached for long order", config.max_reprices));
            }
            
            if short_reprice_count >= config.max_reprices && !short_filled {
                eprintln!("[PRICE CHASER] ❌ Max reprices reached for short order");
                let _ = backend.cancel_order(&long_order.exchange, &long_order.id).await;
                let _ = backend.cancel_order(&short_order.exchange, &short_order.id).await;
                return Err(format!("Max reprices ({}) reached for short order", config.max_reprices));
            }
            
            // If both filled, break early
            if long_filled && short_filled {
                break;
            }
        }

        // STEP 4: Check final status
        // Scenario 4 fix: Final verification check before cancelling
        // This catches cases where orders filled but API was slow to report
        if !long_filled && !short_filled {
            eprintln!("[ATOMIC] ⏱️  Timeout reached - performing final verification check");
            
            // One last check for both orders
            if let Ok(long_status) = backend.get_order_status_detailed(&long_order.exchange, &long_order.id, &opportunity.symbol).await {
                if long_status.is_fully_filled() {
                    eprintln!("[ATOMIC] ✅ LONG order WAS filled (detected in final check)");
                    long_filled = true;
                    long_order.status = OrderStatus::Filled;
                    long_order.size = long_status.filled_quantity;
                }
            }
            
            if let Ok(short_status) = backend.get_order_status_detailed(&short_order.exchange, &short_order.id, &opportunity.symbol).await {
                if short_status.is_fully_filled() {
                    eprintln!("[ATOMIC] ✅ SHORT order WAS filled (detected in final check)");
                    short_filled = true;
                    short_order.status = OrderStatus::Filled;
                    short_order.size = short_status.filled_quantity;
                }
            }
            
            // If still neither filled after final check, cancel both
            if !long_filled && !short_filled {
                eprintln!("[ATOMIC] ❌ Neither order filled within timeout, cancelling both");
                let _ = backend.cancel_order(&long_order.exchange, &long_order.id).await;
                let _ = backend.cancel_order(&short_order.exchange, &short_order.id).await;
                return Err("Neither order filled within 3 second timeout".to_string());
            }
        }

        // If one filled in final check but not the other, hedge it
        if long_filled && !short_filled {
            eprintln!("[ATOMIC] 🚨 Long filled (final check), short pending → MARKET HEDGING short leg");
            
            // Acquire hedge lock for this symbol to prevent concurrent hedges
            let _hedge_lock = match race_guard.try_acquire_hedge_lock(&opportunity.symbol) {
                Ok(lock) => {
                    eprintln!("[RACE GUARD] ✅ Acquired hedge lock for {}", opportunity.symbol);
                    lock
                }
                Err(e) => {
                    eprintln!("[RACE GUARD] ❌ Failed to acquire hedge lock: {}", e);
                    return Err(format!("Failed to acquire hedge lock: {}", e));
                }
            };
            
            // Initialize timing metrics for final check hedge
            let mut metrics = HedgeTimingMetrics::new();
            let logger = HedgeLogger::default_level();
            
            let hedge_quantity = long_order.size;
            
            // Track cumulative filled quantity
            let mut total_filled = 0.0;
            
            // PRE-FLIGHT DEPTH CHECK before market hedge (always enabled)
            eprintln!("[DEPTH CHECK] Checking depth before short hedge (final check)");
            metrics.record_depth_check_initiated();
            
            match depth_checker.check_depth_for_hedge(
                &short_order.exchange,
                &opportunity.symbol,
                hedge_quantity,
            ).await {
                Ok(hedge_depth) => {
                    metrics.record_depth_check_completed();
                    
                    if hedge_depth.should_abort() {
                        eprintln!("[DEPTH CHECK] ❌ Insufficient depth for short hedge: {:.2}x", 
                            hedge_depth.depth_ratio);
                        
                        // Emergency close long position
                        metrics.finalize();
                        logger.log_timing_summary(&metrics, &opportunity.long_exchange, &opportunity.symbol);
                        
                        Self::emergency_close_position(&backend, &long_order).await?;
                        return Err(format!(
                            "Insufficient depth for short hedge ({:.2}x), long position closed",
                            hedge_depth.depth_ratio
                        ));
                    }
                    
                    if hedge_depth.should_warn() {
                        eprintln!("[DEPTH CHECK] ⚠️  Low liquidity for short hedge: {:.2}x", 
                            hedge_depth.depth_ratio);
                    } else {
                        eprintln!("[DEPTH CHECK] ✅ Sufficient depth for short hedge: {:.2}x", 
                            hedge_depth.depth_ratio);
                    }
                }
                Err(e) => {
                    eprintln!("[DEPTH CHECK] ⚠️  Depth check failed: {} (proceeding with hedge)", e);
                    metrics.record_depth_check_completed();
                }
            }
            
            // Record timestamp before cancellation
            metrics.record_cancel_initiated();
            
            let cancel_result = backend.cancel_order(&short_order.exchange, &short_order.id).await;
            
            // Record timestamp after cancellation completes
            metrics.record_cancel_completed();
            
            // CRITICAL FIX: Check if cancelled order actually filled before placing market order
            eprintln!("[ATOMIC] Checking if cancelled short order filled...");
            match backend.get_order_status_detailed(&short_order.exchange, &short_order.id, &opportunity.symbol).await {
                Ok(status_info) => {
                    total_filled += status_info.filled_quantity;
                    if status_info.filled_quantity > 0.0 {
                        eprintln!("[ATOMIC] ⚠️  Cancelled short order filled {:.4} contracts before cancellation!", status_info.filled_quantity);
                    } else {
                        eprintln!("[ATOMIC] ✅ Cancelled short order did not fill");
                    }
                }
                Err(e) => {
                    eprintln!("[ATOMIC] ⚠️  Failed to check cancelled order status: {} (assuming 0 fill)", e);
                }
            }
            
            // Check if we're already fully hedged from the cancelled order
            if total_filled >= hedge_quantity {
                eprintln!("[ATOMIC] ✅ Already fully hedged from cancelled order ({:.4} contracts)", total_filled);
                short_order.size = total_filled;
                short_filled = true;
                // Skip aggressive limit and market order placement
            } else {
                // Calculate remaining quantity needed
                let remaining_after_cancel = hedge_quantity - total_filled;
                
                // STEP 2: Try aggressive limit order first (at best bid since we're selling short)
                eprintln!("[ATOMIC] Attempting aggressive limit for remaining {:.4} contracts at best bid...", remaining_after_cancel);
                
                match backend.get_best_bid(&opportunity.short_exchange, &opportunity.symbol).await {
                    Ok(best_bid) => {
                        let limit_order = SimulatedOrder {
                            id: format!("aggressive_hedge_{}", uuid::Uuid::new_v4()),
                            exchange: opportunity.short_exchange.clone(),
                            symbol: opportunity.symbol.clone(),
                            side: OrderSide::Short,
                            order_type: OrderType::Limit,
                            price: best_bid,
                            size: remaining_after_cancel,
                            queue_position: None,
                            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                            filled_at: None,
                            fill_price: None,
                            status: OrderStatus::Pending,
                        };
                        
                        match backend.place_order(limit_order.clone()).await {
                            Ok(placed_order) => {
                                eprintln!("[ATOMIC] Aggressive limit placed: {} at ${:.4}", placed_order.id, best_bid);
                                
                                // Wait 2 seconds for fill
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                
                                // Check how much the limit order filled (may be partial)
                                match backend.get_order_status_detailed(&opportunity.short_exchange, &placed_order.id, &opportunity.symbol).await {
                                    Ok(status_info) => {
                                        total_filled += status_info.filled_quantity;
                                        if status_info.filled_quantity >= remaining_after_cancel {
                                            eprintln!("[ATOMIC] ✅ Aggressive limit fully filled! Hedge complete (maker fee)");
                                            short_order = placed_order;
                                            short_order.size = total_filled;
                                            short_filled = true;
                                            
                                            metrics.finalize();
                                            logger.log_timing_summary(&metrics, &opportunity.short_exchange, &opportunity.symbol);
                                        } else if status_info.filled_quantity > 0.0 {
                                            eprintln!("[ATOMIC] ⚠️  Limit partially filled {:.4} contracts, cancelling for market order", status_info.filled_quantity);
                                            let _ = backend.cancel_order(&opportunity.short_exchange, &placed_order.id).await;
                                        } else {
                                            eprintln!("[ATOMIC] ⚠️  Limit didn't fill - cancelling and using market order");
                                            let _ = backend.cancel_order(&opportunity.short_exchange, &placed_order.id).await;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[ATOMIC] ⚠️  Failed to check limit order status: {} (assuming 0 fill)", e);
                                        let _ = backend.cancel_order(&opportunity.short_exchange, &placed_order.id).await;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("[ATOMIC] ⚠️  Failed to place aggressive limit: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[ATOMIC] ⚠️  Failed to get best bid: {}", e);
                    }
                }
                
                // Check if we're already fully hedged after aggressive limit
                if total_filled >= hedge_quantity {
                    eprintln!("[ATOMIC] ✅ Already fully hedged ({:.4} contracts total)", total_filled);
                    short_order.size = total_filled;
                    short_filled = true;
                } else {
                    // STEP 3: Market order for REMAINING quantity only
                    let remaining_quantity = hedge_quantity - total_filled;
                    eprintln!("[ATOMIC] Placing market order for REMAINING {:.4} contracts (total filled so far: {:.4})", 
                        remaining_quantity, total_filled);
                    
                    let short_market_template = SimulatedOrder {
                        id: String::new(),
                        exchange: opportunity.short_exchange.clone(),
                        symbol: opportunity.symbol.clone(),
                        side: OrderSide::Short,
                        order_type: OrderType::Market,
                        price: opportunity.short_price,
                        size: remaining_quantity,  // FIX: Use remaining quantity, not full quantity
                        queue_position: None,
                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                        filled_at: None,
                        fill_price: None,
                        status: OrderStatus::Pending,
                    };
                    
                    metrics.record_market_order_initiated();
                    
                    // Check cancel result for logging
                    let cancel_error = cancel_result.as_ref().err().map(|e| e.to_string());
                    
                    let market_order_result = Self::place_market_order_with_retry(
                        &backend, 
                        short_market_template, 
                        remaining_quantity,
                        Some(&mut metrics), 
                        Some(&logger)
                    ).await;
                    
                    // Log cancellation
                    logger.log_cancel_initiated(&short_order.exchange, &short_order.id);
                    let cancel_elapsed = metrics.cancel_duration().map(|d| d.as_millis()).unwrap_or(0);
                    logger.log_cancel_result(
                        &short_order.exchange,
                        &CancellationResult::Cancelled,
                        cancel_elapsed
                    );
                    
                    if let Some(e) = cancel_error {
                        eprintln!("[ATOMIC] ⚠️  Cancel warning: {} (proceeding with hedge)", e);
                    }
                    
                    logger.log_market_order_placed(
                        &opportunity.short_exchange,
                        "pending",
                        remaining_quantity
                    );
                    
                    match market_order_result {
                        Ok(filled_order) => {
                            metrics.record_market_order_accepted();
                            
                            short_order = filled_order;
                            short_filled = true;
                            
                            metrics.record_market_order_filled();
                            
                            let fill_elapsed = metrics.market_order_fill_duration()
                                .map(|d| d.as_millis()).unwrap_or(0);
                            logger.log_market_order_filled(
                                &opportunity.short_exchange,
                                &short_order.id,
                                fill_elapsed
                            );
                            
                            eprintln!("[ATOMIC] Total filled: {:.4} contracts (target: {:.4})", total_filled + remaining_quantity, hedge_quantity);
                            
                            metrics.finalize();
                            logger.log_timing_summary(&metrics, &opportunity.short_exchange, &opportunity.symbol);
                        }
                        Err(e) => {
                            eprintln!("[ATOMIC] ❌ CRITICAL: Short hedge failed after final check");
                            
                            metrics.finalize();
                            logger.log_timing_summary(&metrics, &opportunity.short_exchange, &opportunity.symbol);
                            
                            if let Err(close_err) = Self::emergency_close_position(&backend, &long_order).await {
                                return Err(format!("CRITICAL: Long filled, short hedge failed, emergency close failed: {} | {}", e, close_err));
                            }
                            return Err(format!("Long filled but short hedge failed (position closed): {}", e));
                        }
                    }
                }
            }
        }
        
        if short_filled && !long_filled {
            eprintln!("[ATOMIC] 🚨 Short filled (final check), long pending → MARKET HEDGING long leg");
            
            // Acquire hedge lock for this symbol to prevent concurrent hedges
            let _hedge_lock = match race_guard.try_acquire_hedge_lock(&opportunity.symbol) {
                Ok(lock) => {
                    eprintln!("[RACE GUARD] ✅ Acquired hedge lock for {}", opportunity.symbol);
                    lock
                }
                Err(e) => {
                    eprintln!("[RACE GUARD] ❌ Failed to acquire hedge lock: {}", e);
                    return Err(format!("Failed to acquire hedge lock: {}", e));
                }
            };
            
            // Initialize timing metrics for final check hedge
            let mut metrics = HedgeTimingMetrics::new();
            let logger = HedgeLogger::default_level();
            
            let hedge_quantity = short_order.size;
            
            // Track cumulative filled quantity
            let mut total_filled = 0.0;
            
            // PRE-FLIGHT DEPTH CHECK before market hedge (always enabled)
            eprintln!("[DEPTH CHECK] Checking depth before long hedge (final check)");
            metrics.record_depth_check_initiated();
            
            match depth_checker.check_depth_for_hedge(
                &long_order.exchange,
                &opportunity.symbol,
                hedge_quantity,
            ).await {
                Ok(hedge_depth) => {
                    metrics.record_depth_check_completed();
                    
                    if hedge_depth.should_abort() {
                        eprintln!("[DEPTH CHECK] ❌ Insufficient depth for long hedge: {:.2}x", 
                            hedge_depth.depth_ratio);
                        
                        // Emergency close short position
                        metrics.finalize();
                        logger.log_timing_summary(&metrics, &opportunity.short_exchange, &opportunity.symbol);
                        
                        Self::emergency_close_position(&backend, &short_order).await?;
                        return Err(format!(
                            "Insufficient depth for long hedge ({:.2}x), short position closed",
                            hedge_depth.depth_ratio
                        ));
                    }
                    
                    if hedge_depth.should_warn() {
                        eprintln!("[DEPTH CHECK] ⚠️  Low liquidity for long hedge: {:.2}x", 
                            hedge_depth.depth_ratio);
                    } else {
                        eprintln!("[DEPTH CHECK] ✅ Sufficient depth for long hedge: {:.2}x", 
                            hedge_depth.depth_ratio);
                    }
                }
                Err(e) => {
                    eprintln!("[DEPTH CHECK] ⚠️  Depth check failed: {} (proceeding with hedge)", e);
                    metrics.record_depth_check_completed();
                }
            }
            
            // Record timestamp before cancellation
            metrics.record_cancel_initiated();
            
            let cancel_result = backend.cancel_order(&long_order.exchange, &long_order.id).await;
            
            // Record timestamp after cancellation completes
            metrics.record_cancel_completed();
            
            // CRITICAL FIX: Check if cancelled order actually filled before placing market order
            eprintln!("[ATOMIC] Checking if cancelled long order filled...");
            match backend.get_order_status_detailed(&long_order.exchange, &long_order.id, &opportunity.symbol).await {
                Ok(status_info) => {
                    total_filled += status_info.filled_quantity;
                    if status_info.filled_quantity > 0.0 {
                        eprintln!("[ATOMIC] ⚠️  Cancelled long order filled {:.4} contracts before cancellation!", status_info.filled_quantity);
                    } else {
                        eprintln!("[ATOMIC] ✅ Cancelled long order did not fill");
                    }
                }
                Err(e) => {
                    eprintln!("[ATOMIC] ⚠️  Failed to check cancelled order status: {} (assuming 0 fill)", e);
                }
            }
            
            // Check if we're already fully hedged from the cancelled order
            if total_filled >= hedge_quantity {
                eprintln!("[ATOMIC] ✅ Already fully hedged from cancelled order ({:.4} contracts)", total_filled);
                long_order.size = total_filled;
                long_filled = true;
                // Skip aggressive limit and market order placement
            } else {
                // Calculate remaining quantity needed
                let remaining_after_cancel = hedge_quantity - total_filled;
                
                // STEP 2: Try aggressive limit order first (at best ask since we're buying long)
                eprintln!("[ATOMIC] Attempting aggressive limit for remaining {:.4} contracts at best ask...", remaining_after_cancel);
                
                match backend.get_best_ask(&opportunity.long_exchange, &opportunity.symbol).await {
                    Ok(best_ask) => {
                        let limit_order = SimulatedOrder {
                            id: format!("aggressive_hedge_{}", uuid::Uuid::new_v4()),
                            exchange: opportunity.long_exchange.clone(),
                            symbol: opportunity.symbol.clone(),
                            side: OrderSide::Long,
                            order_type: OrderType::Limit,
                            price: best_ask,
                            size: remaining_after_cancel,
                            queue_position: None,
                            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                            filled_at: None,
                            fill_price: None,
                            status: OrderStatus::Pending,
                        };
                        
                        match backend.place_order(limit_order.clone()).await {
                            Ok(placed_order) => {
                                eprintln!("[ATOMIC] Aggressive limit placed: {} at ${:.4}", placed_order.id, best_ask);
                                
                                // Wait 2 seconds for fill
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                
                                // Check how much the limit order filled (may be partial)
                                match backend.get_order_status_detailed(&opportunity.long_exchange, &placed_order.id, &opportunity.symbol).await {
                                    Ok(status_info) => {
                                        total_filled += status_info.filled_quantity;
                                        if status_info.filled_quantity >= remaining_after_cancel {
                                            eprintln!("[ATOMIC] ✅ Aggressive limit fully filled! Hedge complete (maker fee)");
                                            long_order = placed_order;
                                            long_order.size = total_filled;
                                            long_filled = true;
                                            
                                            metrics.finalize();
                                            logger.log_timing_summary(&metrics, &opportunity.long_exchange, &opportunity.symbol);
                                        } else if status_info.filled_quantity > 0.0 {
                                            eprintln!("[ATOMIC] ⚠️  Limit partially filled {:.4} contracts, cancelling for market order", status_info.filled_quantity);
                                            let _ = backend.cancel_order(&opportunity.long_exchange, &placed_order.id).await;
                                        } else {
                                            eprintln!("[ATOMIC] ⚠️  Limit didn't fill - cancelling and using market order");
                                            let _ = backend.cancel_order(&opportunity.long_exchange, &placed_order.id).await;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[ATOMIC] ⚠️  Failed to check limit order status: {} (assuming 0 fill)", e);
                                        let _ = backend.cancel_order(&opportunity.long_exchange, &placed_order.id).await;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("[ATOMIC] ⚠️  Failed to place aggressive limit: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[ATOMIC] ⚠️  Failed to get best ask: {}", e);
                    }
                }
                
                // Check if we're already fully hedged after aggressive limit
                if total_filled >= hedge_quantity {
                    eprintln!("[ATOMIC] ✅ Already fully hedged ({:.4} contracts total)", total_filled);
                    long_order.size = total_filled;
                    long_filled = true;
                } else {
                    // STEP 3: Market order for REMAINING quantity only
                    let remaining_quantity = hedge_quantity - total_filled;
                    eprintln!("[ATOMIC] Placing market order for REMAINING {:.4} contracts (total filled so far: {:.4})", 
                        remaining_quantity, total_filled);
                    
                    let long_market_template = SimulatedOrder {
                        id: String::new(),
                        exchange: opportunity.long_exchange.clone(),
                        symbol: opportunity.symbol.clone(),
                        side: OrderSide::Long,
                        order_type: OrderType::Market,
                        price: opportunity.long_price,
                        size: remaining_quantity,  // FIX: Use remaining quantity, not full quantity
                        queue_position: None,
                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                        filled_at: None,
                        fill_price: None,
                        status: OrderStatus::Pending,
                    };
                    
                    metrics.record_market_order_initiated();
                    
                    // Check cancel result for logging
                    let cancel_error = cancel_result.as_ref().err().map(|e| e.to_string());
                    
                    let market_order_result = Self::place_market_order_with_retry(
                        &backend, 
                        long_market_template, 
                        remaining_quantity,
                        Some(&mut metrics), 
                        Some(&logger)
                    ).await;
                    
                    // Log cancellation
                    logger.log_cancel_initiated(&long_order.exchange, &long_order.id);
                    let cancel_elapsed = metrics.cancel_duration().map(|d| d.as_millis()).unwrap_or(0);
                    logger.log_cancel_result(
                        &long_order.exchange,
                        &CancellationResult::Cancelled,
                        cancel_elapsed
                    );
                    
                    if let Some(e) = cancel_error {
                        eprintln!("[ATOMIC] ⚠️  Cancel warning: {} (proceeding with hedge)", e);
                    }
                    
                    logger.log_market_order_placed(
                        &opportunity.long_exchange,
                        "pending",
                        remaining_quantity
                    );
                    
                    match market_order_result {
                        Ok(filled_order) => {
                            metrics.record_market_order_accepted();
                            
                            long_order = filled_order;
                            long_filled = true;
                            
                            metrics.record_market_order_filled();
                            
                            let fill_elapsed = metrics.market_order_fill_duration()
                                .map(|d| d.as_millis()).unwrap_or(0);
                            logger.log_market_order_filled(
                                &opportunity.long_exchange,
                                &long_order.id,
                                fill_elapsed
                            );
                            
                            eprintln!("[ATOMIC] Total filled: {:.4} contracts (target: {:.4})", total_filled + remaining_quantity, hedge_quantity);
                            
                            metrics.finalize();
                            logger.log_timing_summary(&metrics, &opportunity.long_exchange, &opportunity.symbol);
                        }
                        Err(e) => {
                            eprintln!("[ATOMIC] ❌ CRITICAL: Long hedge failed after final check");
                            
                            metrics.finalize();
                            logger.log_timing_summary(&metrics, &opportunity.long_exchange, &opportunity.symbol);
                            
                            if let Err(close_err) = Self::emergency_close_position(&backend, &short_order).await {
                                return Err(format!("CRITICAL: Short filled, long hedge failed, emergency close failed: {} | {}", e, close_err));
                            }
                            return Err(format!("Short filled but long hedge failed (position closed): {}", e));
                        }
                    }
                }
            }
        }

        if !long_filled || !short_filled {
            // This shouldn't happen due to market hedging, but handle it
            eprintln!("[ATOMIC] ❌ CRITICAL: One leg filled but hedge failed");
            return Err("One leg filled but hedge failed - this should not happen".to_string());
        }

        // STEP 5: Both legs filled - create trade
        let trade_id = Uuid::new_v4().to_string();
        let entry_spread_bps = ((opportunity.short_price - opportunity.long_price) / opportunity.long_price) * 10000.0;

        eprintln!("[ATOMIC] ✅ Both legs filled successfully! Trade ID: {}", trade_id);
        
        // Log repricing statistics
        if long_reprice_count > 0 || short_reprice_count > 0 {
            long_metrics.finalize();
            short_metrics.finalize();
            
            eprintln!("[REPRICING SUMMARY]");
            eprintln!("  Long: {} reprices, {:.2} bps improvement, {}ms total", 
                long_metrics.reprice_count, 
                long_metrics.price_improvement_bps,
                long_metrics.reprice_total_time_ms);
            eprintln!("  Short: {} reprices, {:.2} bps improvement, {}ms total", 
                short_metrics.reprice_count, 
                short_metrics.price_improvement_bps,
                short_metrics.reprice_total_time_ms);
        }

        // STEP 6: Place exit orders (passive strategy)
        let spread_amount = opportunity.short_price - opportunity.long_price;
        let target_capture = spread_amount * 0.9;
        
        let long_exit_price = opportunity.long_price + target_capture;
        let short_exit_price = opportunity.short_price - target_capture;

        let stop_loss_spread = spread_amount * 1.3;
        let long_stop_price = opportunity.short_price - stop_loss_spread;
        let short_stop_price = opportunity.long_price + stop_loss_spread;

        eprintln!("[EXIT ORDERS] Target: Long ${:.4} | Short ${:.4}", long_exit_price, short_exit_price);

        let long_quantity = long_order.size;
        let short_quantity = short_order.size;

        let long_exit_order_template = SimulatedOrder {
            id: String::new(),
            exchange: opportunity.long_exchange.clone(),
            symbol: opportunity.symbol.clone(),
            side: OrderSide::Short,
            order_type: OrderType::Limit,
            price: long_exit_price,
            size: long_quantity,
            queue_position: None,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            filled_at: None,
            fill_price: None,
            status: OrderStatus::Pending,
        };

        let short_exit_order_template = SimulatedOrder {
            id: String::new(),
            exchange: opportunity.short_exchange.clone(),
            symbol: opportunity.symbol.clone(),
            side: OrderSide::Long,
            order_type: OrderType::Limit,
            price: short_exit_price,
            size: short_quantity,
            queue_position: None,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            filled_at: None,
            fill_price: None,
            status: OrderStatus::Pending,
        };

        let long_exit_order = match backend.place_order(long_exit_order_template).await {
            Ok(order) => {
                eprintln!("[EXIT ORDERS] Long exit order placed: {} at ${:.4}", order.id, long_exit_price);
                Some(order)
            }
            Err(e) => {
                eprintln!("[EXIT ORDERS] WARNING: Failed to place long exit order: {}", e);
                None
            }
        };

        let short_exit_order = match backend.place_order(short_exit_order_template).await {
            Ok(order) => {
                eprintln!("[EXIT ORDERS] Short exit order placed: {} at ${:.4}", order.id, short_exit_price);
                Some(order)
            }
            Err(e) => {
                eprintln!("[EXIT ORDERS] WARNING: Failed to place short exit order: {}", e);
                None
            }
        };

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
            long_exit_order,
            short_exit_order,
            stop_loss_triggered: false,
            stop_loss_long_price: long_stop_price,
            stop_loss_short_price: short_stop_price,
            leg_out_event: None,
        };

        eprintln!("[ATOMIC] ✅ Trade setup complete! Trade ID: {}", paper_trade.id);

        Ok(paper_trade)
    }
}
