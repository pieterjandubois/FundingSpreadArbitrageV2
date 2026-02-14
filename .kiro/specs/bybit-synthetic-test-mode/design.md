# Bybit Synthetic Test Mode Design

## Overview

This design implements a safe testing environment that validates the low-latency arbitrage system using only Bybit demo. It integrates real WebSocket market data with synthetic opportunity generation to test all execution paths without requiring a second exchange.

## Architecture

### High-Level Data Flow

```
┌─────────────────┐     ┌──────────────┐     ┌────────────────┐
│ Bybit Demo      │────▶│ MarketPipeline│────▶│ Strategy Thread│
│ WebSocket       │     │ (SPSC Queue)  │     │ (Core 1)       │
│ (Core 2)        │     │               │     │                │
└─────────────────┘     └──────────────┘     └────────────────┘
                                                      │
                                                      ▼
                                              ┌────────────────┐
                                              │ Synthetic      │
                                              │ Opportunity    │
                                              │ Generator      │
                                              └────────────────┘
                                                      │
                                                      ▼
                                              ┌────────────────┐
                                              │ EntryExecutor  │
                                              │ (Atomic)       │
                                              └────────────────┘
                                                      │
                                                      ▼
                                              ┌────────────────┐
                                              │ Bybit Demo API │
                                              │ (Both Legs)    │
                                              └────────────────┘
```

### Key Components

1. **BybitWebSocketConnector**: Connects to Bybit demo, pushes to MarketPipeline
2. **SyntheticOpportunityGenerator**: Creates arbitrage opportunities from single-exchange data
3. **SingleExchangeExecutor**: Adapts dual-leg execution for single exchange
4. **TestMetricsCollector**: Tracks performance and validates requirements

## Component Design

### 1. Bybit WebSocket Connector



#### Implementation

```rust
use arbitrage2::strategy::pipeline::MarketProducer;
use tokio_tungstenite::{connect_async, tungstenite::Message};

struct BybitWebSocketConnector {
    producer: MarketProducer,
    symbols: Vec<String>,
}

impl BybitWebSocketConnector {
    async fn connect_and_stream(&self) -> Result<(), Box<dyn Error>> {
        let url = "wss://stream-demo.bybit.com/v5/public/linear";
        let (ws_stream, _) = connect_async(url).await?;
        
        // Subscribe to book ticker for all symbols
        let subscribe_msg = json!({
            "op": "subscribe",
            "args": self.symbols.iter()
                .map(|s| format!("tickers.{}", s))
                .collect::<Vec<_>>()
        });
        
        // Stream messages and push to pipeline
        while let Some(msg) = ws_stream.next().await {
            let data = msg?;
            if let Message::Text(text) = data {
                if let Ok(update) = self.parse_to_market_update(&text) {
                    self.producer.push(update);
                }
            }
        }
        
        Ok(())
    }
    
    fn parse_to_market_update(&self, json_str: &str) -> Result<MarketUpdate, Error> {
        // Parse Bybit JSON to MarketUpdate
        // Use symbol_to_id mapping for efficient lookups
        // Extract bid, ask, timestamp
        unimplemented!()
    }
}
```

### 2. Synthetic Opportunity Generator (Integrates Dashboard Logic)

The generator replicates the EXACT qualification logic from `src/bin/dashboard.rs` to ensure synthetic opportunities match production criteria.

#### Dashboard Qualification Logic (from dashboard.rs lines 250-400)

```rust
// HARD CONSTRAINTS (must all pass):
// 1. Funding delta > 0.01% per 8 hours (0.0001)
let funding_delta_substantial = funding_delta.abs() > 0.0001;

// 2. Order book depth >= position_size * 2.0 on both legs
let depth_sufficient = depth_long >= estimated_position_size * 2.0 
    && depth_short >= estimated_position_size * 2.0;

// 3. Confidence score >= 70
let confidence_score = calculate_confidence_score_with_gravity(
    spread_bps, funding_delta, long_ex, short_ex
);

// 4. Projected profit > 0 after all costs
let projected_profit_bps = spread_bps - total_fees_bps 
    - funding_cost_bps - slippage_bps;

// Only show if: confidence >= 70 AND projected_profit > 0
if confidence_score >= 70 && projected_profit_bps > 0.0 {
    // Valid opportunity
}
```

#### Confidence Score Calculation (from dashboard.rs lines 650-700)

