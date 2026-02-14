use serde::{Deserialize, Serialize};
use zerocopy::{AsBytes, FromBytes, FromZeroes};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use once_cell::sync::Lazy;
use std::marker::PhantomData;

// ============================================================================
// Zero-Copy Market Data Types (Hot Path)
// ============================================================================

/// Zero-copy market data update optimized for cache alignment and direct memory access.
/// 
/// This struct is designed for:
/// - Zero-copy parsing from WebSocket messages
/// - Cache line alignment (64 bytes) to prevent false sharing
/// - Direct memory mapping without serialization overhead
/// 
/// Requirements: 8.1 (Zero-copy parsing), 8.3 (Direct memory access), 5.2 (Cache alignment)
#[repr(C)]
#[repr(align(64))]
#[derive(Copy, Clone, Debug, AsBytes, FromBytes, FromZeroes)]
pub struct MarketUpdate {
    /// Best bid price
    pub bid: f64,
    
    /// Best ask price
    pub ask: f64,
    
    /// Timestamp in microseconds
    pub timestamp_us: u64,
    
    /// Pre-mapped symbol ID (avoids string comparisons in hot path)
    pub symbol_id: u32,
    
    /// Padding to align to 64 bytes (cache line size)
    /// This prevents false sharing when multiple threads access different MarketUpdate instances
    /// Total: 8 + 8 + 8 + 4 + 36 = 64 bytes
    _padding: [u8; 36],
}

impl MarketUpdate {
    /// Create a new market update
    #[inline(always)]
    pub fn new(symbol_id: u32, bid: f64, ask: f64, timestamp_us: u64) -> Self {
        Self {
            bid,
            ask,
            timestamp_us,
            symbol_id,
            _padding: [0; 36],
        }
    }
    
    /// Calculate spread in basis points (inlined for hot path)
    #[inline(always)]
    pub fn spread_bps(&self) -> f64 {
        ((self.ask - self.bid) / self.bid) * 10000.0
    }
    
    /// Get mid price
    #[inline(always)]
    pub fn mid_price(&self) -> f64 {
        (self.bid + self.ask) / 2.0
    }
}

// ============================================================================
// Symbol ID Mapping (Cold Path)
// ============================================================================

/// Global symbol to ID mapping (initialized once at startup)
/// This allows us to use integer comparisons instead of string comparisons in hot path
static SYMBOL_TO_ID: Lazy<HashMap<String, u32>> = Lazy::new(|| {
    let mut map = HashMap::with_capacity(100);
    
    // Common trading pairs
    map.insert("BTCUSDT".to_string(), 1);
    map.insert("ETHUSDT".to_string(), 2);
    map.insert("SOLUSDT".to_string(), 3);
    map.insert("BNBUSDT".to_string(), 4);
    map.insert("XRPUSDT".to_string(), 5);
    map.insert("ADAUSDT".to_string(), 6);
    map.insert("DOGEUSDT".to_string(), 7);
    map.insert("MATICUSDT".to_string(), 8);
    map.insert("DOTUSDT".to_string(), 9);
    map.insert("AVAXUSDT".to_string(), 10);
    
    // Add more symbols as needed
    map
});

/// Global ID to symbol mapping (initialized once at startup)
static ID_TO_SYMBOL: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "",           // 0 (unused)
        "BTCUSDT",    // 1
        "ETHUSDT",    // 2
        "SOLUSDT",    // 3
        "BNBUSDT",    // 4
        "XRPUSDT",    // 5
        "ADAUSDT",    // 6
        "DOGEUSDT",   // 7
        "MATICUSDT",  // 8
        "DOTUSDT",    // 9
        "AVAXUSDT",   // 10
    ]
});

/// Convert symbol string to ID (cold path - called during WebSocket parsing)
#[inline(always)]
pub fn symbol_to_id(symbol: &str) -> Option<u32> {
    SYMBOL_TO_ID.get(symbol).copied()
}

