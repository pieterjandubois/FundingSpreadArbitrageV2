# Paper Trading Monitor - Design Document

## Architecture Overview

The paper trading system operates as an integrated component of the strategy runner:

```
Strategy Runner (scanning loop)
    ↓
Opportunity Detection (confidence ≥ 70, profit > 0)
    ↓
Trade Entry Executor
    ├─ Allocate capital
    ├─ Calculate position size
    ├─ Execute atomic dual-leg entry
    └─ Track active trade
    ↓
Position Manager (continuous monitoring)
    ├─ Update current prices
    ├─ Calculate unrealized P&L
    ├─ Check exit conditions
    └─ Detect leg-out risk
    ↓
Trade Exit Executor
    ├─ Execute exit orders
    ├─ Calculate actual profit
    └─ Return capital to pool
    ↓
Portfolio Manager (state persistence)
    ├─ Track closed trades
    ├─ Calculate metrics
    └─ Store to Redis
    ↓
Monitor Binary (real-time display)
    ├─ Display active trades
    ├─ Display portfolio metrics
    └─ Update every 1 second
```

## Core Data Structures

### PaperTrade
```rust
struct PaperTrade {
    id: String,                       // UUID
    symbol: String,
    long_exchange: String,
    short_exchange: String,
    entry_time: u64,                  // Unix timestamp
    entry_long_price: f64,
    entry_short_price: f64,
    entry_spread_bps: f64,
    position_size_usd: f64,           // Capital allocated
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

enum TradeStatus {
    Pending,      // Waiting for fills
    Active,       // Both sides filled
    Exiting,      // Exit orders placed
    Closed,       // Trade complete
}
```

### SimulatedOrder
```rust
struct SimulatedOrder {
    id: String,
    exchange: String,
    symbol: String,
    side: OrderSide,              // Long or Short
    order_type: OrderType,        // Limit or Market
    price: f64,
    size: f64,
    queue_position: Option<QueuePosition>,
    created_at: u64,
    filled_at: Option<u64>,
    fill_price: Option<f64>,
    status: OrderStatus,
}

enum OrderSide { Long, Short }
enum OrderType { Limit, Market }
enum OrderStatus { Pending, Filled, Cancelled }
```

### QueuePosition
```rust
struct QueuePosition {
    price: f64,
    cumulative_volume_at_price: f64,
    resting_depth_at_entry: f64,
    fill_threshold_pct: f64,      // 20% of resting depth
    is_filled: bool,
}

impl QueuePosition {
    fn should_fill(&self) -> bool {
        self.cumulative_volume_at_price >= 
            (self.resting_depth_at_entry * self.fill_threshold_pct)
    }
}
```

### LegOutEvent
```rust
struct LegOutEvent {
    filled_leg: String,           // "long" or "short"
    filled_at: u64,
    unfilled_leg: String,
    hedge_executed: bool,
    hedge_price: f64,
}
```

### PortfolioState
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

## Module Responsibilities

### 1. Entry Executor (`src/strategy/entry.rs`)

**Responsibilities:**
- Validate trade opportunity meets criteria
- Calculate position size based on spread and available capital
- Execute atomic dual-leg entry
- Track queue positions for both orders
- Implement 500ms timeout for harder leg fill

**Key Functions:**
```rust
pub async fn execute_entry(
    opportunity: &ArbitrageOpportunity,
    portfolio: &mut PortfolioManager,
) -> Result<PaperTrade, String>

fn calculate_position_size(
    spread_bps: f64,
    available_capital: f64,
    fees_bps: f64,
    funding_cost_bps: f64,
) -> f64

fn identify_harder_leg(long_ex: &str, short_ex: &str) -> String

fn calculate_slippage(position_size: f64, order_book_depth: f64) -> f64
```

**Position Sizing Logic:**
```
base_size = (spread_bps - fees - funding_cost) / spread_bps * available_capital
capped_size = min(base_size, available_capital * 0.5)
final_size = max(capped_size, 100.0)  // Minimum $100
```

### 2. Position Manager (`src/strategy/positions.rs`)

**Responsibilities:**
- Monitor active trades continuously
- Update current prices and spreads
- Calculate unrealized P&L
- Check exit conditions
- Detect leg-out risk

