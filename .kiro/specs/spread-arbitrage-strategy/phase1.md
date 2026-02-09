# P0 Phase - Complete Implementation Overview

## Phase 1: Lead-Lag Detection & Funding Gravity

This phase implements the two critical P0 signals that separate a "dumb bot" from a high-conviction arbitrage strategy.

---

## 1. Lead-Lag Detection (The Anchor)

### What it does:
- Identifies which exchange leads price discovery (typically Binance)
- Detects when laggard exchanges are at extreme spreads while the anchor is reverting
- Signals high-conviction entry opportunities

### Why it matters:
- In crypto, one exchange (usually Binance) discovers price first
- Other exchanges (Bybit, OKX) follow 10-50ms later
- When Binance reverts but Bybit is still at extreme spread, that's a high-conviction entry
- We're essentially betting on mean reversion with a known catalyst

### Implementation steps:
1. Add `ExchangePriceHistory` struct to track price history per exchange
2. Store last 60 prices (30 seconds) per exchange with timestamps
3. Calculate mid-price: `(bid + ask) / 2`
4. Calculate correlation coefficient between each exchange and Binance
5. Detect reversion: price moved 0.15% (15 bps) in one direction, then moved back
6. Signal entry when: Binance reverts AND laggard still at extreme spread

### Key parameters:
- Window size: 60 prices (30 seconds)
- Correlation threshold: >0.65 (strong follower)
- Reversion threshold: 0.15% (15 bps) - accounts for fees + slippage
- Minimum spread for entry: >15 bps (to cover fees)

### Integration point:
`src/bin/dashboard.rs` - add to `AppState` and use in `recalculate_opportunities()`

---

## 2. Funding Gravity (Time-to-Payout Weighted Spread)

### What it does:
- Weights the spread by time remaining until funding payout
- Increases confidence when <15 minutes to payout (arbitrageurs flood market)
- Captures the "rubber band" moment before it snaps

### Why it matters:
- Funding payout is a mechanical force that must converge
- As payout approaches, arbitrageurs flood the market
- Spread naturally compresses in final 15 minutes
- We want to enter when the "rubber band" is most stretched but about to snap

### Implementation steps:
1. Create funding rate parser for each exchange (extract from Redis)
2. Calculate time to next funding payout per exchange
3. Weight spread: `weighted_spread = spread_bps * (time_remaining_minutes / 480)`
4. Apply confidence boost: +15 points at <15min, +30 points at <5min
5. Integrate into confidence scoring

### Exchange payout schedules:
- Binance, Bybit, Bitget, Gate: 00:00, 08:00, 16:00 UTC (8-hour)
- KuCoin: 01:00, 09:00, 17:00 UTC (8-hour)
- OKX: 16:00 UTC (24-hour)
- Hyperliquid, Paradex: Continuous (no fixed payout)

### Key parameters:
- Boost at <15min: +15 points
- Boost at <5min: +30 points
- Normalization: 480 minutes (8 hours)

### Integration point:
`src/bin/dashboard.rs` - add to `calculate_confidence_score()`

---

## 3. Toxic Flow Filter (Already Implemented ✅)

**Status:** Complete in `src/bin/dashboard.rs`
- Detects 3x spread increase in <10 seconds
- Rejects opportunities (protection against price gaps)

---

## 4. Z-Score of Basis (Already Implemented ✅)

**Status:** Complete in `src/bin/dashboard.rs`
- Calculates (current_spread - mean) / std_dev
- Uses 10-minute history for statistical significance
- Threshold: >2 std devs = high-conviction entry

---

## Implementation Sequence

| Step | Component | Complexity | Dependencies | Est. Time |
|------|-----------|-----------|--------------|-----------|
| 1 | Add ExchangePriceHistory struct | Low | None | 15 min |
| 2 | Track mid-prices in recalculate_opportunities() | Low | Step 1 | 15 min |
| 3 | Implement correlation calculation | Medium | Step 2 | 30 min |
| 4 | Implement reversion detection | Medium | Step 2 | 20 min |
| 5 | Implement lead-lag signal logic | Medium | Steps 3-4 | 20 min |
| 6 | Create funding rate parser | Medium | None | 30 min |
| 7 | Calculate time to next payout | Low | Step 6 | 15 min |
| 8 | Implement funding gravity weighting | Low | Step 7 | 15 min |
| 9 | Apply confidence boost logic | Low | Steps 7-8 | 15 min |
| 10 | Integrate into confidence scoring | Low | Steps 5, 9 | 15 min |
| 11 | Update dashboard display | Low | Steps 5, 9 | 20 min |
| 12 | Test and validate | Medium | All | 30 min |

---

## Data Flow

```
Redis (real-time data)
    ↓
update_from_redis()
    ├─ Extract bid/ask prices
    ├─ Extract funding rates
    └─ Normalize symbols
    ↓
recalculate_opportunities()
    ├─ Calculate mid-prices
    ├─ Track price history (60 prices per exchange)
    ├─ Calculate correlations
    ├─ Detect reversions
    ├─ Detect lead-lag signals
    ├─ Extract funding rates
    ├─ Calculate time to payout
    ├─ Apply funding gravity boost
    ├─ Calculate confidence score
    ├─ Filter by hard constraints
    └─ Display opportunities
    ↓
Dashboard UI
    ├─ Show spread (color-coded)
    ├─ Show confidence score
    ├─ Show funding delta
    ├─ Show lead-lag signal status
    ├─ Show time to next payout
    └─ Show funding gravity boost
```

