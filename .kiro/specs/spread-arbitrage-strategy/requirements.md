# Spread Arbitrage Strategy - Requirements

## Overview
Implement a strategy that detects and simulates delta-neutral arbitrage trades based on funding rate differentials and confluence metrics across multiple futures exchanges. This is a **paper trading** implementation with $20,000 simulated capital - no live orders are placed. The strategy prioritizes execution fidelity over optimistic profit projections.

## User Stories

### 1. Confluence Metric Calculation with Hard Constraints
**As a** trader  
**I want** the bot to calculate a confidence score based on multiple confluence metrics with hard constraint gatekeeping  
**So that** I can identify high-conviction arbitrage opportunities that are actually executable

**Acceptance Criteria:**
- 1.1 Calculate Funding Rate Differential (FRA - FRB) and project next funding rate
- 1.2 Calculate Order Book Imbalance (OBI) ratio from top 5-10 levels
- 1.3 Retrieve Open Interest (OI) and compare to 24-hour average
- 1.4 Calculate VWAP deviation (distance from 1-hour VWAP in standard deviations)
- 1.5 Calculate volatility (ATR) and detect if market is calming down
- 1.6 Identify liquidation clusters from available data
- 1.7 Implement hard constraints (gatekeeper logic): if any constraint fails, confidence score = 0 regardless of soft metrics
- 1.8 Hard constraints: (a) Order book depth ≥ 2x position size on both legs, (b) Exchange latency <200ms, (c) Funding delta >0.01% per 8h
- 1.9 Combine soft metrics into confidence score (0-100) only if all hard constraints pass
- 1.10 Score >70 indicates high-conviction trade opportunity
- 1.11 Monitor exchange latency continuously: if WebSocket latency >200ms, flag as stale data and reduce confidence by 30 points

### 2. Opportunity Scanner
**As a** trader  
**I want** the bot to continuously scan for spread opportunities across all trading pairs  
**So that** I can capture arbitrage trades as they emerge

**Acceptance Criteria:**
- 2.1 Monitor all USDT pairs across connected exchanges (Binance, Bybit, KuCoin, OKX, Bitget, Gate, Hyperliquid, Paradex)
- 2.2 Calculate spread between highest and lowest prices for each pair
- 2.3 Filter opportunities where funding rate differential is substantial (>0.01% per 8h)
- 2.4 Account for trading fees and funding costs in spread calculation
- 2.5 Rank opportunities by confidence score
- 2.6 Log top 20 opportunities with highest confidence scores
- 2.7 Reject opportunities where projected profit < 0 after all fees and slippage

### 3. Paper Trade Entry Logic with Realistic Execution
**As a** trader  
**I want** the bot to simulate delta-neutral positions with realistic execution fidelity  
**So that** I can validate the strategy with demo capital that reflects real-world conditions

**Acceptance Criteria:**
- 3.1 Simulate long on cheaper exchange, short on expensive exchange simultaneously
- 3.2 Use simulated limit orders with queue position tracking: fill only if cumulative volume at price exceeds 20% of resting depth at entry
- 3.3 Position size based on spread size and available simulated capital ($20,000), capped at 50% per trade
- 3.4 Entry only when confidence score ≥70 AND all hard constraints pass
- 3.5 Log entry price, funding rates, and projected profit
- 3.6 Track available capital and prevent over-leveraging
- 3.7 Implement atomic-like execution: if Leg A (harder fill, typically smaller exchange) doesn't fill within 500ms, cancel Leg B or hedge with marketable limit order
- 3.8 Simulate realistic slippage: 2-5 bps depending on order book depth and position size relative to resting depth
- 3.9 Track which leg is "harder" (smaller exchange/lower liquidity) and prioritize its fill
- 3.10 If one leg fills and other doesn't within 500ms, immediately hedge the naked position with market order

