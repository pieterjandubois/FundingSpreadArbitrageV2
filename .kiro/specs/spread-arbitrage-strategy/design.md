# Spread Arbitrage Strategy - Design Document

## Architecture Overview

The strategy operates as a modular pipeline with execution fidelity as the core principle:

```
Data Collection (Redis) 
    ↓
Exchange Latency Monitoring
    ↓
Confluence Metric Calculation (with Hard Constraints)
    ↓
Opportunity Scanner
    ↓
Trade Entry Decision (Atomic Execution)
    ↓
Position Management (Leg-Out Risk Tracking)
    ↓
Trade Exit & Monitoring
```

## Core Components

### 1. Data Structures

#### ExchangeLatency
```rust
struct ExchangeLatency {
    exchange: String,
    server_time_ms: u64,
    local_time_ms: u64,
    latency_ms: u64,
    is_stale: bool,  // true if latency > 200ms
}
```

#### HardConstraints
```rust
struct HardConstraints {
    order_book_depth_sufficient: bool,  // depth >= 2x position size
    exchange_latency_ok: bool,          // all exchanges < 200ms
    funding_delta_substantial: bool,    // > 0.01% per 8h
    
    fn passes_all(&self) -> bool {
        self.order_book_depth_sufficient && 
        self.exchange_latency_ok && 
        self.funding_delta_substantial
    }
}
```

#### ConfluenceMetrics
```rust
struct ConfluenceMetrics {
    funding_delta: f64,           // FRA - FRB (%)
    funding_delta_projected: f64, // Next funding rate delta
    obi_ratio: f64,               // Order Book Imbalance (-1 to 1)
    oi_current: f64,              // Current Open Interest
    oi_24h_avg: f64,              // 24-hour average OI
    vwap_deviation: f64,          // Standard deviations from VWAP
    atr: f64,                     // Average True Range
    atr_trend: bool,              // true if decreasing (calming)
    liquidation_cluster_distance: f64, // Distance to nearest cluster (%)
    hard_constraints: HardConstraints,
}

impl ConfluenceMetrics {
    fn calculate_confidence_score(&self) -> u8 {
        // If hard constraints fail, return 0
        if !self.hard_constraints.passes_all() {
            return 0;
        }
        
        // Weighted scoring: 0-100 (only soft metrics)
        // Funding Delta: weight 9
        // OBI: weight 8
        // OI: weight 7
        // VWAP: weight 6
        // Volatility: weight 5
        // Liquidation: weight 5
    }
}
```

#### QueuePosition
```rust
struct QueuePosition {
    price: f64,
    cumulative_volume_at_price: f64,
    resting_depth_at_entry: f64,
    fill_threshold_pct: f64,  // 20% of resting depth
    is_filled: bool,
}

impl QueuePosition {
    fn should_fill(&self) -> bool {
        self.cumulative_volume_at_price >= 
            (self.resting_depth_at_entry * self.fill_threshold_pct)
    }
}
```

#### SimulatedOrder
```rust
struct SimulatedOrder {
    id: String,
    exchange: String,
    symbol: String,
    side: OrderSide,  // Long or Short
    order_type: OrderType,  // Limit or Market
    price: f64,
    size: f64,
    queue_position: Option<QueuePosition>,
    created_at: u64,
    filled_at: Option<u64>,
    fill_price: Option<f64>,
    status: OrderStatus,  // Pending, Filled, Cancelled
}

enum OrderSide { Long, Short }
enum OrderType { Limit, Market }
enum OrderStatus { Pending, Filled, Cancelled }
```

#### ArbitrageOpportunity
```rust
struct ArbitrageOpportunity {
    symbol: String,
    long_exchange: String,
    short_exchange: String,
    long_price: f64,
    short_price: f64,
    spread_bps: f64,              // Basis points
    funding_delta_8h: f64,        // 8-hour funding differential
    confidence_score: u8,
    projected_profit_usd: f64,    // After fees and funding
    projected_profit_after_slippage: f64,  // Realistic profit
    metrics: ConfluenceMetrics,
    order_book_depth_long: f64,
    order_book_depth_short: f64,
}
```