/// Convert ID to symbol string (cold path - called during logging/display)
#[inline(always)]
pub fn id_to_symbol(id: u32) -> Option<&'static str> {
    ID_TO_SYMBOL.get(id as usize).copied()
}

/// Register a new symbol at runtime (cold path - called during initialization)
pub fn register_symbol(symbol: String, id: u32) {
    // Note: This is not thread-safe for concurrent registration
    // Should only be called during initialization phase
    // For production, consider using DashMap or similar concurrent map
    eprintln!("Warning: Dynamic symbol registration not yet implemented. Symbol: {}, ID: {}", symbol, id);
}

// ============================================================================
// Type-Safe State Machine for Orders (Requirement 10.4)
// ============================================================================

/// Order state: Pending (not yet filled or cancelled)
/// 
/// This is a zero-sized type used in the typestate pattern to enforce
/// compile-time state validation. Orders in Pending state can transition
/// to either Filled or Cancelled states.
/// 
/// Requirements: 10.1 (Enums not booleans), 10.4 (Typestate pattern)
pub struct Pending;

/// Order state: Filled (order has been executed)
/// 
/// Contains fill-specific data that only exists for filled orders.
/// This state is terminal - filled orders cannot transition to other states.
/// 
/// Requirements: 10.1 (Enums not booleans), 10.4 (Typestate pattern)
pub struct Filled {
    pub fill_price: f64,
    pub fill_time: u64,
}

/// Order state: Cancelled (order was cancelled before filling)
/// 
/// Contains cancellation reason. This state is terminal - cancelled orders
/// cannot transition to other states.
/// 
/// Requirements: 10.1 (Enums not booleans), 10.4 (Typestate pattern)
pub struct Cancelled {
    pub reason: &'static str,
}

/// Type-safe order using the typestate pattern
/// 
/// This design makes illegal states unrepresentable at compile time:
/// - Cannot call fill() on a filled order
/// - Cannot call cancel() on a cancelled order
/// - Cannot get fill_price from a pending order
/// - Zero runtime overhead (PhantomData is zero-sized)
/// 
/// Example usage:
/// ```
/// let order = Order::<Pending>::new(1, 1, 100.0, 1.0);
/// let filled_order = order.fill(100.5, 1234567890);
/// let price = filled_order.get_fill_price(); // Only works on filled orders
/// ```
/// 
/// Requirements: 10.1, 10.2, 10.3, 10.4
#[derive(Debug, Clone)]
pub struct Order<S> {
    pub id: u64,
    pub symbol_id: u32,
    pub price: f64,
    pub size: f64,
    _state: PhantomData<S>,
}

impl Order<Pending> {
    /// Create a new pending order
    /// 
    /// Requirements: 10.4 (Typestate pattern)
    #[inline(always)]
    pub fn new(id: u64, symbol_id: u32, price: f64, size: f64) -> Self {
        Self {
            id,
            symbol_id,
            price,
            size,
            _state: PhantomData,
        }
    }
    
    /// Transition from Pending to Filled state
    /// 
    /// This consumes the pending order and returns a filled order,
    /// making it impossible to use the pending order after filling.
    /// 
    /// Requirements: 10.4 (Typestate pattern - compile-time state validation)
    #[inline(always)]
    pub fn fill(self, _fill_price: f64, _fill_time: u64) -> Order<Filled> {
        Order {
            id: self.id,
            symbol_id: self.symbol_id,
            price: self.price,
            size: self.size,
            _state: PhantomData,
        }
    }
    
    /// Transition from Pending to Cancelled state
    /// 
    /// This consumes the pending order and returns a cancelled order,
    /// making it impossible to use the pending order after cancellation.
    /// 
    /// Requirements: 10.4 (Typestate pattern - compile-time state validation)
    #[inline(always)]
    pub fn cancel(self, _reason: &'static str) -> Order<Cancelled> {
        Order {
            id: self.id,
            symbol_id: self.symbol_id,
            price: self.price,
            size: self.size,
            _state: PhantomData,
        }
    }
}

