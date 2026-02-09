use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardConstraints {
    pub order_book_depth_sufficient: bool,
    pub exchange_latency_ok: bool,
    pub funding_delta_substantial: bool,
}

impl HardConstraints {
    pub fn passes_all(&self) -> bool {
        self.order_book_depth_sufficient && self.exchange_latency_ok && self.funding_delta_substantial
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfluenceMetrics {
    pub funding_delta: f64,
    pub funding_delta_projected: f64,
    pub obi_ratio: f64,
    pub oi_current: f64,
    pub oi_24h_avg: f64,
    pub vwap_deviation: f64,
    pub atr: f64,
    pub atr_trend: bool,
    pub liquidation_cluster_distance: f64,
    pub hard_constraints: HardConstraints,
}

impl ConfluenceMetrics {
    pub fn calculate_confidence_score(&self) -> u8 {
        if !self.hard_constraints.passes_all() {
            return 0;
        }

        let mut score = 0.0;
        let weights = [9.0, 8.0, 7.0, 6.0, 5.0, 5.0];
        let total_weight = weights.iter().sum::<f64>();

        // Funding Delta: weight 9 (higher delta = higher score)
        let funding_score = (self.funding_delta.abs() / 0.01).min(1.0) * 100.0;
        score += (funding_score * weights[0]) / total_weight;

        // OBI: weight 8 (extreme imbalance = higher score)
        let obi_score = self.obi_ratio.abs().min(1.0) * 100.0;
        score += (obi_score * weights[1]) / total_weight;

        // OI: weight 7 (OI above average = higher score)
        let oi_score = if self.oi_current > self.oi_24h_avg {
            ((self.oi_current / self.oi_24h_avg - 1.0).min(0.5) / 0.5) * 100.0
        } else {
            0.0
        };
        score += (oi_score * weights[2]) / total_weight;

        // VWAP: weight 6 (deviation from VWAP = higher score)
        let vwap_score = self.vwap_deviation.abs().min(3.0) / 3.0 * 100.0;
        score += (vwap_score * weights[3]) / total_weight;

        // ATR Trend: weight 5 (calming trend = higher score)
        let atr_score = if self.atr_trend { 100.0 } else { 0.0 };
        score += (atr_score * weights[4]) / total_weight;

        // Liquidation: weight 5 (proximity to clusters = higher score)
        let liq_score = (1.0 - (self.liquidation_cluster_distance / 100.0).min(1.0)) * 100.0;
        score += (liq_score * weights[5]) / total_weight;

        (score as u8).min(100)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuePosition {
    pub price: f64,
    pub cumulative_volume_at_price: f64,
    pub resting_depth_at_entry: f64,
    pub fill_threshold_pct: f64,
    pub is_filled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum OrderSide {
    #[default]
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum OrderType {
    #[default]
    Limit,
    Market,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum OrderStatus {
    #[default]
    Pending,
    Filled,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SimulatedOrder {
    pub id: String,
    pub exchange: String,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub price: f64,
    pub size: f64,
    pub queue_position: Option<QueuePosition>,
    pub created_at: u64,
    pub filled_at: Option<u64>,
    pub fill_price: Option<f64>,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    pub symbol: String,
    pub long_exchange: String,
    pub short_exchange: String,
    pub long_price: f64,
    pub short_price: f64,
    pub spread_bps: f64,
    pub funding_delta_8h: f64,
    pub confidence_score: u8,
    pub projected_profit_usd: f64,
    pub projected_profit_after_slippage: f64,
    pub metrics: ConfluenceMetrics,
    pub order_book_depth_long: f64,
    pub order_book_depth_short: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TradeStatus {
    Pending,
    Active,
    Exiting,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegOutEvent {
    pub filled_leg: String,
    pub filled_at: u64,
    pub unfilled_leg: String,
    pub hedge_executed: bool,
    pub hedge_price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTrade {
    pub id: String,
    pub symbol: String,
    pub long_exchange: String,
    pub short_exchange: String,
    pub entry_time: u64,
    pub entry_long_price: f64,
    pub entry_short_price: f64,
    pub entry_spread_bps: f64,
    pub position_size_usd: f64,
    pub funding_delta_entry: f64,
    pub projected_profit_usd: f64,
    pub actual_profit_usd: f64,
    pub status: TradeStatus,
    pub exit_reason: Option<String>,
    pub exit_spread_bps: Option<f64>,
    pub exit_time: Option<u64>,
    pub long_order: SimulatedOrder,
    pub short_order: SimulatedOrder,
    pub leg_out_event: Option<LegOutEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioState {
    pub starting_capital: f64,
    pub available_capital: f64,
    pub total_open_positions: f64,
    pub active_trades: Vec<PaperTrade>,
    pub closed_trades: Vec<PaperTrade>,
    pub cumulative_pnl: f64,
    pub win_count: u32,
    pub loss_count: u32,
    pub leg_out_count: u32,
    pub leg_out_total_loss: f64,
    pub last_updated: u64,
}

impl PortfolioState {
    pub fn new(starting_capital: f64) -> Self {
        Self {
            starting_capital,
            available_capital: starting_capital,
            total_open_positions: 0.0,
            active_trades: Vec::new(),
            closed_trades: Vec::new(),
            cumulative_pnl: 0.0,
            win_count: 0,
            loss_count: 0,
            leg_out_count: 0,
            leg_out_total_loss: 0.0,
            last_updated: 0,
        }
    }
}

pub fn serialize_to_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioMetrics {
    pub total_trades: u32,
    pub win_rate: f64,
    pub cumulative_pnl: f64,
    pub pnl_percentage: f64,
    pub available_capital: f64,
    pub utilization_pct: f64,
    pub leg_out_count: u32,
    pub leg_out_loss_pct: f64,
    pub realistic_apr: f64,
}