**Key Functions:**
```rust
pub fn calculate_unrealized_pnl(
    entry_long_price: f64,
    entry_short_price: f64,
    current_long_price: f64,
    current_short_price: f64,
    position_size: f64,
) -> f64

pub fn check_exit_conditions(
    trade: &PaperTrade,
    current_funding_delta: f64,
    current_spread_bps: f64,
    unrealized_pnl: f64,
) -> Option<String>  // Returns exit reason if should exit

pub fn detect_leg_out(
    long_filled: bool,
    short_filled: bool,
    time_since_entry_ms: u64,
) -> bool
```

**Exit Condition Checks:**
1. Profit target: `unrealized_pnl >= projected_profit * 0.9`
2. Loss limit: `unrealized_pnl <= -projected_profit * 0.3`
3. Funding convergence: `current_funding_delta < 0.005%`
4. Leg-out timeout: `time_since_entry > 500ms AND one_leg_filled AND other_not_filled`

### 3. Portfolio Manager (`src/strategy/portfolio.rs`)

**Responsibilities:**
- Manage capital allocation
- Track active and closed trades
- Calculate portfolio metrics
- Persist state to Redis

**Key Functions:**
```rust
pub async fn open_trade(
    &mut self,
    trade: PaperTrade,
) -> Result<(), String>

pub async fn close_trade(
    &mut self,
    trade_id: &str,
    actual_profit: f64,
    exit_reason: String,
) -> Result<(), String>

pub fn get_available_capital(&self) -> f64

pub fn get_portfolio_metrics(&self) -> PortfolioMetrics
```

**Metrics Calculation:**
```rust
struct PortfolioMetrics {
    total_trades: u32,
    win_rate: f64,                    // win_count / total_trades
    cumulative_pnl: f64,
    pnl_percentage: f64,              // cumulative_pnl / starting_capital
    available_capital: f64,
    utilization_pct: f64,             // total_open_positions / starting_capital
    leg_out_count: u32,
    leg_out_loss_pct: f64,            // leg_out_total_loss / cumulative_pnl
    realistic_apr: f64,               // (cumulative_pnl / starting_capital) / (days_elapsed / 365)
}
```

### 4. Monitor Binary (`src/bin/trading-monitor.rs`)

**Responsibilities:**
- Display active trades in real-time
- Display portfolio metrics
- Update every 1 second
- Color-code P&L (green/red)

**Display Layout:**
```
┌─ Portfolio Summary ─────────────────────────────────────────┐
│ Capital: $20,000 | Available: $15,000 | Utilization: 25%   │
│ Trades: 5 | Win Rate: 80% | Cumulative P&L: +$250 (+1.25%) │
│ Realistic APR: 45.6% | Leg-Out Events: 1                   │
└─────────────────────────────────────────────────────────────┘

┌─ Active Trades (3 total) ───────────────────────────────────┐
│ Ticker | Entry Spread | Current Spread | Unrealized P&L    │
│ BTCUSDT | 5.2 bps | 3.1 bps | +$45.20 (green)             │
│ ETHUSDT | 8.1 bps | 6.5 bps | +$32.10 (green)             │
│ BNBUSDT | 12.3 bps | 15.2 bps | -$18.50 (red)             │
└─────────────────────────────────────────────────────────────┘

┌─ Recent Exits ──────────────────────────────────────────────┐
│ ADAUSDT | Closed | Profit: +$28.50 | Reason: profit_target │
│ DOGEUSDT | Closed | Profit: -$12.30 | Reason: loss_limit   │
└─────────────────────────────────────────────────────────────┘
```

## Data Flow

### Entry Flow
```
1. Opportunity detected (confidence ≥ 70, profit > 0)
2. Entry executor validates opportunity
3. Calculate position size
4. Identify harder leg
5. Place limit order on harder leg
6. Wait up to 500ms for fill
7. If filled: place limit order on easier leg
8. Wait up to 500ms for fill
9. If both filled: create PaperTrade with status=Active
10. If one doesn't fill: cancel other and reject trade
11. Deduct capital from available pool
12. Add to active_trades list
```

### Monitoring Flow
```
Every 1 second:
1. For each active trade:
   a. Fetch current prices
   b. Calculate current spread
   c. Calculate unrealized P&L
   d. Check exit conditions
   e. Check leg-out risk
2. For each exiting trade:
   a. Check if exit orders filled
   b. If both filled: close trade
   c. Return capital to pool
3. Update portfolio metrics
4. Persist state to Redis
5. Display in monitor binary
```