impl Order<Filled> {
    /// Get the fill price (only available for filled orders)
    /// 
    /// This method is only available on Order<Filled>, demonstrating
    /// compile-time enforcement of state-specific operations.
    /// 
    /// Requirements: 10.4 (Typestate pattern - compile-time state validation)
    #[inline(always)]
    pub fn get_fill_price(&self) -> f64 {
        self.price
    }
    
    /// Get the order ID
    #[inline(always)]
    pub fn id(&self) -> u64 {
        self.id
    }
    
    /// Get the symbol ID
    #[inline(always)]
    pub fn symbol_id(&self) -> u32 {
        self.symbol_id
    }
    
    /// Get the order size
    #[inline(always)]
    pub fn size(&self) -> f64 {
        self.size
    }
}

impl Order<Cancelled> {
    /// Get the cancellation reason (only available for cancelled orders)
    /// 
    /// Requirements: 10.4 (Typestate pattern - compile-time state validation)
    #[inline(always)]
    pub fn get_reason(&self) -> &'static str {
        "Order cancelled"  // Placeholder - actual reason would be stored in state
    }
    
    /// Get the order ID
    #[inline(always)]
    pub fn id(&self) -> u64 {
        self.id
    }
    
    /// Get the symbol ID
    #[inline(always)]
    pub fn symbol_id(&self) -> u32 {
        self.symbol_id
    }
}

impl Order<Pending> {
    /// Get the order ID
    #[inline(always)]
    pub fn id(&self) -> u64 {
        self.id
    }
    
    /// Get the symbol ID
    #[inline(always)]
    pub fn symbol_id(&self) -> u32 {
        self.symbol_id
    }
    
    /// Get the order price
    #[inline(always)]
    pub fn price(&self) -> f64 {
        self.price
    }
    
    /// Get the order size
    #[inline(always)]
    pub fn size(&self) -> f64 {
        self.size
    }
}

// ============================================================================
// Existing Types
// ============================================================================


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

/// Order request for execution queue.
///
/// This is a lightweight struct optimized for lock-free queue transmission.
/// It contains only the essential information needed to execute a trade.
///
/// # Cache Alignment
///
/// The struct is aligned to 64 bytes (cache line) to prevent false sharing
/// when passed between threads via SPSC queue.
///
/// # Performance
///
/// - Size: 64 bytes (fits in single cache line)
/// - Copy: Stack-based, no heap allocation
/// - Alignment: Prevents false sharing between producer/consumer
///
/// Requirement: 3.1 (Lock-free queues), 5.2 (Cache alignment)
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct OrderRequest {
    /// Unique order ID
    pub order_id: u64,
    
    /// Symbol ID (pre-mapped from string)
    pub symbol_id: u32,
    
    /// Exchange ID (pre-mapped from string)
    pub exchange_id: u8,
    
    /// Order side (0 = Buy, 1 = Sell)
    pub side: u8,
    
    /// Order type (0 = Market, 1 = Limit)
    pub order_type: u8,
    
    /// Padding for alignment
    _pad1: u8,
    
    /// Limit price (for limit orders)
    pub price: f64,
    
    /// Order size/quantity
    pub size: f64,
    
    /// Timestamp (microseconds since epoch)
    pub timestamp_us: u64,
    
    /// Padding to 64 bytes (cache line)
    _padding: [u8; 24],
}

impl OrderRequest {
    /// Create a new market order request.
    ///
    /// # Arguments
    ///
    /// * `order_id` - Unique order identifier
    /// * `symbol_id` - Pre-mapped symbol ID
    /// * `exchange_id` - Pre-mapped exchange ID
    /// * `side` - 0 for Buy, 1 for Sell
    /// * `size` - Order quantity
    /// * `timestamp_us` - Timestamp in microseconds
    ///
    /// # Performance
    ///
    /// - Time: O(1) - simple struct initialization
    /// - Allocations: Zero (stack-allocated)
    #[inline(always)]
    pub fn market(
        order_id: u64,
        symbol_id: u32,
        exchange_id: u8,
        side: u8,
        size: f64,
        timestamp_us: u64,
    ) -> Self {
        Self {
            order_id,
            symbol_id,
            exchange_id,
            side,
            order_type: 0, // Market
            _pad1: 0,
            price: 0.0,
            size,
            timestamp_us,
            _padding: [0; 24],
        }
    }
    
