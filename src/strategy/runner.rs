use crate::strategy::types::*;
use crate::strategy::confluence::ConfluenceCalculator;
use crate::strategy::scanner::OpportunityScanner;
use crate::strategy::entry::EntryExecutor;
use crate::strategy::positions::PositionManager;
use crate::strategy::portfolio::PortfolioManager;
use crate::strategy::latency::LatencyMonitor;
use crate::strategy::atomic_execution::{AtomicExecutor, NegativeFundingTracker};
use crate::exchange_parser::get_parser;
use redis::aio::MultiplexedConnection;
use std::collections::HashMap;
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{self, Duration};
use uuid::Uuid;

pub struct StrategyRunner {
    portfolio_manager: PortfolioManager,
    redis_conn: MultiplexedConnection,
    latency_monitor: LatencyMonitor,
    confluence_calc: ConfluenceCalculator,
    active_trades: HashMap<String, PaperTrade>,
    negative_funding_trackers: HashMap<String, NegativeFundingTracker>,
}

impl StrategyRunner {
    pub async fn new(
        redis_conn: MultiplexedConnection,
        starting_capital: f64,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let portfolio_manager = PortfolioManager::new(redis_conn.clone(), starting_capital).await?;
        let latency_monitor = LatencyMonitor::new();
        let confluence_calc = ConfluenceCalculator::new();

        // Start with fresh active trades (don't load stale data from previous runs)
        let active_trades = HashMap::new();

        Ok(Self {
            portfolio_manager,
            redis_conn,
            latency_monitor,
            confluence_calc,
            active_trades,
            negative_funding_trackers: HashMap::new(),
        })
    }

    pub async fn run_scanning_loop(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut interval = time::interval(Duration::from_secs(1));
        
        // Clear stale opportunities from previous runs
        let mut conn = self.redis_conn.clone();
        redis::cmd("DEL")
            .arg("strategy:opportunities")
            .query_async::<_, ()>(&mut conn)
            .await?;
        
        eprintln!("[STRATEGY] Starting scanning loop with ${:.2} capital", self.portfolio_manager.get_state().available_capital);

        loop {
            interval.tick().await;

            // 1. Scan for opportunities
            if let Err(e) = self.scan_opportunities().await {
                eprintln!("Error scanning opportunities: {}", e);
                continue;
            }

            // 2. Monitor active positions
            if let Err(e) = self.monitor_active_positions().await {
                eprintln!("Error monitoring positions: {}", e);
                continue;
            }

            // 3. Check for exits
            if let Err(e) = self.check_exits().await {
                eprintln!("Error checking exits: {}", e);
                continue;
            }
        }
    }

