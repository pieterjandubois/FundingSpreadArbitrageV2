# Streaming Opportunity Detection - Requirements

## Overview
Integrate the low-latency streaming pipeline with opportunity detection to eliminate Redis polling and achieve end-to-end streaming architecture for bybit-synthetic-test.

## Current State (Problems)

### Architecture Gap
1. **WebSocket connectors** write market data to Redis (string keys/values)
2. **Streaming pipeline** expects MarketUpdate structs with symbol_id (pre-mapped integers)
3. **Dashboard** reads from Redis, calculates opportunities, but doesn't write them anywhere
4. **Strategy runner** has two modes:
   - LEGACY: Polls Redis for pre-calculated opportunities (500ms delay)
   - STREAMING: Consumes from pipeline but has TODO for opportunity detection
5. **bybit-synthetic-test** currently uses LEGACY mode (defeats the purpose of low-latency work)

### The Problem
- Streaming pipeline was built but opportunity detection wasn't integrated
- Dashboard calculates opportunities but they're not accessible to strategy runner
- bybit-synthetic-test falls back to 500ms polling instead of using streaming architecture

## Goals

### Primary Goal
Enable bybit-synthetic-test to use full streaming architecture:
```
WebSocket → Pipeline → Opportunity Detection → Trade Execution
```

### Performance Targets
- **Latency**: < 1ms from market update to opportunity detection
- **No polling**: Eliminate 500ms Redis polling delay
- **Lock-free**: Use existing pipeline's lock-free queue architecture

## User Stories

### 1. As a trader, I want opportunities detected immediately when market data arrives
**Acceptance Criteria:**
- 1.1: Market updates flow through streaming pipeline (not Redis)
- 1.2: Opportunity detection runs on every market update
- 1.3: No polling delays (process immediately)
- 1.4: Latency < 1ms from update to detection

### 2. As a trader, I want the dashboard to show opportunities in real-time using streaming
**Acceptance Criteria:**
- 2.1: Dashboard consumes from opportunity queue (not Redis)
- 2.2: Dashboard updates immediately when opportunities change
- 2.3: Dashboard shows exact same opportunities that strategy is trading
- 2.4: No legacy Redis polling code in dashboard
- 2.5: Dashboard latency < 100ms (acceptable for UI updates)

### 3. As a trader, I want bybit-synthetic-test to execute trades with minimal latency
**Acceptance Criteria:**
- 3.1: Uses streaming mode (not legacy polling)
- 3.2: Executes real order on Bybit demo
- 3.3: Simulates other leg instantly
- 3.4: End-to-end latency < 5ms from opportunity detection to order submission

### 4. As a developer, I want a clean architecture with no legacy code
**Acceptance Criteria:**
- 4.1: All components use streaming architecture
- 4.2: No Redis polling in any component
- 4.3: Single source of truth for opportunity detection
- 4.4: Reusable, testable components
- 4.5: Clear separation: hot path (trading) vs monitoring (dashboard)

## Architecture Options

### Option A: Dashboard Writes to Redis (Quick Fix - NOT RECOMMENDED)
- Dashboard calculates opportunities and writes to `strategy:opportunities` key
- Strategy runner polls Redis every 500ms
- **Pros**: Simple, works immediately
- **Cons**: Defeats purpose of streaming pipeline, 500ms latency, Redis bottleneck

### Option B: Opportunity Detection in Strategy Runner Only
- Strategy runner detects opportunities from streaming pipeline
- Dashboard reads from Redis for monitoring only (cold path)
- **Pros**: True streaming for trading, low latency
- **Cons**: Dashboard still uses legacy Redis polling, inconsistent architecture

### Option C: Shared Opportunity Detection Service (RECOMMENDED)
- Centralized OpportunityDetector service consumes from market pipeline
- Publishes detected opportunities to lock-free opportunity queue
- Strategy runner consumes from opportunity queue (hot path)
- Dashboard also consumes from opportunity queue (monitoring)
- **Pros**: 
  - Single source of truth for opportunities
  - Both dashboard and strategy use streaming
  - Clean separation of concerns
  - Reusable, testable
  - No legacy code or Redis polling
- **Cons**: Additional component (but cleaner architecture)

## Recommended Approach: Option C (Shared Service)

### Complete Streaming Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     WebSocket Connectors                         │
│  (Bybit, OKX, KuCoin, Bitget, Hyperliquid, Paradex)            │
└────────────────────────┬────────────────────────────────────────┘
                         │ Market data (JSON strings)
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Redis Bridge                                │
│  • Converts JSON → MarketUpdate structs                         │
│  • Maps (exchange, symbol) → symbol_id                          │
│  • Pushes to MarketPipeline (hot path)                          │
│  • Writes to Redis (cold path - persistence)                    │
└────────────────────────┬────────────────────────────────────────┘
                         │ MarketUpdate structs
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                    MarketPipeline (Lock-free SPSC)              │
│  • Stores latest bid/ask for all symbols                        │
│  • Lock-free queue (already implemented)                        │
└────────────────────────┬────────────────────────────────────────┘
                         │ MarketUpdate stream
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│              OpportunityDetector Service (NEW)                   │
│  • Consumes from MarketPipeline                                 │
│  • Maintains market state (MarketDataStore)                     │
│  • Detects arbitrage opportunities                              │
│  • Calculates spread, funding, confidence                       │
│  • Applies hard constraints                                     │
│  • Generates ArbitrageOpportunity structs                       │
└────────────────────────┬────────────────────────────────────────┘
                         │ ArbitrageOpportunity structs
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│              OpportunityQueue (Lock-free MPSC)                   │
│  • Stores detected opportunities                                │
│  • Multiple consumers (strategy + dashboard)                    │
│  • Similar to MarketPipeline design                             │
└────────────┬───────────────────────────────┬────────────────────┘
             │                               │
             │ (Hot Path)                    │ (Monitoring)
             ▼                               ▼