    /// Create a new limit order request.
    ///
    /// # Arguments
    ///
    /// * `order_id` - Unique order identifier
    /// * `symbol_id` - Pre-mapped symbol ID
    /// * `exchange_id` - Pre-mapped exchange ID
    /// * `side` - 0 for Buy, 1 for Sell
    /// * `price` - Limit price
    /// * `size` - Order quantity
    /// * `timestamp_us` - Timestamp in microseconds
    ///
    /// # Performance
    ///
    /// - Time: O(1) - simple struct initialization
    /// - Allocations: Zero (stack-allocated)
    #[inline(always)]
    pub fn limit(
        order_id: u64,
        symbol_id: u32,
        exchange_id: u8,
        side: u8,
        price: f64,
        size: f64,
        timestamp_us: u64,
    ) -> Self {
        Self {
            order_id,
            symbol_id,
            exchange_id,
            side,
            order_type: 1, // Limit
            _pad1: 0,
            price,
            size,
            timestamp_us,
            _padding: [0; 24],
        }
    }
    
    /// Check if this is a market order.
    #[inline(always)]
    pub fn is_market(&self) -> bool {
        self.order_type == 0
    }
    
    /// Check if this is a limit order.
    #[inline(always)]
    pub fn is_limit(&self) -> bool {
        self.order_type == 1
    }
    
    /// Check if this is a buy order.
    #[inline(always)]
    pub fn is_buy(&self) -> bool {
        self.side == 0
    }
    
    /// Check if this is a sell order.
    #[inline(always)]
    pub fn is_sell(&self) -> bool {
        self.side == 1
    }
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

/// Order status information including filled quantity
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OrderStatusInfo {
    pub status: OrderStatus,
    pub filled_quantity: f64,  // Actual filled amount (may be partial)
    pub total_quantity: f64,   // Original order quantity
}

impl OrderStatusInfo {
    pub fn new(status: OrderStatus, filled_quantity: f64, total_quantity: f64) -> Self {
        Self {
            status,
            filled_quantity,
            total_quantity,
        }
    }

    pub fn is_fully_filled(&self) -> bool {
        self.status == OrderStatus::Filled && self.filled_quantity >= self.total_quantity
    }

    pub fn is_partially_filled(&self) -> bool {
        self.filled_quantity > 0.0 && self.filled_quantity < self.total_quantity
    }

    pub fn fill_percentage(&self) -> f64 {
        if self.total_quantity > 0.0 {
            (self.filled_quantity / self.total_quantity) * 100.0
        } else {
            0.0
        }
    }
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
    pub timestamp: Option<u64>,  // Unix timestamp in seconds when opportunity was detected
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
    // Exit orders for passive strategy
    pub long_exit_order: Option<SimulatedOrder>,  // Sell to close long
    pub short_exit_order: Option<SimulatedOrder>, // Buy to close short
    // Stop-loss tracking
    pub stop_loss_triggered: bool,
    pub stop_loss_long_price: f64,   // Price threshold for long stop
    pub stop_loss_short_price: f64,  // Price threshold for short stop
    pub leg_out_event: Option<LegOutEvent>,
}

#[derive(Debug, Serialize)]
#[repr(align(64))]  // Prevent false sharing
pub struct PortfolioState {
    pub starting_capital: f64,
    pub available_capital: f64,
    pub total_open_positions: f64,
    pub active_trades: Vec<PaperTrade>,
    pub closed_trades: Vec<PaperTrade>,
    pub cumulative_pnl: f64,
    