### 4. Paper Trade Exit Logic
**As a** trader  
**I want** the bot to simulate position exits when profit targets or risk limits are hit  
**So that** I can validate exit logic with demo capital

**Acceptance Criteria:**
- 4.1 Exit when funding rates are equalizing (delta <0.005% per 8h)
- 4.2 Exit when 90% of projected profit is realized
- 4.3 Exit if spread widens beyond entry spread + 50 bps
- 4.4 Exit if limit orders don't fill on one side: if first limit order doesn't fill within 500ms, cancel and use market order to prevent naked exposure
- 4.5 Use simulated limit orders for exit with queue position tracking (same 20% volume threshold)
- 4.6 Log exit price, actual profit, and reason for exit
- 4.7 Return capital to available pool after exit
- 4.8 Track "Leg-Out" risk: if one side exits but other doesn't, immediately close naked position at market

### 5. Risk Management with Execution Reality
**As a** trader  
**I want** the bot to enforce strict risk controls on simulated trades  
**So that** I can validate risk management logic against real-world execution

**Acceptance Criteria:**
- 5.1 Maximum position size per trade: Depends on order book depth. If slippage reduces profit potential below 0, reject trade
- 5.2 Maximum total open positions: Unlimited based on available capital (no hard cap)
- 5.3 Stop-loss if spread widens beyond 100 bps from entry
- 5.4 Prevent entry if either exchange is experiencing high slippage (>50 bps)
- 5.5 Prevent entry if order book depth <2x position size on either leg
- 5.6 Log all risk violations and rejected trades
- 5.7 Track "Leg-Out" events: when one side fills but other doesn't, log as risk event
- 5.8 Monitor for DEX gas price sensitivity (Hyperliquid/Paradex): if gas cost changes >$0.20, recalculate profit and reject if negative

### 6. Performance Monitoring with Realistic Metrics
**As a** trader  
**I want** to monitor simulated strategy performance in real-time  
**So that** I can validate the strategy is working correctly and understand real-world viability

**Acceptance Criteria:**
- 6.1 Track total simulated trades executed
- 6.2 Track win rate and average profit per trade
- 6.3 Track cumulative PnL against $20,000 starting capital
- 6.4 Display active positions with entry price, current spread, and projected exit
- 6.5 Log all trades to persistent storage for analysis
- 6.6 Display current available capital and utilization percentage
- 6.7 Track "Leg-Out" events separately: count and total loss from one-sided fills
- 6.8 Calculate realistic APR: (cumulative_pnl / starting_capital) / (days_elapsed / 365)
- 6.9 Compare paper trading PnL to projected PnL to identify over-optimism

## Technical Constraints
- All code in Rust unless impossible
- Use existing Redis infrastructure for data storage
- Respect all exchange rate limits
- No new external dependencies without approval
- Maintain compatibility with existing data collection modules
- Paper trading only - no actual orders placed on exchanges
- Prioritize execution fidelity over optimistic profit projections

## Implementation Notes

### Queue Position Tracking
Limit orders should not fill just because price was touched. Implement volume tracking:
- Track cumulative volume at limit price
- Fill only when cumulative volume ≥ 20% of resting depth at entry
- This prevents over-reporting profits by 30-50%

### Atomic-like Execution
Prevent "Leg-Out" risk (one side fills, other doesn't):
- Identify harder leg (typically smaller exchange)
- If harder leg doesn't fill within 500ms, cancel easier leg
- If one leg fills before other, immediately hedge with market order
- Log all leg-out events as risk violations

### Realistic Slippage Calculation
- Base slippage: 2 bps
- Additional slippage: (position_size / order_book_depth) * 100 bps
- Cap at 5 bps for realistic simulation
- Recalculate projected profit after slippage; reject if negative

### Exchange Latency Monitoring
- Track WebSocket latency for each exchange
- If latency >200ms, mark data as stale
- Reduce confidence score by 30 points
- Prevent entry if any exchange has stale data
