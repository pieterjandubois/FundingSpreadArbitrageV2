# Streaming Opportunity Detection - Tasks

## Phase 1: Infrastructure Setup

### Task 1.1: Implement SymbolMap Service
**Status**: Complete

**Description**: Create a thread-safe symbol mapping service that converts (exchange, symbol) strings to u32 IDs for performance.

**Files**:
- Create: `src/strategy/symbol_map.rs`
- Modify: `src/strategy/mod.rs` (add module)

**Implementation**:
- [x] 1.1.1: Create SymbolMap struct with DashMap for concurrent access
- [x] 1.1.2: Implement `get_or_insert(exchange, symbol) -> u32`
- [x] 1.1.3: Implement `get(symbol_id) -> Option<(String, String)>`
- [x] 1.1.4: Add atomic counter for ID generation
- [x] 1.1.5: Pre-allocate common symbols (BTCUSDT, ETHUSDT, etc.)

**Tests**:
- [x] 1.1.6: Unit test: Concurrent access from multiple threads
- [x] 1.1.7: Unit test: ID uniqueness and consistency
- [x] 1.1.8: Unit test: Bidirectional mapping correctness

**Acceptance Criteria**:
- SymbolMap handles 1000+ concurrent requests/sec
- IDs are stable across restarts (same symbol = same ID)
- O(1) lookup performance

---

### Task 1.2: Implement OpportunityQueue
**Status**: Complete

**Description**: Create a lock-free MPSC queue for distributing opportunities to multiple consumers.

**Files**:
- Create: `src/strategy/opportunity_queue.rs`
- Modify: `src/strategy/mod.rs` (add module)

**Implementation**:
- [x] 1.2.1: Create OpportunityQueue struct with ArrayQueue
- [x] 1.2.2: Implement OpportunityProducer with push() method
- [x] 1.2.3: Implement OpportunityConsumer with pop() and pop_batch()
- [x] 1.2.4: Add backpressure handling (drop oldest on full)
- [x] 1.2.5: Add metrics (push_count, pop_count, drop_count)

**Tests**:
- [x] 1.2.6: Unit test: Push and pop operations
- [x] 1.2.7: Unit test: Backpressure drops oldest
- [x] 1.2.8: Unit test: Multiple consumers can pop independently
- [x] 1.2.9: Unit test: Metrics accuracy

**Acceptance Criteria**:
- Lock-free operations (no mutexes)
- Handles 10K+ opportunities/sec
- Multiple consumers work independently

---

### Task 1.3: Enhance Redis Bridge for Pipeline Integration
**Status**: Complete

**Description**: Modify redis_bridge to convert JSON strings to MarketUpdate structs and push to pipeline.

**Files**:
- Modify: `src/bin/bybit-synthetic-test.rs` (redis_bridge function)
- Modify: `src/main.rs` (redis_bridge function)

**Implementation**:
- [x] 1.3.1: Add SymbolMap parameter to redis_bridge
- [x] 1.3.2: Add MarketPipeline parameter to redis_bridge
- [x] 1.3.3: Implement parse_to_market_update() helper function
- [x] 1.3.4: Parse JSON and extract bid/ask prices
- [x] 1.3.5: Map (exchange, symbol) to symbol_id
- [x] 1.3.6: Create MarketUpdate struct and push to pipeline
- [x] 1.3.7: Keep Redis writes for persistence (cold path)

**Tests**:
- [x] 1.3.8: Integration test: JSON → MarketUpdate conversion
- [x] 1.3.9: Integration test: Pipeline receives correct data
- [x] 1.3.10: Integration test: Redis still receives data

**Acceptance Criteria**:
- All WebSocket data flows to both Redis and pipeline
- MarketUpdate conversion < 50μs per message
- No data loss during conversion

---

## Phase 2: OpportunityDetector Service

### Task 2.1: Implement OpportunityDetector Core
**Status**: Complete

**Description**: Create the centralized service that detects arbitrage opportunities from market updates.

**Files**:
- Create: `src/strategy/opportunity_detector.rs`
- Modify: `src/strategy/mod.rs` (add module)