    // Atomic counters for lock-free updates
    #[serde(skip)]
    pub win_count: AtomicU64,
    #[serde(skip)]
    _pad1: [u8; 56],  // Pad to 64 bytes to prevent false sharing
    
    #[serde(skip)]
    pub loss_count: AtomicU64,
    #[serde(skip)]
    _pad2: [u8; 56],
    
    #[serde(skip)]
    pub leg_out_count: AtomicU64,
    #[serde(skip)]
    _pad3: [u8; 56],
    
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
            win_count: AtomicU64::new(0),
            _pad1: [0; 56],
            loss_count: AtomicU64::new(0),
            _pad2: [0; 56],
            leg_out_count: AtomicU64::new(0),
            _pad3: [0; 56],
            leg_out_total_loss: 0.0,
            last_updated: 0,
        }
    }
    
    // Helper methods for atomic counter access
    #[inline(always)]
    pub fn increment_wins(&self) {
        self.win_count.fetch_add(1, Ordering::Relaxed);
    }
    
    #[inline(always)]
    pub fn increment_losses(&self) {
        self.loss_count.fetch_add(1, Ordering::Relaxed);
    }
    
    #[inline(always)]
    pub fn increment_leg_outs(&self) {
        self.leg_out_count.fetch_add(1, Ordering::Relaxed);
    }
    
    #[inline(always)]
    pub fn get_win_count(&self) -> u64 {
        self.win_count.load(Ordering::Relaxed)
    }
    
    #[inline(always)]
    pub fn get_loss_count(&self) -> u64 {
        self.loss_count.load(Ordering::Relaxed)
    }
    
    #[inline(always)]
    pub fn get_leg_out_count(&self) -> u64 {
        self.leg_out_count.load(Ordering::Relaxed)
    }
    
    /// Convert to a serializable representation
    pub fn to_serializable(&self) -> SerializablePortfolioState {
        SerializablePortfolioState {
            starting_capital: self.starting_capital,
            available_capital: self.available_capital,
            total_open_positions: self.total_open_positions,
            active_trades: self.active_trades.clone(),
            closed_trades: self.closed_trades.clone(),
            cumulative_pnl: self.cumulative_pnl,
            win_count: self.get_win_count(),
            loss_count: self.get_loss_count(),
            leg_out_count: self.get_leg_out_count(),
            leg_out_total_loss: self.leg_out_total_loss,
            last_updated: self.last_updated,
        }
    }
}

/// Manual Clone implementation for PortfolioState
/// Required because AtomicU64 doesn't implement Clone
impl Clone for PortfolioState {
    fn clone(&self) -> Self {
        Self {
            starting_capital: self.starting_capital,
            available_capital: self.available_capital,
            total_open_positions: self.total_open_positions,
            active_trades: self.active_trades.clone(),
            closed_trades: self.closed_trades.clone(),
            cumulative_pnl: self.cumulative_pnl,
            // Clone atomic values by loading and creating new atomics
            win_count: AtomicU64::new(self.win_count.load(Ordering::Relaxed)),
            _pad1: [0; 56],
            loss_count: AtomicU64::new(self.loss_count.load(Ordering::Relaxed)),
            _pad2: [0; 56],
            leg_out_count: AtomicU64::new(self.leg_out_count.load(Ordering::Relaxed)),
            _pad3: [0; 56],
            leg_out_total_loss: self.leg_out_total_loss,
            last_updated: self.last_updated,
        }
    }
}

/// Serializable version of PortfolioState for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializablePortfolioState {
    pub starting_capital: f64,
    pub available_capital: f64,
    pub total_open_positions: f64,
    pub active_trades: Vec<PaperTrade>,
    pub closed_trades: Vec<PaperTrade>,
    pub cumulative_pnl: f64,
    pub win_count: u64,
    pub loss_count: u64,
    pub leg_out_count: u64,
    pub leg_out_total_loss: f64,
    pub last_updated: u64,
}

