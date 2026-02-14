use crate::strategy::types::{PortfolioState, TradeStatus, PaperTrade, PortfolioMetrics};
use redis::aio::MultiplexedConnection;
use std::error::Error;
use tokio::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Portfolio manager with optimized locking strategy
/// 
/// Uses RwLock for read-heavy operations (checking capital, getting metrics)
/// while minimizing lock hold time by performing expensive operations
/// (Redis writes) outside the critical section.
pub struct PortfolioManager {
    /// Portfolio state protected by RwLock for concurrent reads
    state: RwLock<PortfolioState>,
    redis_conn: MultiplexedConnection,
    start_time: u64,
    redis_prefix: String,
}

impl PortfolioManager {
    pub async fn new(
        redis_conn: MultiplexedConnection,
        starting_capital: f64,
        redis_prefix: Option<String>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let redis_prefix = redis_prefix.unwrap_or_else(|| "trade".to_string());
        let state = PortfolioState::new(starting_capital);
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let state_key = format!("strategy:{}:portfolio:state", redis_prefix);
        let metrics_key = format!("strategy:{}:portfolio:metrics", redis_prefix);
        
        let manager = Self { 
            state: RwLock::new(state),
            redis_conn, 
            start_time,
            redis_prefix,
        };
        
        // Clear old portfolio state from Redis to start fresh
        eprintln!("[PORTFOLIO] Clearing old state from Redis (prefix: {})", manager.redis_prefix);
        redis::cmd("DEL")
            .arg(&state_key)
            .arg(&metrics_key)
            .query_async::<_, ()>(&mut manager.redis_conn.clone())
            .await?;
        
        eprintln!("[PORTFOLIO] Persisting fresh portfolio state");
        manager.persist_state().await?;
        Ok(manager)
    }

    /// Fast read-only access to portfolio state
    /// Uses read lock to allow concurrent access from multiple threads
    #[inline]
    pub async fn get_state(&self) -> PortfolioState {
        // Acquire read lock (allows concurrent readers)
        let state = self.state.read().await;
        // Clone only the necessary data while holding the lock
        state.clone()
    }

    /// Fast read-only access to available capital
    /// Minimizes lock hold time by reading only the required field
    #[inline]
    pub async fn get_available_capital(&self) -> f64 {
        self.state.read().await.available_capital
    }

    /// Fast read-only access to starting capital
    #[inline]
    pub async fn get_starting_capital(&self) -> f64 {
        self.state.read().await.starting_capital
    }

    /// Open a new trade with optimized locking
    /// Minimizes critical section by preparing data before acquiring write lock
    pub async fn open_trade(&mut self, trade: PaperTrade) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Validate capital constraints with read lock first (fast path)
        {
            let state = self.state.read().await;
            if trade.position_size_usd > state.available_capital {
                return Err(format!(
                    "Position size ${:.2} exceeds available capital ${:.2}",
                    trade.position_size_usd, state.available_capital
                ).into());
            }
        } // Read lock released here

        // Acquire write lock only for state modification (critical section)
        {
            let mut state = self.state.write().await;
            
            // Double-check capital after acquiring write lock (TOCTOU protection)
            if trade.position_size_usd > state.available_capital {
                return Err(format!(
                    "Position size ${:.2} exceeds available capital ${:.2}",
                    trade.position_size_usd, state.available_capital
                ).into());
            }

            // Deduct capital from available pool
            state.available_capital -= trade.position_size_usd;
            state.total_open_positions += trade.position_size_usd;

            // Add to active trades
            state.active_trades.push(trade.clone());
        } // Write lock released here - minimize hold time

        // Perform expensive I/O operations outside the lock
        self.log_trade_entry(&trade).await?;
        self.persist_state().await?;