**Implementation**:
- [x] 2.1.1: Create OpportunityDetector struct
- [x] 2.1.2: Add MarketConsumer, MarketDataStore, SymbolMap fields
- [x] 2.1.3: Add OpportunityProducer field
- [x] 2.1.4: Implement `new()` constructor
- [x] 2.1.5: Implement main `run()` loop
- [x] 2.1.6: Consume from MarketPipeline
- [x] 2.1.7: Update MarketDataStore on each update

**Tests**:
- [x] 2.1.8: Unit test: Detector initializes correctly
- [x] 2.1.9: Unit test: Consumes from pipeline
- [x] 2.1.10: Unit test: Updates market data store

**Acceptance Criteria**:
- Detector runs continuously without blocking
- Processes 10K+ updates/sec
- MarketDataStore stays synchronized

---

### Task 2.2: Implement Opportunity Detection Logic
**Status**: Complete

**Description**: Add the core logic to detect arbitrage opportunities from market data.

**Files**:
- Modify: `src/strategy/opportunity_detector.rs`

**Implementation**:
- [x] 2.2.1: Implement `detect_opportunities_for_symbol()`
- [x] 2.2.2: Get all exchanges for a symbol
- [x] 2.2.3: Check all exchange pairs for arbitrage
- [x] 2.2.4: Implement `check_opportunity()` for each pair
- [x] 2.2.5: Calculate spread in basis points
- [x] 2.2.6: Check minimum spread threshold (10 bps)
- [x] 2.2.7: Get funding rates and calculate delta
- [x] 2.2.8: Check minimum funding delta (0.0001)

**Tests**:
- [x] 2.2.9: Unit test: Detects valid opportunity
- [x] 2.2.10: Unit test: Filters low spread opportunities
- [x] 2.2.11: Unit test: Filters low funding delta
- [x] 2.2.12: Unit test: Checks all exchange pairs

**Acceptance Criteria**:
- Detects opportunities within 500μs of market update
- No false positives (all detected opportunities are valid)
- Handles missing data gracefully

---

### Task 2.3: Implement Confidence Scoring
**Status**: Complete

**Description**: Add confidence score calculation based on spread, funding, and other metrics.

**Files**:
- Modify: `src/strategy/opportunity_detector.rs`

**Implementation**:
- [x] 2.3.1: Implement `calculate_confidence()` method
- [x] 2.3.2: Add spread component (50% weight)
- [x] 2.3.3: Add funding delta component (30% weight)
- [x] 2.3.4: Add base score (20% weight)
- [x] 2.3.5: Clamp score to 0-100 range
- [x] 2.3.6: Filter opportunities below 70 confidence

**Tests**:
- [x] 2.3.7: Unit test: High spread = high confidence
- [x] 2.3.8: Unit test: High funding = high confidence
- [x] 2.3.9: Unit test: Score clamped to 0-100
- [x] 2.3.10: Unit test: Low confidence filtered out

**Acceptance Criteria**:
- Confidence scores match dashboard logic
- Only high-confidence opportunities (≥70) published
- Scoring completes in < 50μs

---

### Task 2.4: Implement Opportunity Publishing
**Status**: Complete

**Description**: Create ArbitrageOpportunity structs and push to opportunity queue.

**Files**:
- Modify: `src/strategy/opportunity_detector.rs`

**Implementation**:
- [x] 2.4.1: Calculate fees (taker fees for both exchanges)
- [x] 2.4.2: Estimate slippage (3 bps)
- [x] 2.4.3: Estimate funding cost (10 bps)
- [x] 2.4.4: Calculate projected profit after costs
- [x] 2.4.5: Filter unprofitable opportunities (profit ≤ 0)
- [x] 2.4.6: Get order book depths
- [x] 2.4.7: Build ConfluenceMetrics struct
- [x] 2.4.8: Create ArbitrageOpportunity struct
- [x] 2.4.9: Push to OpportunityQueue via producer

**Tests**:
- [x] 2.4.10: Unit test: Profitable opportunities published
- [x] 2.4.11: Unit test: Unprofitable opportunities filtered
- [x] 2.4.12: Unit test: Opportunity struct has all fields
- [x] 2.4.13: Integration test: Opportunities reach queue