### Exit Flow
```
1. Exit condition triggered (profit target, loss limit, etc.)
2. Place exit limit orders on both sides
3. Wait up to 500ms for fills
4. If one side fills and other doesn't:
   a. Execute market order on unfilled side
   b. Log as leg-out exit event
5. Calculate actual profit/loss
6. Update trade status to Closed
7. Return capital to available pool
8. Log trade to persistent storage
9. Update portfolio metrics
```

## Redis Schema

```
# Active trades
strategy:trades:active:{trade_id}         # PaperTrade JSON

# Closed trades (historical)
strategy:trades:closed:{trade_id}         # PaperTrade JSON

# Portfolio state
strategy:portfolio:state                  # PortfolioState JSON

# Metrics
strategy:metrics:daily                    # Daily performance metrics
strategy:leg_out_events                   # Leg-out risk events

# Monitoring
strategy:monitor:active_trades            # List of active trade IDs
strategy:monitor:portfolio_metrics        # Current portfolio metrics
```

## Correctness Properties

### Property 1: Capital Conservation
**Validates: Requirements 1.5, 4.6**

For all trades, the sum of available capital + open positions must equal starting capital.

```
available_capital + sum(position_size for all active trades) == starting_capital
```

### Property 2: Atomic Execution
**Validates: Requirements 5.4, 5.6**

If one leg fills, the other must fill or be hedged within 500ms. No naked positions allowed.

```
(leg_a.filled AND NOT leg_b.filled) => 
    (time_since_leg_a_fill <= 500ms AND (leg_b.filled OR hedge_executed))
```

### Property 3: Queue Position Accuracy
**Validates: Requirements 5.2, 5.3**

Limit orders only fill when cumulative volume at price ≥ 20% of resting depth.

```
order.filled => cumulative_volume_at_price >= (resting_depth * 0.20)
```

### Property 4: Slippage Realism
**Validates: Requirements 5.3**

Actual slippage must be between 2-5 bps based on position size vs order book depth.

```
0.0002 <= actual_slippage <= 0.0005
actual_slippage >= (position_size / order_book_depth) * 0.0003
```

### Property 5: PnL Accuracy
**Validates: Requirements 4.3, 4.9**

Cumulative P&L must equal sum of all closed trade profits, accounting for leg-out losses.

```
cumulative_pnl == sum(actual_profit for all closed trades) - leg_out_total_loss
```

### Property 6: Exit Condition Enforcement
**Validates: Requirements 3.1, 3.2, 3.3**

Trades must exit when any exit condition is met.

```
(unrealized_pnl >= projected_profit * 0.9) => trade_exits
(unrealized_pnl <= -projected_profit * 0.3) => trade_exits
(funding_delta < 0.005%) => trade_exits
```

### Property 7: Leg-Out Detection
**Validates: Requirements 5.5, 5.6**

All leg-out events must be logged and tracked separately.

```
(leg_a.filled AND NOT leg_b.filled AND time > 500ms) => 
    leg_out_event.logged AND portfolio.leg_out_count++
```

## Implementation Strategy

### Phase 1: Core Data Structures
1. Define all structs (PaperTrade, SimulatedOrder, etc.)
2. Implement serialization/deserialization
3. Create Redis schema

### Phase 2: Entry Execution
1. Implement position sizing logic
2. Implement harder leg identification
3. Implement atomic dual-leg entry
4. Implement queue position tracking

### Phase 3: Position Management
1. Implement unrealized P&L calculation
2. Implement exit condition checking
3. Implement leg-out detection
4. Implement continuous monitoring loop

### Phase 4: Portfolio Management
1. Implement capital tracking
2. Implement metrics calculation
3. Implement state persistence
4. Implement trade logging

### Phase 5: Monitor Binary
1. Create real-time display
2. Implement color-coding
3. Implement 1-second update loop
4. Implement scrolling for large trade lists

## Testing Strategy

- Unit tests for position sizing logic
- Unit tests for P&L calculation
- Unit tests for exit condition checking
- Integration tests for atomic execution
- Property-based tests for capital conservation
- Property-based tests for PnL accuracy
- Simulation tests with historical data

</content>
</invoke>