```rust
fn calculate_confidence_score_with_gravity(
    spread_bps: f64,
    funding_delta: f64,
    long_exchange: &str,
    short_exchange: &str
) -> u8 {
    // Base score (80% weight)
    let spread_score = (spread_bps / 50.0).min(1.0) * 100.0;
    let funding_score = (funding_delta.abs() / 0.01).min(1.0) * 100.0;
    let mut score = spread_score * 0.5 + funding_score * 0.3;
    
    // OBI boost (10% weight): +20 if aligned, +10 if partial
    let obi_boost = calculate_obi_boost(long_exchange, short_exchange);
    score = score.saturating_add(obi_boost).min(100);
    
    // OI boost (10% weight): +15 if low OI, -10 if high OI
    let oi_boost = calculate_oi_boost(long_exchange, short_exchange);
    score = if oi_boost < 0 {
        score.saturating_sub(oi_boost.abs() as u8)
    } else {
        score.saturating_add(oi_boost as u8).min(100)
    };
    
    // Funding gravity boost: +30 if <5min to payout, +15 if <15min
    let gravity_boost = calculate_funding_gravity_boost(long_exchange);
    score = score.saturating_add(gravity_boost).min(100);
    
    score
}
```

#### Synthetic Generator Implementation

```rust
struct SyntheticOpportunityGenerator {
    config: SyntheticConfig,
    market_data_store: Arc<RwLock<MarketDataStore>>,
}

struct SyntheticConfig {
    synthetic_spread_bps: f64,      // Default: 15 bps
    synthetic_funding_delta: f64,   // Default: 0.01% (0.0001)
    estimated_position_size: f64,   // Default: $1000
    symbols_to_trade: Vec<String>,  // e.g., ["BTCUSDT", "ETHUSDT"]
}

impl SyntheticOpportunityGenerator {
    fn generate_opportunity(
        &self,
        symbol: &str,
        real_bid: f64,
        real_ask: f64,
    ) -> Option<ArbitrageOpportunity> {
        let real_mid = (real_bid + real_ask) / 2.0;
        
        // Calculate synthetic prices
        let spread_bps = self.config.synthetic_spread_bps;
        let long_price = real_mid * (1.0 - spread_bps / 20000.0);
        let short_price = real_mid * (1.0 + spread_bps / 20000.0);
        
        // Simulate "long" and "short" exchanges (both Bybit, different prices)
        let long_exchange = "bybit";
        let short_exchange = "bybit_synthetic";
        
        // Apply HARD CONSTRAINTS from dashboard
        
        // 1. Funding delta check
        let funding_delta = self.config.synthetic_funding_delta;
        if funding_delta.abs() <= 0.0001 {
            return None; // Fails constraint
        }
        
        // 2. Depth check (estimate based on spread)
        let estimated_depth = self.estimate_depth(spread_bps);
        let position_size = self.config.estimated_position_size;
        if estimated_depth < position_size * 2.0 {
            return None; // Fails constraint
        }
        
        // 3. Calculate confidence score (dashboard formula)
        let confidence_score = self.calculate_confidence_score(
            spread_bps,
            funding_delta,
            long_exchange,
            short_exchange,
        );
        
        if confidence_score < 70 {
            return None; // Fails constraint
        }
        
        // 4. Calculate projected profit
        let long_fee_bps = get_exchange_taker_fee(long_exchange);
        let short_fee_bps = get_exchange_taker_fee(short_exchange);
        let total_fees_bps = long_fee_bps + short_fee_bps;
        let funding_cost_bps = 10.0; // 10 bps for funding cost
        let slippage_bps = self.calculate_slippage(position_size, estimated_depth);
        
        let projected_profit_bps = spread_bps 
            - total_fees_bps 
            - funding_cost_bps 
            - slippage_bps;
        
        if projected_profit_bps <= 0.0 {
            return None; // Fails constraint
        }
        
        // All constraints passed - create opportunity
        Some(ArbitrageOpportunity {
            symbol: symbol.to_string(),
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            long_price,
            short_price,
            spread_bps,
            funding_delta_8h: funding_delta,
            order_book_depth_long: estimated_depth,
            order_book_depth_short: estimated_depth,
            confidence_score,
            projected_profit_after_slippage: projected_profit_bps,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        })
    }
    
    fn calculate_confidence_score(
        &self,
        spread_bps: f64,
        funding_delta: f64,
        _long_exchange: &str,
        _short_exchange: &str,
    ) -> u8 {
        // Replicate dashboard formula exactly
        let spread_score = (spread_bps / 50.0).min(1.0) * 100.0;
        let funding_score = (funding_delta.abs() / 0.01).min(1.0) * 100.0;
        let mut score = spread_score * 0.5 + funding_score * 0.3;
        
        // For synthetic mode, we can't calculate real OBI/OI/gravity
        // So we use conservative estimates:
        // - No OBI boost (assume neutral)
        // - No OI boost (assume normal)
        // - No gravity boost (assume mid-cycle)
        
        score as u8
    }
    
    fn estimate_depth(&self, spread_bps: f64) -> f64 {
        // Dashboard fallback logic (lines 280-290)
        if spread_bps > 200.0 {
            5000.0  // Low liquidity altcoins
        } else if spread_bps > 100.0 {
            10000.0 // Medium liquidity
        } else {
            50000.0 // High liquidity
        }
    }
    
    fn calculate_slippage(&self, position_size: f64, depth: f64) -> f64 {
        // Dashboard formula (lines 320-325)
        let slippage_bps = 2.0 + (position_size / depth) * 3.0;
        slippage_bps.min(5.0)
    }
}

#[inline(always)]
fn get_exchange_taker_fee(exchange: &str) -> f64 {
    // Dashboard formula (lines 1200-1215)
    match exchange.to_lowercase().as_str() {
        "binance" => 4.0,
        "okx" => 5.0,
        "bybit" | "bybit_synthetic" => 5.5,
        "bitget" => 6.0,
        "kucoin" => 6.0,
        "hyperliquid" => 3.5,
        "paradex" => 5.0,
        "gateio" => 6.0,
        _ => 6.0,
    }
}
```