┌──────────────────────────┐    ┌──────────────────────────────┐
│   Strategy Runner        │    │       Dashboard              │
│  • Consumes opportunities│    │  • Consumes opportunities    │
│  • Validates & executes  │    │  • Displays in UI            │
│  • Bybit real + simulate │    │  • Updates every 100ms       │
│  • < 5ms latency         │    │  • No calculation logic      │
└──────────────────────────┘    └──────────────────────────────┘
```

### Key Benefits

1. **Single Source of Truth**: OpportunityDetector is the only place that calculates opportunities
2. **True Streaming**: No Redis polling anywhere - all components use lock-free queues
3. **Low Latency**: < 5ms end-to-end from market update to trade execution
4. **Clean Separation**: Hot path (trading) and monitoring (dashboard) clearly separated
5. **Reusable**: OpportunityDetector can be used by other services
6. **Testable**: Each component can be tested independently
7. **No Legacy Code**: All Redis polling and old logic removed

### Data Flow Example

1. **Market Update**: Bybit WebSocket receives BTCUSDT bid=50000, ask=50001
2. **Redis Bridge**: Converts to MarketUpdate{symbol_id: 1, bid: 50000, ask: 50001}
3. **MarketPipeline**: Stores in lock-free queue
4. **OpportunityDetector**: 
   - Reads update
   - Checks all exchange pairs for BTCUSDT
   - Finds: OKX bid=50010 (can sell) vs Bybit ask=50001 (can buy)
   - Calculates spread: 9 bps
   - Checks funding, depth, latency
   - Generates ArbitrageOpportunity
5. **OpportunityQueue**: Stores opportunity
6. **Strategy Runner**: Consumes opportunity, executes trade (< 5ms)
7. **Dashboard**: Consumes opportunity, displays in UI (< 100ms)

### Phase 1: Integrate WebSocket → Pipeline
**Goal**: Get market data flowing through pipeline instead of just Redis

1.1: Create symbol mapping service (exchange+symbol → symbol_id)
1.2: Modify redis_bridge to convert string data to MarketUpdate structs
1.3: Push MarketUpdate to pipeline (hot path)
1.4: Keep Redis writes for persistence (cold path)
1.5: Verify pipeline receives data correctly

### Phase 2: Centralized Opportunity Detection Service
**Goal**: Single service that detects opportunities from streaming data

2.1: Create OpportunityDetector service that:
  - Consumes from market pipeline
  - Maintains market state (latest bid/ask per exchange+symbol)
  - Detects arbitrage opportunities on each update
  - Uses dashboard's qualification logic (spread, funding, confidence)
  - Publishes to opportunity queue

2.2: Create OpportunityQueue (lock-free SPSC or MPSC)
  - Similar to MarketPipeline
  - Stores detected opportunities
  - Multiple consumers (strategy + dashboard)

2.3: Implement opportunity detection logic:
  - Check all exchange pairs for each symbol
  - Calculate spread, funding delta
  - Apply hard constraints (depth, latency, funding)
  - Calculate confidence score
  - Generate ArbitrageOpportunity structs

### Phase 3: Strategy Runner Integration
**Goal**: Strategy consumes opportunities from queue

3.1: Strategy runner consumes from opportunity queue
3.2: Remove scan_opportunities() method (Redis polling)
3.3: Remove legacy mode from run_scanning_loop()
3.4: Execute trades immediately on opportunity detection
3.5: Verify end-to-end latency < 5ms

### Phase 4: Dashboard Streaming Integration
**Goal**: Dashboard uses streaming architecture

4.1: Dashboard consumes from opportunity queue
4.2: Remove Redis polling from dashboard
4.3: Remove opportunity calculation logic from dashboard
4.4: Dashboard displays opportunities in real-time
4.5: Update UI every 100ms (batch updates for smooth rendering)

### Phase 5: Cleanup and Optimization
**Goal**: Remove all legacy code

5.1: Delete legacy Redis polling code
5.2: Remove dashboard's opportunity calculation logic
5.3: Optimize symbol mapping (use perfect hash or array)
5.4: Add metrics and monitoring
5.5: Performance testing and validation

## Technical Constraints

### Symbol Mapping
- Pipeline uses u32 symbol_id for performance
- Need bidirectional map: (exchange, symbol) ↔ symbol_id
- Map must be lock-free or use RwLock (read-heavy)

### Market State
- Need to track latest bid/ask for all exchange+symbol pairs
- Use MarketDataStore (already exists)
- Store in memory for fast lookups

### Opportunity Detection Logic
- Reuse dashboard's qualification logic
- Check spread, funding delta, confidence
- Must be fast (< 1ms per update)

## Non-Functional Requirements

### Performance
- Market update processing: < 100μs
- Opportunity detection: < 1ms
- No allocations in hot path
- Lock-free where possible

### Reliability
- Handle missing data gracefully
- Don't crash on bad market updates
- Log errors but continue processing

### Maintainability
- Clear separation: hot path vs cold path
- Reusable opportunity detection logic
- Well-documented symbol mapping

## Out of Scope
- Multi-exchange routing (already implemented)
- Position management (already implemented)
- Risk management (already implemented)
- Historical data storage

## Success Metrics
- bybit-synthetic-test uses streaming mode
- Latency < 5ms end-to-end
- No Redis polling in hot path
- Dashboard shows real-time opportunities
- Legacy mode code removed