#### PaperTrade
```rust
struct PaperTrade {
    id: String,                   // UUID
    symbol: String,
    long_exchange: String,
    short_exchange: String,
    entry_time: u64,              // Unix timestamp
    entry_long_price: f64,
    entry_short_price: f64,
    entry_spread_bps: f64,
    position_size_usd: f64,       // Capital allocated
    funding_delta_entry: f64,
    projected_profit_usd: f64,
    actual_profit_usd: f64,
    status: TradeStatus,
    exit_reason: Option<String>,
    exit_time: Option<u64>,
    
    // Execution tracking
    long_order: SimulatedOrder,
    short_order: SimulatedOrder,
    leg_out_event: Option<LegOutEvent>,
}

struct LegOutEvent {
    filled_leg: String,  // "long" or "short"
    filled_at: u64,
    unfilled_leg: String,
    hedge_executed: bool,
    hedge_price: f64,
}

enum TradeStatus {
    Pending,      // Waiting for fills
    Active,       // Both sides filled
    Exiting,      // Exit orders placed
    Closed,       // Trade complete
}
```

#### PortfolioState
```rust
struct PortfolioState {
    starting_capital: f64,        // $20,000
    available_capital: f64,
    total_open_positions: f64,    // USD value
    active_trades: Vec<PaperTrade>,
    closed_trades: Vec<PaperTrade>,
    cumulative_pnl: f64,
    win_count: u32,
    loss_count: u32,
    leg_out_count: u32,
    leg_out_total_loss: f64,
}
```

### 2. Exchange Latency Monitoring Module

**Location:** `src/strategy/latency.rs`

#### Latency Tracking
- Poll each exchange's server time every 100ms
- Calculate: `latency = local_time - server_time`
- Maintain rolling 10-second average
- Flag as stale if latency > 200ms

#### Confidence Impact
- If any exchange latency > 200ms: reduce confidence by 30 points
- If all exchanges latency > 200ms: confidence = 0
- Log all latency violations

### 3. Confluence Metric Calculation Module

**Location:** `src/strategy/confluence.rs`

#### Hard Constraints Check (First)
```
IF order_book_depth < 2x position_size:
    confidence_score = 0
    return

IF any_exchange_latency > 200ms:
    confidence_score = 0
    return

IF funding_delta <= 0.01% per 8h:
    confidence_score = 0
    return
```

#### Soft Metrics (Only if Hard Constraints Pass)
- Funding Rate Differential: weight 9/50
- Order Book Imbalance: weight 8/50
- Open Interest: weight 7/50
- VWAP Deviation: weight 6/50
- Volatility (ATR): weight 5/50
- Liquidation Clusters: weight 5/50

#### Realistic Slippage Calculation
```rust
fn calculate_slippage(position_size: f64, order_book_depth: f64) -> f64 {
    let base_slippage = 0.0002;  // 2 bps
    let depth_ratio = position_size / order_book_depth;
    let additional_slippage = depth_ratio * 0.0003;  // Up to 3 bps
    
    (base_slippage + additional_slippage).min(0.0005)  // Cap at 5 bps
}
```

### 4. Opportunity Scanner Module

**Location:** `src/strategy/scanner.rs`

#### Scanning Loop
- Runs every 1 second
- Iterates through all USDT pairs in Redis
- For each pair, finds highest and lowest prices across exchanges
- Calculates spread in basis points

#### Filtering Criteria
1. Hard constraints pass (latency, depth, funding delta)
2. Spread > fees + funding costs (minimum 5 bps)
3. Confidence score ≥ 70
4. Projected profit after slippage > 0
5. Both exchanges have sufficient order book depth (>$50k at 2 bps)

#### Ranking & Output
- Rank by confidence score (descending)
- Log top 20 opportunities to Redis and stdout
- Store in Redis: `strategy:opportunities:{timestamp}` (TTL: 60s)

### 5. Trade Entry Module with Atomic Execution

**Location:** `src/strategy/entry.rs`

#### Entry Decision Logic
```
IF confidence_score >= 70 AND available_capital >= position_size:
    1. Identify harder leg (typically smaller exchange)
    2. Calculate realistic slippage
    3. Recalculate projected profit after slippage
    4. IF projected_profit < 0: reject trade
    5. Create PaperTrade with status=Pending
    6. Place limit order on harder leg first
    7. Wait up to 500ms for harder leg fill
    8. IF harder leg fills:
        - Place limit order on easier leg
        - Wait up to 500ms for easier leg fill
        - IF easier leg fills: status=Active
        - IF easier leg doesn't fill: hedge with market order
    9. IF harder leg doesn't fill within 500ms:
        - Cancel easier leg (if placed)
        - Reject trade
    10. Deduct capital from available pool
```

