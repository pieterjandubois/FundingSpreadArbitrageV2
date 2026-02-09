# Paper Trading Monitor - Implementation Tasks

## Phase 1: Core Data Structures

- [x] 1.1 Define PaperTrade, SimulatedOrder, QueuePosition, LegOutEvent structs in `src/strategy/types.rs`
- [x] 1.2 Define PortfolioState struct in `src/strategy/types.rs`
- [x] 1.3 Implement serialization/deserialization for all structs
- [x] 1.4 Create Redis schema documentation

## Phase 2: Entry Execution

- [x] 2.1 Implement position sizing logic in `src/strategy/entry.rs`
- [x] 2.2 Implement harder leg identification in `src/strategy/entry.rs`
- [x] 2.3 Implement slippage calculation in `src/strategy/entry.rs`
- [x] 2.4 Implement atomic dual-leg entry executor in `src/strategy/entry.rs`
- [x] 2.5 Implement queue position tracking in `src/strategy/entry.rs`
- [x] 2.6 Integrate entry executor into strategy runner

## Phase 3: Position Management

- [x] 3.1 Implement unrealized P&L calculation in `src/strategy/positions.rs`
- [x] 3.2 Implement exit condition checking in `src/strategy/positions.rs`
- [x] 3.3 Implement leg-out detection in `src/strategy/positions.rs`
- [x] 3.4 Implement continuous position monitoring loop in strategy runner
- [x] 3.5 Implement exit order execution in strategy runner

## Phase 4: Portfolio Management

- [x] 4.1 Implement PortfolioManager in `src/strategy/portfolio.rs`
- [x] 4.2 Implement capital tracking and allocation
- [x] 4.3 Implement metrics calculation (win rate, APR, etc.)
- [x] 4.4 Implement state persistence to Redis
- [x] 4.5 Implement trade logging

## Phase 5: Monitor Binary

- [x] 5.1 Create `src/bin/trading-monitor.rs` with real-time display
- [x] 5.2 Implement portfolio summary display
- [x] 5.3 Implement active trades table
- [x] 5.4 Implement recent exits table
- [x] 5.5 Implement color-coding for P&L
- [x] 5.6 Implement 1-second update loop
- [x] 5.7 Implement scrolling for large trade lists

## Phase 6: Testing & Validation

- [x] 6.1 Write unit tests for position sizing
- [x] 6.2 Write unit tests for P&L calculation
- [x] 6.3 Write unit tests for exit condition checking
- [x] 6.4 Write property-based tests for capital conservation
- [x] 6.5 Write property-based tests for PnL accuracy
- [x] 6.6 Write integration tests for atomic execution
- [x] 6.7 Manual testing with live data

</content>
</invoke>