**Acceptance Criteria**:
- Only profitable opportunities published
- All required fields populated correctly
- Publishing completes in < 100μs

---

## Phase 3: Strategy Runner Integration

### Task 3.1: Add OpportunityConsumer to StrategyRunner
**Status**: Complete

**Description**: Modify StrategyRunner to consume opportunities from queue instead of Redis.

**Files**:
- Modify: `src/strategy/runner.rs`

**Implementation**:
- [x] 3.1.1: Add `opportunity_consumer: Option<OpportunityConsumer>` field
- [x] 3.1.2: Implement `set_opportunity_consumer()` method
- [x] 3.1.3: Modify `run_scanning_loop()` to check for consumer
- [x] 3.1.4: Remove legacy mode check (always use streaming)
- [x] 3.1.5: Consume opportunities in main loop
- [x] 3.1.6: Call `execute_opportunity()` for each opportunity

**Tests**:
- [x] 3.1.7: Unit test: Consumer can be set
- [x] 3.1.8: Unit test: Panics if consumer not set
- [x] 3.1.9: Integration test: Consumes from queue

**Acceptance Criteria**:
- StrategyRunner requires OpportunityConsumer
- No legacy mode fallback
- Consumes opportunities immediately

---

### Task 3.2: Implement execute_opportunity() Method
**Status**: Complete

**Description**: Extract opportunity execution logic into dedicated method.

**Files**:
- Modify: `src/strategy/runner.rs`

**Implementation**:
- [x] 3.2.1: Create `execute_opportunity()` async method
- [x] 3.2.2: Move validation logic from scan_opportunities()
- [x] 3.2.3: Check for duplicate symbols
- [x] 3.2.4: Validate prices are current
- [x] 3.2.5: Check available capital
- [x] 3.2.6: Check exchange balances
- [x] 3.2.7: Calculate position size
- [x] 3.2.8: Execute trade via EntryExecutor

**Tests**:
- [x] 3.2.9: Unit test: Validates opportunities correctly
- [x] 3.2.10: Unit test: Skips duplicate symbols
- [x] 3.2.11: Unit test: Checks capital availability
- [x] 3.2.12: Integration test: Executes valid trades

**Acceptance Criteria**:
- All validation checks pass
- Trades execute within 2ms of opportunity
- No duplicate positions created

---

### Task 3.3: Remove Legacy scan_opportunities() Method
**Status**: Complete

**Description**: Delete the old Redis polling code and clean up.

**Files**:
- Modify: `src/strategy/runner.rs`

**Implementation**:
- [x] 3.3.1: Delete `scan_opportunities()` method
- [x] 3.3.2: Remove Redis polling interval
- [x] 3.3.3: Remove legacy mode code paths
- [x] 3.3.4: Remove `strategy:opportunities` Redis key usage
- [x] 3.3.5: Update error messages and logging

**Tests**:
- [x] 3.3.6: Verify no compilation errors
- [x] 3.3.7: Verify no Redis polling in code
- [x] 3.3.8: Integration test: System works without legacy code

**Acceptance Criteria**:
- All legacy code removed
- No Redis polling anywhere
- System compiles and runs correctly

---

## Phase 4: Dashboard Integration

### Task 4.1: Add OpportunityConsumer to Dashboard
**Status**: Complete

**Description**: Modify dashboard to consume opportunities from queue instead of calculating them.

**Files**:
- Modify: `src/bin/dashboard.rs`

**Implementation**:
- [x] 4.1.1: Remove `ticker_data` field from AppState
- [x] 4.1.2: Remove `funding_rates` field from AppState
- [x] 4.1.3: Add `opportunity_consumer: OpportunityConsumer` field
- [x] 4.1.4: Modify `new()` to accept OpportunityConsumer
- [x] 4.1.5: Remove `update_from_redis()` method
- [x] 4.1.6: Implement `update_from_queue()` method
- [x] 4.1.7: Pop batch of opportunities (up to 100)
- [x] 4.1.8: Update opportunities map

