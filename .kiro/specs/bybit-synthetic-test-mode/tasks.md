# Bybit Synthetic Test Mode - Implementation Tasks

## Phase 1: Configuration and Foundation

- [x] 1. Create SyntheticConfig structure
  - [x] 1.1 Define SyntheticConfig struct with fields: synthetic_spread_bps, synthetic_funding_delta, estimated_position_size, max_concurrent_trades, symbols_to_trade
  - [x] 1.2 Implement from_env() method to load from environment variables
  - [x] 1.3 Add validation for config values (spread > 0, funding_delta > 0, etc.)
  - [x] 1.4 Add default values: spread_bps=15.0, funding_delta=0.0001, position_size=1000.0, max_trades=3
  - [x] 1.5 Write unit tests for config loading and validation

- [x] 2. Create Bybit WebSocket connector module
  - [x] 2.1 Create src/strategy/bybit_websocket.rs module
  - [x] 2.2 Define BybitWebSocketConnector struct with MarketProducer field
  - [x] 2.3 Implement connect_async() to Bybit demo WebSocket (wss://stream-demo.bybit.com/v5/public/linear)
  - [x] 2.4 Implement subscription logic for book ticker streams
  - [x] 2.5 Add reconnection logic with exponential backoff

## Phase 2: Market Data Integration

- [x] 3. Implement Bybit message parsing
  - [x] 3.1 Parse Bybit JSON ticker messages to extract bid, ask, timestamp
  - [x] 3.2 Map Bybit symbols to internal symbol IDs using existing symbol_to_id mapping
  - [x] 3.3 Convert parsed data to MarketUpdate struct (symbol_id, bid, ask, timestamp_us)
  - [x] 3.4 Handle parsing errors gracefully (log and skip invalid messages)
  - [x] 3.5 Write unit tests for message parsing with sample Bybit JSON

- [x] 4. Integrate WebSocket with MarketPipeline
  - [x] 4.1 Push parsed MarketUpdate to MarketProducer in message handler
  - [x] 4.2 Handle backpressure (if queue full, drop oldest and retry)
  - [x] 4.3 Add metrics tracking: messages received, parsed, pushed, dropped
  - [x] 4.4 Implement thread pinning for WebSocket thread (pin to core 2)
  - [x] 4.5 Write integration test: WebSocket → Pipeline → Consumer

## Phase 3: Synthetic Opportunity Generator

- [x] 5. Create SyntheticOpportunityGenerator module
  - [x] 5.1 Create src/strategy/synthetic_generator.rs module
  - [x] 5.2 Define SyntheticOpportunityGenerator struct with config and market_data_store fields
  - [x] 5.3 Implement generate_opportunity() method with synthetic price calculation
  - [x] 5.4 Calculate synthetic prices: long_price = real_mid * (1 - spread_bps/20000), short_price = real_mid * (1 + spread_bps/20000)
  - [x] 5.5 Add symbol filtering (only generate for configured symbols)

- [x] 6. Implement dashboard qualification logic
  - [x] 6.1 Copy get_exchange_taker_fee() function from dashboard.rs (lines 1200-1215)
  - [x] 6.2 Implement funding delta check: funding_delta.abs() > 0.0001
  - [x] 6.3 Implement depth estimation based on spread: estimate_depth(spread_bps)
  - [x] 6.4 Implement depth sufficiency check: depth >= position_size * 2.0
  - [x] 6.5 Verify all hard constraints match dashboard.rs lines 250-290

- [x] 7. Implement confidence score calculation
  - [x] 7.1 Implement calculate_confidence_score() matching dashboard.rs lines 650-700
  - [x] 7.2 Calculate spread component: (spread_bps / 50.0).min(1.0) * 100.0 * 0.5
  - [x] 7.3 Calculate funding component: (funding_delta.abs() / 0.01).min(1.0) * 100.0 * 0.3
  - [x] 7.4 Add conservative OBI/OI/gravity estimates (no boost for synthetic mode)
  - [x] 7.5 Verify confidence score >= 70 threshold

- [x] 8. Implement profitability calculation
  - [x] 8.1 Calculate total fees: long_fee_bps + short_fee_bps
  - [x] 8.2 Calculate slippage: 2.0 + (position_size / depth) * 3.0, capped at 5.0
  - [x] 8.3 Calculate projected profit: spread_bps - total_fees_bps - funding_cost_bps - slippage_bps
  - [x] 8.4 Verify projected_profit > 0 threshold
  - [x] 8.5 Write unit tests for profitability calculation with various scenarios

- [x] 9. Create ArbitrageOpportunity from synthetic data
  - [x] 9.1 Populate all ArbitrageOpportunity fields with synthetic values
  - [x] 9.2 Set long_exchange = "bybit", short_exchange = "bybit_synthetic"
  - [x] 9.3 Set timestamp to current system time
  - [x] 9.4 Add logging for generated opportunities (symbol, spread, confidence, profit)
  - [x] 9.5 Write integration test: real market data → synthetic opportunity

## Phase 4: Execution Integration

- [x] 10. Create SingleExchangeExecutor adapter
  - [x] 10.1 Create src/strategy/single_exchange_executor.rs module
  - [x] 10.2 Define SingleExchangeExecutor struct with backend and entry_executor fields
  - [x] 10.3 Implement execute_synthetic_trade() method
  - [x] 10.4 Call EntryExecutor::execute_atomic_entry_real() with synthetic opportunity
  - [x] 10.5 Handle execution results (success, partial fill, failure)

- [x] 11. Integrate with existing atomic execution logic
  - [x] 11.1 Verify execute_atomic_entry_real() works with single exchange (both legs on Bybit)
  - [x] 11.2 Test atomic execution: both legs fill successfully
  - [x] 11.3 Test cancellation: one leg fills, other times out
  - [x] 11.4 Test emergency close: one leg fills, hedge fails
  - [x] 11.5 Verify HedgeTimingMetrics, HedgeLogger, RaceConditionGuard are used

- [x] 12. Add position tracking and monitoring
  - [x] 12.1 Track active trades in memory (Vec<PaperTrade>)
  - [x] 12.2 Monitor active positions for P&L changes
  - [x] 12.3 Implement exit logic when profitable or stop-loss triggered
  - [x] 12.4 Add logging for trade lifecycle (entry, monitoring, exit)
  - [x] 12.5 Enforce max_concurrent_trades limit

## Phase 5: Metrics and Monitoring

- [x] 13. Create TestMetricsCollector
  - [x] 13.1 Create src/strategy/test_metrics.rs module
  - [x] 13.2 Define TestMetricsCollector struct with atomic counters and latency vectors
  - [x] 13.3 Implement record_opportunity(), record_success(), record_failure() methods
  - [x] 13.4 Implement latency tracking: record_websocket_latency(), record_queue_latency(), etc.
  - [x] 13.5 Add thread-safe access using Arc<Mutex<>> for latency vectors

- [x] 14. Implement latency measurement
  - [x] 14.1 Measure WebSocket → Queue latency (target: <0.5ms)
  - [x] 14.2 Measure Queue → Strategy latency (target: <0.1ms)
  - [x] 14.3 Measure opportunity detection latency (target: <2ms)
  - [x] 14.4 Measure order placement latency (target: <5ms)
  - [x] 14.5 Calculate p50, p95, p99 percentiles

- [x] 15. Implement metrics reporting
  - [x] 15.1 Implement report_summary() method to print all metrics
  - [x] 15.2 Report: opportunities generated, trades executed, success rate
  - [x] 15.3 Report: latency percentiles (p50, p95, p99) for each stage
  - [x] 15.4 Report: edge cases (partial fills, cancellations, timeouts, emergency closes)
  - [x] 15.5 Add periodic reporting (every 60 seconds during runtime)

## Phase 6: Main Binary Implementation

- [x] 16. Create bybit-synthetic-test binary
  - [x] 16.1 Create src/bin/bybit-synthetic-test.rs
  - [x] 16.2 Implement main() function with tokio runtime
  - [x] 16.3 Load SyntheticConfig from environment
  - [x] 16.4 Initialize Bybit demo backend and sync server time
  - [x] 16.5 Add error handling and logging

- [x] 17. Set up streaming pipeline
  - [x] 17.1 Create MarketPipeline with default capacity
  - [x] 17.2 Get producer and consumer handles
  - [x] 17.3 Initialize MarketDataStore
  - [x] 17.4 Initialize TestMetricsCollector
  - [x] 17.5 Verify pipeline is ready before starting threads

- [x] 18. Implement WebSocket thread
  - [x] 18.1 Spawn tokio task for WebSocket connector
  - [x] 18.2 Pin WebSocket thread to core 2 using thread_pinning module
  - [x] 18.3 Connect to Bybit demo WebSocket
  - [x] 18.4 Subscribe to configured symbols
  - [x] 18.5 Handle WebSocket errors and reconnection

- [x] 19. Implement strategy thread
  - [x] 19.1 Spawn tokio task for strategy logic
  - [x] 19.2 Pin strategy thread to core 1 using thread_pinning module
  - [x] 19.3 Create SyntheticOpportunityGenerator
  - [x] 19.4 Create SingleExchangeExecutor
  - [x] 19.5 Implement main loop: consume → generate → execute

- [x] 20. Implement main strategy loop
  - [x] 20.1 Pop MarketUpdate from consumer (non-blocking)
  - [x] 20.2 Update MarketDataStore with new data
  - [x] 20.3 Generate synthetic opportunity if conditions met
  - [x] 20.4 Execute trade if opportunity is valid
  - [x] 20.5 Add small sleep (100μs) to avoid busy-waiting

- [x] 21. Add graceful shutdown
  - [x] 21.1 Set up Ctrl+C signal handler
  - [x] 21.2 Set shutdown flag when signal received
  - [x] 21.3 Close all active positions before exit
  - [x] 21.4 Print final metrics summary
  - [x] 21.5 Clean up resources (WebSocket, threads, etc.)

## Phase 7: Testing and Validation

- [x] 22. Write unit tests for synthetic generator
  - [x] 22.1 Test synthetic price calculation with various spreads
  - [x] 22.2 Test hard constraint validation (funding, depth, confidence, profit)
  - [x] 22.3 Test confidence score calculation matches dashboard
  - [x] 22.4 Test profitability calculation with various fee structures
  - [x] 22.5 Test edge cases (zero spread, negative profit, insufficient depth)

- [x] 23. Write integration test: Scenario 1 (Happy Path)
  - [x] 23.1 Generate synthetic opportunity with 15 bps spread
  - [x] 23.2 Execute trade with both legs
  - [x] 23.3 Verify both legs fill within 500ms
  - [x] 23.4 Verify trade becomes active
  - [x] 23.5 Verify P&L tracking works

- [x] 24. Write integration test: Scenario 2 (Cancellation)
  - [x] 24.1 Generate opportunity
  - [x] 24.2 Simulate one leg filling, other timing out
  - [x] 24.3 Verify system cancels filled leg
  - [x] 24.4 Verify atomic execution maintained
  - [x] 24.5 Verify no unhedged position remains

- [x] 25. Write integration test: Scenario 3 (Emergency Close)
  - [x] 25.1 Generate opportunity
  - [x] 25.2 Simulate long leg fills, short leg fails
  - [x] 25.3 Verify emergency close is triggered
  - [x] 25.4 Verify close completes in <1 second
  - [x] 25.5 Verify position is fully closed

- [x] 26. Write integration test: Scenario 4 (Partial Fill)
  - [x] 26.1 Generate opportunity
  - [x] 26.2 Simulate partial fill on one leg
  - [x] 26.3 Verify retry logic activates
  - [x] 26.4 Verify remaining quantity is filled
  - [x] 26.5 Verify total filled quantity matches target

- [x] 27. Write integration test: Scenario 5 (Backpressure)
  - [x] 27.1 Generate 100 opportunities per second
  - [x] 27.2 Verify queue doesn't overflow
  - [x] 27.3 Verify oldest data is dropped when full
  - [x] 27.4 Verify system remains stable
  - [x] 27.5 Verify no crashes or panics

- [x] 28. Write integration test: Scenario 6 (Reconnection)
  - [x] 28.1 Start WebSocket connection
  - [x] 28.2 Simulate disconnect after 10 seconds
  - [x] 28.3 Verify automatic reconnection
  - [x] 28.4 Verify data flow resumes
  - [x] 28.5 Verify no data loss after reconnect

- [x] 29. Write integration test: Scenario 7 (Graceful Shutdown)
  - [x] 29.1 Start system with active trades
  - [x] 29.2 Send Ctrl+C signal
  - [x] 29.3 Verify active trades are closed
  - [x] 29.4 Verify metrics are reported
  - [x] 29.5 Verify clean shutdown (no panics)

## Phase 8: Performance Validation

- [x] 30. Run 24-hour stability test
  - [x] 30.1 Start bybit-synthetic-test binary
  - [x] 30.2 Monitor for 24 hours continuously
  - [x] 30.3 Verify no crashes or panics
  - [x] 30.4 Verify memory usage remains stable (<100MB)
  - [x] 30.5 Document any issues or anomalies

- [x] 31. Validate latency requirements
  - [x] 31.1 Collect latency metrics over 1 hour
  - [x] 31.2 Verify P99 WebSocket → Queue < 0.5ms
  - [x] 31.3 Verify P99 Queue → Strategy < 0.1ms
  - [x] 31.4 Verify P99 opportunity detection < 2ms
  - [x] 31.5 Verify P99 end-to-end < 10ms

- [x] 32. Validate throughput requirements
  - [x] 32.1 Generate high-frequency market data (1000+ updates/sec)
  - [x] 32.2 Verify system processes all updates without dropping
  - [x] 32.3 Verify CPU utilization < 50% on strategy thread
  - [x] 32.4 Verify no memory leaks over extended run
  - [x] 32.5 Document actual throughput achieved

- [x] 33. Validate execution success rate
  - [x] 33.1 Execute 100+ synthetic trades
  - [x] 33.2 Calculate success rate (both legs filled)
  - [x] 33.3 Verify success rate > 95%
  - [x] 33.4 Analyze failures (categorize by type)
  - [x] 33.5 Document edge cases encountered

## Phase 9: Documentation and Deployment

- [x] 34. Create user documentation
  - [x] 34.1 Write README for bybit-synthetic-test binary
  - [x] 34.2 Document environment variables and configuration
  - [x] 34.3 Document how to run the test (cargo run --bin bybit-synthetic-test)
  - [x] 34.4 Document expected output and metrics
  - [x] 34.5 Document troubleshooting common issues

- [x] 35. Create deployment guide
  - [x] 35.1 Document Bybit demo account setup
  - [x] 35.2 Document environment variable configuration
  - [x] 35.3 Document how to interpret metrics
  - [x] 35.4 Document migration path to production (Phase 1-5)
  - [x] 35.5 Document safety limits and risk controls

- [x] 36. Final validation and sign-off
  - [x] 36.1 Run all 7 test scenarios successfully
  - [x] 36.2 Verify all performance requirements met
  - [x] 36.3 Verify all functional requirements met
  - [x] 36.4 Create final validation report
  - [x] 36.5 Get sign-off for production migration planning

## Notes

- All tasks should be completed in order within each phase
- Each task should include appropriate error handling and logging
- All code should follow existing project conventions and style
- Integration tests should use Bybit demo environment (no real money)
- Performance tests should be run on representative hardware
- Documentation should be clear and actionable for other developers