---

## Code Changes Summary

### File: `src/bin/dashboard.rs`

#### 1. Add structs:
```rust
#[derive(Clone, Debug)]
struct ExchangePriceHistory {
    exchange: String,
    prices: VecDeque<(u64, f64)>,  // (timestamp_ms, mid_price)
    correlation_with_binance: f64,
}

#[derive(Clone, Debug)]
struct LeadLagSignal {
    symbol: String,
    anchor_exchange: String,
    laggard_exchange: String,
    anchor_reverting: bool,
    laggard_at_extreme: bool,
    signal_strength: f64,  // 0.0 to 1.0
}

#[derive(Clone, Debug)]
struct FundingGravity {
    symbol: String,
    exchange: String,
    funding_rate: f64,
    time_to_payout_minutes: u64,
    weighted_spread: f64,
    confidence_boost: u8,
}
```

#### 2. Add to `AppState`:
```rust
struct AppState {
    ticker_data: BTreeMap<String, Vec<(String, f64, f64)>>,
    funding_rates: BTreeMap<String, BTreeMap<String, f64>>,
    opportunities: BTreeMap<String, TradeOpportunity>,
    spread_history: BTreeMap<String, SpreadHistory>,
    price_histories: BTreeMap<String, ExchangePriceHistory>,  // NEW
    lead_lag_signals: BTreeMap<String, LeadLagSignal>,        // NEW
    funding_gravity: BTreeMap<String, FundingGravity>,        // NEW
    should_quit: bool,
    scroll_offset: usize,
}
```

#### 3. Add methods:
- `track_price_history()` - store mid-prices
- `calculate_correlation()` - correlation between exchanges
- `detect_reversion()` - detect price reversions
- `detect_lead_lag_signal()` - identify anchor + laggard
- `parse_funding_rate()` - extract from Redis
- `time_to_next_payout()` - calculate minutes to payout
- `calculate_funding_gravity_boost()` - boost confidence
- `update_confidence_score()` - integrate all signals

#### 4. Update:
- `recalculate_opportunities()` - call new methods
- `calculate_confidence_score()` - add lead-lag + funding gravity
- `ui()` - display new signals

---

## Constants to Define

```rust
const LEAD_LAG_WINDOW_SIZE: usize = 60;           // 30 seconds
const CORRELATION_THRESHOLD: f64 = 0.65;          // Strong follower
const REVERSION_THRESHOLD_BPS: f64 = 15.0;        // 0.15%
const MIN_SPREAD_FOR_ENTRY_BPS: f64 = 15.0;       // Must exceed fees
const FUNDING_BOOST_15MIN: u8 = 15;               // <15 min to payout
const FUNDING_BOOST_5MIN: u8 = 30;                // <5 min to payout
const FUNDING_NORMALIZATION_MINUTES: u64 = 480;   // 8 hours

// Exchange funding payout times (UTC)
const BINANCE_PAYOUT_HOURS: &[u32] = &[0, 8, 16];
const BYBIT_PAYOUT_HOURS: &[u32] = &[0, 8, 16];
const BITGET_PAYOUT_HOURS: &[u32] = &[0, 8, 16];
const GATE_PAYOUT_HOURS: &[u32] = &[0, 8, 16];
const KUCOIN_PAYOUT_HOURS: &[u32] = &[1, 9, 17];
const OKX_PAYOUT_HOURS: &[u32] = &[16];
// Hyperliquid, Paradex: continuous (no fixed payout)
```

---

## Success Criteria

✅ Lead-lag detection identifies Binance as anchor
✅ Reversion detection triggers on 15 bps moves
✅ Funding gravity boost applies correctly
✅ Confidence score incorporates all signals
✅ Dashboard displays all new metrics
✅ No false positives from toxic flow
✅ Z-score validation still working

---

## Testing Strategy

### Unit Tests:
- Correlation calculation with known data
- Reversion detection with synthetic price movements
- Time-to-payout calculation for each exchange
- Funding gravity boost logic

### Integration Tests:
- Lead-lag signal with real Redis data
- Confidence score with all signals combined
- Dashboard display with new metrics

### Manual Testing:
- Verify Binance identified as anchor
- Verify laggards detected correctly
- Verify funding gravity boost near payout times
- Verify no false positives from toxic flow

---

## Notes

- All implementations use existing Redis data (no new collection needed)
- Lean-code principle: extend existing structures, don't create new files
- Memory footprint: ~60 prices × 8 exchanges × 100 symbols = ~480KB
- Performance: correlation calculation is O(n) where n=60, negligible overhead
- Funding rate data already collected via WebSocket (Bybit, OKX, etc.)

---

## Next Steps

1. Start with Step 1: Add ExchangePriceHistory struct
2. Implement Steps 2-5: Lead-lag detection
3. Implement Steps 6-9: Funding gravity
4. Integrate into confidence scoring (Step 10)
5. Update dashboard display (Step 11)
6. Test and validate (Step 12)