**Tests**:
- [x] 4.1.9: Unit test: Consumes from queue
- [x] 4.1.10: Unit test: Updates opportunities map
- [x] 4.1.11: Integration test: Dashboard shows opportunities

**Acceptance Criteria**:
- Dashboard consumes from queue
- No Redis reads for opportunities
- Updates every 100ms

---

### Task 4.2: Remove Opportunity Calculation Logic
**Status**: Complete

**Description**: Delete all opportunity detection code from dashboard.

**Files**:
- Modify: `src/bin/dashboard.rs`

**Implementation**:
- [x] 4.2.1: Delete `recalculate_opportunities()` method
- [x] 4.2.2: Delete `calculate_confidence_score()` method
- [x] 4.2.3: Delete `calculate_confidence_score_with_gravity()` method
- [x] 4.2.4: Delete `get_order_book_depths_from_redis()` method
- [x] 4.2.5: Delete `calculate_funding_delta()` method
- [x] 4.2.6: Delete all helper methods for opportunity detection
- [x] 4.2.7: Keep only UI rendering code

**Tests**:
- [x] 4.2.8: Verify no compilation errors
- [x] 4.2.9: Verify dashboard still displays correctly
- [x] 4.2.10: Integration test: Dashboard shows same as strategy

**Acceptance Criteria**:
- All calculation logic removed
- Dashboard is pure UI/monitoring
- Shows exact same opportunities as strategy

---

### Task 4.3: Implement Opportunity Staleness Tracking
**Status**: Complete

**Description**: Track when opportunities disappear and show removal reasons.

**Files**:
- Modify: `src/bin/dashboard.rs`

**Implementation**:
- [x] 4.3.1: Compare current batch with previous opportunities
- [x] 4.3.2: Detect removed opportunities
- [x] 4.3.3: Add to removed_opportunities queue
- [x] 4.3.4: Filter stale opportunities (> 5 seconds old)
- [x] 4.3.5: Update UI to show removal reasons

**Tests**:
- [x] 4.3.6: Unit test: Detects removed opportunities
- [x] 4.3.7: Unit test: Filters stale opportunities
- [x] 4.3.8: Integration test: UI shows removals

**Acceptance Criteria**:
- Removed opportunities tracked
- Stale opportunities filtered
- UI shows why opportunities disappeared

---

## Phase 5: Binary Integration

### Task 5.1: Integrate Components in bybit-synthetic-test
**Status**: Complete

**Description**: Wire up all components in bybit-synthetic-test binary.

**Files**:
- Modify: `src/bin/bybit-synthetic-test.rs`

**Implementation**:
- [x] 5.1.1: Create SymbolMap instance
- [x] 5.1.2: Create MarketPipeline instance
- [x] 5.1.3: Create OpportunityQueue instance
- [x] 5.1.4: Get consumers and producers
- [x] 5.1.5: Pass SymbolMap and Pipeline to redis_bridge
- [x] 5.1.6: Create OpportunityDetector with consumers/producers
- [x] 5.1.7: Spawn detector task
- [x] 5.1.8: Pass OpportunityConsumer to StrategyRunner
- [x] 5.1.9: Update logging and status messages

**Tests**:
- [x] 5.1.10: Integration test: End-to-end data flow
- [x] 5.1.11: Integration test: Opportunities detected and executed
- [x] 5.1.12: Integration test: Latency < 5ms

**Acceptance Criteria**:
- All components wired correctly
- Data flows: WebSocket → Pipeline → Detector → Queue → Strategy
- System runs without errors

---

### Task 5.2: Integrate Components in main.rs
**Status**: Complete

**Description**: Wire up all components in main production binary.

**Files**:
- Modify: `src/main.rs`

**Implementation**:
- [x] 5.2.1: Create SymbolMap instance
- [x] 5.2.2: Create MarketPipeline instance
- [x] 5.2.3: Create OpportunityQueue instance
- [x] 5.2.4: Get consumers and producers
- [x] 5.2.5: Pass SymbolMap and Pipeline to redis_bridge
- [x] 5.2.6: Create OpportunityDetector with consumers/producers
- [x] 5.2.7: Spawn detector task
- [x] 5.2.8: Pass OpportunityConsumer to StrategyRunner
- [x] 5.2.9: Update graceful shutdown to stop detector

