# Trade Validation Criteria - Complete Algorithm Workflow

## Overview
The algorithm validates trades through multiple filtering layers. A trade must pass ALL criteria to be executed.

## SUMMARY: REJECTION REASONS

A trade is REJECTED if ANY of these conditions are true:

1. **Hard Constraint Failure**: latency > 200ms OR depth insufficient OR funding delta < 0.01%
2. **Low Confidence**: confidence_score < 70
3. **Negative Spread**: spread_bps <= 0
4. **Unprofitable**: net_profit_bps <= 0 OR realistic_profit_bps <= 0
5. **Invalid Position Size**: position_size <= 0 OR position_size > available_capital
6. **Insufficient Capital**: available_capital <= 0 OR position_size > available_capital
7. **Duplicate Symbol**: already have active trade for this symbol
8. **Atomic Execution Failed**: harder leg didn't fill within 500ms
9. **No Prices Available**: couldn't fetch current prices from Redis

---

## SUMMARY: ACCEPTANCE CRITERIA

A trade is ACCEPTED and EXECUTED if ALL of these are true:

1. ✓ Hard constraints pass (latency, depth, funding delta)
2. ✓ Confidence score >= 70
3. ✓ Spread is positive and > fees + funding costs
4. ✓ Realistic profit after slippage > 0
5. ✓ Position size is valid and <= available capital
6. ✓ Available capital > 0
7. ✓ No duplicate symbol already trading
8. ✓ Atomic execution succeeds (both legs fill within 500ms)
9. ✓ Current prices available from Redis

---

## EXECUTION FLOW DIAGRAM

```
Opportunity from Dashboard
    ↓
[1] Hard Constraints Check
    ├─ Latency < 200ms? ✓
    ├─ Order book depth sufficient? ✓
    └─ Funding delta > 0.01%? ✓
    ↓
[2] Calculate Soft Metrics
    ├─ Funding rate differential
    ├─ Order book imbalance
    ├─ Open interest
    ├─ VWAP deviation
    ├─ Volatility trend
    └─ Liquidation clusters
    ↓
[3] Confidence Score >= 70? ✓
    ↓
[4] Spread & Profitability
    ├─ Spread > 0? ✓
    ├─ Net profit > 0? ✓
    └─ Realistic profit > 0? ✓
    ↓
[5] Position Sizing
    ├─ Calculate position size
    ├─ Size > 0? ✓
    └─ Size <= available capital? ✓
    ↓
[6] Duplicate Check
    └─ No active trade for symbol? ✓
    ↓
[7] Atomic Execution
    ├─ Harder leg fills in 500ms? ✓
    └─ Easier leg fills in 500ms? ✓
    ↓
[8] Trade Active
    ├─ Deduct capital
    ├─ Monitor spread
    └─ Check exit conditions
    ↓
[9] Exit Triggered
    ├─ Place exit orders
    ├─ Calculate actual profit
    └─ Return capital
    ↓
[10] Trade Closed
     └─ Update portfolio metrics
---

## PHASE 1: OPPORTUNITY DISCOVERY (Dashboard)

### 1.1 Hard Constraints (Must ALL Pass)
These are **gatekeeper criteria**. If ANY fails, confidence_score = 0 and trade is rejected.

#### 1.1.1 Order Book Depth Sufficient
```
order_book_depth_long >= position_size * 2.0
AND
order_book_depth_short >= position_size * 2.0
```
- Ensures we can fill both sides without massive slippage
- Prevents trading illiquid pairs

#### 1.1.2 Exchange Latency OK
```
ALL exchanges have latency < 200ms
```
- Latency = local_time - server_time
- Measured every 100ms
- If ANY exchange > 200ms: confidence = 0
- Prevents stale price data

#### 1.1.3 Funding Delta Substantial
```
|funding_delta| > 0.01% per 8 hours
```
- funding_delta = funding_rate_long - funding_rate_short
- Must be meaningful to justify the trade
- Too small = not worth the execution risk

---

## PHASE 2: SOFT METRICS (Only if Hard Constraints Pass)

These are weighted to calculate confidence_score (0-100):

### 2.1 Funding Rate Differential (Weight: 9/50)
- Higher funding delta = higher score
- Normalized: (|funding_delta| / 0.01) * 100, capped at 100

### 2.2 Order Book Imbalance (Weight: 8/50)
```
OBI = (bid_volume - ask_volume) / (bid_volume + ask_volume)
```
- Range: -1 to +1
- Extreme imbalance = higher score
- Indicates directional pressure

### 2.3 Open Interest (Weight: 7/50)
```
IF current_OI > 24h_average_OI:
    score = ((current_OI / avg_OI - 1.0) / 0.5) * 100
ELSE:
    score = 0
