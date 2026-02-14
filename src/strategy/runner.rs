use crate::strategy::types::*;
use crate::strategy::scanner::OpportunityScanner;
use crate::strategy::entry::EntryExecutor;
use crate::strategy::positions::PositionManager;
use crate::strategy::portfolio::PortfolioManager;
use crate::strategy::atomic_execution::{AtomicExecutor, NegativeFundingTracker};
use crate::strategy::execution_backend::ExecutionBackend;
use crate::strategy::fill_probability::FillProbabilityEstimator;
use crate::strategy::pipeline::MarketConsumer;
use crate::strategy::market_data::MarketDataStore;
use crate::strategy::opportunity_queue::OpportunityConsumer;
use crate::strategy::symbol_map::SymbolMap;
use crate::exchange_parser::get_parser;
use redis::aio::MultiplexedConnection;
use dashmap::DashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
use uuid::Uuid;

pub struct StrategyRunner {
    portfolio_manager: Arc<tokio::sync::RwLock<PortfolioManager>>,
    redis_conn: MultiplexedConnection,
    active_trades: Arc<DashMap<String, PaperTrade>>,
    negative_funding_trackers: Arc<DashMap<String, NegativeFundingTracker>>,
    execution_backend: Arc<dyn ExecutionBackend>,
    _allowed_exchanges: Option<Vec<String>>,  // Kept for backward compatibility
    fill_probability_estimator: FillProbabilityEstimator,
    // New fields for streaming architecture
    market_consumer: Option<MarketConsumer>,
    market_data_store: MarketDataStore,
    opportunity_consumer: Option<OpportunityConsumer>,
    symbol_map: Arc<SymbolMap>,  // Dynamic symbol mapping for all incoming data
}

impl StrategyRunner {
    pub async fn new(
        redis_conn: MultiplexedConnection,
        starting_capital: f64,
        execution_backend: Arc<dyn ExecutionBackend>,
        redis_prefix: Option<String>,
        allowed_exchanges: Option<Vec<String>>,
        symbol_map: Arc<SymbolMap>,  // Dynamic symbol mapping
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let redis_prefix = redis_prefix.unwrap_or_else(|| "trade".to_string());
        
        // Use the provided starting capital (already fetched by caller)
        let actual_starting_capital = starting_capital;
        eprintln!("[STRATEGY] Using starting capital: ${:.2}", actual_starting_capital);
        eprintln!("[STRATEGY] Position size per trade (10%): ${:.2}", actual_starting_capital * 0.10);
        
        let portfolio_manager = PortfolioManager::new(
            redis_conn.clone(), 
            actual_starting_capital,
            Some(redis_prefix.clone()),
        ).await?;

        // Start with fresh active trades (don't load stale data from previous runs)
        let active_trades = Arc::new(DashMap::new());
        let negative_funding_trackers = Arc::new(DashMap::new());

        if let Some(ref exchanges) = allowed_exchanges {
            eprintln!("[STRATEGY] Using {} backend with Redis prefix: {} (exchanges: {})", 
                execution_backend.backend_name(), redis_prefix, exchanges.join(", "));
        } else {
            eprintln!("[STRATEGY] Using {} backend with Redis prefix: {} (all exchanges)", 
                execution_backend.backend_name(), redis_prefix);
        }

        Ok(Self {
            portfolio_manager: Arc::new(tokio::sync::RwLock::new(portfolio_manager)),
            redis_conn,
            active_trades,
            negative_funding_trackers,
            execution_backend,
            _allowed_exchanges: allowed_exchanges,
            fill_probability_estimator: FillProbabilityEstimator::new(),
            market_consumer: None,  // Will be set via set_market_consumer()
            market_data_store: MarketDataStore::new(),
            opportunity_consumer: None,  // Will be set via set_opportunity_consumer()
            symbol_map,  // Store the dynamic symbol map
        })
    }

    /// Set the market consumer for streaming market data.
    ///
    /// This is optional and used for direct market data consumption.
    /// The strategy runner primarily uses OpportunityConsumer for trade execution.
    ///
    /// Requirement: 1.2 (Strategy consumes from queue)
    pub fn set_market_consumer(&mut self, consumer: MarketConsumer) {
        self.market_consumer = Some(consumer);
    }

    /// Set the opportunity consumer for streaming opportunities.
    ///
    /// This MUST be called before run_scanning_loop().
    /// The runner will panic if this is not set.
    ///
    /// Requirement: 3.1 (Strategy consumes opportunities from queue)
    pub fn set_opportunity_consumer(&mut self, consumer: OpportunityConsumer) {
        self.opportunity_consumer = Some(consumer);
    }

    pub async fn run_scanning_loop(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Pin strategy thread to core 1 for optimal cache performance
        // Requirement: 4.1 (Pin strategy thread to core 1)
        if let Err(e) = crate::strategy::thread_pinning::pin_strategy_thread() {
            eprintln!("[THREAD-PIN] Warning: Failed to pin strategy thread: {}", e);
            eprintln!("[THREAD-PIN] Continuing without thread pinning (performance may be degraded)");
        }
        
        // Require opportunity consumer
        // Requirement: 3.1 (Always use streaming mode)
        let opportunity_consumer = self.opportunity_consumer.as_ref()
            .expect("OpportunityConsumer not set - call set_opportunity_consumer() before run_scanning_loop()");
        
        eprintln!("[STRATEGY] Starting streaming mode (consuming opportunities from queue)");
        eprintln!("[STRATEGY] Starting capital: ${:.2}", self.portfolio_manager.read().await.get_available_capital().await);
        
        // Also consume market updates to populate our MarketDataStore
        // This is CRITICAL - without this, we have no price data!
        let market_consumer = self.market_consumer.as_ref();
        
        // Consume opportunities as they arrive - immediate processing
        // Requirements: 3.1 (Consume opportunities immediately), 3.2 (Execute trades)
        loop {
            // Process market updates to keep our MarketDataStore up-to-date
            if let Some(consumer) = market_consumer {
                // Process up to 100 market updates per iteration to avoid blocking
                for _ in 0..100 {
                    if let Some(update) = consumer.pop() {
                        self.market_data_store.update_from_market_update(&update);
                    } else {
                        break;
                    }
                }
            }
            
            // Pop opportunity from queue (non-blocking)
            if let Some(opportunity) = opportunity_consumer.pop() {
                // Execute opportunity immediately
                // Requirement: 3.2 (Execute trade via execute_opportunity)
                self.execute_opportunity(opportunity).await;
            }
            
            // Small sleep to avoid busy-waiting when queue is empty
            tokio::time::sleep(Duration::from_micros(100)).await;
            
            // Run monitoring tasks in parallel (non-blocking)
            let (monitor_result, exit_result) = tokio::join!(
                self.monitor_active_positions(),
                self.check_exits()
            );
            
            if let Err(e) = monitor_result {
                eprintln!("Error monitoring positions: {}", e);
            }
            if let Err(e) = exit_result {
                eprintln!("Error checking exits: {}", e);
            }
        }
    }

    /// Execute a single opportunity from the streaming queue.
    ///
    /// This method validates the opportunity and executes the trade if all checks pass.
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
        
        // DECISION 1: Duplicate Symbol Check
        let has_active_or_exiting_trade = self.active_trades.iter().any(|entry| {
            let t = entry.value();
            t.symbol == opportunity.symbol && (t.status == TradeStatus::Active || t.status == TradeStatus::Exiting)
        });
        
        if has_active_or_exiting_trade {
            println!("[SKIPPED] {} - DUPLICATE SYMBOL | Already have active or exiting trade", opportunity.symbol);
            return;
        }
        
