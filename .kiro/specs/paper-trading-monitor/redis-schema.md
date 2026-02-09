# Redis Schema Documentation - Paper Trading Monitor

## Overview

This document describes the Redis schema used by the paper trading system to store and manage trade data, portfolio state, and performance metrics. The schema is designed for efficient access patterns, real-time monitoring, and historical analysis.

## Key Patterns and Purposes

### 1. Active Trades

**Key Pattern:** `strategy:trades:active:{trade_id}`

**Purpose:** Store currently open trades that are being monitored for exit conditions.

**Data Type:** String (JSON)

**TTL/Expiration:** None (persists until trade is closed)

**Access Patterns:**
- Retrieve single active trade by ID
- Iterate over all active trades for monitoring loop
- Update trade state (prices, P&L, status)
- Move trade from active to closed when exit condition met

**Example JSON Structure:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "symbol": "BTCUSDT",
  "long_exchange": "binance",
  "short_exchange": "bybit",
  "entry_time": 1704067200,
  "entry_long_price": 42500.50,
  "entry_short_price": 42495.30,
  "entry_spread_bps": 12.2,
  "position_size_usd": 5000.0,
  "funding_delta_entry": 0.0015,
  "projected_profit_usd": 61.0,
  "actual_profit_usd": null,
  "status": "Active",
  "exit_reason": null,
  "exit_time": null,
  "long_order": {
    "id": "order_long_001",
    "exchange": "binance",
    "symbol": "BTCUSDT",
    "side": "Long",
    "order_type": "Limit",
    "price": 42500.50,
    "size": 0.1176,
    "queue_position": {
      "price": 42500.50,
      "cumulative_volume_at_price": 2.5,
      "resting_depth_at_entry": 15.0,
      "fill_threshold_pct": 0.20,
      "is_filled": true
    },
    "created_at": 1704067200,
    "filled_at": 1704067201,
    "fill_price": 42500.50,
    "status": "Filled"
  },
  "short_order": {
    "id": "order_short_001",
    "exchange": "bybit",
    "symbol": "BTCUSDT",
    "side": "Short",
    "order_type": "Limit",
    "price": 42495.30,
    "size": 0.1176,
    "queue_position": {
      "price": 42495.30,
      "cumulative_volume_at_price": 3.2,
      "resting_depth_at_entry": 18.0,
      "fill_threshold_pct": 0.20,
      "is_filled": true
    },
    "created_at": 1704067200,
    "filled_at": 1704067202,
    "fill_price": 42495.30,
    "status": "Filled"
  },
  "leg_out_event": null
}
```

### 2. Closed Trades (Historical)

**Key Pattern:** `strategy:trades:closed:{trade_id}`

**Purpose:** Store completed trades for historical analysis, performance tracking, and audit trail.

**Data Type:** String (JSON)

**TTL/Expiration:** None (persists indefinitely for historical analysis)

**Access Patterns:**
- Retrieve single closed trade by ID
- Iterate over all closed trades to calculate metrics
- Filter closed trades by date range
- Analyze win/loss distribution

**Example JSON Structure:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "symbol": "BTCUSDT",
  "long_exchange": "binance",
  "short_exchange": "bybit",
  "entry_time": 1704067200,
  "entry_long_price": 42500.50,
  "entry_short_price": 42495.30,
  "entry_spread_bps": 12.2,
  "position_size_usd": 5000.0,
  "funding_delta_entry": 0.0015,
  "projected_profit_usd": 61.0,
  "actual_profit_usd": 48.50,
  "status": "Closed",
  "exit_reason": "profit_target",
  "exit_time": 1704067320,
  "long_order": {
    "id": "order_long_001",
    "exchange": "binance",
    "symbol": "BTCUSDT",
    "side": "Long",
    "order_type": "Limit",
    "price": 42500.50,
    "size": 0.1176,
    "queue_position": {
      "price": 42500.50,
      "cumulative_volume_at_price": 2.5,
      "resting_depth_at_entry": 15.0,
      "fill_threshold_pct": 0.20,
      "is_filled": true
    },
    "created_at": 1704067200,
    "filled_at": 1704067201,
    "fill_price": 42500.50,
    "status": "Filled"
  },
  "short_order": {
    "id": "order_short_001",
    "exchange": "bybit",
    "symbol": "BTCUSDT",
    "side": "Short",
    "order_type": "Limit",
    "price": 42495.30,
    "size": 0.1176,
    "queue_position": {
      "price": 42495.30,
      "cumulative_volume_at_price": 3.2,
      "resting_depth_at_entry": 18.0,
      "fill_threshold_pct": 0.20,
      "is_filled": true
    },
    "created_at": 1704067200,
    "filled_at": 1704067202,
    "fill_price": 42495.30,
    "status": "Filled"
  },
  "leg_out_event": null
}
```