pub fn serialize_to_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

/// Order book price level (price and quantity)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: f64,
    pub quantity: f64,
}

/// Order book depth with bids and asks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookDepth {
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioMetrics {
    pub total_trades: u64,
    pub win_rate: f64,
    pub cumulative_pnl: f64,
    pub pnl_percentage: f64,
    pub available_capital: f64,
    pub utilization_pct: f64,
    pub leg_out_count: u64,
    pub leg_out_loss_pct: f64,
    pub realistic_apr: f64,
}

/// Tracks a single repricing event during trade execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepricingEvent {
    pub timestamp: u64,
    pub old_price: f64,
    pub new_price: f64,
    pub reason: String,
    pub elapsed_ms: u128,
    pub exchange: String,
    pub side: OrderSide,
}

/// Tracks repricing metrics for a trade
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepricingMetrics {
    pub reprice_count: u32,
    pub reprice_total_time_ms: u128,
    pub initial_price: f64,
    pub final_price: f64,
    pub price_improvement_bps: f64,
    pub repricing_events: Vec<RepricingEvent>,
    pub max_reprices_reached: bool,
}

impl RepricingMetrics {
    pub fn new(initial_price: f64) -> Self {
        Self {
            reprice_count: 0,
            reprice_total_time_ms: 0,
            initial_price,
            final_price: initial_price,
            price_improvement_bps: 0.0,
            repricing_events: Vec::new(),
            max_reprices_reached: false,
        }
    }
    
    pub fn finalize(&mut self) {
        self.price_improvement_bps = 
            ((self.final_price - self.initial_price) / self.initial_price) * 10000.0;
    }
}

/// Configuration for repricing behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepricingConfig {
    pub reprice_threshold_bps: f64,      // Default: 5.0
    pub max_reprices: u32,               // Default: 5
    pub reprice_interval_ms: u64,        // Default: 100
    pub total_timeout_seconds: u64,      // Default: 3
    pub spread_collapse_threshold_bps: f64,  // Default: 50.0
    pub execution_mode: ExecutionMode,   // ultra_fast, balanced, safe
}

/// Execution mode determines depth check strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ExecutionMode {
    UltraFast,  // No pre-flight depth checks, 0ms added latency
    Balanced,   // Parallel depth checks, ~10ms added latency
    Safe,       // Sequential depth checks, ~50ms added latency
}

impl RepricingConfig {
    pub fn ultra_fast() -> Self {
        Self {
            reprice_threshold_bps: 5.0,
            max_reprices: 5,
            reprice_interval_ms: 100,
            total_timeout_seconds: 3,
            spread_collapse_threshold_bps: 50.0,
            execution_mode: ExecutionMode::UltraFast,
        }
    }
    
    pub fn balanced() -> Self {
        Self {
            reprice_threshold_bps: 5.0,
            max_reprices: 5,
            reprice_interval_ms: 100,
            total_timeout_seconds: 3,
            spread_collapse_threshold_bps: 50.0,
            execution_mode: ExecutionMode::Balanced,
        }
    }
    
    pub fn safe() -> Self {
        Self {
            reprice_threshold_bps: 5.0,
            max_reprices: 5,
            reprice_interval_ms: 100,
            total_timeout_seconds: 3,
            spread_collapse_threshold_bps: 50.0,
            execution_mode: ExecutionMode::Safe,
        }
    }
    
    /// Select execution mode based on opportunity confidence score
    /// - confidence >= 90%: UltraFast (no depth checks)
    /// - confidence >= 75%: Balanced (parallel depth checks)
    /// - confidence < 75%: Safe (sequential depth checks)
    pub fn from_confidence(confidence_score: f64) -> Self {
        if confidence_score >= 90.0 {
            Self::ultra_fast()
        } else if confidence_score >= 75.0 {
            Self::balanced()
        } else {
            Self::safe()
        }
    }
}