```
- Rising OI = more participants = higher confidence

### 2.4 VWAP Deviation (Weight: 6/50)
```
deviation = (current_price - VWAP) / VWAP
```
- Deviation from volume-weighted average price
- Indicates price extremes

### 2.5 Volatility Trend (Weight: 5/50)
```
atr_trend = recent_ATR < previous_ATR
```
- Calming volatility = higher score
- Reduces execution risk

### 2.6 Liquidation Cluster Distance (Weight: 5/50)
```
distance = min_distance_to_liquidation_cluster / current_price
```
- Proximity to liquidation clusters = higher score
- Indicates support/resistance levels

### 2.7 Confidence Score Calculation
```
IF NOT hard_constraints.passes_all():
    confidence_score = 0
ELSE:
    confidence_score = weighted_average(all_soft_metrics)
    confidence_score = min(confidence_score, 100)
```

---

## PHASE 3: SPREAD & PROFITABILITY VALIDATION

### 3.1 Spread Calculation
```
spread_bps = ((short_bid - long_ask) / long_ask) * 10000
```
- Must be positive (short_bid > long_ask)
- Measured in basis points (1 bps = 0.01%)

### 3.2 Fee Calculation
```
total_fee_bps = long_exchange_taker_fee + short_exchange_taker_fee

Exchange Taker Fees:
- Binance: 4.0 bps (0.04%)
- OKX: 5.0 bps (0.05%)
- Bybit: 5.5 bps (0.055%)
- Bitget: 6.0 bps (0.06%)
- KuCoin: 6.0 bps (0.06%)
- Hyperliquid: 3.5 bps (0.035%)
- Paradex: 5.0 bps (0.05%)
- Gate: 6.0 bps (0.06%)
```

### 3.3 Funding Cost Estimation
```
funding_cost_bps = funding_delta_8h * 3  (rough estimate for 8-hour hold)
```
- Funding rates are paid every 8 hours
- Estimate cost for typical hold duration

### 3.4 Net Profit Calculation
```
net_profit_bps = spread_bps - total_fee_bps - funding_cost_bps
```

### 3.5 Profitability Threshold
```
IF net_profit_bps <= 0:
    REJECT trade (not profitable)
```
- Must have positive expected profit after all costs

### 3.6 Slippage Calculation
```
base_slippage = 0.0002 (2 bps)
depth_ratio = position_size / order_book_depth
additional_slippage = depth_ratio * 0.0003
actual_slippage = (base_slippage + additional_slippage).min(0.0005)
```
- Larger positions = more slippage
- Capped at 5 bps maximum

### 3.7 Realistic Profit After Slippage
```
realistic_profit_bps = net_profit_bps - actual_slippage
```

### 3.8 Final Profitability Check
```
IF realistic_profit_bps <= 0:
    REJECT trade (not profitable after slippage)
```

---

## PHASE 4: POSITION SIZING

### 4.1 Position Size Calculation
```
base_size = (net_profit_bps / spread_bps) * available_capital

IF base_size <= 0:
    position_size = 0 (reject)
ELSE:
    capped_size = min(base_size, available_capital * 0.5)
    adaptive_minimum = max(10.0, available_capital * 0.01)
    position_size = max(capped_size, adaptive_minimum)
```

### 4.2 Capital Constraints
```
IF position_size > available_capital:
    REJECT trade (insufficient capital)

IF available_capital <= 0:
    REJECT trade (no capital left)
```

### 4.3 Position Size Validation
```
IF position_size <= 0:
    REJECT trade (invalid size)

IF position_size > available_capital * 0.5:
    REJECT trade (exceeds 50% cap)
```

---

## PHASE 5: DUPLICATE PREVENTION

### 5.1 Symbol Uniqueness
```
IF active_trades.any(|t| t.symbol == opportunity.symbol):
    REJECT trade (already have active trade for this symbol)
```
- Prevents opening multiple trades on same pair
- Reduces correlation risk

---

## PHASE 6: CONFIDENCE THRESHOLD

### 6.1 Minimum Confidence Score
```
IF confidence_score < 70:
    REJECT trade (insufficient confidence)
```
- 70 is the minimum threshold
- Filters out low-quality opportunities

---

## PHASE 7: ATOMIC EXECUTION

### 7.1 Harder Leg Identification
```
Harder leg = smaller exchange or DEX
Exchange ranking (hardest to easiest):
1. Paradex (DEX)
2. Hyperliquid (DEX)
3. Bitget
4. Gate
5. KuCoin
6. OKX
7. Bybit
8. Binance (easiest)
```

### 7.2 Harder Leg Execution
```
1. Place limit order on harder leg
2. Wait up to 500ms for fill
3. IF filled: proceed to step 4
4. IF NOT filled: CANCEL and REJECT trade
```

### 7.3 Easier Leg Execution
```
1. Place limit order on easier leg
2. Wait up to 500ms for fill
3. IF filled: trade is ACTIVE
4. IF NOT filled: execute market order (hedge)
```

### 7.4 Queue Position Tracking
```
Order fills ONLY IF:
cumulative_volume_at_price >= (resting_depth_at_entry * 0.20)
```
- 20% of resting depth must trade at our price
- Prevents over-reporting fills

### 7.5 Atomic Execution Result
```
IF both_legs_filled:
    trade.status = Active
    deduct capital from available_pool