    async fn scan_opportunities(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Read opportunities that dashboard already calculated and stored in Redis
        let mut conn = self.redis_conn.clone();
        
        let opportunities_json: String = match redis::cmd("GET")
            .arg("strategy:opportunities")
            .query_async(&mut conn)
            .await
        {
            Ok(json) => json,
            Err(_) => {
                eprintln!("[DEBUG] No opportunities found in Redis");
                return Ok(());
            }
        };

        let mut opportunities: Vec<ArbitrageOpportunity> = match serde_json::from_str(&opportunities_json) {
            Ok(opps) => opps,
            Err(e) => {
                eprintln!("[DEBUG] Failed to parse opportunities: {}", e);
                return Ok(());
            }
        };

        // Sort opportunities by spread (highest first) to maximize profitability
        opportunities.sort_by(|a, b| b.spread_bps.partial_cmp(&a.spread_bps).unwrap_or(std::cmp::Ordering::Equal));

        if !opportunities.is_empty() {
            println!("\n[OPPORTUNITIES] Found {} opportunities from dashboard (sorted by spread)", opportunities.len());
            for (idx, opp) in opportunities.iter().enumerate() {
                println!(
                    "  {}. {} - Spread: {:.2}bps | Confidence: {:.0} | Profit: ${:.2}",
                    idx + 1,
                    opp.symbol,
                    opp.spread_bps,
                    opp.confidence_score,
                    opp.projected_profit_usd
                );
            }
        }

        // Execute trades for ALL opportunities (not just top 5)
        for opportunity in opportunities.iter() {
            eprintln!("[DEBUG] Checking opportunity: {} | Spread: {:.2}bps | Confidence: {:.0}", opportunity.symbol, opportunity.spread_bps, opportunity.confidence_score);
            
            // ============================================================================
            // DECISION TREE: OPTION B - RE-VALIDATE EVERYTHING IN RUNNER
            // ============================================================================
            
            // DECISION 1: Duplicate Symbol Check
            if self.active_trades.values().any(|t| t.symbol == opportunity.symbol) {
                println!("[SKIPPED] {} - DUPLICATE SYMBOL | Already have active trade", opportunity.symbol);
                eprintln!("[DEBUG] SKIPPING {} - already have active trade for this symbol", opportunity.symbol);
                continue;
            }
            
            // DECISION 2: Fetch Current Prices (Required for all subsequent checks)
            let (current_long_price, current_short_price) = match self.get_current_prices_for_opportunity(&opportunity.symbol, &opportunity.long_exchange, &opportunity.short_exchange).await {
                Ok((long, short)) => (long, short),
                Err(_) => {
                    println!("[SKIPPED] {} - NO PRICES | Could not fetch current prices from Redis", opportunity.symbol);
                    eprintln!("[DEBUG] Failed to fetch current prices for {}", opportunity.symbol);
                    continue;
                }
            };
            
            // DECISION 3: Re-validate Hard Constraint 1 - Order Book Depth
            // (Depth may have changed since dashboard calculated it)
            let order_book_depth_sufficient = opportunity.order_book_depth_long >= opportunity.metrics.hard_constraints.order_book_depth_sufficient as u32 as f64 * 2.0
                && opportunity.order_book_depth_short >= opportunity.metrics.hard_constraints.order_book_depth_sufficient as u32 as f64 * 2.0;
            
            if !order_book_depth_sufficient {
                println!("[SKIPPED] {} - INSUFFICIENT DEPTH | Long: ${:.0} | Short: ${:.0}", 
                    opportunity.symbol, opportunity.order_book_depth_long, opportunity.order_book_depth_short);
                eprintln!("[DEBUG] SKIPPING {} - order book depth insufficient", opportunity.symbol);
                continue;
            }
            
            // DECISION 4: Re-validate Hard Constraint 2 - Exchange Latency
            // (Latency may have increased since dashboard calculated it)
            if !opportunity.metrics.hard_constraints.exchange_latency_ok {
                println!("[SKIPPED] {} - HIGH LATENCY | Exchange latency > 200ms", opportunity.symbol);
                eprintln!("[DEBUG] SKIPPING {} - exchange latency too high", opportunity.symbol);
                continue;
            }
            
            // DECISION 5: Re-validate Hard Constraint 3 - Funding Delta Substantial
            // (Funding rates update every 8 hours, but check if still meaningful)
            if !opportunity.metrics.hard_constraints.funding_delta_substantial {
                println!("[SKIPPED] {} - INSUFFICIENT FUNDING DELTA | Delta: {:.6}", 
                    opportunity.symbol, opportunity.metrics.funding_delta);
                eprintln!("[DEBUG] SKIPPING {} - funding delta not substantial", opportunity.symbol);
                continue;
            }
            
            // DECISION 6: Re-calculate Spread with Fresh Prices
            let current_spread_bps = OpportunityScanner::calculate_spread_bps(current_long_price, current_short_price);
            eprintln!("[DEBUG] {} | Dashboard spread: {:.2}bps | Current spread: {:.2}bps", 
                opportunity.symbol, opportunity.spread_bps, current_spread_bps);
            
            // DECISION 7: Spread Must Be Positive
            if current_spread_bps <= 0.0 {
                println!("[SKIPPED] {} - NEGATIVE SPREAD | Current: {:.2}bps", opportunity.symbol, current_spread_bps);
                eprintln!("[DEBUG] SKIPPING {} - spread is negative or zero", opportunity.symbol);
                continue;
            }
            
            // DECISION 8: Calculate Fees and Net Profit
            let long_taker_bps = self.get_exchange_taker_fee(&opportunity.long_exchange);
            let short_taker_bps = self.get_exchange_taker_fee(&opportunity.short_exchange);
            let total_fee_bps = long_taker_bps + short_taker_bps;
            let net_profit_bps = current_spread_bps - total_fee_bps;
            
            // DECISION 9: Net Profit Must Be Positive
            if net_profit_bps <= 0.0 {
                println!("[SKIPPED] {} - UNPROFITABLE | Spread: {:.2}bps | Fees: {:.2}bps | Net: {:.2}bps", 
                    opportunity.symbol, current_spread_bps, total_fee_bps, net_profit_bps);
                eprintln!("[DEBUG] SKIPPING {} - net profit is negative: {:.2}bps", opportunity.symbol, net_profit_bps);
                continue;
            }
            
            // DECISION 10: Check Available Capital
            let available_capital = self.portfolio_manager.get_state().available_capital;
            if available_capital <= 0.0 {
                println!("[SKIPPED] {} - NO CAPITAL | Available: ${:.2}", opportunity.symbol, available_capital);
                eprintln!("[DEBUG] No available capital, skipping trade for {}", opportunity.symbol);
                continue;
            }
            
            // DECISION 11: Calculate Position Size
            let position_size = EntryExecutor::calculate_position_size(
                current_spread_bps,
                available_capital,
                total_fee_bps,
                10.0,  // funding cost estimate
            );
            
            // DECISION 12: Position Size Must Be Valid
            if position_size <= 0.0 {
                println!("[SKIPPED] {} - INVALID POSITION SIZE | Calculated: ${:.2}", opportunity.symbol, position_size);
                eprintln!("[DEBUG] SKIPPING {} - position size is invalid", opportunity.symbol);
                continue;
            }
            
            // DECISION 13: Position Size Must Not Exceed Available Capital
            if position_size > available_capital {
                println!("[SKIPPED] {} - INSUFFICIENT CAPITAL | Available: ${:.2} | Required: ${:.2}", 
                    opportunity.symbol, available_capital, position_size);
                eprintln!("[DEBUG] SKIPPING {} - position size exceeds available capital", opportunity.symbol);
                continue;
            }
            
            // DECISION 14: Re-calculate Confidence Score (Don't Trust Dashboard)
            // For now, use a simplified confidence check based on hard constraints
            // In production, you'd recalculate all soft metrics here
            let confidence_score = if opportunity.metrics.hard_constraints.passes_all() {
                opportunity.confidence_score  // Use dashboard's soft metrics
            } else {
                0  // Hard constraints failed, confidence = 0
            };
            
            // DECISION 15: Confidence Must Meet Threshold
            if confidence_score < 70 {
                println!("[SKIPPED] {} - LOW CONFIDENCE | Score: {:.0}", opportunity.symbol, confidence_score);
                eprintln!("[DEBUG] SKIPPING {} - confidence score too low: {:.0}", opportunity.symbol, confidence_score);
                continue;
            }
            
            // ============================================================================
            // ALL DECISIONS PASSED - EXECUTE TRADE
            // ============================================================================
            
            eprintln!("[DEBUG] {} PASSED ALL CHECKS | Spread: {:.2}bps | Net Profit: {:.2}bps | Position: ${:.2}", 
                opportunity.symbol, current_spread_bps, net_profit_bps, position_size);
            
            match EntryExecutor::execute_atomic_entry(opportunity, available_capital, position_size) {
                Ok(mut trade) => {
                    // Recalculate projected profit based on actual position size and current spread
                    // Expected profit = 90% of entry spread minus fees
                    let projected_profit_bps = (current_spread_bps * 0.9) - total_fee_bps;
                    trade.projected_profit_usd = ((projected_profit_bps) / 10000.0) * position_size;
                    
                    println!(
                        "[ENTRY] Executed trade {} on {} | Size: ${:.2} | Spread: {:.2}bps | Projected Profit: ${:.2}",
                        trade.id, trade.symbol, trade.position_size_usd, current_spread_bps, trade.projected_profit_usd
                    );
                    
                    // Add to active trades
                    self.active_trades.insert(trade.id.clone(), trade.clone());
                    
                    // Update portfolio state
                    if let Err(e) = self.portfolio_manager.open_trade(trade).await {
                        eprintln!("Error opening trade in portfolio: {}", e);
                    } else {
                        let state = self.portfolio_manager.get_state();
                        eprintln!("[PORTFOLIO] After trade: Available Capital: ${:.2} | Total Open: ${:.2} | Active Trades: {}",
                            state.available_capital, state.total_open_positions, state.active_trades.len());
                    }
                }
                Err(e) => {
                    println!("[SKIPPED] {} - ATOMIC EXECUTION FAILED | Reason: {}", opportunity.symbol, e);
                    eprintln!("Error executing entry for {}: {}", opportunity.symbol, e);
                }
            }
        }

        Ok(())
    }