### 3. Portfolio State

**Key Pattern:** `strategy:portfolio:state`

**Purpose:** Store the current portfolio state including capital allocation, active/closed trades, and cumulative metrics.

**Data Type:** String (JSON)

**TTL/Expiration:** None (continuously updated)

**Access Patterns:**
- Retrieve current portfolio state for display
- Update available capital when trades open/close
- Update cumulative metrics after each trade
- Check capital constraints before entering new trades

**Example JSON Structure:**
```json
{
  "starting_capital": 20000.0,
  "available_capital": 8500.0,
  "total_open_positions": 11500.0,
  "active_trades": [
    "550e8400-e29b-41d4-a716-446655440000",
    "660e8400-e29b-41d4-a716-446655440001",
    "770e8400-e29b-41d4-a716-446655440002"
  ],
  "closed_trades": [
    "440e8400-e29b-41d4-a716-446655440000",
    "330e8400-e29b-41d4-a716-446655440001"
  ],
  "cumulative_pnl": 250.50,
  "win_count": 8,
  "loss_count": 2,
  "leg_out_count": 1,
  "leg_out_total_loss": 45.30
}
```

### 4. Daily Performance Metrics

**Key Pattern:** `strategy:metrics:daily`

**Purpose:** Store aggregated daily performance metrics for trend analysis and reporting.

**Data Type:** String (JSON)

**TTL/Expiration:** None (persists for historical analysis)

**Access Patterns:**
- Retrieve daily metrics for dashboard display
- Calculate weekly/monthly aggregates
- Analyze performance trends over time

**Example JSON Structure:**
```json
{
  "date": "2024-01-01",
  "trades_executed": 12,
  "trades_won": 10,
  "trades_lost": 2,
  "win_rate": 0.833,
  "daily_pnl": 450.75,
  "daily_pnl_percentage": 2.25,
  "leg_out_events": 1,
  "leg_out_loss": 45.30,
  "average_trade_duration_seconds": 125,
  "max_drawdown": 0.05,
  "sharpe_ratio": 1.45,
  "realistic_apr": 0.456
}
```

### 5. Leg-Out Risk Events

**Key Pattern:** `strategy:leg_out_events`