### 3. Single-Exchange Execution Adapter

Adapts the dual-leg execution logic to work on a single exchange by using slightly different prices.

```rust
struct SingleExchangeExecutor {
    backend: Arc<dyn ExecutionBackend>,
    entry_executor: EntryExecutor,
}

impl SingleExchangeExecutor {
    async fn execute_synthetic_trade(
        &self,
        opportunity: &ArbitrageOpportunity,
        position_size: f64,
    ) -> Result<PaperTrade, String> {
        // Both legs execute on Bybit demo, but at different prices
        // This tests the atomic execution logic without needing two exchanges
        
        // Use the existing execute_atomic_entry_real() function
        // It will place both orders on Bybit demo
        EntryExecutor::execute_atomic_entry_real(
            opportunity,
            position_size,
            position_size,
            self.backend.clone(),
        ).await
    }
}
```

### 4. Test Metrics Collector

Tracks performance metrics and validates requirements.

```rust
struct TestMetricsCollector {
    // Latency tracking
    websocket_to_queue_latencies: Vec<Duration>,
    queue_to_strategy_latencies: Vec<Duration>,
    opportunity_detection_latencies: Vec<Duration>,
    order_placement_latencies: Vec<Duration>,
    
    // Execution tracking
    opportunities_generated: AtomicU64,
    trades_executed: AtomicU64,
    trades_successful: AtomicU64,
    trades_failed: AtomicU64,
    emergency_closes: AtomicU64,
    
    // Edge case tracking
    partial_fills: AtomicU64,
    cancellations: AtomicU64,
    timeouts: AtomicU64,
}

impl TestMetricsCollector {
    fn report_summary(&self) {
        println!("\n=== Test Metrics Summary ===");
        println!("Opportunities Generated: {}", self.opportunities_generated.load(Ordering::Relaxed));
        println!("Trades Executed: {}", self.trades_executed.load(Ordering::Relaxed));
        println!("Success Rate: {:.2}%", self.calculate_success_rate());
        println!("\nLatency Percentiles:");
        println!("  P50: {:?}", self.calculate_p50());
        println!("  P95: {:?}", self.calculate_p95());
        println!("  P99: {:?}", self.calculate_p99());
    }
}
```

## Binary Structure