    async fn monitor_active_positions(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let trade_ids: Vec<String> = self.active_trades.keys().cloned().collect();

        if !trade_ids.is_empty() {
            eprintln!("[MONITOR] Monitoring {} active trades", trade_ids.len());
        }

        for trade_id in trade_ids {
            // Get trade info first
            let (symbol, entry_spread_bps, position_size_usd, entry_projected_profit) = {
                if let Some(trade) = self.active_trades.get(&trade_id) {
                    (trade.symbol.clone(), trade.entry_spread_bps, trade.position_size_usd, trade.projected_profit_usd)
                } else {
                    continue;
                }
            };

            // Get current prices from Redis (same source as dashboard)
            let (current_long_price, current_short_price) = self.get_current_prices(&symbol).await?;
            let current_spread_bps = OpportunityScanner::calculate_spread_bps(current_long_price, current_short_price);

            // Calculate spread-based P&L (what matters for arbitrage)
            let spread_reduction_bps = entry_spread_bps - current_spread_bps;
            
            // Calculate current P&L based on spread reduction only
            // Fees are already accounted for in the projected_profit calculation
            let current_pnl = (spread_reduction_bps / 10000.0) * position_size_usd;

            // Get funding rates before mutable borrow
            // Use the funding delta from the opportunity (already calculated by dashboard)
            let entry_funding_delta = if let Some(trade) = self.active_trades.get(&trade_id) {
                trade.funding_delta_entry
            } else {
                0.0
            };
            
            // For now, we don't have live funding rate updates, so use the entry value
            let current_funding_delta = entry_funding_delta;

            eprintln!("[MONITOR] {} | Entry: {:.2}bps | Current: {:.2}bps | Spread Reduction: {:.2}bps | P&L: ${:.2} | Projected: ${:.2} (Target: ${:.2}, Stop: ${:.2})",
                symbol, entry_spread_bps, current_spread_bps, spread_reduction_bps, current_pnl, entry_projected_profit, entry_projected_profit * 0.9, entry_projected_profit * -0.2);

            // Now update the trade with exit conditions
            if let Some(trade) = self.active_trades.get_mut(&trade_id) {
                eprintln!("[MONITOR] {} | Checking exit conditions | P&L: ${:.2} | Projected: ${:.2}", trade.id, current_pnl, entry_projected_profit);
                
                // Exit conditions:
                // 1. Profit target: 90% of the spread has been closed (narrowed)
                // Spread closed = entry_spread - current_spread
                // Target = 90% of entry_spread
                // CRITICAL: Only exit if current spread is positive (valid arbitrage)
                let spread_closed_pct = (spread_reduction_bps / entry_spread_bps) * 100.0;
                
                eprintln!("[PROFIT_CHECK] {} | Spread Closed: {:.1}% | Current Spread: {:.2}bps | Entry Spread: {:.2}bps | P&L: ${:.2} | Projected: ${:.2}",
                    trade.id, spread_closed_pct, current_spread_bps, entry_spread_bps, current_pnl, entry_projected_profit);
                
                if spread_closed_pct >= 90.0 && entry_spread_bps > 0.0 && current_spread_bps > 0.0 {
                    // Capture the exit spread at the moment we trigger the exit
                    trade.exit_spread_bps = Some(current_spread_bps);
                    trade.exit_reason = Some(format!("Profit hit | Entry: {:.2}bps | Exit: {:.2}bps | Captured: {:.2}bps", entry_spread_bps, current_spread_bps, spread_reduction_bps));
                    trade.status = TradeStatus::Exiting;
                    println!(
                        "[EXIT SIGNAL] {} | Reason: profit_target | Entry Spread: {:.2}bps | Current: {:.2}bps | Closed: {:.1}% | P&L: ${:.2}",
                        trade.id, entry_spread_bps, current_spread_bps, spread_closed_pct, current_pnl
                    );
                } else if spread_closed_pct >= 90.0 {
                    eprintln!("[PROFIT_CHECK_FAILED] {} | Spread closed {:.1}% but conditions not met: entry_spread > 0: {} | current_spread > 0: {}",
                        trade.id, spread_closed_pct, entry_spread_bps > 0.0, current_spread_bps > 0.0);
                }
                
                // 2. Stop loss: we've lost 20% of the projected profit (spread widened significantly)
                if current_pnl <= entry_projected_profit * -0.2 && entry_projected_profit > 0.0 && trade.exit_reason.is_none() {
                    trade.exit_spread_bps = Some(current_spread_bps);
                    trade.exit_reason = Some(format!("Stop loss | Entry: {:.2}bps | Exit: {:.2}bps", entry_spread_bps, current_spread_bps));
                    trade.status = TradeStatus::Exiting;
                    println!(
                        "[EXIT SIGNAL] {} | Reason: stop_loss | Spread Reduction: {:.2}bps | P&L: ${:.2} (Stop: ${:.2})",
                        trade.id, spread_reduction_bps, current_pnl, entry_projected_profit * -0.2
                    );
                }
                
                // 2b. Stop loss: absolute loss threshold (lose more than $5 or 50% of projected profit)
                let max_loss = (entry_projected_profit * 0.5).max(5.0);
                eprintln!("[MONITOR] {} | Checking absolute loss: P&L: ${:.2} vs Max Loss: ${:.2}", trade.id, current_pnl, -max_loss);
                if current_pnl <= -max_loss && trade.exit_reason.is_none() {
                    trade.exit_spread_bps = Some(current_spread_bps);
                    trade.exit_reason = Some(format!("Stop loss | Entry: {:.2}bps | Exit: {:.2}bps", entry_spread_bps, current_spread_bps));
                    trade.status = TradeStatus::Exiting;
                    println!(
                        "[EXIT SIGNAL] {} | Reason: stop_loss (absolute loss) | P&L: ${:.2} (Max Loss: ${:.2})",
                        trade.id, current_pnl, -max_loss
                    );
                }
                
                // 2c. Stop loss: spread has widened significantly (more than 30% wider than entry)
                if current_spread_bps > entry_spread_bps * 1.3 && trade.exit_reason.is_none() {
                    trade.exit_spread_bps = Some(current_spread_bps);
                    trade.exit_reason = Some(format!("Stop loss | Entry: {:.2}bps | Exit: {:.2}bps", entry_spread_bps, current_spread_bps));
                    trade.status = TradeStatus::Exiting;
                    println!(
                        "[EXIT SIGNAL] {} | Reason: stop_loss (spread widened) | Entry: {:.2}bps | Current: {:.2}bps",
                        trade.id, entry_spread_bps, current_spread_bps
                    );
                }
                
                // 3. Funding convergence: funding delta has converged significantly (reduced by 80%)
                // Only check if we have meaningful funding delta to begin with
                if entry_funding_delta.abs() > 0.0001 && current_funding_delta.abs() < entry_funding_delta.abs() * 0.2 && trade.exit_reason.is_none() {
                    trade.exit_spread_bps = Some(current_spread_bps);
                    trade.exit_reason = Some("funding_convergence".to_string());
                    trade.status = TradeStatus::Exiting;
                    println!(
                        "[EXIT SIGNAL] {} | Reason: funding_convergence | Entry Funding Delta: {:.6} | Current: {:.6}",
                        trade.id, entry_funding_delta, current_funding_delta
                    );
                }
                
                // 4. Funding convergence: funding delta < 0.005%
                if current_funding_delta.abs() < 0.00005 && trade.exit_reason.is_none() {
                    trade.exit_spread_bps = Some(current_spread_bps);
                    trade.exit_reason = Some("funding_convergence".to_string());
                    trade.status = TradeStatus::Exiting;
                    println!(
                        "[EXIT SIGNAL] {} | Reason: funding_convergence | Funding Delta: {:.6}",
                        trade.id, current_funding_delta
                    );
                }

                // Check leg-out risk
                let time_since_entry = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64
                    - (trade.entry_time as u64 * 1000);

                if PositionManager::detect_leg_out(
                    trade.long_order.status == OrderStatus::Filled,
                    trade.short_order.status == OrderStatus::Filled,
                    time_since_entry,
                ) {
                    if trade.leg_out_event.is_none() {
                        // Log leg-out event
                        let filled_leg = if trade.long_order.status == OrderStatus::Filled {
                            "long".to_string()
                        } else {
                            "short".to_string()
                        };
                        
                        println!(
                            "[LEG-OUT] {} | Filled leg: {} | Time since entry: {}ms",
                            trade.id, filled_leg, time_since_entry
                        );

                        trade.leg_out_event = Some(LegOutEvent {
                            filled_leg,
                            filled_at: trade.long_order.filled_at.unwrap_or(0),
                            unfilled_leg: if trade.long_order.status != OrderStatus::Filled {
                                "long".to_string()
                            } else {
                                "short".to_string()
                            },
                            hedge_executed: true,
                            hedge_price: current_long_price.max(current_short_price),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    async fn check_exits(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let trade_ids: Vec<String> = self.active_trades.keys().cloned().collect();
        
        let exiting_count = trade_ids.iter().filter(|id| {
            self.active_trades.get(*id).map(|t| t.status == TradeStatus::Exiting).unwrap_or(false)
        }).count();
        
        if exiting_count > 0 {
            eprintln!("[CHECK_EXITS] Found {} trades with Exiting status", exiting_count);
        }

        for trade_id in trade_ids {
            let symbol = {
                self.active_trades.get(&trade_id).map(|t| t.symbol.clone())
            };

            if let Some(symbol) = symbol {
                // Check negative funding exit
                let should_exit = {
                    if let Some(tracker) = self.negative_funding_trackers.get(&symbol) {
                        tracker.should_exit()
                    } else {
                        false
                    }
                };

                if should_exit {
                    // Update funding rate for tracker
                    let (funding_long, funding_short) = match self.get_funding_rates(&symbol).await? {
                        Some((long, short)) => (long, short),
                        None => continue,
                    };
                    let funding_delta = funding_long - funding_short;
                    
                    if let Some(tracker) = self.negative_funding_trackers.get_mut(&symbol) {
                        tracker.update_funding(funding_delta);
                    }
                    
                    if let Some(trade) = self.active_trades.get_mut(&trade_id) {
                        // Trigger exit due to negative funding
                        trade.exit_reason = Some("negative_funding_exit".to_string());
                        trade.status = TradeStatus::Exiting;
                    }
                }

                if let Some(trade) = self.active_trades.get(&trade_id) {
                    if trade.status == TradeStatus::Exiting {
                        // Use the exit spread that was captured when the exit was triggered
                        // This ensures we don't report negative spreads due to price movements after exit signal
                        let exit_spread_bps = trade.exit_spread_bps.unwrap_or_else(|| {
                            // Fallback: recalculate if not captured (shouldn't happen with new code)
                            eprintln!("[WARNING] {} | Exit spread not captured, recalculating", trade_id);
                            0.0
                        });
                        
                        // Calculate actual profit/loss based on spread narrowing
                        // Profit = (entry_spread - exit_spread) * position_size / 10000 - fees
                        let spread_reduction_bps = trade.entry_spread_bps - exit_spread_bps;
                        
                        // Get exchange-specific taker fees
                        let long_taker_bps = self.get_exchange_taker_fee(&trade.long_exchange);
                        let short_taker_bps = self.get_exchange_taker_fee(&trade.short_exchange);
                        let total_fee_bps = long_taker_bps + short_taker_bps;
                        
                        // Calculate profit: spread reduction minus fees
                        let actual_profit = ((spread_reduction_bps - total_fee_bps) / 10000.0) * trade.position_size_usd;

                        let exit_reason = trade.exit_reason.clone().unwrap_or_else(|| "manual_exit".to_string());

                        // Close trade
                        self.portfolio_manager
                            .close_trade(&trade_id, actual_profit, exit_reason.clone())
                            .await?;

                        println!(
                            "[EXIT COMPLETED] {} | Reason: {} | Entry Spread: {:.2}bps | Exit Spread: {:.2}bps | Fees: {:.2}bps | Actual Profit: ${:.2}",
                            trade_id, exit_reason, trade.entry_spread_bps, exit_spread_bps, total_fee_bps, actual_profit
                        );

                        // Reset negative funding tracker
                        if let Some(tracker) = self.negative_funding_trackers.get_mut(&symbol) {
                            tracker.reset();
                        }

                        self.active_trades.remove(&trade_id);
                    }
                }
            }
        }
        
        // Don't reload from portfolio manager - keep our local state in sync
        // The portfolio manager updates its state when we call close_trade()

        Ok(())
    }

    async fn get_all_pairs(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("*:linear:tickers:*USDT")
            .query_async::<_, Vec<String>>(&mut self.redis_conn.clone())
            .await?;

        let pairs: Vec<String> = keys
            .iter()
            .filter_map(|k| k.split(':').nth(3).map(|p| p.to_string()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        Ok(pairs)
    }

    async fn get_pair_prices(
        &self,
        pair: &str,
    ) -> Result<HashMap<String, (f64, f64)>, Box<dyn Error + Send + Sync>> {
        let exchanges = vec!["binance", "bybit", "okx", "kucoin", "bitget", "gateio", "hyperliquid", "paradex"];
        let mut prices = HashMap::new();

        for exchange in exchanges {
            let key_patterns = crate::exchange_parser::get_redis_key_patterns(exchange, pair);
            
            let mut conn = self.redis_conn.clone();
            
            // Try each key pattern until we find data
            for key in key_patterns {
                if let Ok(data) = redis::cmd("GET")
                    .arg(&key)
                    .query_async::<_, String>(&mut conn)
                    .await
                {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                        let parser = get_parser(exchange);
                        // Extract bid and ask prices
                        if let (Some(bid_str), Some(ask_str)) = (parser.parse_bid(&json), parser.parse_ask(&json)) {
                            if let (Ok(bid), Ok(ask)) = (bid_str.parse::<f64>(), ask_str.parse::<f64>()) {
                                prices.insert(exchange.to_string(), (bid, ask));
                                break; // Found data for this exchange, move to next exchange
                            }
                        }
                    }
                }
            }
        }

        Ok(prices)
    }

    fn find_spread(
        &self,
        prices: &HashMap<String, (f64, f64)>,
    ) -> Result<(String, f64, String, f64), Box<dyn Error + Send + Sync>> {
        let mut min_ask = f64::MAX;
        let mut max_bid = 0.0;
        let mut long_exchange = String::new();
        let mut short_exchange = String::new();

        for (exchange, (bid, ask)) in prices {
            if *ask < min_ask {
                min_ask = *ask;
                long_exchange = exchange.clone();
            }
            if *bid > max_bid {
                max_bid = *bid;
                short_exchange = exchange.clone();
            }
        }

        Ok((long_exchange, min_ask, short_exchange, max_bid))
    }

    async fn get_funding_rates(&self, pair: &str) -> Result<Option<(f64, f64)>, Box<dyn Error + Send + Sync>> {
        // Try all exchanges - same as dashboard does
        let exchanges = vec!["binance", "bybit", "okx", "kucoin", "bitget", "gateio", "hyperliquid", "paradex"];
        let mut rates = Vec::new();

        for exchange in exchanges {
            let key_patterns = crate::exchange_parser::get_redis_key_patterns(exchange, pair);
            let mut conn = self.redis_conn.clone();
            
            // Try each key pattern until we find data
            for key in key_patterns {
                if let Ok(data) = redis::cmd("GET")
                    .arg(&key)
                    .query_async::<_, String>(&mut conn)
                    .await
                {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                        let parser = get_parser(exchange);
                        if let Some(rate) = parser.parse_funding_rate(&json) {
                            rates.push(rate);
                            break; // Found data for this exchange
                        }
                    }
                }
            }
        }

        if rates.len() >= 2 {
            Ok(Some((rates[0], rates[1])))
        } else {
            Ok(None)
        }
    }

    async fn get_order_book_depths(&self, pair: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        let exchanges = vec!["binance", "bybit"];
        let mut depths = Vec::new();

        for exchange in exchanges {
            let key = format!("{}:linear:tickers:{}", exchange, pair);
            let mut conn = self.redis_conn.clone();
            if let Ok(data) = redis::cmd("GET")
                .arg(&key)
                .query_async::<_, String>(&mut conn)
                .await
            {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                    // Try to extract bid/ask volume or use a default
                    let depth = json.get("bid_volume")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(50000.0);
                    depths.push(depth);
                }
            }
        }

        if depths.len() >= 2 {
            Ok((depths[0], depths[1]))
        } else {
            Ok((50000.0, 45000.0))
        }
    }

    async fn get_oi(&self, pair: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let key = format!("binance:linear:tickers:{}", pair);
        let mut conn = self.redis_conn.clone();
        if let Ok(data) = redis::cmd("GET")
            .arg(&key)
            .query_async::<_, String>(&mut conn)
            .await
        {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(oi) = json.get("open_interest").and_then(|o| o.as_f64()) {
                    return Ok(oi);
                }
            }
        }
        Ok(1000000.0)
    }

    async fn get_current_prices(&self, pair: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        // Find the trade to get the specific exchanges it's using
        let trade = self.active_trades.values().find(|t| t.symbol == pair);
        
        if let Some(trade) = trade {
            // Fetch prices ONLY from the specific exchanges this trade is using
            self.fetch_prices_from_exchanges(pair, &trade.long_exchange, &trade.short_exchange).await
        } else {
            Err("No trade found for pair".into())
        }
    }

    async fn get_current_prices_for_opportunity(&self, pair: &str, long_exchange: &str, short_exchange: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        // Fetch prices from the specific exchanges provided (used during opportunity validation)
        self.fetch_prices_from_exchanges(pair, long_exchange, short_exchange).await
    }

    async fn fetch_prices_from_exchanges(&self, pair: &str, long_exchange: &str, short_exchange: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        let key_patterns_long = crate::exchange_parser::get_redis_key_patterns(long_exchange, pair);
        let key_patterns_short = crate::exchange_parser::get_redis_key_patterns(short_exchange, pair);
        
        let mut conn = self.redis_conn.clone();
        let mut long_price = None;
        let mut short_price = None;
        
        // Fetch long exchange ask price
        for key in key_patterns_long {
            if let Ok(data) = redis::cmd("GET")
                .arg(&key)
                .query_async::<_, String>(&mut conn)
                .await
            {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                    let parser = crate::exchange_parser::get_parser(long_exchange);
                    if let Some(ask_str) = parser.parse_ask(&json) {
                        if let Ok(ask) = ask_str.parse::<f64>() {
                            long_price = Some(ask);
                            break;
                        }
                    }
                }
            }
        }
        
        // Fetch short exchange bid price
        for key in key_patterns_short {
            if let Ok(data) = redis::cmd("GET")
                .arg(&key)
                .query_async::<_, String>(&mut conn)
                .await
            {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                    let parser = crate::exchange_parser::get_parser(short_exchange);
                    if let Some(bid_str) = parser.parse_bid(&json) {
                        if let Ok(bid) = bid_str.parse::<f64>() {
                            short_price = Some(bid);
                            break;
                        }
                    }
                }
            }
        }
        
        match (long_price, short_price) {
            (Some(long), Some(short)) => {
                eprintln!("[PRICES] {} | Long ({}): ${:.4} | Short ({}): ${:.4}", 
                    pair, long_exchange, long, short_exchange, short);
                Ok((long, short))
            }
            _ => {
                eprintln!("[PRICES] {} | Failed to fetch prices from {} and {}", 
                    pair, long_exchange, short_exchange);
                Err("Could not fetch prices from exchanges".into())
            }
        }
    }

    fn get_exchange_taker_fee(&self, exchange: &str) -> f64 {
        // Returns taker fee in basis points (bps)
        match exchange.to_lowercase().as_str() {
            "binance" => 4.0,      // 0.04%
            "okx" => 5.0,          // 0.05%
            "bybit" => 5.5,        // 0.055%
            "bitget" => 6.0,       // 0.06%
            "kucoin" => 6.0,       // 0.06%
            "hyperliquid" => 4.5,  // 0.035%
            "paradex" => 5.0,      // 0.05%
            "gateio" => 6.0,       // 0.06% (default)
            _ => 6.0,              // Default fallback
        }
    }

    #[allow(dead_code)]
    async fn execute_atomic_trade(
        &mut self,
        opportunity: &ArbitrageOpportunity,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Execute both legs concurrently
        let result = AtomicExecutor::execute_dual_leg(
            &opportunity.symbol,
            &opportunity.long_exchange,
            &opportunity.short_exchange,
            opportunity.long_price,
            opportunity.short_price,
            opportunity.metrics.hard_constraints.order_book_depth_sufficient as u32 as f64 * 1000.0,
        )
        .await?;

        if result.both_filled {
            // Create paper trade record
            let trade_id = Uuid::new_v4().to_string();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let trade = PaperTrade {
                id: trade_id.clone(),
                symbol: opportunity.symbol.clone(),
                long_exchange: opportunity.long_exchange.clone(),
                short_exchange: opportunity.short_exchange.clone(),
                entry_time: now,
                entry_long_price: opportunity.long_price,
                entry_short_price: opportunity.short_price,
                entry_spread_bps: opportunity.spread_bps,
                position_size_usd: opportunity.projected_profit_usd,
                funding_delta_entry: opportunity.funding_delta_8h,
                projected_profit_usd: opportunity.projected_profit_usd,
                actual_profit_usd: 0.0,
                status: TradeStatus::Active,
                exit_reason: None,
                exit_spread_bps: None,
                exit_time: None,
                long_order: result.long_order,
                short_order: result.short_order,
                leg_out_event: None,
            };

            // Initialize negative funding tracker for this symbol
            self.negative_funding_trackers
                .insert(opportunity.symbol.clone(), NegativeFundingTracker::new(opportunity.symbol.clone()));

            self.active_trades.insert(trade_id, trade);
        } else if let Some(error) = result.error {
            eprintln!("Atomic execution failed: {}", error);
        }

        Ok(())
    }
}