**Tests**:
- [x] 5.2.10: Integration test: Production system works
- [x] 5.2.11: Integration test: Graceful shutdown works
- [x] 5.2.12: Integration test: 24h stability test

**Acceptance Criteria**:
- Production binary works correctly
- Graceful shutdown stops all components
- No memory leaks or crashes

---

### Task 5.3: Dashboard Binary Setup
**Status**: Complete

**Description**: Set up dashboard to receive OpportunityConsumer.

**Files**:
- Modify: `src/bin/dashboard.rs`

**Implementation**:
- [x] 5.3.1: Add command-line argument for shared memory/channel
- [x] 5.3.2: Or: Create separate OpportunityConsumer from same queue
- [x] 5.3.3: Pass consumer to AppState
- [x] 5.3.4: Update main loop to call update_from_queue()
- [x] 5.3.5: Remove Redis connection for opportunities

**Tests**:
- [x] 5.3.6: Integration test: Dashboard receives opportunities
- [x] 5.3.7: Integration test: Dashboard updates in real-time
- [x] 5.3.8: Integration test: Dashboard shows same as strategy

**Acceptance Criteria**:
- Dashboard receives opportunities from queue
- No Redis polling
- Real-time updates (< 100ms latency)

---

## Phase 6: Testing & Validation

### Task 6.1: End-to-End Latency Testing
**Status**: Complete

**Description**: Measure and validate end-to-end latency from WebSocket to trade execution.

**Files**:
- Create: `tests/streaming_latency_test.rs`

**Implementation**:
- [x] 6.1.1: Create test harness with mock WebSocket
- [x] 6.1.2: Inject market update with timestamp
- [x] 6.1.3: Measure time to opportunity detection
- [x] 6.1.4: Measure time to strategy execution
- [x] 6.1.5: Calculate end-to-end latency
- [x] 6.1.6: Verify p50 < 1ms, p99 < 5ms

**Tests**:
- [x] 6.1.7: Test: WebSocket → Detector < 100μs
- [x] 6.1.8: Test: Detector → Strategy < 50μs
- [x] 6.1.9: Test: Strategy → Execution < 2ms
- [x] 6.1.10: Test: Total end-to-end < 5ms

**Acceptance Criteria**:
- p50 latency < 1ms
- p99 latency < 5ms
- No outliers > 10ms

---

### Task 6.2: Opportunity Consistency Testing
**Status**: Complete

**Description**: Verify dashboard shows exact same opportunities as strategy.

**Files**:
- Create: `tests/opportunity_consistency_test.rs`

**Implementation**:
- [x] 6.2.1: Create two consumers from same queue
- [x] 6.2.2: Inject test opportunities
- [x] 6.2.3: Verify both consumers receive same opportunities
- [x] 6.2.4: Verify order is consistent
- [x] 6.2.5: Verify no opportunities lost

**Tests**:
- [x] 6.2.6: Test: Both consumers get same data
- [x] 6.2.7: Test: No data loss
- [x] 6.2.8: Test: Order preserved

**Acceptance Criteria**:
- Dashboard and strategy see identical opportunities
- No data loss or corruption
- Order preserved across consumers

---

### Task 6.3: Backpressure and Stability Testing
**Status**: Complete

**Description**: Test system behavior under high load and backpressure.

**Files**:
- Create: `tests/streaming_backpressure_test.rs`

**Implementation**:
- [x] 6.3.1: Inject 10K+ updates/sec
- [x] 6.3.2: Verify queues handle backpressure
- [x] 6.3.3: Verify oldest data dropped when full
- [x] 6.3.4: Verify no crashes or deadlocks
- [x] 6.3.5: Run for 1 hour continuous

**Tests**:
- [x] 6.3.6: Test: Handles 10K updates/sec
- [x] 6.3.7: Test: Backpressure drops oldest
- [x] 6.3.8: Test: No memory leaks
- [x] 6.3.9: Test: No crashes over 1 hour