        Ok(())
    }

    /// Close a trade with optimized locking
    /// Minimizes critical section by performing calculations outside the lock
    pub async fn close_trade(&mut self, trade_id: &str, actual_profit: f64, exit_reason: String) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Prepare data outside the lock
        let exit_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let is_profit = actual_profit > 0.0;
        
        // Find and update trade with write lock (critical section)
        let trade_opt = {
            let mut state = self.state.write().await;
            
            if let Some(pos) = state.active_trades.iter().position(|t| t.id == trade_id) {
                let mut trade = state.active_trades.remove(pos);
                trade.actual_profit_usd = actual_profit;
                trade.exit_reason = Some(exit_reason.clone());
                trade.status = TradeStatus::Closed;
                trade.exit_time = Some(exit_time);

                // Update portfolio state
                state.total_open_positions -= trade.position_size_usd;
                state.available_capital += trade.position_size_usd + actual_profit;
                state.cumulative_pnl += actual_profit;

                // Track leg-out losses
                let is_leg_out = trade.leg_out_event.is_some();
                if is_leg_out && actual_profit < 0.0 {
                    state.leg_out_total_loss += actual_profit.abs();
                }

                state.closed_trades.push(trade.clone());
                
                Some((trade, is_leg_out))
            } else {
                None
            }
        }; // Write lock released here

        // Update atomic counters outside the write lock (lock-free)
        if let Some((trade, is_leg_out)) = trade_opt {
            // Use atomic operations for counters (lock-free)
            let state = self.state.read().await;
            if is_profit {
                state.increment_wins();
            } else {
                state.increment_losses();
            }
            
            if is_leg_out && actual_profit < 0.0 {
                state.increment_leg_outs();
            }
            drop(state); // Release read lock

            // Perform expensive I/O operations outside the lock
            self.log_trade_exit(&trade, &exit_reason).await?;
            self.persist_state().await?;
        }
        
        Ok(())
    }

    /// Get portfolio metrics with read lock (allows concurrent access)
    pub async fn get_portfolio_metrics(&self) -> PortfolioMetrics {
        // Acquire read lock for consistent snapshot
        let state = self.state.read().await;
        
        // Load atomic counters with Relaxed ordering (lock-free)
        let win_count = state.get_win_count();
        let loss_count = state.get_loss_count();
        let leg_out_count = state.get_leg_out_count();
        
        let total_trades = win_count + loss_count;
        let win_rate = if total_trades > 0 {
            (win_count as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };

        let pnl_percentage = (state.cumulative_pnl / state.starting_capital) * 100.0;
        let utilization_pct = (state.total_open_positions / state.starting_capital) * 100.0;

        let leg_out_loss_pct = if state.cumulative_pnl != 0.0 {
            (state.leg_out_total_loss / state.cumulative_pnl.abs()) * 100.0
        } else {
            0.0
        };

        let days_elapsed = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() - self.start_time) as f64 / 86400.0;

        let realistic_apr = if days_elapsed > 0.0 {
            ((state.cumulative_pnl / state.starting_capital) / (days_elapsed / 365.0)) * 100.0
        } else {
            0.0
        };

        PortfolioMetrics {
            total_trades,
            win_rate,
            cumulative_pnl: state.cumulative_pnl,
            pnl_percentage,
            available_capital: state.available_capital,
            utilization_pct,
            leg_out_count,
            leg_out_loss_pct,
            realistic_apr,
        }
    } // Read lock released here

    async fn log_trade_entry(&self, trade: &PaperTrade) -> Result<(), Box<dyn Error + Send + Sync>> {
        let log_entry = format!(
            "[ENTRY] {} | {} -> {} | Entry Spread: {:.2}bps | Size: ${:.2} | Projected Profit: ${:.2}",
            trade.id,
            trade.long_exchange,
            trade.short_exchange,
            trade.entry_spread_bps,
            trade.position_size_usd,
            trade.projected_profit_usd
        );

        // Store in Redis
        redis::cmd("LPUSH")
            .arg("strategy:trade_log:entries")
            .arg(&log_entry)
            .query_async::<_, ()>(&mut self.redis_conn.clone())
            .await?;

        Ok(())
    }

    async fn log_trade_exit(&self, trade: &PaperTrade, exit_reason: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let log_entry = format!(
            "[EXIT] {} | Reason: {} | Actual Profit: ${:.2} | Exit Time: {}",
            trade.id,
            exit_reason,
            trade.actual_profit_usd,
            trade.exit_time.unwrap_or(0)
        );

        // Store in Redis
        redis::cmd("LPUSH")
            .arg("strategy:trade_log:exits")
            .arg(&log_entry)
            .query_async::<_, ()>(&mut self.redis_conn.clone())
            .await?;

        Ok(())
    }

    /// Persist state to Redis (called outside critical section)
    async fn persist_state(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let state_key = format!("strategy:{}:portfolio:state", self.redis_prefix);
        let metrics_key = format!("strategy:{}:portfolio:metrics", self.redis_prefix);
        
        // Acquire read lock to get consistent snapshot
        let serializable_state = {
            let state = self.state.read().await;
            state.to_serializable()
        }; // Read lock released here
        
        // Perform expensive serialization and I/O outside the lock
        let json = serde_json::to_string(&serializable_state)?;
        redis::cmd("SET")
            .arg(&state_key)
            .arg(json)
            .query_async::<_, ()>(&mut self.redis_conn.clone())
            .await?;

        // Also persist metrics
        let metrics = self.get_portfolio_metrics().await;
        let metrics_json = serde_json::to_string(&metrics)?;
        redis::cmd("SET")
            .arg(&metrics_key)
            .arg(metrics_json)
            .query_async::<_, ()>(&mut self.redis_conn.clone())
            .await?;

        Ok(())
    }
}