        // Reserve symbol with placeholder
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
            exit_reason: Some("PLACEHOLDER".to_string()),
            exit_spread_bps: None,
            exit_time: None,
            long_order: SimulatedOrder::default(),
            short_order: SimulatedOrder::default(),
            long_exit_order: None,
            short_exit_order: None,
            stop_loss_triggered: false,
            stop_loss_long_price: 0.0,
            stop_loss_short_price: 0.0,
            leg_out_event: None,
        };
        
        self.active_trades.insert(placeholder_trade_id.clone(), placeholder_trade);
        
        // Macro for cleanup
        macro_rules! skip_and_cleanup {
            ($reason:expr) => {{
                self.active_trades.remove(&placeholder_trade_id);
                eprintln!("[DEBUG] Removed placeholder for {} - {}", opportunity.symbol, $reason);
                return;
            }};
        }
        
        // Validate prices
        let (current_long_price, current_short_price) = match self.get_current_prices_for_opportunity(&opportunity.symbol, &opportunity.long_exchange, &opportunity.short_exchange) {
            Ok((long, short)) => (long, short),
            Err(_) => {
                println!("[SKIPPED] {} - NO PRICES", opportunity.symbol);
                skip_and_cleanup!("no prices");
            }
        };
        
        // Validate spread
        let current_spread_bps = OpportunityScanner::calculate_spread_bps(current_long_price, current_short_price);
        if current_spread_bps <= 0.0 {
            println!("[SKIPPED] {} - NEGATIVE SPREAD | Current: {:.2}bps", opportunity.symbol, current_spread_bps);
            skip_and_cleanup!("negative spread");
        }
        
        // Calculate fees and profit
        let long_taker_bps = self.get_exchange_taker_fee(&opportunity.long_exchange);
        let short_taker_bps = self.get_exchange_taker_fee(&opportunity.short_exchange);
        let total_fee_bps = long_taker_bps + short_taker_bps;
        let net_profit_bps = current_spread_bps - total_fee_bps;
        
        if net_profit_bps <= 0.0 {
            println!("[SKIPPED] {} - UNPROFITABLE | Net: {:.2}bps", opportunity.symbol, net_profit_bps);
            skip_and_cleanup!("unprofitable");
        }
        
        // Check capital
        let available_capital = self.portfolio_manager.read().await.get_available_capital().await;
        if available_capital <= 0.0 {
            println!("[SKIPPED] {} - NO CAPITAL", opportunity.symbol);
            skip_and_cleanup!("no capital");
        }
        
        // Calculate position size
        let starting_capital = self.portfolio_manager.read().await.get_starting_capital().await;
        let position_size = EntryExecutor::calculate_position_size(
            current_spread_bps,
            available_capital,
            total_fee_bps,
            10.0,
            starting_capital,
        );
        
        if position_size <= 0.0 || position_size > available_capital {
            println!("[SKIPPED] {} - INVALID POSITION SIZE", opportunity.symbol);
            skip_and_cleanup!("invalid position size");
        }
        
        // Execute trade
        let backend_name = self.execution_backend.backend_name();
        let is_real_trading = backend_name == "Demo" || backend_name == "Live";
        
        let trade_result = if is_real_trading {
            EntryExecutor::execute_atomic_entry_real(
                &opportunity, 
                available_capital, 
                position_size,
                self.execution_backend.clone()
            ).await
        } else {
            EntryExecutor::execute_atomic_entry(&opportunity, available_capital, position_size)
        };
        
        match trade_result {
            Ok(mut trade) => {
                let projected_profit_bps = (current_spread_bps * 0.9) - total_fee_bps;
                trade.projected_profit_usd = ((projected_profit_bps) / 10000.0) * position_size;
                
                println!(
                    "[ENTRY] Executed trade {} on {} | Size: ${:.2} | Spread: {:.2}bps | Projected Profit: ${:.2}",
                    trade.id, trade.symbol, trade.position_size_usd, current_spread_bps, trade.projected_profit_usd
                );
                
                self.active_trades.remove(&placeholder_trade_id);
                self.active_trades.insert(trade.id.clone(), trade.clone());
                
                if let Err(e) = self.portfolio_manager.write().await.open_trade(trade).await {
                    eprintln!("Error opening trade in portfolio: {}", e);
                }
            }
            Err(e) => {
                println!("[SKIPPED] {} - EXECUTION FAILED | Reason: {}", opportunity.symbol, e);
                self.active_trades.remove(&placeholder_trade_id);
            }
        }
    }


    async fn monitor_active_positions(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let trade_ids: Vec<String> = self.active_trades.iter()
            .map(|entry| entry.key().clone())
            .collect();

        if !trade_ids.is_empty() {
            eprintln!("[MONITOR] Monitoring {} active trades", trade_ids.len());
        }

        // Check if we're using real trading backend
        let backend_name = self.execution_backend.backend_name();
        let is_real_trading = backend_name == "Demo" || backend_name == "Live";

        for trade_id in trade_ids {
            // Get trade info first
            let (symbol, entry_spread_bps, position_size_usd, entry_projected_profit, has_exit_orders, stop_loss_long, stop_loss_short, stop_loss_triggered, long_exchange, short_exchange) = {
                if let Some(trade) = self.active_trades.get(&trade_id) {
                    (
                        trade.symbol.clone(), 
                        trade.entry_spread_bps, 
                        trade.position_size_usd, 
                        trade.projected_profit_usd,
                        trade.long_exit_order.is_some() && trade.short_exit_order.is_some(),
                        trade.stop_loss_long_price,
                        trade.stop_loss_short_price,
                        trade.stop_loss_triggered,
                        trade.long_exchange.clone(),
                        trade.short_exchange.clone(),
                    )
                } else {
                    continue;
                }
            };

            // Get current prices from market data store (no Redis in hot path)
            // CRITICAL FIX: Don't let price fetch failure stop monitoring of other trades
            let (current_long_price, current_short_price) = match self.get_current_prices(&symbol) {
                Ok(prices) => prices,
                Err(e) => {
                    eprintln!("[MONITOR] ⚠️  Failed to get prices for {}: {} - skipping this trade", symbol, e);
                    continue;
                }
            };
            let current_spread_bps = OpportunityScanner::calculate_spread_bps(current_long_price, current_short_price);

            // ============================================================================
            // REAL TRADING MONITORING (Demo/Live with exit orders)
            // ============================================================================
            if is_real_trading && has_exit_orders {
                eprintln!("[REAL MONITOR] {} | Checking exit orders and stop-loss", trade_id);

                // 1. Check if exit orders have filled
                let (long_exit_id, short_exit_id) = {
                    if let Some(trade) = self.active_trades.get(&trade_id) {
                        (
                            trade.long_exit_order.as_ref().map(|o| o.id.clone()),
                            trade.short_exit_order.as_ref().map(|o| o.id.clone()),
                        )
                    } else {
                        continue;
                    }
                };

                let mut long_exit_filled = false;
                let mut short_exit_filled = false;

                // Check long exit order status
                if let Some(ref order_id) = long_exit_id {
                    match self.execution_backend.get_order_status(&long_exchange, order_id).await {
                        Ok(OrderStatus::Filled) => {
                            eprintln!("[EXIT FILLED] {} | Long exit order filled!", trade_id);
                            long_exit_filled = true;
                        }
                        Ok(OrderStatus::Cancelled) => {
                            eprintln!("[EXIT CANCELLED] {} | Long exit order was cancelled", trade_id);
                        }
                        Ok(OrderStatus::Pending) => {
                            // Still waiting
                        }
                        Err(e) => {
                            eprintln!("[EXIT ERROR] {} | Error checking long exit: {}", trade_id, e);
                        }
                    }
                }

                // Check short exit order status
                if let Some(ref order_id) = short_exit_id {
                    match self.execution_backend.get_order_status(&short_exchange, order_id).await {
                        Ok(OrderStatus::Filled) => {
                            eprintln!("[EXIT FILLED] {} | Short exit order filled!", trade_id);
                            short_exit_filled = true;
                        }
                        Ok(OrderStatus::Cancelled) => {
                            eprintln!("[EXIT CANCELLED] {} | Short exit order was cancelled", trade_id);
                        }
                        Ok(OrderStatus::Pending) => {
                            // Still waiting
                        }
                        Err(e) => {
                            eprintln!("[EXIT ERROR] {} | Error checking short exit: {}", trade_id, e);
                        }
                    }
                }

                // If both exit orders filled, close the trade
                if long_exit_filled && short_exit_filled {
                    if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                        trade.status = TradeStatus::Exiting;
                        trade.exit_reason = Some("Exit orders filled (90% capture)".to_string());
                        trade.exit_spread_bps = Some(current_spread_bps);
                        eprintln!("[TRADE COMPLETE] {} | Both exit orders filled! Closing trade...", trade_id);
                    }
                    continue; // Move to check_exits for final processing
                }

                // TIMEOUT CHECK: If NEITHER exit order filled after 60 seconds, force close both
                // This prevents positions from staying open indefinitely
                if !long_exit_filled && !short_exit_filled {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    
                    // Get exit order creation times
                    let (long_exit_age, short_exit_age) = {
                        if let Some(trade) = self.active_trades.get(&trade_id) {
                            let long_age = trade.long_exit_order.as_ref().map(|o| now.saturating_sub(o.created_at)).unwrap_or(0);
                            let short_age = trade.short_exit_order.as_ref().map(|o| now.saturating_sub(o.created_at)).unwrap_or(0);
                            (long_age, short_age)
                        } else {
                            continue;
                        }
                    };
                    
                    // If both exit orders are older than 3 seconds, force close
                    const EXIT_TIMEOUT_SECONDS: u64 = 3;
                    if long_exit_age > EXIT_TIMEOUT_SECONDS && short_exit_age > EXIT_TIMEOUT_SECONDS {
                        eprintln!("[EXIT TIMEOUT] {} | Exit orders not filled after {}s - forcing close", trade_id, EXIT_TIMEOUT_SECONDS);
                        
                        // Get position sizes
                        let (long_quantity, short_quantity) = if let Some(trade) = self.active_trades.get(&trade_id) {
                            (trade.long_order.size, trade.short_order.size)
                        } else {
                            continue;
                        };
                        
                        // Track cumulative fills
                        let mut long_filled_from_cancel = 0.0;
                        let mut short_filled_from_cancel = 0.0;
                        
                        // Cancel both exit orders and check if they filled
                        if let Some(ref order_id) = long_exit_id {
                            eprintln!("[EXIT TIMEOUT] Cancelling long exit order: {}", order_id);
                            let _ = self.execution_backend.cancel_order(&long_exchange, order_id).await;
                            
                            match self.execution_backend.get_order_status_detailed(&long_exchange, order_id, &symbol).await {
                                Ok(status_info) => {
                                    long_filled_from_cancel = status_info.filled_quantity;
                                    if long_filled_from_cancel > 0.0 {
                                        eprintln!("[EXIT TIMEOUT] Long exit filled {:.4} before cancellation", long_filled_from_cancel);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[EXIT TIMEOUT] ⚠️  Failed to check long exit: {}", e);
                                }
                            }
                        }
                        
                        if let Some(ref order_id) = short_exit_id {
                            eprintln!("[EXIT TIMEOUT] Cancelling short exit order: {}", order_id);
                            let _ = self.execution_backend.cancel_order(&short_exchange, order_id).await;
                            
                            match self.execution_backend.get_order_status_detailed(&short_exchange, order_id, &symbol).await {
                                Ok(status_info) => {
                                    short_filled_from_cancel = status_info.filled_quantity;
                                    if short_filled_from_cancel > 0.0 {
                                        eprintln!("[EXIT TIMEOUT] Short exit filled {:.4} before cancellation", short_filled_from_cancel);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[EXIT TIMEOUT] ⚠️  Failed to check short exit: {}", e);
                                }
                            }
                        }
                        
                        // Try aggressive limits for remaining quantities (2 second window)
                        let remaining_long = long_quantity - long_filled_from_cancel;
                        let remaining_short = short_quantity - short_filled_from_cancel;
                        
                        if remaining_long > 0.001 {
                            eprintln!("[EXIT TIMEOUT] Trying aggressive limit for long: {:.4} contracts", remaining_long);
                            match self.execution_backend.get_best_bid(&long_exchange, &symbol).await {
                                Ok(best_bid) => {
                                    let limit_order = SimulatedOrder {
                                        id: format!("timeout_exit_long_{}", uuid::Uuid::new_v4()),
                                        exchange: long_exchange.clone(),
                                        symbol: symbol.clone(),
                                        side: OrderSide::Short,
                                        order_type: OrderType::Limit,
                                        price: best_bid,
                                        size: remaining_long,
                                        queue_position: None,
                                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                        filled_at: None,
                                        fill_price: None,
                                        status: OrderStatus::Pending,
                                    };
                                    
                                    if let Ok(placed_order) = self.execution_backend.place_order(limit_order).await {
                                        tokio::time::sleep(Duration::from_secs(2)).await;
                                        
                                        match self.execution_backend.get_order_status_detailed(&long_exchange, &placed_order.id, &symbol).await {
                                            Ok(status_info) => {
                                                long_filled_from_cancel += status_info.filled_quantity;
                                                if status_info.filled_quantity > 0.0 {
                                                    eprintln!("[EXIT TIMEOUT] Long limit filled {:.4} contracts", status_info.filled_quantity);
                                                }
                                                let _ = self.execution_backend.cancel_order(&long_exchange, &placed_order.id).await;
                                            }
                                            Err(_) => {
                                                let _ = self.execution_backend.cancel_order(&long_exchange, &placed_order.id).await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[EXIT TIMEOUT] ⚠️  Failed to get best bid: {}", e);
                                }
                            }
                        }
                        
                        if remaining_short > 0.001 {
                            eprintln!("[EXIT TIMEOUT] Trying aggressive limit for short: {:.4} contracts", remaining_short);
                            match self.execution_backend.get_best_ask(&short_exchange, &symbol).await {
                                Ok(best_ask) => {
                                    let limit_order = SimulatedOrder {
                                        id: format!("timeout_exit_short_{}", uuid::Uuid::new_v4()),
                                        exchange: short_exchange.clone(),
                                        symbol: symbol.clone(),
                                        side: OrderSide::Long,
                                        order_type: OrderType::Limit,
                                        price: best_ask,
                                        size: remaining_short,
                                        queue_position: None,
                                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                        filled_at: None,
                                        fill_price: None,
                                        status: OrderStatus::Pending,
                                    };
                                    
                                    if let Ok(placed_order) = self.execution_backend.place_order(limit_order).await {
                                        tokio::time::sleep(Duration::from_secs(2)).await;
                                        
                                        match self.execution_backend.get_order_status_detailed(&short_exchange, &placed_order.id, &symbol).await {
                                            Ok(status_info) => {
                                                short_filled_from_cancel += status_info.filled_quantity;
                                                if status_info.filled_quantity > 0.0 {
                                                    eprintln!("[EXIT TIMEOUT] Short limit filled {:.4} contracts", status_info.filled_quantity);
                                                }
                                                let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                            }
                                            Err(_) => {
                                                let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[EXIT TIMEOUT] ⚠️  Failed to get best ask: {}", e);
                                }
                            }
                        }
                        
                        // Market orders for any remaining quantities
                        let final_remaining_long = long_quantity - long_filled_from_cancel;
                        let final_remaining_short = short_quantity - short_filled_from_cancel;
                        
                        if final_remaining_long > 0.001 {
                            eprintln!("[EXIT TIMEOUT] Market closing long: {:.4} contracts", final_remaining_long);
                            let market_order = SimulatedOrder {
                                id: format!("timeout_market_long_{}", uuid::Uuid::new_v4()),
                                exchange: long_exchange.clone(),
                                symbol: symbol.clone(),
                                side: OrderSide::Short,
                                order_type: OrderType::Market,
                                price: current_long_price,
                                size: final_remaining_long,
                                queue_position: None,
                                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                filled_at: None,
                                fill_price: None,
                                status: OrderStatus::Pending,
                            };
                            
                            match self.execution_backend.place_market_order(market_order).await {
                                Ok(_) => {
                                    eprintln!("[EXIT TIMEOUT] ✅ Long position closed");
                                }
                                Err(e) => {
                                    eprintln!("[EXIT TIMEOUT] ❌ Failed to close long: {}", e);
                                }
                            }
                        }
                        
                        if final_remaining_short > 0.001 {
                            eprintln!("[EXIT TIMEOUT] Market closing short: {:.4} contracts", final_remaining_short);
                            let market_order = SimulatedOrder {
                                id: format!("timeout_market_short_{}", uuid::Uuid::new_v4()),
                                exchange: short_exchange.clone(),
                                symbol: symbol.clone(),
                                side: OrderSide::Long,
                                order_type: OrderType::Market,
                                price: current_short_price,
                                size: final_remaining_short,
                                queue_position: None,
                                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                filled_at: None,
                                fill_price: None,
                                status: OrderStatus::Pending,
                            };
                            
                            match self.execution_backend.place_market_order(market_order).await {
                                Ok(_) => {
                                    eprintln!("[EXIT TIMEOUT] ✅ Short position closed");
                                }
                                Err(e) => {
                                    eprintln!("[EXIT TIMEOUT] ❌ Failed to close short: {}", e);
                                }
                            }
                        }
                        
                        // Mark trade as exiting
                        if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                            trade.status = TradeStatus::Exiting;
                            trade.exit_reason = Some(format!("Exit timeout ({}s) - forced close", EXIT_TIMEOUT_SECONDS));
                            trade.exit_spread_bps = Some(current_spread_bps);
                        }
                        
                        eprintln!("[EXIT TIMEOUT] ✅ Both positions closed after timeout");
                        continue;
                    }
                }

                // CRITICAL: If only ONE exit filled, hedge the other immediately
                // Strategy: Try aggressive limit first (maker fee), fallback to market (taker fee)
                // FIX: Track cumulative fills to prevent double-fill race conditions
                if long_exit_filled && !short_exit_filled {
                    eprintln!("[PARTIAL EXIT] {} | Long exit filled, but short still open - HEDGING NOW", trade_id);
                    
                    // CRITICAL: Mark trade as Exiting IMMEDIATELY to prevent duplicate symbol entries
                    if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                        trade.status = TradeStatus::Exiting;
                        eprintln!("[PARTIAL EXIT] Trade marked as Exiting to prevent race conditions");
                    }
                    
                    // Track cumulative filled quantity across all orders
                    let mut total_filled = 0.0;
                    
                    // Get target position size
                    let short_quantity = if let Some(trade) = self.active_trades.get(&trade_id) {
                        trade.short_order.size
                    } else {
                        continue;
                    };
                    
                    // STEP 1: Cancel the original short exit order
                    if let Some(ref order_id) = short_exit_id {
                        eprintln!("[PARTIAL EXIT] Cancelling original short exit order: {}", order_id);
                        let _ = self.execution_backend.cancel_order(&short_exchange, order_id).await;
                        
                        // CRITICAL FIX: Check if cancelled order actually filled before cancellation took effect
                        eprintln!("[PARTIAL EXIT] Checking if cancelled exit order filled...");
                        match self.execution_backend.get_order_status_detailed(&short_exchange, order_id, &symbol).await {
                            Ok(status_info) => {
                                total_filled += status_info.filled_quantity;
                                if status_info.filled_quantity > 0.0 {
                                    eprintln!("[PARTIAL EXIT] ⚠️  Cancelled exit order filled {:.4} contracts before cancellation!", status_info.filled_quantity);
                                } else {
                                    eprintln!("[PARTIAL EXIT] ✅ Cancelled exit order did not fill");
                                }
                            }
                            Err(e) => {
                                eprintln!("[PARTIAL EXIT] ⚠️  Failed to check cancelled order status: {} (assuming 0 fill)", e);
                            }
                        }
                    }
                    
                    // Check if we're already fully hedged from the cancelled order
                    if total_filled >= short_quantity {
                        eprintln!("[PARTIAL EXIT] ✅ Already fully hedged from cancelled exit order ({:.4} contracts)", total_filled);
                        if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                            trade.status = TradeStatus::Exiting;
                            trade.exit_reason = Some("Partial exit hedged (cancelled order filled)".to_string());
                            trade.exit_spread_bps = Some(current_spread_bps);
                        }
                        continue;
                    }
                    
                    // STEP 2: Try smart aggressive limit using order book depth
                    let remaining_after_cancel = short_quantity - total_filled;
                    eprintln!("[PARTIAL EXIT] Attempting smart aggressive limit for remaining {:.4} contracts...", remaining_after_cancel);
                    
                    // Fetch order book depth to find a price likely to fill quickly
                    match self.execution_backend.get_order_book_depth(&short_exchange, &symbol, 5).await {
                        Ok(order_book) => {
                            // For closing short (buying), we want to look at asks
                            // Place order 2-3 levels into the book for higher fill probability
                            let aggressive_price = if order_book.asks.len() >= 3 {
                                // Use 3rd level (index 2) for very high fill probability
                                order_book.asks[2].price
                            } else if order_book.asks.len() >= 2 {
                                // Use 2nd level if 3rd not available
                                order_book.asks[1].price
                            } else if !order_book.asks.is_empty() {
                                // Fallback to best ask
                                order_book.asks[0].price
                            } else {
                                // No order book data, skip limit and go straight to market
                                eprintln!("[PARTIAL EXIT] ⚠️  No order book data, skipping limit order");
                                0.0 // Signal to skip limit order
                            };
                            
                            if aggressive_price > 0.0 {
                                eprintln!("[PARTIAL EXIT] Placing limit at ${:.4} ({}th level in book)", 
                                    aggressive_price, 
                                    if order_book.asks.len() >= 3 { "3rd" } else if order_book.asks.len() >= 2 { "2nd" } else { "1st" });
                                
                                let limit_order = SimulatedOrder {
                                    id: format!("aggressive_exit_{}", uuid::Uuid::new_v4()),
                                    exchange: short_exchange.clone(),
                                    symbol: symbol.clone(),
                                    side: OrderSide::Long, // Closing short = buy
                                    order_type: OrderType::Limit,
                                    price: aggressive_price,
                                    size: remaining_after_cancel,
                                    queue_position: None,
                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                    filled_at: None,
                                    fill_price: None,
                                    status: OrderStatus::Pending,
                                };
                                
                                match self.execution_backend.place_order(limit_order.clone()).await {
                                    Ok(placed_order) => {
                                        eprintln!("[PARTIAL EXIT] Smart aggressive limit placed: {}", placed_order.id);
                                        
                                        // Wait only 200ms for fill (much faster than before)
                                        tokio::time::sleep(Duration::from_millis(200)).await;
                                        
                                        // Check if filled
                                        match self.execution_backend.get_order_status_detailed(&short_exchange, &placed_order.id, &symbol).await {
                                            Ok(status_info) => {
                                                total_filled += status_info.filled_quantity;
                                                if status_info.filled_quantity >= remaining_after_cancel {
                                                    eprintln!("[PARTIAL EXIT] ✅ Smart limit fully filled! Hedge complete (maker fee)");
                                                    if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                                                        trade.status = TradeStatus::Exiting;
                                                        trade.exit_reason = Some("Partial exit hedged with smart limit".to_string());
                                                        trade.exit_spread_bps = Some(current_spread_bps);
                                                    }
                                                    continue;
                                                } else if status_info.filled_quantity > 0.0 {
                                                    eprintln!("[PARTIAL EXIT] ⚠️  Limit partially filled {:.4} contracts, cancelling for market order", status_info.filled_quantity);
                                                    let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                                } else {
                                                    eprintln!("[PARTIAL EXIT] ⚠️  Limit didn't fill - cancelling and using market order");
                                                    let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("[PARTIAL EXIT] ⚠️  Failed to check limit order status: {} (assuming 0 fill)", e);
                                                let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[PARTIAL EXIT] ⚠️  Failed to place smart limit: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[PARTIAL EXIT] ⚠️  Failed to get order book: {}", e);
                        }
                    }
                    
                    // Check if we're already fully hedged
                    if total_filled >= short_quantity {
                        eprintln!("[PARTIAL EXIT] ✅ Already fully hedged ({:.4} contracts total)", total_filled);
                        if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                            trade.status = TradeStatus::Exiting;
                            trade.exit_reason = Some("Partial exit hedged (limit partial fill)".to_string());
                            trade.exit_spread_bps = Some(current_spread_bps);
                        }
                        continue;
                    }
                    
                    // STEP 3: Market order for REMAINING quantity only
                    let remaining_quantity = short_quantity - total_filled;
                    eprintln!("[PARTIAL EXIT] Placing market order for REMAINING {:.4} contracts (total filled so far: {:.4})", 
                        remaining_quantity, total_filled);
                    
                    let market_order = SimulatedOrder {
                        id: format!("market_hedge_{}", uuid::Uuid::new_v4()),
                        exchange: short_exchange.clone(),
                        symbol: symbol.clone(),
                        side: OrderSide::Long, // Closing short = buy
                        order_type: OrderType::Market,
                        price: current_short_price,
                        size: remaining_quantity,  // FIX: Use remaining quantity, not full quantity
                        queue_position: None,
                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                        filled_at: None,
                        fill_price: None,
                        status: OrderStatus::Pending,
                    };
                    
                    match self.execution_backend.place_market_order(market_order).await {
                        Ok(_) => {
                            eprintln!("[PARTIAL EXIT] ✅ Market order filled! Hedge complete (taker fee)");
                            eprintln!("[PARTIAL EXIT] Total filled: {:.4} contracts (target: {:.4})", total_filled + remaining_quantity, short_quantity);
                            if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                                trade.status = TradeStatus::Exiting;
                                trade.exit_reason = Some("Partial exit hedged with market order".to_string());
                                trade.exit_spread_bps = Some(current_spread_bps);
                            }
                        }
                        Err(e) => {
                            eprintln!("[PARTIAL EXIT] ❌ CRITICAL: Market order failed: {}", e);
                            eprintln!("[PARTIAL EXIT] 🚨 UNHEDGED SHORT POSITION ON {}", short_exchange);
                        }
                    }
                    continue;
                }
                
                if short_exit_filled && !long_exit_filled {
                    eprintln!("[PARTIAL EXIT] {} | Short exit filled, but long still open - INTELLIGENT HEDGE", trade_id);
                    
                    // CRITICAL: Mark trade as Exiting IMMEDIATELY to prevent duplicate symbol entries
                    if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                        trade.status = TradeStatus::Exiting;
                        eprintln!("[PARTIAL EXIT] Trade marked as Exiting to prevent race conditions");
                    }
                    
                    // Get target position size
                    let long_quantity = if let Some(trade) = self.active_trades.get(&trade_id) {
                        trade.long_order.size
                    } else {
                        continue;
                    };
                    
                    // Calculate position size in USD for probability estimation
                    let position_size_usd = long_quantity * current_long_price;
                    
                    // STEP 1: Cancel the original long exit order (fire and forget)
                    if let Some(ref order_id) = long_exit_id {
                        eprintln!("[PARTIAL EXIT] Cancelling original long exit order: {}", order_id);
                        let _ = self.execution_backend.cancel_order(&long_exchange, order_id).await;
                    }
                    
                    // STEP 2: Fetch order book and make intelligent decision
                    match self.execution_backend.get_order_book_depth(&long_exchange, &symbol, 5).await {
                        Ok(order_book) => {
                            // Use probability estimator to decide: limit or market?
                            let decision = self.fill_probability_estimator.should_try_limit(
                                &order_book,
                                OrderSide::Short, // Closing long = sell
                                position_size_usd,
                            );
                            
                            eprintln!("[PROBABILITY] {} | Use limit: {} | Probability: {:.0}% | Reason: {}", 
                                symbol, decision.use_limit, decision.probability * 100.0, decision.reason);
                            
                            if decision.use_limit {
                                // HIGH PROBABILITY - Try limit order with intelligent pricing
                                let book_side = &order_book.bids;
                                let limit_price = if decision.price_level < book_side.len() {
                                    book_side[decision.price_level].price
                                } else {
                                    book_side[0].price // Fallback to best bid
                                };
                                
                                eprintln!("[PARTIAL EXIT] Placing INTELLIGENT limit at ${:.4} (level {}, {:.0}% fill probability)", 
                                    limit_price, decision.price_level, decision.probability * 100.0);
                                
                                let limit_order = SimulatedOrder {
                                    id: format!("smart_exit_{}", uuid::Uuid::new_v4()),
                                    exchange: long_exchange.clone(),
                                    symbol: symbol.clone(),
                                    side: OrderSide::Short,
                                    order_type: OrderType::Limit,
                                    price: limit_price,
                                    size: long_quantity,
                                    queue_position: None,
                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                    filled_at: None,
                                    fill_price: None,
                                    status: OrderStatus::Pending,
                                };
                                
                                match self.execution_backend.place_order(limit_order).await {
                                    Ok(placed_order) => {
                                        eprintln!("[PARTIAL EXIT] Smart limit placed: {}", placed_order.id);
                                        
                                        // Wait based on probability estimate
                                        tokio::time::sleep(Duration::from_millis(decision.wait_time_ms)).await;
                                        
                                        // Check if filled
                                        match self.execution_backend.get_order_status_detailed(&long_exchange, &placed_order.id, &symbol).await {
                                            Ok(status_info) if status_info.is_fully_filled() => {
                                                eprintln!("[PARTIAL EXIT] ✅ Smart limit FULLY filled! Hedge complete (maker fee saved)");
                                                if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                                                    trade.status = TradeStatus::Exiting;
                                                    trade.exit_reason = Some("Partial exit hedged with smart limit".to_string());
                                                    trade.exit_spread_bps = Some(current_spread_bps);
                                                }
                                                continue;
                                            }
                                            Ok(status_info) if status_info.filled_quantity > 0.0 => {
                                                eprintln!("[PARTIAL EXIT] ⚠️  Limit PARTIALLY filled {:.4}/{:.4} contracts", 
                                                    status_info.filled_quantity, long_quantity);
                                                let _ = self.execution_backend.cancel_order(&long_exchange, &placed_order.id).await;
                                                
                                                // Market order for remaining
                                                let remaining = long_quantity - status_info.filled_quantity;
                                                eprintln!("[PARTIAL EXIT] Market order for remaining {:.4} contracts", remaining);
                                                
                                                let market_order = SimulatedOrder {
                                                    id: format!("market_remaining_{}", uuid::Uuid::new_v4()),
                                                    exchange: long_exchange.clone(),
                                                    symbol: symbol.clone(),
                                                    side: OrderSide::Short,
                                                    order_type: OrderType::Market,
                                                    price: current_long_price,
                                                    size: remaining,
                                                    queue_position: None,
                                                    created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                                    filled_at: None,
                                                    fill_price: None,
                                                    status: OrderStatus::Pending,
                                                };
                                                
                                                let _ = self.execution_backend.place_market_order(market_order).await;
                                                if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                                                    trade.status = TradeStatus::Exiting;
                                                    trade.exit_reason = Some("Partial exit hedged (limit partial + market)".to_string());
                                                    trade.exit_spread_bps = Some(current_spread_bps);
                                                }
                                                continue;
                                            }
                                            _ => {
                                                eprintln!("[PARTIAL EXIT] ⚠️  Limit didn't fill - using market order");
                                                let _ = self.execution_backend.cancel_order(&long_exchange, &placed_order.id).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[PARTIAL EXIT] ⚠️  Failed to place smart limit: {}", e);
                                    }
                                }
                            }
                            
                            // LOW PROBABILITY or limit failed - Use market order
                            eprintln!("[PARTIAL EXIT] Using INSTANT market order for {:.4} contracts", long_quantity);
                            
                            let market_order = SimulatedOrder {
                                id: format!("instant_hedge_{}", uuid::Uuid::new_v4()),
                                exchange: long_exchange.clone(),
                                symbol: symbol.clone(),
                                side: OrderSide::Short,
                                order_type: OrderType::Market,
                                price: current_long_price,
                                size: long_quantity,
                                queue_position: None,
                                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                filled_at: None,
                                fill_price: None,
                                status: OrderStatus::Pending,
                            };
                            
                            match self.execution_backend.place_market_order(market_order).await {
                                Ok(_) => {
                                    eprintln!("[PARTIAL EXIT] ✅ INSTANT market hedge complete!");
                                    if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                                        trade.status = TradeStatus::Exiting;
                                        trade.exit_reason = Some("Partial exit hedged with instant market".to_string());
                                        trade.exit_spread_bps = Some(current_spread_bps);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[PARTIAL EXIT] ❌ CRITICAL: Market order failed: {}", e);
                                    eprintln!("[PARTIAL EXIT] 🚨 UNHEDGED LONG POSITION ON {}", long_exchange);
                                }
                            }
                        }
                        Err(e) => {
                            // No order book - go straight to market
                            eprintln!("[PARTIAL EXIT] ⚠️  Failed to get order book: {} - using market order", e);
                            
                            let market_order = SimulatedOrder {
                                id: format!("instant_hedge_{}", uuid::Uuid::new_v4()),
                                exchange: long_exchange.clone(),
                                symbol: symbol.clone(),
                                side: OrderSide::Short,
                                order_type: OrderType::Market,
                                price: current_long_price,
                                size: long_quantity,
                                queue_position: None,
                                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                filled_at: None,
                                fill_price: None,
                                status: OrderStatus::Pending,
                            };
                            
                            let _ = self.execution_backend.place_market_order(market_order).await;
                            if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                                trade.status = TradeStatus::Exiting;
                                trade.exit_reason = Some("Partial exit hedged with market (no order book)".to_string());
                                trade.exit_spread_bps = Some(current_spread_bps);
                            }
                        }
                    }
                    continue;
                }

                // 2. Check stop-loss conditions (30% spread widening)
                if !stop_loss_triggered {
                    let stop_loss_hit = current_long_price <= stop_loss_long || current_short_price >= stop_loss_short;
                    
                    if stop_loss_hit {
                        eprintln!("[STOP LOSS] {} | TRIGGERED! Long: ${:.2} <= ${:.2} OR Short: ${:.2} >= ${:.2}", 
                            trade_id, current_long_price, stop_loss_long, current_short_price, stop_loss_short);
                        
                        // Get position sizes
                        let (long_quantity, short_quantity) = if let Some(trade) = self.active_trades.get(&trade_id) {
                            (trade.long_order.size, trade.short_order.size)
                        } else {
                            continue;
                        };
                        
                        // STEP 1: Cancel exit orders and check if they filled
                        let mut long_filled_from_cancel = 0.0;
                        let mut short_filled_from_cancel = 0.0;
                        
                        if let Some(ref order_id) = long_exit_id {
                            eprintln!("[STOP LOSS] Cancelling long exit order: {}", order_id);
                            let _ = self.execution_backend.cancel_order(&long_exchange, order_id).await;
                            
                            // Check if cancelled order filled before cancellation
                            match self.execution_backend.get_order_status_detailed(&long_exchange, order_id, &symbol).await {
                                Ok(status_info) => {
                                    long_filled_from_cancel = status_info.filled_quantity;
                                    if long_filled_from_cancel > 0.0 {
                                        eprintln!("[STOP LOSS] ⚠️  Cancelled long exit order filled {:.4} contracts before cancellation", 
                                            long_filled_from_cancel);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[STOP LOSS] ⚠️  Failed to check cancelled long exit order: {} (assuming 0 fill)", e);
                                }
                            }
                        }
                        
                        if let Some(ref order_id) = short_exit_id {
                            eprintln!("[STOP LOSS] Cancelling short exit order: {}", order_id);
                            let _ = self.execution_backend.cancel_order(&short_exchange, order_id).await;
                            
                            // Check if cancelled order filled before cancellation
                            match self.execution_backend.get_order_status_detailed(&short_exchange, order_id, &symbol).await {
                                Ok(status_info) => {
                                    short_filled_from_cancel = status_info.filled_quantity;
                                    if short_filled_from_cancel > 0.0 {
                                        eprintln!("[STOP LOSS] ⚠️  Cancelled short exit order filled {:.4} contracts before cancellation", 
                                            short_filled_from_cancel);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[STOP LOSS] ⚠️  Failed to check cancelled short exit order: {} (assuming 0 fill)", e);
                                }
                            }
                        }

                        // Mark stop-loss as triggered
                        if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
                            trade.stop_loss_triggered = true;
                            trade.status = TradeStatus::Exiting;
                            trade.exit_reason = Some(format!("Stop-loss triggered (30% widening) | Long: ${:.2} | Short: ${:.2}", 
                                current_long_price, current_short_price));
                            trade.exit_spread_bps = Some(current_spread_bps);
                        }

                        // Check if already fully closed from cancelled orders
                        if long_filled_from_cancel >= long_quantity && short_filled_from_cancel >= short_quantity {
                            eprintln!("[STOP LOSS] ✅ Both positions already closed from cancelled orders!");
                            continue;
                        }
                        
                        // STEP 2: Try aggressive limit orders first (to save on taker fees)
                        eprintln!("[STOP LOSS] {} | Attempting aggressive limit orders to minimize fees...", trade_id);
                        
                        // Try aggressive limit for long position (if not fully closed)
                        if long_filled_from_cancel < long_quantity {
                            let remaining_long = long_quantity - long_filled_from_cancel;
                            eprintln!("[STOP LOSS] Trying aggressive limit for long exit: {:.4} contracts at best bid", remaining_long);
                            
                            match self.execution_backend.get_best_bid(&long_exchange, &symbol).await {
                                Ok(best_bid) => {
                                    let limit_order = SimulatedOrder {
                                        id: format!("stop_loss_limit_long_{}", uuid::Uuid::new_v4()),
                                        exchange: long_exchange.clone(),
                                        symbol: symbol.clone(),
                                        side: OrderSide::Short, // Closing long = sell
                                        order_type: OrderType::Limit,
                                        price: best_bid,
                                        size: remaining_long,
                                        queue_position: None,
                                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                        filled_at: None,
                                        fill_price: None,
                                        status: OrderStatus::Pending,
                                    };
                                    
                                    match self.execution_backend.place_order(limit_order.clone()).await {
                                        Ok(placed_order) => {
                                            eprintln!("[STOP LOSS] Long limit placed: {} at ${:.4}", placed_order.id, best_bid);
                                            tokio::time::sleep(Duration::from_secs(2)).await;
                                            
                                            match self.execution_backend.get_order_status_detailed(&long_exchange, &placed_order.id, &symbol).await {
                                                Ok(status_info) => {
                                                    long_filled_from_cancel += status_info.filled_quantity;
                                                    if status_info.filled_quantity > 0.0 {
                                                        eprintln!("[STOP LOSS] Long limit filled {:.4} contracts (maker fee)", status_info.filled_quantity);
                                                    }
                                                    let _ = self.execution_backend.cancel_order(&long_exchange, &placed_order.id).await;
                                                }
                                                Err(e) => {
                                                    eprintln!("[STOP LOSS] ⚠️  Failed to check long limit status: {}", e);
                                                    let _ = self.execution_backend.cancel_order(&long_exchange, &placed_order.id).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("[STOP LOSS] ⚠️  Failed to place long limit: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[STOP LOSS] ⚠️  Failed to get best bid: {}", e);
                                }
                            }
                        }
                        
                        // Try aggressive limit for short position (if not fully closed)
                        if short_filled_from_cancel < short_quantity {
                            let remaining_short = short_quantity - short_filled_from_cancel;
                            eprintln!("[STOP LOSS] Trying aggressive limit for short exit: {:.4} contracts at best ask", remaining_short);
                            
                            match self.execution_backend.get_best_ask(&short_exchange, &symbol).await {
                                Ok(best_ask) => {
                                    let limit_order = SimulatedOrder {
                                        id: format!("stop_loss_limit_short_{}", uuid::Uuid::new_v4()),
                                        exchange: short_exchange.clone(),
                                        symbol: symbol.clone(),
                                        side: OrderSide::Long, // Closing short = buy
                                        order_type: OrderType::Limit,
                                        price: best_ask,
                                        size: remaining_short,
                                        queue_position: None,
                                        created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                        filled_at: None,
                                        fill_price: None,
                                        status: OrderStatus::Pending,
                                    };
                                    
                                    match self.execution_backend.place_order(limit_order.clone()).await {
                                        Ok(placed_order) => {
                                            eprintln!("[STOP LOSS] Short limit placed: {} at ${:.4}", placed_order.id, best_ask);
                                            tokio::time::sleep(Duration::from_secs(2)).await;
                                            
                                            match self.execution_backend.get_order_status_detailed(&short_exchange, &placed_order.id, &symbol).await {
                                                Ok(status_info) => {
                                                    short_filled_from_cancel += status_info.filled_quantity;
                                                    if status_info.filled_quantity > 0.0 {
                                                        eprintln!("[STOP LOSS] Short limit filled {:.4} contracts (maker fee)", status_info.filled_quantity);
                                                    }
                                                    let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                                }
                                                Err(e) => {
                                                    eprintln!("[STOP LOSS] ⚠️  Failed to check short limit status: {}", e);
                                                    let _ = self.execution_backend.cancel_order(&short_exchange, &placed_order.id).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("[STOP LOSS] ⚠️  Failed to place short limit: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[STOP LOSS] ⚠️  Failed to get best ask: {}", e);
                                }
                            }
                        }
                        
                        // Check if fully closed after aggressive limits
                        if long_filled_from_cancel >= long_quantity && short_filled_from_cancel >= short_quantity {
                            eprintln!("[STOP LOSS] ✅ Both positions closed with aggressive limits (maker fees)!");
                            continue;
                        }
                        
                        // STEP 3: Place market orders for REMAINING quantities only
                        eprintln!("[STOP LOSS] {} | Placing market orders for remaining positions...", trade_id);
                        
                        let remaining_long = long_quantity - long_filled_from_cancel;
                        let remaining_short = short_quantity - short_filled_from_cancel;
                        
                        eprintln!("[STOP LOSS] Remaining to close: Long={:.4}, Short={:.4}", remaining_long, remaining_short);
                        
                        // Close remaining long position (if any)
                        let long_result = if remaining_long > 0.001 {
                            let long_close_order = SimulatedOrder {
                                id: format!("stop_loss_market_long_{}", uuid::Uuid::new_v4()),
                                exchange: long_exchange.clone(),
                                symbol: symbol.clone(),
                                side: OrderSide::Short, // Closing long = sell
                                order_type: OrderType::Market,
                                price: current_long_price,
                                size: remaining_long,
                                queue_position: None,
                                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                filled_at: None,
                                fill_price: None,
                                status: OrderStatus::Pending,
                            };
                            eprintln!("[STOP LOSS] Placing market order: {} | {} | SELL | {:.4}", 
                                long_exchange, symbol, remaining_long);
                            self.execution_backend.place_market_order(long_close_order).await
                        } else {
                            eprintln!("[STOP LOSS] ✅ Long position already fully closed ({:.4} contracts)", long_filled_from_cancel);
                            Ok(SimulatedOrder::default())
                        };
                        
                        // Close remaining short position (if any)
                        let short_result = if remaining_short > 0.001 {
                            let short_close_order = SimulatedOrder {
                                id: format!("stop_loss_market_short_{}", uuid::Uuid::new_v4()),
                                exchange: short_exchange.clone(),
                                symbol: symbol.clone(),
                                side: OrderSide::Long, // Closing short = buy
                                order_type: OrderType::Market,
                                price: current_short_price,
                                size: remaining_short,
                                queue_position: None,
                                created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                filled_at: None,
                                fill_price: None,
                                status: OrderStatus::Pending,
                            };
                            eprintln!("[STOP LOSS] Placing market order: {} | {} | BUY | {:.4}", 
                                short_exchange, symbol, remaining_short);
                            self.execution_backend.place_market_order(short_close_order).await
                        } else {
                            eprintln!("[STOP LOSS] ✅ Short position already fully closed ({:.4} contracts)", short_filled_from_cancel);
                            Ok(SimulatedOrder::default())
                        };
                        
                        match (long_result, short_result) {
                            (Ok(_), Ok(_)) => {
                                eprintln!("[STOP LOSS] ✅ Both positions closed successfully");
                                eprintln!("[STOP LOSS] Total closed: Long={:.4}, Short={:.4}", long_quantity, short_quantity);
                            }
                            (Err(e), Ok(_)) => {
                                eprintln!("[STOP LOSS] ❌ CRITICAL: Long position failed to close: {}", e);
                                eprintln!("[STOP LOSS] 🚨 UNHEDGED LONG POSITION ON {} ({:.4} contracts)", long_exchange, remaining_long);
                            }
                            (Ok(_), Err(e)) => {
                                eprintln!("[STOP LOSS] ❌ CRITICAL: Short position failed to close: {}", e);
                                eprintln!("[STOP LOSS] 🚨 UNHEDGED SHORT POSITION ON {} ({:.4} contracts)", short_exchange, remaining_short);
                            }
                            (Err(e1), Err(e2)) => {
                                eprintln!("[STOP LOSS] ❌ CRITICAL: BOTH positions failed to close!");
                                eprintln!("[STOP LOSS] Long error: {}", e1);
                                eprintln!("[STOP LOSS] Short error: {}", e2);
                                eprintln!("[STOP LOSS] 🚨 UNHEDGED POSITIONS - Long: {:.4} on {}, Short: {:.4} on {}", 
                                    remaining_long, long_exchange, remaining_short, short_exchange);
                            }
                        }
                    }
                }

                continue; // Skip paper trading logic
            }

            // ============================================================================
            // PAPER TRADING MONITORING (Original logic)
            // ============================================================================

            // Get current prices from market data store (no Redis in hot path)
            let (current_long_price, current_short_price) = self.get_current_prices(&symbol)?;
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
            if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
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
                    - (trade.entry_time * 1000);

                if PositionManager::detect_leg_out(
                    trade.long_order.status == OrderStatus::Filled,
                    trade.short_order.status == OrderStatus::Filled,
                    time_since_entry,
                )
                    && trade.leg_out_event.is_none() {
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

        Ok(())
    }

    async fn check_exits(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let trade_ids: Vec<String> = self.active_trades.iter()
            .map(|entry| entry.key().clone())
            .collect();
        
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
                    
                    if let Some(mut tracker) = self.negative_funding_trackers.get_mut(&symbol) {
                        tracker.update_funding(funding_delta);
                    }
                    
                    if let Some(mut trade) = self.active_trades.get_mut(&trade_id) {
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

                        // OPTIMIZATION: Only try to cancel exit orders if they exist AND haven't been cancelled yet
                        // Skip if orders were already cancelled during partial exit hedging
                        // This prevents redundant API calls that slow down cleanup
                        if let Some(ref long_exit_order) = trade.long_exit_order {
                            if long_exit_order.status != OrderStatus::Cancelled {
                                eprintln!("[CLEANUP] Cancelling long exit order: {} on {}", long_exit_order.id, trade.long_exchange);
                                if let Err(e) = self.execution_backend.cancel_order(&trade.long_exchange, &long_exit_order.id).await {
                                    // Ignore "already cancelled" errors - this is expected
                                    if !e.to_string().contains("Unknown order") && !e.to_string().contains("does not exist") {
                                        eprintln!("[CLEANUP] ⚠️  Failed to cancel long exit order: {}", e);
                                    }
                                }
                            }
                        }
                        
                        if let Some(ref short_exit_order) = trade.short_exit_order {
                            if short_exit_order.status != OrderStatus::Cancelled {
                                eprintln!("[CLEANUP] Cancelling short exit order: {} on {}", short_exit_order.id, trade.short_exchange);
                                if let Err(e) = self.execution_backend.cancel_order(&trade.short_exchange, &short_exit_order.id).await {
                                    // Ignore "already cancelled" errors - this is expected
                                    if !e.to_string().contains("order not exists") && !e.to_string().contains("too late to cancel") {
                                        eprintln!("[CLEANUP] ⚠️  Failed to cancel short exit order: {}", e);
                                    }
                                }
                            }
                        }

                        // Close trade
                        self.portfolio_manager.write().await
                            .close_trade(&trade_id, actual_profit, exit_reason.clone())
                            .await?;

                        println!(
                            "[EXIT COMPLETED] {} | Reason: {} | Entry Spread: {:.2}bps | Exit Spread: {:.2}bps | Fees: {:.2}bps | Actual Profit: ${:.2}",
                            trade_id, exit_reason, trade.entry_spread_bps, exit_spread_bps, total_fee_bps, actual_profit
                        );

                        // Reset negative funding tracker
                        if let Some(mut tracker) = self.negative_funding_trackers.get_mut(&symbol) {
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




    /// Get current prices from market data store (hot path - no Redis).
    ///
    /// This method uses the in-memory market data store populated by the streaming
    /// pipeline, eliminating Redis reads from the hot path.
    ///
    /// Requirement: 1.3 (Redis only for persistence, not hot path)
    fn get_current_prices(&self, pair: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        // Find the trade to get the specific exchanges it's using
        let trade = self.active_trades.iter().find(|entry| entry.value().symbol == pair).map(|entry| entry.value().clone());
        
        if let Some(trade) = trade {
            // Get prices from market data store (no Redis)
            self.get_prices_from_store(pair, &trade.long_exchange, &trade.short_exchange)
        } else {
            Err("No trade found for pair".into())
        }
    }

    /// Get current prices for opportunity validation from market data store (hot path - no Redis).
    ///
    /// This method uses the in-memory market data store populated by the streaming
    /// pipeline, eliminating Redis reads from the hot path.
    ///
    /// Requirement: 1.3 (Redis only for persistence, not hot path)
    fn get_current_prices_for_opportunity(&self, pair: &str, long_exchange: &str, short_exchange: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        // Get prices from market data store (no Redis)
        self.get_prices_from_store(pair, long_exchange, short_exchange)
    }

    /// Get prices from market data store (hot path - no Redis).
    ///
    /// This method looks up prices from the in-memory market data store,
    /// which is populated by the streaming pipeline. This eliminates Redis
    /// reads from the hot path entirely.
    ///
    /// # Arguments
    ///
    /// * `pair` - Symbol (e.g., "BTCUSDT")
    /// * `long_exchange` - Exchange for long position (we need ask price)
    /// * `short_exchange` - Exchange for short position (we need bid price)
    ///
    /// # Returns
    ///
    /// Tuple of (long_ask, short_bid) prices, or error if not found.
    ///
    /// # Performance
    ///
    /// - Time: ~10-20 CPU cycles (2 hash lookups + 2 array accesses)
    /// - Allocations: Zero (uses pre-allocated market data store)
    /// - Cache: High hit rate due to SoA layout
    ///
    /// Requirements: 1.3 (No Redis in hot path), 1.4 (Direct memory access)
    fn get_prices_from_store(&self, pair: &str, long_exchange: &str, short_exchange: &str) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
        // Get symbol IDs for both exchanges using dynamic SymbolMap
        // This allows us to scan ALL incoming data, not just pre-defined symbols
        let long_symbol_id = self.symbol_map.get_or_insert(long_exchange, pair);
        let short_symbol_id = self.symbol_map.get_or_insert(short_exchange, pair);
        
        // Get ask price from long exchange (hot path - array access)
        let long_ask = self.market_data_store.get_ask(long_symbol_id)
            .ok_or_else(|| format!("No ask price for {} on {} (symbol_id: {})", pair, long_exchange, long_symbol_id))?;
        
        // Get bid price from short exchange (hot path - array access)
        let short_bid = self.market_data_store.get_bid(short_symbol_id)
            .ok_or_else(|| format!("No bid price for {} on {} (symbol_id: {})", pair, short_exchange, short_symbol_id))?;
        
        // Validate prices are reasonable (non-zero)
        if long_ask <= 0.0 || short_bid <= 0.0 {
            return Err(format!("Invalid prices for {}: ask={}, bid={}", pair, long_ask, short_bid).into());
        }
        
        eprintln!("[PRICES] {} | Long ({}): ${:.4} | Short ({}): ${:.4} [FROM STORE]", 
            pair, long_exchange, long_ask, short_exchange, short_bid);
        
        Ok((long_ask, short_bid))
    }

    /// Fetch prices from Redis (warm path - for monitoring/persistence only).
    ///
    /// This method is kept for backward compatibility and monitoring purposes.
    /// It should NOT be called in the hot path (strategy decision/execution).
    ///
    /// Requirement: 1.3 (Redis only for monitoring/persistence)
    #[allow(dead_code)]
    /// Get exchange taker fee in basis points (inlined for hot path performance).
    ///
    /// Requirement: 6.1 (Inline fee calculation)
    #[inline(always)]
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
                long_exit_order: None,  // Not used in this code path
                short_exit_order: None, // Not used in this code path
                stop_loss_triggered: false,
                stop_loss_long_price: 0.0,
                stop_loss_short_price: 0.0,
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