### New Binary: `src/bin/bybit-synthetic-test.rs`

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 1. Load configuration
    let config = SyntheticConfig::from_env()?;
    
    // 2. Initialize Bybit demo backend
    let backend = Arc::new(TestnetBackend::new(config.testnet_config));
    backend.sync_server_time().await?;
    
    // 3. Create streaming pipeline
    let pipeline = MarketPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // 4. Start WebSocket connector (separate thread, pinned to core 2)
    let ws_handle = tokio::spawn(async move {
        pin_to_core(2);
        let connector = BybitWebSocketConnector::new(producer, config.symbols);
        connector.connect_and_stream().await
    });
    
    // 5. Start strategy thread (pinned to core 1)
    let strategy_handle = tokio::spawn(async move {
        pin_to_core(1);
        
        let generator = SyntheticOpportunityGenerator::new(config);
        let executor = SingleExchangeExecutor::new(backend);
        let metrics = TestMetricsCollector::new();
        
        // Main loop: consume market data, generate opportunities, execute trades
        loop {
            if let Some(update) = consumer.pop() {
                // Update market data store
                market_data_store.update_from_market_update(&update);
                
                // Generate synthetic opportunity
                if let Some(opp) = generator.generate_opportunity(
                    &update.symbol,
                    update.bid,
                    update.ask,
                ) {
                    metrics.record_opportunity();
                    
                    // Execute trade
                    match executor.execute_synthetic_trade(&opp, 100.0).await {
                        Ok(trade) => metrics.record_success(),
                        Err(e) => metrics.record_failure(&e),
                    }
                }
            }
            
            tokio::time::sleep(Duration::from_micros(100)).await;
        }
    });
    
    // 6. Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;
    
    // 7. Graceful shutdown
    metrics.report_summary();
    
    Ok(())
}
```

## Configuration

### Environment Variables

```bash
# Bybit demo credentials
BYBIT_DEMO_API_KEY=your_key
BYBIT_DEMO_API_SECRET=your_secret

# Synthetic test configuration
SYNTHETIC_SPREAD_BPS=15.0
SYNTHETIC_FUNDING_DELTA=0.0001
ESTIMATED_POSITION_SIZE=1000.0
MAX_CONCURRENT_TRADES=3
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT
```

## Testing Scenarios

### Scenario 1: Happy Path (Both Legs Fill)
- Generate opportunity with 15 bps spread
- Place limit orders on both legs
- Both fill within 500ms
- Trade becomes active
- Monitor P&L
- Exit when profitable

### Scenario 2: One Leg Times Out (Cancellation)
- Generate opportunity
- Place both orders
- One fills, other times out
- System cancels filled leg
- Verify atomic execution

### Scenario 3: Hedge Fails (Emergency Close)
- Generate opportunity
- Long leg fills
- Short leg fails to place
- System executes emergency close
- Verify <1 second close time

### Scenario 4: Partial Fill (Retry Logic)
- Generate opportunity
- Order partially fills
- System retries for remaining quantity
- Verify retry logic works

### Scenario 5: High Throughput (Backpressure)
- Generate 100 opportunities/second
- Verify queue doesn't overflow
- Verify oldest data is dropped
- Verify no crashes

### Scenario 6: WebSocket Disconnect (Reconnection)
- Simulate WebSocket disconnect
- Verify automatic reconnection
- Verify no data loss after reconnect

### Scenario 7: Graceful Shutdown (Active Trades)
- Start trades
- Send Ctrl+C
- Verify active trades are closed
- Verify clean shutdown

## Success Criteria

### Functional Requirements
- All 7 test scenarios pass
- Opportunity qualification matches dashboard logic exactly
- Atomic execution works correctly
- Emergency close completes in <1 second

### Performance Requirements
- P99 latency <10ms end-to-end
- Process 1000+ market updates/second
- Zero data loss during normal operation
- <100MB memory usage

### Reliability Requirements
- Run for 24 hours without crashes
- Handle WebSocket disconnects gracefully
- Recover from all Bybit API errors
- Clean shutdown with active trades

## Migration Path to Production

After validating with synthetic test mode:

1. **Phase 1**: Run synthetic test for 24 hours, validate all metrics
2. **Phase 2**: Add second exchange (Bitget/OKX demo) for real cross-exchange testing
3. **Phase 3**: Test with live exchanges at $10 position sizes
4. **Phase 4**: Gradually increase position sizes as confidence builds
5. **Phase 5**: Full production deployment

## Appendix: Dashboard Logic Reference

The synthetic test mode replicates these key functions from `src/bin/dashboard.rs`:

- `recalculate_opportunities()` (lines 200-400): Opportunity qualification
- `calculate_confidence_score_with_gravity()` (lines 650-700): Confidence scoring
- `get_exchange_taker_fee()` (lines 1200-1215): Fee calculation
- Hard constraints (lines 250-290): Funding delta, depth, confidence, profitability

This ensures synthetic opportunities are production-equivalent and test results are representative of live performance.