**Purpose:** Store all leg-out events (one side fills, other doesn't) for risk analysis and strategy refinement.

**Data Type:** List (JSON array)

**TTL/Expiration:** None (persists for historical analysis)

**Access Patterns:**
- Append new leg-out events
- Retrieve all leg-out events for analysis
- Calculate leg-out frequency and impact

**Example JSON Structure (array element):**
```json
{
  "trade_id": "550e8400-e29b-41d4-a716-446655440000",
  "symbol": "BTCUSDT",
  "timestamp": 1704067250,
  "filled_leg": "long",
  "filled_at": 1704067201,
  "filled_price": 42500.50,
  "unfilled_leg": "short",
  "hedge_executed": true,
  "hedge_price": 42490.00,
  "hedge_loss": 52.50,
  "reason": "timeout_500ms"
}
```

### 6. Active Trades List (Monitoring)

**Key Pattern:** `strategy:monitor:active_trades`

**Purpose:** Maintain a list of active trade IDs for efficient iteration during monitoring loop.

**Data Type:** Set (Redis Set)

**TTL/Expiration:** None (updated as trades open/close)

**Access Patterns:**
- Get all active trade IDs for monitoring loop
- Add trade ID when trade opens
- Remove trade ID when trade closes
- Check if specific trade is active

**Operations:**
```
SADD strategy:monitor:active_trades {trade_id}      # Add trade
SREM strategy:monitor:active_trades {trade_id}      # Remove trade
SMEMBERS strategy:monitor:active_trades             # Get all active trades
SISMEMBER strategy:monitor:active_trades {trade_id} # Check if active
```

### 7. Portfolio Metrics (Monitoring)

**Key Pattern:** `strategy:monitor:portfolio_metrics`

**Purpose:** Store current portfolio metrics for real-time display in monitor binary.

**Data Type:** String (JSON)

**TTL/Expiration:** None (continuously updated every 1 second)

**Access Patterns:**
- Retrieve current metrics for display
- Update metrics after each trade or every monitoring cycle
- Calculate derived metrics (win rate, APR, etc.)

**Example JSON Structure:**
```json
{
  "timestamp": 1704067320,
  "total_trades": 10,
  "win_rate": 0.80,
  "cumulative_pnl": 250.50,
  "pnl_percentage": 1.25,
  "available_capital": 8500.0,
  "utilization_pct": 0.575,
  "leg_out_count": 1,
  "leg_out_loss_pct": 0.18,
  "realistic_apr": 0.456,
  "active_trade_count": 3,
  "recent_exits": [
    {
      "trade_id": "440e8400-e29b-41d4-a716-446655440000",
      "symbol": "ETHUSDT",
      "exit_time": 1704067300,
      "actual_profit": 32.10,
      "exit_reason": "profit_target"
    },
    {
      "trade_id": "330e8400-e29b-41d4-a716-446655440001",
      "symbol": "BNBUSDT",
      "exit_time": 1704067280,
      "actual_profit": -18.50,
      "exit_reason": "loss_limit"
    }
  ]
}
```

## Data Structure Definitions

### PaperTrade
```rust
struct PaperTrade {
    id: String,                       // UUID
    symbol: String,                   // e.g., "BTCUSDT"
    long_exchange: String,            // e.g., "binance"
    short_exchange: String,           // e.g., "bybit"
    entry_time: u64,                  // Unix timestamp (seconds)
    entry_long_price: f64,            // Entry price on long exchange
    entry_short_price: f64,           // Entry price on short exchange
    entry_spread_bps: f64,            // Spread in basis points
    position_size_usd: f64,           // Capital allocated to trade
    funding_delta_entry: f64,         // Funding rate delta at entry
    projected_profit_usd: f64,        // Expected profit if spread closes
    actual_profit_usd: f64,           // Realized profit (null if active)
    status: TradeStatus,              // Pending, Active, Exiting, Closed
    exit_reason: Option<String>,      // profit_target, loss_limit, funding_convergence, leg_out
    exit_time: Option<u64>,           // Unix timestamp when trade closed
    long_order: SimulatedOrder,       // Long side order details
    short_order: SimulatedOrder,      // Short side order details
    leg_out_event: Option<LegOutEvent>, // Leg-out event if applicable
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
    id: String,                       // Order ID
    exchange: String,                 // Exchange name
    symbol: String,                   // Trading pair
    side: OrderSide,                  // Long or Short
    order_type: OrderType,            // Limit or Market
    price: f64,                       // Order price
    size: f64,                        // Order size in base asset
    queue_position: Option<QueuePosition>, // Queue tracking for limit orders
    created_at: u64,                  // Unix timestamp
    filled_at: Option<u64>,           // Unix timestamp when filled
    fill_price: Option<f64>,          // Actual fill price
    status: OrderStatus,              // Pending, Filled, Cancelled
}

enum OrderSide { Long, Short }
enum OrderType { Limit, Market }
enum OrderStatus { Pending, Filled, Cancelled }
```

### QueuePosition
```rust
struct QueuePosition {
    price: f64,                       // Limit order price
    cumulative_volume_at_price: f64,  // Volume that has traded at this price
    resting_depth_at_entry: f64,      // Resting depth when order placed
    fill_threshold_pct: f64,          // 0.20 (20% of resting depth)
    is_filled: bool,                  // Whether order has filled
}
```

### LegOutEvent
```rust
struct LegOutEvent {
    filled_leg: String,               // "long" or "short"
    filled_at: u64,                   // Unix timestamp
    unfilled_leg: String,             // "long" or "short"
    hedge_executed: bool,             // Whether hedge was executed
    hedge_price: f64,                 // Price at which hedge was executed
}
```

### PortfolioState
```rust
struct PortfolioState {
    starting_capital: f64,            // $20,000
    available_capital: f64,           // Unallocated capital
    total_open_positions: f64,        // USD value of active trades
    active_trades: Vec<String>,       // List of active trade IDs
    closed_trades: Vec<String>,       // List of closed trade IDs
    cumulative_pnl: f64,              // Total profit/loss
    win_count: u32,                   // Number of winning trades
    loss_count: u32,                  // Number of losing trades
    leg_out_count: u32,               // Number of leg-out events
    leg_out_total_loss: f64,          // Total loss from leg-outs
}
```

### PortfolioMetrics
```rust
struct PortfolioMetrics {
    timestamp: u64,                   // Unix timestamp
    total_trades: u32,                // Total trades executed
    win_rate: f64,                    // win_count / total_trades
    cumulative_pnl: f64,              // Total profit/loss
    pnl_percentage: f64,              // cumulative_pnl / starting_capital
    available_capital: f64,           // Unallocated capital
    utilization_pct: f64,             // total_open_positions / starting_capital
    leg_out_count: u32,               // Number of leg-out events
    leg_out_loss_pct: f64,            // leg_out_total_loss / cumulative_pnl
    realistic_apr: f64,               // (cumulative_pnl / starting_capital) / (days_elapsed / 365)
    active_trade_count: u32,          // Number of currently active trades
    recent_exits: Vec<RecentExit>,    // Last few closed trades
}

struct RecentExit {
    trade_id: String,
    symbol: String,
    exit_time: u64,
    actual_profit: f64,
    exit_reason: String,
}
```

## TTL/Expiration Policies

| Key Pattern | TTL | Reason |
|---|---|---|
| `strategy:trades:active:{trade_id}` | None | Persists until trade closes |
| `strategy:trades:closed:{trade_id}` | None | Persists indefinitely for history |
| `strategy:portfolio:state` | None | Continuously updated |
| `strategy:metrics:daily` | None | Persists for historical analysis |
| `strategy:leg_out_events` | None | Persists for risk analysis |
| `strategy:monitor:active_trades` | None | Updated as trades open/close |
| `strategy:monitor:portfolio_metrics` | None | Updated every 1 second |

## Access Patterns and Typical Operations

### 1. Entry Execution
```
1. Check available capital: GET strategy:portfolio:state
2. Create new trade: SET strategy:trades:active:{trade_id} {json}
3. Add to active list: SADD strategy:monitor:active_trades {trade_id}
4. Update portfolio: SET strategy:portfolio:state {json}
```

### 2. Monitoring Loop (Every 1 second)
```
1. Get active trades: SMEMBERS strategy:monitor:active_trades
2. For each trade:
   a. GET strategy:trades:active:{trade_id}
   b. Calculate unrealized P&L
   c. Check exit conditions
3. Update metrics: SET strategy:monitor:portfolio_metrics {json}
```

### 3. Trade Exit
```
1. GET strategy:trades:active:{trade_id}
2. Calculate actual profit
3. SET strategy:trades:closed:{trade_id} {json}
4. DEL strategy:trades:active:{trade_id}
5. SREM strategy:monitor:active_trades {trade_id}
6. Update portfolio: SET strategy:portfolio:state {json}
7. Update metrics: SET strategy:monitor:portfolio_metrics {json}
```

### 4. Leg-Out Event
```
1. LPUSH strategy:leg_out_events {json}
2. Update trade: SET strategy:trades:active:{trade_id} {json}
3. Update portfolio leg_out_count
```

### 5. Monitor Display
```
1. GET strategy:monitor:portfolio_metrics
2. SMEMBERS strategy:monitor:active_trades
3. For each active trade: GET strategy:trades:active:{trade_id}
4. Display in real-time UI
```

### 6. Historical Analysis
```
1. SMEMBERS strategy:monitor:active_trades (get active trades)
2. Iterate over all closed trades: GET strategy:trades:closed:{trade_id}
3. Calculate win rate, APR, etc.
4. LRANGE strategy:leg_out_events 0 -1 (get all leg-out events)
```

## Performance Considerations

### Memory Usage
- Each active trade: ~2-3 KB (JSON)
- Each closed trade: ~2-3 KB (JSON)
- Portfolio state: ~1 KB
- Metrics: ~2 KB
- Leg-out events: ~500 bytes each

**Example:** 100 active trades + 1000 closed trades ≈ 3 MB

### Query Performance
- Single trade lookup: O(1)
- Get all active trades: O(n) where n = number of active trades
- Iterate closed trades: O(m) where m = number of closed trades
- Update metrics: O(1)

### Optimization Strategies
1. Use Redis Sets for active trade IDs (fast membership checks)
2. Store metrics separately for quick retrieval
3. Archive old closed trades to separate storage if needed
4. Use Redis Streams for event logging if audit trail needed

## Serialization Format

All JSON structures use UTF-8 encoding and follow these conventions:

- **Numbers:** Floating-point numbers use standard JSON format (e.g., 42500.50)
- **Timestamps:** Unix timestamps in seconds (u64)
- **Strings:** UTF-8 encoded, no special escaping needed
- **Null values:** Use JSON `null` for optional fields

## Example Redis Commands

### Store Active Trade
```bash
SET strategy:trades:active:550e8400-e29b-41d4-a716-446655440000 '{"id":"550e8400-e29b-41d4-a716-446655440000",...}'
```

### Retrieve Active Trade
```bash
GET strategy:trades:active:550e8400-e29b-41d4-a716-446655440000
```

### Get All Active Trade IDs
```bash
SMEMBERS strategy:monitor:active_trades
```

### Add Leg-Out Event
```bash
LPUSH strategy:leg_out_events '{"trade_id":"550e8400-e29b-41d4-a716-446655440000",...}'
```

### Get Portfolio State
```bash
GET strategy:portfolio:state
```

### Update Portfolio Metrics
```bash
SET strategy:monitor:portfolio_metrics '{"timestamp":1704067320,...}'
```

## Migration and Backup

### Backup Strategy
1. Regularly export closed trades to persistent storage
2. Maintain daily snapshots of portfolio state
3. Archive leg-out events for compliance

### Data Retention
- Active trades: Until closed (typically minutes to hours)
- Closed trades: Indefinitely (for historical analysis)
- Metrics: Indefinitely (for trend analysis)
- Leg-out events: Indefinitely (for risk analysis)

## Consistency Guarantees

The Redis schema maintains the following consistency properties:

1. **Capital Conservation:** `available_capital + sum(position_size for active trades) == starting_capital`
2. **Trade Uniqueness:** Each trade ID appears in either active or closed list, never both
3. **Metrics Accuracy:** Metrics are recalculated after each trade open/close
4. **Atomic Updates:** Portfolio state and metrics are updated together

