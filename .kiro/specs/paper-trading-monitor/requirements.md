# Paper Trading Monitor - Requirements

## Overview
Implement a paper trading execution engine that automatically executes spread arbitrage trades when opportunities meet strategy criteria, and provides real-time monitoring of active positions and portfolio performance.

## User Stories

### 1. Automatic Trade Execution
**As a** trader  
**I want** the bot to automatically execute trades when opportunities meet confidence and profit criteria  
**So that** I can validate the strategy with realistic execution without manual intervention

**Acceptance Criteria:**
- 1.1 Execute trade when confidence score ≥ 70 AND projected profit > 0
- 1.2 Allocate capital: $20,000 total, split equally across 8 exchanges ($2,500 each)
- 1.3 Position size: up to 50% of available capital per trade
- 1.4 Use 1x leverage only (no margin)
- 1.5 Track available capital and prevent over-leveraging
- 1.6 Log all trade entries with timestamp, prices, and projected profit

### 2. Active Position Monitoring
**As a** trader  
**I want** to see all active trades with current spread and unrealized P&L  
**So that** I can understand portfolio exposure and trade performance in real-time

**Acceptance Criteria:**
- 2.1 Display active trade count
- 2.2 Show ticker, entry prices, current prices, and current spread for each trade
- 2.3 Show long exchange and short exchange for each trade
- 2.4 Calculate and display unrealized P&L for each trade
- 2.5 Calculate and display projected P&L (if exit conditions met)
- 2.6 Update display every 1 second
- 2.7 Color-code P&L: green (positive), red (negative)

### 3. Exit Condition Monitoring
**As a** trader  
**I want** the bot to automatically exit trades when profit targets or loss limits are hit  
**So that** I can validate exit logic and risk management

**Acceptance Criteria:**
- 3.1 Exit when 90% of projected profit is realized (spread tightening)
- 3.2 Exit when loss exceeds 30% of projected profit potential
- 3.3 Exit when funding rates converge (delta < 0.005% per 8h)
- 3.4 Log all exits with timestamp, exit prices, actual profit, and exit reason
- 3.5 Return capital to available pool after exit

### 4. Portfolio Performance Tracking
**As a** trader  
**I want** to see overall portfolio metrics and performance  
**So that** I can validate strategy profitability and risk management

**Acceptance Criteria:**
- 4.1 Display total trades executed (count)
- 4.2 Display win rate (winning trades / total trades)
- 4.3 Display cumulative P&L (total profit/loss)
- 4.4 Display portfolio total P&L change from starting capital
- 4.5 Display current available capital
- 4.6 Display capital utilization percentage
- 4.7 Display realistic APR: (cumulative_pnl / starting_capital) / (days_elapsed / 365)
- 4.8 Track leg-out events separately (one side fills, other doesn't)
- 4.9 Compare paper trading P&L to projected P&L to identify over-optimism

### 5. Trade Execution Fidelity
**As a** trader  
**I want** realistic trade execution simulation  
**So that** I can validate strategy viability with real-world conditions

**Acceptance Criteria:**
- 5.1 Simulate limit orders with queue position tracking
- 5.2 Fill only when cumulative volume at price ≥ 20% of resting depth
- 5.3 Simulate realistic slippage: 2-5 bps based on position size vs order book depth
- 5.4 Implement atomic execution: if one leg fills, other must fill or be hedged within 500ms
- 5.5 Log all leg-out events (one side fills, other doesn't)
- 5.6 Prevent naked positions: hedge unfilled leg with market order if timeout

## Technical Constraints
- All code in Rust
- Use existing Redis infrastructure for data storage
- Paper trading only - no actual orders placed
- Prioritize execution fidelity over optimistic profit projections
- Maintain compatibility with existing strategy runner

## Implementation Notes

### Capital Allocation
- Starting capital: $20,000
- Split equally: $2,500 per exchange (8 exchanges)
- Position size per trade: up to 50% of available capital
- Minimum position size: $100 (to avoid dust)

### Exit Conditions (Priority Order)
1. Profit target: 90% of projected profit realized
2. Loss limit: loss > 30% of projected profit
3. Funding convergence: delta < 0.005% per 8h
4. Leg-out hedge: if one side fills, immediately hedge other

### Realistic Slippage
- Base: 2 bps
- Additional: (position_size / order_book_depth) * 3 bps
- Cap: 5 bps maximum

### Queue Position Tracking
- Track cumulative volume at limit price
- Fill only when cumulative volume ≥ 20% of resting depth at entry
- Prevents over-reporting profits by 30-50%

</content>
</invoke>