use crate::strategy::types::{PortfolioState, TradeStatus, PaperTrade, serialize_to_json, PortfolioMetrics};
use redis::aio::MultiplexedConnection;
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct PortfolioManager {
    state: PortfolioState,
    redis_conn: MultiplexedConnection,
    start_time: u64,
}

impl PortfolioManager {
    pub async fn new(redis_conn: MultiplexedConnection, starting_capital: f64) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let state = PortfolioState::new(starting_capital);
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let manager = Self { state, redis_conn, start_time };
        
        // Clear old portfolio state from Redis to start fresh
        eprintln!("[PORTFOLIO] Clearing old state from Redis");
        redis::cmd("DEL")
            .arg("strategy:portfolio:state")
            .arg("strategy:portfolio:metrics")
            .query_async::<_, ()>(&mut manager.redis_conn.clone())
            .await?;
        
        eprintln!("[PORTFOLIO] Persisting fresh portfolio state");
        manager.persist_state().await?;
        Ok(manager)
    }

    pub fn get_state(&self) -> &PortfolioState {
        &self.state
    }

    pub async fn open_trade(&mut self, trade: PaperTrade) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Validate capital constraints
        if trade.position_size_usd > self.state.available_capital {
            return Err(format!(
                "Position size ${:.2} exceeds available capital ${:.2}",
                trade.position_size_usd, self.state.available_capital
            ).into());
        }

        // Deduct capital from available pool
        self.state.available_capital -= trade.position_size_usd;
        self.state.total_open_positions += trade.position_size_usd;

        // Add to active trades
        self.state.active_trades.push(trade.clone());

        // Log trade entry
        self.log_trade_entry(&trade).await?;

        // Persist state
        self.persist_state().await?;

        Ok(())
    }

    pub async fn close_trade(&mut self, trade_id: &str, actual_profit: f64, exit_reason: String) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(pos) = self.state.active_trades.iter().position(|t| t.id == trade_id) {
            let mut trade = self.state.active_trades.remove(pos);
            trade.actual_profit_usd = actual_profit;
            trade.exit_reason = Some(exit_reason.clone());
            trade.status = TradeStatus::Closed;
            trade.exit_time = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());

            self.state.total_open_positions -= trade.position_size_usd;
            self.state.available_capital += trade.position_size_usd + actual_profit;
            self.state.cumulative_pnl += actual_profit;

            if actual_profit > 0.0 {
                self.state.win_count += 1;
            } else {
                self.state.loss_count += 1;
            }

            // Track leg-out losses
            if let Some(_leg_out_event) = &trade.leg_out_event {
                if actual_profit < 0.0 {
                    self.state.leg_out_count += 1;
                    self.state.leg_out_total_loss += actual_profit.abs();
                }
            }

            // Log trade exit
            self.log_trade_exit(&trade, &exit_reason).await?;

            self.state.closed_trades.push(trade);
            self.persist_state().await?;
        }
        Ok(())
    }

    pub fn get_portfolio_metrics(&self) -> PortfolioMetrics {
        let total_trades = self.state.win_count + self.state.loss_count;
        let win_rate = if total_trades > 0 {
            (self.state.win_count as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };

        let pnl_percentage = (self.state.cumulative_pnl / self.state.starting_capital) * 100.0;
        let utilization_pct = (self.state.total_open_positions / self.state.starting_capital) * 100.0;

        let leg_out_loss_pct = if self.state.cumulative_pnl != 0.0 {
            (self.state.leg_out_total_loss / self.state.cumulative_pnl.abs()) * 100.0
        } else {
            0.0
        };

        let days_elapsed = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() - self.start_time) as f64 / 86400.0;

        let realistic_apr = if days_elapsed > 0.0 {
            ((self.state.cumulative_pnl / self.state.starting_capital) / (days_elapsed / 365.0)) * 100.0
        } else {
            0.0
        };

        PortfolioMetrics {
            total_trades,
            win_rate,
            cumulative_pnl: self.state.cumulative_pnl,
            pnl_percentage,
            available_capital: self.state.available_capital,
            utilization_pct,
            leg_out_count: self.state.leg_out_count,
            leg_out_loss_pct,
            realistic_apr,
        }
    }

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

    async fn persist_state(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let json = serialize_to_json(&self.state)?;
        redis::cmd("SET")
            .arg("strategy:portfolio:state")
            .arg(json)
            .query_async::<_, ()>(&mut self.redis_conn.clone())
            .await?;

        // Also persist metrics
        let metrics = self.get_portfolio_metrics();
        let metrics_json = serde_json::to_string(&metrics)?;
        redis::cmd("SET")
            .arg("strategy:portfolio:metrics")
            .arg(metrics_json)
            .query_async::<_, ()>(&mut self.redis_conn.clone())
            .await?;

        Ok(())
    }
}