#### Harder Leg Identification
```rust
fn identify_harder_leg(long_exchange: &str, short_exchange: &str) -> String {
    // Smaller exchanges are harder to fill
    // DEXes (Hyperliquid, Paradex) are harder than CEXes
    // Bitget < Gate < KuCoin < OKX < Bybit < Binance
    
    match (long_exchange, short_exchange) {
        ("binance", _) => short_exchange.to_string(),
        (_, "binance") => long_exchange.to_string(),
        // ... etc
    }
}
```

#### Position Sizing
- Base size: `(spread_bps - fees - funding_cost) / spread_bps * available_capital`
- Cap: max 50% of available capital per trade
- Adjust for order book depth impact
- Minimum: $100 (to avoid dust)

#### Queue Position Tracking
- Track cumulative volume at limit price
- Fill only when cumulative volume ≥ 20% of resting depth at entry
- Update every 100ms from order book data
- Prevents over-reporting profits by 30-50%

### 6. Position Management Module

**Location:** `src/strategy/positions.rs`

#### Active Position Tracking
- Store all active trades in memory (backed by Redis)
- Update current spread every second
- Calculate unrealized PnL
- Monitor exit conditions continuously

#### Leg-Out Risk Detection
```
EVERY 100ms:
    IF long_order.filled AND NOT short_order.filled:
        IF time_since_long_fill > 500ms:
            - Log leg-out event
            - Execute market order on short side
            - Update trade.leg_out_event
            - Increment portfolio.leg_out_count
    
    IF short_order.filled AND NOT long_order.filled:
        IF time_since_short_fill > 500ms:
            - Log leg-out event
            - Execute market order on long side
            - Update trade.leg_out_event
            - Increment portfolio.leg_out_count
```

#### Exit Condition Monitoring
1. **Funding Rate Convergence**: delta < 0.005% per 8h
2. **Profit Target**: 90% of projected profit realized
3. **Spread Widening**: spread > entry_spread + 50 bps
4. **Stop Loss**: spread > entry_spread + 100 bps
5. **Leg-Out Hedge**: if one side fills, immediately hedge other

### 7. Trade Exit Module

**Location:** `src/strategy/exit.rs`

#### Exit Execution
```
WHEN exit_condition_triggered:
    1. Place exit limit orders on both sides
    2. Track queue position for both orders
    3. Wait up to 500ms for fills
    4. IF one side fills and other doesn't:
        - Execute market order on unfilled side
        - Log as leg-out exit event
    5. Calculate actual profit/loss
    6. Update trade status to Closed
    7. Return capital to available pool
    8. Log trade to persistent storage
    9. Update portfolio metrics
```

#### Exit Order Fills
- Use same queue position tracking as entry
- Fill only if 20% of resting depth trades
- If one side doesn't fill in 500ms, use market order

### 8. Portfolio Management Module

**Location:** `src/strategy/portfolio.rs`

#### State Management
- Initialize with $20,000 capital
- Track available capital (starting - open positions)
- Maintain list of active and closed trades
- Calculate cumulative PnL
- Track leg-out events separately

#### Metrics Calculation
- Win rate: `win_count / (win_count + loss_count)`
- Average profit: `cumulative_pnl / total_trades`
- Capital utilization: `total_open_positions / starting_capital`
- Leg-out loss rate: `leg_out_total_loss / cumulative_pnl`
- Realistic APR: `(cumulative_pnl / starting_capital) / (days_elapsed / 365)`

#### Persistence
- Store all closed trades to Redis: `strategy:trades:closed:{trade_id}`
- Store portfolio state: `strategy:portfolio:state`
- Update every trade completion

### 9. Monitoring & Logging Module

**Location:** `src/strategy/monitor.rs`

#### Real-time Display
- Current available capital
- Active positions (count, total USD)
- Top 5 opportunities (symbol, spread, confidence)
- Portfolio metrics (PnL, win rate, utilization, leg-out count)
- Exchange latency status
- Update frequency: 5 seconds

#### Logging
- All trades: entry/exit prices, profit, reason, leg-out events
- Rejected opportunities: why rejected (hard constraint failure)
- Risk violations: what limit was breached
- Leg-out events: which leg filled, hedge execution
- Latency violations: which exchange, latency value
- Errors: any calculation or data issues

## Data Flow

