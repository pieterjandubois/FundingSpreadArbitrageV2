# P1 Phase - Confluence Metrics Implementation

## Phase 2: Order Book Imbalance (OBI), Open Interest (OI), VWAP, ATR

This phase implements soft metrics that enhance signal quality by adding confluence to the P0 signals.

---

## Overview

**Why Phase 2?**
- P0 signals (lead-lag, funding gravity) are high-conviction but rare
- P1 metrics add confluence to increase entry frequency
- These are "soft" metrics (not hard constraints) - they boost confidence but don't reject trades
- Together they create a weighted scoring system for better risk/reward

**Data Availability:**
- OBI: Requires order book data (OKX, KuCoin, Bitget have this; others need proxy)
- OI: Available from all exchanges via API
- VWAP: Requires trade data (not currently collected)
- ATR: Requires OHLC data (not currently collected)
- Liquidation clusters: Requires liquidation data (not currently collected)

---

## 1. Order Book Imbalance (OBI)

### What it does:
Measures buy vs sell pressure at each exchange by analyzing order book depth.

### Formula:
```
OBI = (Bid Volume - Ask Volume) / (Bid Volume + Ask Volume)
Range: -1 (all selling) to +1 (all buying)
```

### Implementation:
- Calculate from top 5-10 levels of order book
- For long exchange (cheap): want positive OBI (buying pressure)
- For short exchange (expensive): want negative OBI (selling pressure)
- Boost confidence if both conditions met

### Data source:
- OKX: Already collecting `books5` (top 5 levels)
- KuCoin: Already collecting order book data
- Bitget: Already collecting order book data
- Others: Use bid-ask spread width as proxy

### Confidence boost:
- If OBI aligns with convergence direction: +10 points
- If OBI strongly aligns: +20 points

---

## 2. Open Interest (OI)

### What it does:
Tracks total open positions to identify if market is overbought/oversold.

### Implementation:
- Fetch current OI from each exchange
- Calculate 24-hour average OI
- Compare current to average
- If OI is elevated: market is crowded (higher risk)
- If OI is normal: market is balanced (lower risk)

### Data source:
- All exchanges provide OI via REST API
- Need to poll every 1-5 minutes
- Store 24-hour rolling average

### Confidence boost:
- If OI < 24h average: +15 points (market not crowded)
- If OI > 24h average: -10 points (market crowded, reduce confidence)

---

## 3. VWAP Deviation

### What it does:
Measures how far current price is from Volume-Weighted Average Price (1-hour window).

### Formula:
```
VWAP = sum(price * volume) / sum(volume)
Deviation = (current_price - VWAP) / VWAP
Z-score = deviation / std_dev
```

### Implementation:
- Requires trade data (not currently collected)
- **For now: SKIP** - implement in Phase 3 when trade data available
- Placeholder: use bid-ask midpoint as proxy

### Confidence boost:
- If price >1 std dev above VWAP: +10 points (potential reversal)
- If price <1 std dev below VWAP: +5 points (stable)

---

## 4. Volatility (ATR) & Trend Detection

### What it does:
Measures market volatility and identifies if market is calming down.

### Formula:
```
ATR = Average True Range over 14 periods
Trend = ATR_current < ATR_14d_avg (market calming)
```

### Implementation:
- Requires OHLC data (not currently collected)
- **For now: SKIP** - implement in Phase 3 when OHLC data available
- Placeholder: use spread volatility as proxy

### Confidence boost:
- If ATR decreasing (market calming): +15 points
- If ATR increasing (market volatile): -10 points

---

## Implementation Plan for Phase 2

### Immediately Implementable (using existing data):
1. **Order Book Imbalance (OBI)** - Uses existing order book data
2. **Open Interest (OI)** - Requires new API polling but straightforward

### Deferred to Phase 3 (need new data collection):
1. VWAP Deviation - Needs trade data
2. ATR/Volatility - Needs OHLC data

---

## Phase 2 Implementation Steps

### Step 1: Add OBI calculation
- Parse order book data from Redis
- Calculate bid/ask volumes at top 5-10 levels
- Compute OBI ratio

### Step 2: Add OI tracking
- Create OI polling task
- Store current OI and 24h average
- Calculate OI ratio

### Step 3: Integrate OBI into confidence scoring
- Check if OBI aligns with convergence
- Apply +10 or +20 boost

### Step 4: Integrate OI into confidence scoring
- Check if OI is elevated or normal
- Apply +15 or -10 boost

### Step 5: Update dashboard display
- Show OBI for each exchange
- Show OI ratio
- Show total confluence boost

### Step 6: Test and validate
- Verify OBI calculations
- Verify OI tracking
- Verify confidence score updates

---

## Data Structure Updates

### Add to AppState:
```rust
struct OBIMetrics {
    exchange: String,
    bid_volume: f64,
    ask_volume: f64,
    obi_ratio: f64,  // -1 to +1
}

struct OIMetrics {
    symbol: String,
    exchange: String,
    current_oi: f64,
    oi_24h_avg: f64,
    oi_ratio: f64,  // current / average
}

// Add to AppState:
obi_metrics: BTreeMap<String, BTreeMap<String, OBIMetrics>>,  // symbol -> exchange -> metrics
oi_metrics: BTreeMap<String, BTreeMap<String, OIMetrics>>,    // symbol -> exchange -> metrics
```

---

## Confidence Scoring Update

Current scoring (P0):
```
score = (spread_score * 0.6) + (funding_delta_score * 0.4) + funding_gravity_boost
```

New scoring (P0 + P1):
```
score = (spread_score * 0.5) + (funding_delta_score * 0.3) + (obi_score * 0.1) + (oi_score * 0.1) + funding_gravity_boost
```

Weights:
- Spread: 50% (most important)
- Funding Delta: 30% (mechanical force)
- OBI: 10% (market structure)
- OI: 10% (market crowding)
- Funding Gravity: +0 to +30 points (time-based boost)

---

## Success Criteria

✅ OBI calculated correctly from order book data
✅ OI tracked and 24h average maintained
✅ OBI boost applied when aligned with convergence
✅ OI boost applied based on crowding
✅ Confidence score updated with new weights
✅ Dashboard displays OBI and OI metrics
✅ No regression in P0 signal quality

---

## Notes

- Phase 2 focuses on metrics we can calculate NOW with existing data
- VWAP, ATR, Liquidation clusters deferred to Phase 3 (need new data collection)
- OBI and OI are "soft" metrics - they enhance but don't reject trades
- Weights can be tuned based on backtesting results
- Memory footprint: ~100 bytes per symbol per exchange (negligible)

---

## Next Steps

1. Start with Step 1: Add OBI calculation
2. Implement Steps 2-4: OI tracking and integration
3. Update dashboard (Step 5)
4. Test and validate (Step 6)
5. Move to Phase 3 when ready (VWAP, ATR, Liquidation clusters)