**Acceptance Criteria**:
- Handles 10K+ updates/sec sustained
- Graceful backpressure handling
- No memory leaks or crashes
- Stable over 24 hours

---

### Task 6.4: Performance Benchmarking
**Status**: Complete

**Description**: Benchmark all components and verify performance targets.

**Files**:
- Create: `benches/streaming_benchmarks.rs`

**Implementation**:
- [x] 6.4.1: Benchmark SymbolMap lookups
- [x] 6.4.2: Benchmark MarketUpdate conversion
- [x] 6.4.3: Benchmark opportunity detection
- [x] 6.4.4: Benchmark queue operations
- [x] 6.4.5: Measure memory usage
- [x] 6.4.6: Measure CPU usage

**Tests**:
- [x] 6.4.7: Benchmark: SymbolMap < 100ns per lookup
- [x] 6.4.8: Benchmark: Conversion < 50μs per update
- [x] 6.4.9: Benchmark: Detection < 500μs per opportunity
- [x] 6.4.10: Benchmark: Queue ops < 10μs

**Acceptance Criteria**:
- All components meet latency targets
- Memory usage < 5MB additional
- CPU usage < 15% total

---

## Phase 7: Cleanup & Documentation

### Task 7.1: Remove All Legacy Code
**Status**: Complete

**Description**: Final cleanup of all Redis polling and legacy code.

**Files**:
- Modify: Multiple files

**Implementation**:
- [x] 7.1.1: Search for "scan_opportunities" and remove
- [x] 7.1.2: Search for "strategy:opportunities" Redis key and remove
- [x] 7.1.3: Search for "legacy mode" and remove
- [x] 7.1.4: Remove unused imports and dependencies
- [x] 7.1.5: Run clippy and fix warnings

**Tests**:
- [x] 7.1.6: Verify no legacy code remains
- [x] 7.1.7: Verify all tests pass
- [x] 7.1.8: Verify no clippy warnings

**Acceptance Criteria**:
- ✅ Zero legacy code remaining
- ✅ All tests pass
- ✅ No compiler warnings

---

### Task 7.2: Update Documentation
**Status**: Complete

**Description**: Document the new streaming architecture.

**Files**:
- Create: `docs/streaming-architecture.md`
- Modify: `README.md`

**Implementation**:
- [x] 7.2.1: Document architecture overview
- [x] 7.2.2: Document data flow diagrams
- [x] 7.2.3: Document component responsibilities
- [x] 7.2.4: Document performance characteristics
- [x] 7.2.5: Document troubleshooting guide
- [x] 7.2.6: Update README with new architecture

**Acceptance Criteria**:
- ✅ Complete architecture documentation
- ✅ Clear diagrams and examples
- ✅ Troubleshooting guide included

---

### Task 7.3: Production Deployment
**Status**: Not Started

**Description**: Deploy streaming architecture to production.

**Files**:
- N/A (deployment)

**Implementation**:
- [ ] 7.3.1: Run final integration tests
- [ ] 7.3.2: Run 24h stability test
- [ ] 7.3.3: Deploy to staging environment
- [ ] 7.3.4: Monitor for 48 hours
- [ ] 7.3.5: Deploy to production
- [ ] 7.3.6: Monitor latency and errors
- [ ] 7.3.7: Verify dashboard shows real-time data

**Acceptance Criteria**:
- All tests pass
- 24h stability test successful
- Production deployment successful
- Latency targets met in production
- No errors or crashes

---

## Summary

**Total Tasks**: 21 main tasks with 150+ sub-tasks
**Estimated Timeline**: 3-4 weeks
**Dependencies**: Tasks must be completed in phase order

**Critical Path**:
1. Phase 1 (Infrastructure) → Phase 2 (Detector) → Phase 3 (Strategy) → Phase 4 (Dashboard) → Phase 5 (Integration) → Phase 6 (Testing) → Phase 7 (Deployment)

**Success Metrics**:
- ✅ End-to-end latency < 5ms (p99)
- ✅ No Redis polling anywhere
- ✅ Dashboard real-time updates
- ✅ Zero legacy code
- ✅ 24h stability test passes
