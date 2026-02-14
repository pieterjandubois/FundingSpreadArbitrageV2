// Temporary file to hold the execute_opportunity implementation
// This will be inserted into runner.rs

    /// Execute a single opportunity from the streaming queue.
    ///
    /// This method validates the opportunity and executes the trade if all checks pass.
    /// It replaces the legacy scan_opportunities() method with immediate execution.
    ///
    /// Requirements:
    /// - 3.2.1: Create execute_opportunity() async method
    /// - 3.2.2: Move validation logic from scan_opportunities()
    /// - 3.2.3: Check for duplicate symbols
    /// - 3.2.4: Validate prices are current
    /// - 3.2.5: Check available capital
    /// - 3.2.6: Check exchange balances
    /// - 3.2.7: Calculate position size
    /// - 3.2.8: Execute trade via EntryExecutor
    async fn execute_opportunity(&self, opportunity: ArbitrageOpportunity) {
        eprintln!("[OPPORTUNITY] {} | Spread: {:.2}bps | Confidence: {:.0} | Profit: ${:.2}", 
            opportunity.symbol, opportunity.spread_bps, opportunity.confidence_score, opportunity.projected_profit_usd);
        
        // DECISION 1: Duplicate Symbol Check - Enhanced to prevent position doubling
        // Check for:
        // 1. Active trades on this symbol
        // 2. Trades that are currently exiting (hedging in progress)
        let has_active_or_exiting_trade = self.active_trades.iter().any(|entry| {
            let t = entry.value();
            t.symbol == opportunity.symbol && (t.status == TradeStatus::Active || t.status == TradeStatus::Exiting)
        });
        
        if has_active_or_exiting_trade {
            println!("[SKIPPED] {} - DUPLICATE SYMBOL | Already have active or exiting trade", opportunity.symbol);
            eprintln!("[DEBUG] SKIPPING {} - already have active or exiting trade for this symbol", opportunity.symbol);
            return;
        }
        
        // CRITICAL: Reserve this symbol immediately to prevent race conditions
        // Create a placeholder trade that will be replaced if execution succeeds
        let placeholder_trade_id = format!("placeholder_{}", uuid::Uuid::new_v4());
        let placeholder_trade = PaperTrade {
            id: placeholder_trade_id.clone(),
            symbol: opportunity.symbol.clone(),
            long_exchange: opportunity.long_exchange.clone(),
            short_exchange: opportunity.short_exchange.clone(),
            entry_time: 0,
            entry_long_price: 0.0,
            entry_short_price: 0.0,
            entry_spread_bps: 0.0,
            position_size_usd: 0.0,
            funding_delta_entry: 0.0,
            projected_profit_usd: 0.0,
            actual_profit_usd: 0.0,
            status: TradeStatus::Active,
            exit_reason: Some("PLACEHOLDER - DO NOT USE".to_string()),
            exit_spread_bps: None,
            exit_time: None,
            long_order: SimulatedOrder {
                id: String::new(),
                exchange: String::new(),
                symbol: String::new(),
                side: OrderSide::Long,
                order_type: OrderType::Market,
                price: 0.0,
                size: 0.0,
                queue_position: None,
                created_at: 0,
                filled_at: None,
                fill_price: None,
                status: OrderStatus::Pending,
            },
            short_order: SimulatedOrder {
                id: String::new(),
                exchange: String::new(),
                symbol: String::new(),
                side: OrderSide::Short,
                order_type: OrderType::Market,
                price: 0.0,
                size: 0.0,
                queue_position: None,
                created_at: 0,
                filled_at: None,
                fill_price: None,
                status: OrderStatus::Pending,
            },
            long_exit_order: None,
            short_exit_order: None,
            stop_loss_triggered: false,
            stop_loss_long_price: 0.0,
            stop_loss_short_price: 0.0,
            leg_out_event: None,
        };
        
        // Insert placeholder to reserve the symbol
        self.active_trades.insert(placeholder_trade_id.clone(), placeholder_trade);
        eprintln!("[DEBUG] {} | Symbol reserved with placeholder to prevent race conditions", opportunity.symbol);
        
        // Macro to handle cleanup on early exit
        macro_rules! skip_and_cleanup {
            ($reason:expr) => {{
                self.active_trades.remove(&placeholder_trade_id);
                eprintln!("[DEBUG] Removed placeholder for {} - {}", opportunity.symbol, $reason);
                return;
            }};
        }
        
        // DECISION 2: Fetch Current Prices (Required for all subsequent checks)
        let (current_long_price, current_short_price) = match self.get_current_prices_for_opportunity(&opportunity.symbol, &opportunity.long_exchange, &opportunity.short_exchange) {
            Ok((long, short)) => (long, short),
            Err(_) => {
                println!("[SKIPPED] {} - NO PRICES | Could not fetch current prices from market data store", opportunity.symbol);
                eprintln!("[DEBUG] Failed to fetch current prices for {}", opportunity.symbol);
                skip_and_cleanup!("no prices");
            }
        };
        
        // DECISION 3: Re-validate Hard Constraint 1 - Order Book Depth
        let order_book_depth_sufficient = opportunity.order_book_depth_long >= opportunity.metrics.hard_constraints.order_book_depth_sufficient as u32 as f64 * 2.0
            && opportunity.order_book_depth_short >= opportunity.metrics.hard_constraints.order_book_depth_sufficient as u32 as f64 * 2.0;
        
        if !order_book_depth_sufficient {
            println!("[SKIPPED] {} - INSUFFICIENT DEPTH | Long: ${:.0} | Short: ${:.0}", 
                opportunity.symbol, opportunity.order_book_depth_long, opportunity.order_book_depth_short);
            eprintln!("[DEBUG] SKIPPING {} - order book depth insufficient", opportunity.symbol);
            skip_and_cleanup!("insufficient depth");
        }
        
        // DECISION 4: Re-validate Hard Constraint 2 - Exchange Latency
        if !opportunity.metrics.hard_constraints.exchange_latency_ok {
            println!("[SKIPPED] {} - HIGH LATENCY | Exchange latency > 200ms", opportunity.symbol);
            eprintln!("[DEBUG] SKIPPING {} - exchange latency too high", opportunity.symbol);
            skip_and_cleanup!("high latency");
        }
        
        // DECISION 5: Re-validate Hard Constraint 3 - Funding Delta Substantial
        if !opportunity.metrics.hard_constraints.funding_delta_substantial {
            println!("[SKIPPED] {} - INSUFFICIENT FUNDING DELTA | Delta: {:.6}", 
                opportunity.symbol, opportunity.metrics.funding_delta);
            eprintln!("[DEBUG] SKIPPING {} - funding delta not substantial", opportunity.symbol);
            skip_and_cleanup!("insufficient funding delta");
        }
        
        // DECISION 6: Re-calculate Spread with Fresh Prices
        let current_spread_bps = OpportunityScanner::calculate_spread_bps(current_long_price, current_short_price);
        eprintln!("[DEBUG] {} | Opportunity spread: {:.2}bps | Current spread: {:.2}bps", 
            opportunity.symbol, opportunity.spread_bps, current_spread_bps);
        
        // DECISION 7: Spread Must Be Positive
        if current_spread_bps <= 0.0 {
            println!("[SKIPPED] {} - NEGATIVE SPREAD | Current: {:.2}bps", opportunity.symbol, current_spread_bps);
            eprintln!("[DEBUG] SKIPPING {} - spread is negative or zero", opportunity.symbol);
            skip_and_cleanup!("negative spread");
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
            skip_and_cleanup!("unprofitable");
        }
        
        // DECISION 10: Check Available Capital
        let available_capital = self.portfolio_manager.read().await.get_available_capital().await;
        if available_capital <= 0.0 {
            println!("[SKIPPED] {} - NO CAPITAL | Available: ${:.2}", opportunity.symbol, available_capital);
            eprintln!("[DEBUG] No available capital, skipping trade for {}", opportunity.symbol);
            skip_and_cleanup!("no capital");
        }
        
        // DECISION 10.5: Check BOTH exchanges have sufficient balance (for real trading)
        let backend_name = self.execution_backend.backend_name();
        let is_real_trading = backend_name == "Demo" || backend_name == "Live";
        
        if is_real_trading {
            let testnet_backend = self.execution_backend.clone();
            let is_single_exchange_mode = testnet_backend.backend_name() == "Demo";
            
            if is_single_exchange_mode {
                // Single-exchange mode: only check primary exchange (bybit)
                let primary_balance = match self.execution_backend.get_available_balance("bybit").await {
                    Ok(balance) => balance,
                    Err(e) => {
                        eprintln!("[BALANCE CHECK] Failed to fetch bybit balance: {}", e);
                        skip_and_cleanup!("balance check failed");
                    }
                };
                
                eprintln!("[BALANCE CHECK] Single-exchange mode | bybit balance: ${:.2}", primary_balance);
                
                if primary_balance < 10.0 {
                    println!("[SKIPPED] {} - INSUFFICIENT EXCHANGE BALANCE | Bybit: ${:.2}", 
                        opportunity.symbol, primary_balance);
                    eprintln!("[DEBUG] SKIPPING {} - bybit balance too low: ${:.2}", 
                        opportunity.symbol, primary_balance);
                    skip_and_cleanup!("insufficient exchange balance");
                }
            } else {
                // Normal mode: check both exchanges
                let long_balance = match self.execution_backend.get_available_balance(&opportunity.long_exchange).await {
                    Ok(balance) => balance,
                    Err(e) => {
                        eprintln!("[BALANCE CHECK] Failed to fetch {} balance: {}", opportunity.long_exchange, e);
                        skip_and_cleanup!("balance check failed");
                    }
                };
                
                let short_balance = match self.execution_backend.get_available_balance(&opportunity.short_exchange).await {
                    Ok(balance) => balance,
                    Err(e) => {
                        eprintln!("[BALANCE CHECK] Failed to fetch {} balance: {}", opportunity.short_exchange, e);
                        skip_and_cleanup!("balance check failed");
                    }
                };
                
                eprintln!("[BALANCE CHECK] {} balance: ${:.2} | {} balance: ${:.2}", 
                    opportunity.long_exchange, long_balance, opportunity.short_exchange, short_balance);
                
                let min_exchange_balance = long_balance.min(short_balance);
                
                if min_exchange_balance < 10.0 {
                    println!("[SKIPPED] {} - INSUFFICIENT EXCHANGE BALANCE | Min: ${:.2}", 
                        opportunity.symbol, min_exchange_balance);
                    eprintln!("[DEBUG] SKIPPING {} - minimum exchange balance too low: ${:.2}", 
                        opportunity.symbol, min_exchange_balance);
                    skip_and_cleanup!("insufficient exchange balance");
                }
            }
        }
        
        // DECISION 11: Calculate Position Size
        let starting_capital = self.portfolio_manager.read().await.get_starting_capital().await;
        let position_size = EntryExecutor::calculate_position_size(
            current_spread_bps,
            available_capital,
            total_fee_bps,
            10.0,  // funding cost estimate
            starting_capital,
        );
        
        // DECISION 12: Position Size Must Be Valid
        if position_size <= 0.0 {
            println!("[SKIPPED] {} - INVALID POSITION SIZE | Calculated: ${:.2}", opportunity.symbol, position_size);
            eprintln!("[DEBUG] SKIPPING {} - position size is invalid", opportunity.symbol);
            skip_and_cleanup!("invalid position size");
        }
        
        // DECISION 13: Position Size Must Not Exceed Available Capital
        if position_size > available_capital {
            println!("[SKIPPED] {} - INSUFFICIENT CAPITAL | Available: ${:.2} | Required: ${:.2}", 
                opportunity.symbol, available_capital, position_size);
            eprintln!("[DEBUG] SKIPPING {} - position size exceeds available capital", opportunity.symbol);
            skip_and_cleanup!("position size exceeds capital");
        }
        
        // DECISION 14: Re-calculate Confidence Score
        let confidence_score = if opportunity.metrics.hard_constraints.passes_all() {
            opportunity.confidence_score
        } else {
            0
        };
        
        // DECISION 15: Confidence Must Meet Threshold
        if confidence_score < 70 {
            println!("[SKIPPED] {} - LOW CONFIDENCE | Score: {:.0}", opportunity.symbol, confidence_score);
            eprintln!("[DEBUG] SKIPPING {} - confidence score too low: {:.0}", opportunity.symbol, confidence_score);
            skip_and_cleanup!("low confidence");
        }
        
        // ALL DECISIONS PASSED - EXECUTE TRADE
        eprintln!("[DEBUG] {} PASSED ALL CHECKS | Spread: {:.2}bps | Net Profit: {:.2}bps | Position: ${:.2}", 
            opportunity.symbol, current_spread_bps, net_profit_bps, position_size);
        
        // DECISION 16: Validate symbol is tradeable on both exchanges (for real trading only)
        if is_real_trading {
            let long_tradeable = match self.execution_backend.is_symbol_tradeable(&opportunity.long_exchange, &opportunity.symbol).await {
                Ok(tradeable) => tradeable,
                Err(e) => {
                    eprintln!("[DEBUG] Error checking if {} is tradeable on {}: {}", opportunity.symbol, opportunity.long_exchange, e);
                    false
                }
            };
            
            let short_tradeable = match self.execution_backend.is_symbol_tradeable(&opportunity.short_exchange, &opportunity.symbol).await {
                Ok(tradeable) => tradeable,
                Err(e) => {
                    eprintln!("[DEBUG] Error checking if {} is tradeable on {}: {}", opportunity.symbol, opportunity.short_exchange, e);
                    false
                }
            };
            
            if !long_tradeable {
                println!("[SKIPPED] {} - NOT TRADEABLE | {} does not support this symbol", opportunity.symbol, opportunity.long_exchange);
                eprintln!("[DEBUG] SKIPPING {} - not tradeable on {}", opportunity.symbol, opportunity.long_exchange);
                skip_and_cleanup!("not tradeable on long exchange");
            }
            
            if !short_tradeable {
                println!("[SKIPPED] {} - NOT TRADEABLE | {} does not support this symbol", opportunity.symbol, opportunity.short_exchange);
                eprintln!("[DEBUG] SKIPPING {} - not tradeable on {}", opportunity.symbol, opportunity.short_exchange);
                skip_and_cleanup!("not tradeable on short exchange");
            }
            
            eprintln!("[DEBUG] {} is tradeable on both {} and {}", opportunity.symbol, opportunity.long_exchange, opportunity.short_exchange);
        }
        
        // Wrap trade execution with timeout to prevent freezes
        const TRADE_TIMEOUT_SECS: u64 = 15;
        let trade_execution = async {
            if is_real_trading {
                // Use REAL order placement with ExecutionBackend
                eprintln!("[EXECUTION] Using REAL order placement via {} backend", backend_name);
                EntryExecutor::execute_atomic_entry_real(
                    &opportunity, 
                    available_capital, 
                    position_size,
                    self.execution_backend.clone()
                ).await
            } else {
                // Use simulated order placement for paper trading
                eprintln!("[EXECUTION] Using SIMULATED order placement for paper trading");
                EntryExecutor::execute_atomic_entry(&opportunity, available_capital, position_size)
            }
        };
        
        let trade_result = match tokio::time::timeout(
            Duration::from_secs(TRADE_TIMEOUT_SECS),
            trade_execution
        ).await {
            Ok(result) => result,
            Err(_) => {
                println!("[SKIPPED] {} - EXECUTION TIMEOUT | Trade took longer than {}s", 
                    opportunity.symbol, TRADE_TIMEOUT_SECS);
                eprintln!("[TIMEOUT] Trade execution for {} exceeded {}s timeout", 
                    opportunity.symbol, TRADE_TIMEOUT_SECS);
                self.active_trades.remove(&placeholder_trade_id);
                return;
            }
        };
        
        match trade_result {
            Ok(mut trade) => {
                // Recalculate projected profit based on actual position size and current spread
                let projected_profit_bps = (current_spread_bps * 0.9) - total_fee_bps;
                trade.projected_profit_usd = ((projected_profit_bps) / 10000.0) * position_size;
                
                println!(
                    "[ENTRY] Executed trade {} on {} | Size: ${:.2} | Spread: {:.2}bps | Projected Profit: ${:.2}",
                    trade.id, trade.symbol, trade.position_size_usd, current_spread_bps, trade.projected_profit_usd
                );
                
                // Remove placeholder and add real trade
                self.active_trades.remove(&placeholder_trade_id);
                self.active_trades.insert(trade.id.clone(), trade.clone());
                
                // Update portfolio state
                if let Err(e) = self.portfolio_manager.write().await.open_trade(trade).await {
                    eprintln!("Error opening trade in portfolio: {}", e);
                } else {
                    let pm = self.portfolio_manager.read().await;
                    let state = pm.get_state().await;
                    eprintln!("[PORTFOLIO] After trade: Available Capital: ${:.2} | Total Open: ${:.2} | Active Trades: {}",
                        state.available_capital, state.total_open_positions, state.active_trades.len());
                }
            }
            Err(e) => {
                println!("[SKIPPED] {} - ATOMIC EXECUTION FAILED | Reason: {}", opportunity.symbol, e);
                eprintln!("Error executing entry for {}: {}", opportunity.symbol, e);
                // Remove placeholder since trade failed
                self.active_trades.remove(&placeholder_trade_id);
            }
        }
    }