ELSE:
    trade.status = Rejected
    reverse any partial fills
```

---

## PHASE 8: ACTIVE POSITION MONITORING

### 8.1 Spread Monitoring
```
EVERY 1 second:
    current_spread_bps = calculate_spread(current_prices)
    spread_reduction_bps = entry_spread_bps - current_spread_bps
    current_pnl = (spread_reduction_bps / 10000) * position_size
```

### 8.2 Exit Condition 1: Profit Target
```
IF current_pnl >= entry_projected_profit * 0.9:
    SET exit_reason = "profit_target"
    SET status = Exiting
```
- Exit when we've captured 90% of projected profit
- Locks in gains before spread widens

### 8.3 Exit Condition 2: Stop Loss (Relative)
```
IF current_pnl <= entry_projected_profit * -0.2:
    SET exit_reason = "stop_loss"
    SET status = Exiting
```
- Exit if we've lost 20% of projected profit
- Prevents large losses

### 8.4 Exit Condition 3: Stop Loss (Absolute)
```
max_loss = max(entry_projected_profit * 0.5, 5.0)
IF current_pnl <= -max_loss:
    SET exit_reason = "stop_loss"
    SET status = Exiting
```
- Exit if absolute loss exceeds threshold
- Minimum $5 loss limit

### 8.5 Exit Condition 4: Spread Widening
```
IF current_spread_bps > entry_spread_bps * 1.3:
    SET exit_reason = "stop_loss"
    SET status = Exiting
```
- Exit if spread widens by 30%
- Indicates trade thesis is breaking down

### 8.6 Exit Condition 5: Funding Convergence
```
IF entry_funding_delta.abs() > 0.0001:
    IF current_funding_delta.abs() < entry_funding_delta.abs() * 0.2:
        SET exit_reason = "funding_convergence"
        SET status = Exiting
```
- Exit when funding delta has converged 80%
- Main profit driver is gone

### 8.7 Exit Condition 6: Funding Too Low
```
IF current_funding_delta.abs() < 0.00005:
    SET exit_reason = "funding_convergence"
    SET status = Exiting
```
- Exit if funding delta drops below 0.005%
- Not worth holding anymore

---

## PHASE 9: LEG-OUT RISK DETECTION

### 9.1 Leg-Out Detection
```
EVERY 100ms:
    IF long_order.filled AND NOT short_order.filled:
        time_since_long_fill = now - long_order.filled_at
        IF time_since_long_fill > 500ms:
            EXECUTE market order on short side (hedge)
            LOG leg_out_event
            portfolio.leg_out_count++
    
    IF short_order.filled AND NOT long_order.filled:
        time_since_short_fill = now - short_order.filled_at
        IF time_since_short_fill > 500ms:
            EXECUTE market order on long side (hedge)
            LOG leg_out_event
            portfolio.leg_out_count++
```

### 9.2 Leg-Out Event Tracking
```
leg_out_event = {
    filled_leg: "long" or "short",
    filled_at: timestamp,
    unfilled_leg: "long" or "short",
    hedge_executed: true,
    hedge_price: market_price_at_hedge
}
```

---

## PHASE 10: TRADE EXIT EXECUTION

### 10.1 Exit Order Placement
```
1. Place limit exit orders on both sides
2. Track queue position for both orders
3. Wait up to 500ms for fills
```

### 10.2 Exit Fill Logic
```
IF both_sides_fill:
    actual_profit = (spread_reduction - fees) * position_size / 10000
ELSE IF one_side_fills:
    EXECUTE market order on unfilled side
    actual_profit = (spread_reduction - fees - slippage) * position_size / 10000
```

### 10.3 Trade Closure
```
1. Calculate actual_profit
2. Set trade.status = Closed
3. Return capital to available_pool:
   available_capital += position_size + actual_profit
4. Update portfolio metrics:
   cumulative_pnl += actual_profit
   IF actual_profit > 0: win_count++
   ELSE: loss_count++
5. Log trade to persistent storage
```

---

## PHASE 11: PORTFOLIO MANAGEMENT

### 11.1 Capital Conservation
```
INVARIANT:
available_capital + sum(position_size for all active trades) == starting_capital
```

### 11.2 Metrics Calculation
```
total_trades = win_count + loss_count
win_rate = (win_count / total_trades) * 100
pnl_percentage = (cumulative_pnl / starting_capital) * 100
utilization_pct = (total_open_positions / starting_capital) * 100
leg_out_loss_pct = (leg_out_total_loss / cumulative_pnl.abs()) * 100
realistic_apr = ((cumulative_pnl / starting_capital) / (days_elapsed / 365)) * 100
```

---


```