### Redis Schema
```
# Current market data (from existing collectors)
binance:linear:tickers:{symbol}
bybit:linear:tickers:{symbol}
kucoin:futures:tickers:{symbol}
okx:swap:tickers:{symbol}
bitget:usdt:tickers:{symbol}
gate:futures:tickers:{symbol}
hyperliquid:perps:tickers:{symbol}
paradex:perps:tickers:{symbol}

# Strategy data
strategy:opportunities:{timestamp}        # Top 20 opportunities
strategy:portfolio:state                  # Current portfolio state
strategy:trades:active:{trade_id}         # Active trades
strategy:trades:closed:{trade_id}         # Closed trades (historical)
strategy:metrics:daily                    # Daily performance metrics
strategy:latency:{exchange}               # Exchange latency tracking
strategy:leg_out_events                   # Leg-out risk events
```

## Correctness Properties

### Property 1: Capital Conservation
**Validates: Requirements 3.6, 5.1, 5.2**

For all trades, the sum of available capital + open positions must equal starting capital.

```
available_capital + sum(position_size for all active trades) == starting_capital
```

### Property 2: Hard Constraints Enforcement
**Validates: Requirements 1.7, 1.8, 1.9**

If any hard constraint fails, confidence score must be 0 and trade must be rejected.

```
(NOT hard_constraints.passes_all()) => confidence_score == 0 AND trade_rejected
```

### Property 3: Atomic Execution
**Validates: Requirements 3.7, 3.10, 4.4**

If one leg fills, the other must fill or be hedged within 500ms. No naked positions allowed.

```
(leg_a.filled AND NOT leg_b.filled) => 
    (time_since_leg_a_fill <= 500ms AND (leg_b.filled OR hedge_executed))
```

### Property 4: Queue Position Accuracy
**Validates: Requirements 3.2, 4.5**

Limit orders only fill when cumulative volume at price ≥ 20% of resting depth.

```
order.filled => cumulative_volume_at_price >= (resting_depth * 0.20)
```

### Property 5: Slippage Realism
**Validates: Requirements 3.8, 5.4**

Actual slippage must be between 2-5 bps based on position size vs order book depth.

```
0.0002 <= actual_slippage <= 0.0005
actual_slippage >= (position_size / order_book_depth) * 0.0003
```

### Property 6: Leg-Out Risk Tracking
**Validates: Requirements 4.8, 5.7, 6.7**

All leg-out events must be logged and tracked separately from normal exits.

```
(leg_a.filled AND NOT leg_b.filled AND time > 500ms) => 
    leg_out_event.logged AND portfolio.leg_out_count++
```

### Property 7: PnL Accuracy
**Validates: Requirements 6.2, 6.3, 6.9**

Cumulative PnL must equal sum of all closed trade profits, accounting for leg-out losses.

```
cumulative_pnl == sum(actual_profit for all closed trades) - leg_out_total_loss
```

### Property 8: Latency Impact
**Validates: Requirements 1.11, 5.4**

If exchange latency >200ms, confidence score must be reduced by 30 points or set to 0.

```
(exchange_latency > 200ms) => (confidence_score == 0 OR confidence_score -= 30)
```

## Implementation Strategy

### Phase 1: Core Infrastructure
1. Create data structures (all structs above)
2. Implement Redis schema and serialization
3. Create portfolio state manager
4. Implement latency monitoring

### Phase 2: Confluence Metrics with Hard Constraints
1. Implement hard constraints checker
2. Implement each soft metric calculator
3. Integrate with Redis data
4. Implement confidence score calculation (gatekeeper logic)

### Phase 3: Queue Position Tracking
1. Implement queue position tracking
2. Implement realistic slippage calculation
3. Integrate with order book data

### Phase 4: Opportunity Scanner
1. Implement scanning loop
2. Implement filtering and ranking
3. Implement Redis logging

### Phase 5: Atomic Trade Execution
1. Implement harder leg identification
2. Implement entry logic with 500ms timeout
3. Implement leg-out detection and hedging
4. Implement exit logic with atomic guarantees

### Phase 6: Monitoring & Persistence
1. Implement real-time monitoring
2. Implement trade logging
3. Implement portfolio metrics
4. Implement leg-out event tracking

## Testing Strategy

- Unit tests for each metric calculator
- Unit tests for hard constraints logic
- Unit tests for queue position tracking
- Integration tests for atomic execution
- Property-based tests for capital conservation and PnL accuracy
- Property-based tests for leg-out risk detection
- Simulation tests with historical data
