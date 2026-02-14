# Streaming Opportunity Detection - Design

## Overview

This design implements a complete streaming architecture for opportunity detection that eliminates all Redis polling and provides low-latency trading execution. Both the strategy runner and dashboard will consume from a shared opportunity stream.

## Architecture Components

### 1. Symbol Mapping Service

**Purpose**: Convert (exchange, symbol) strings to u32 symbol_id for performance

**Location**: `src/strategy/symbol_map.rs`

```rust
pub struct SymbolMap {
    // Bidirectional mapping
    to_id: DashMap<(String, String), u32>,  // (exchange, symbol) -> id
    from_id: Vec<(String, String)>,          // id -> (exchange, symbol)
    next_id: AtomicU32,
}

impl SymbolMap {
    pub fn new() -> Self;
    
    /// Get or create symbol_id for (exchange, symbol)
    pub fn get_or_insert(&self, exchange: &str, symbol: &str) -> u32;
    
    /// Get (exchange, symbol) from symbol_id
    pub fn get(&self, symbol_id: u32) -> Option<(String, String)>;
}
```

**Performance**:
- DashMap for lock-free concurrent access
- O(1) lookups
- Atomic counter for ID generation
- Pre-allocate common symbols on startup

### 2. Enhanced Redis Bridge

**Purpose**: Convert WebSocket JSON to MarketUpdate and push to pipeline

**Location**: Modify `src/bin/bybit-synthetic-test.rs` and `src/main.rs`

```rust
async fn redis_bridge(
    mut rx: mpsc::Receiver<(String, String)>,
    redis_queue: Arc<ArrayQueue<(String, String)>>,
    pipeline: Arc<MarketPipeline>,
    symbol_map: Arc<SymbolMap>,
) {
    while let Some((key, value)) = rx.recv().await {
        // Push to Redis queue (cold path - persistence)
        if let Err(rejected) = redis_queue.push((key.clone(), value.clone())) {
            redis_queue.pop();
            let _ = redis_queue.push(rejected);
        }
        
        // Parse and push to pipeline (hot path)
        if let Some(update) = parse_to_market_update(&key, &value, &symbol_map) {
            let producer = pipeline.producer();
            producer.push(update);  // Lock-free push
        }
    }
}

fn parse_to_market_update(
    key: &str,
    value: &str,
    symbol_map: &SymbolMap,
) -> Option<MarketUpdate> {
    // Parse key: "exchange:type:symbol"
    let parts: Vec<&str> = key.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    
    let exchange = parts[0];
    let symbol = parts[parts.len() - 1];
    
    // Parse JSON value
    let json: serde_json::Value = serde_json::from_str(value).ok()?;
    let parser = get_parser(exchange);
    
    let bid = parser.parse_bid(&json)?.parse::<f64>().ok()?;
    let ask = parser.parse_ask(&json)?.parse::<f64>().ok()?;
    
    // Get or create symbol_id
    let symbol_id = symbol_map.get_or_insert(exchange, symbol);
    
    Some(MarketUpdate {
        bid,
        ask,
        timestamp_us: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64,
        symbol_id,
        _padding: [0; 36],
    })
}
```

### 3. OpportunityDetector Service

**Purpose**: Detect arbitrage opportunities from market updates

**Location**: `src/strategy/opportunity_detector.rs`

```rust
use crate::strategy::pipeline::MarketConsumer;
use crate::strategy::market_data::MarketDataStore;
use crate::strategy::types::ArbitrageOpportunity;
use crate::strategy::symbol_map::SymbolMap;
use std::sync::Arc;

pub struct OpportunityDetector {
    market_consumer: MarketConsumer,
    market_data_store: MarketDataStore,
    symbol_map: Arc<SymbolMap>,
    opportunity_producer: OpportunityProducer,
    
    // Configuration
    min_spread_bps: f64,
    min_funding_delta: f64,
    min_confidence: u8,
}

impl OpportunityDetector {
    pub fn new(
        market_consumer: MarketConsumer,
        symbol_map: Arc<SymbolMap>,
        opportunity_producer: OpportunityProducer,
    ) -> Self {
        Self {
            market_consumer,
            market_data_store: MarketDataStore::new(),
            symbol_map,
            opportunity_producer,
            min_spread_bps: 10.0,
            min_funding_delta: 0.0001,
            min_confidence: 70,
        }
    }
    
    /// Main detection loop - runs continuously
    pub async fn run(&mut self) {
        loop {
            // Pop market update (non-blocking)
            if let Some(update) = self.market_consumer.pop() {
                // Update market data store
                self.market_data_store.update_from_market_update(&update);
                
                // Detect opportunities for this symbol
                if let Some((exchange, symbol)) = self.symbol_map.get(update.symbol_id) {
                    self.detect_opportunities_for_symbol(&symbol, &exchange);
                }
            }
            
            // Small sleep to avoid busy-waiting
            tokio::time::sleep(Duration::from_micros(10)).await;
        }
    }
    
    fn detect_opportunities_for_symbol(&mut self, symbol: &str, updated_exchange: &str) {
        // Get all exchanges that have this symbol
        let exchanges = self.get_exchanges_for_symbol(symbol);
        
        if exchanges.len() < 2 {
            return;
        }
        
        // Check all exchange pairs
        for i in 0..exchanges.len() {
            for j in (i + 1)..exchanges.len() {
                let ex1 = &exchanges[i];
                let ex2 = &exchanges[j];
                
                // Get prices from market data store
                let (bid1, ask1) = self.get_prices(ex1, symbol)?;
                let (bid2, ask2) = self.get_prices(ex2, symbol)?;
                
                // Check both directions
                self.check_opportunity(symbol, ex1, ex2, ask1, bid2);
                self.check_opportunity(symbol, ex2, ex1, ask2, bid1);
            }
        }
    }
    
    fn check_opportunity(
        &mut self,
        symbol: &str,
        long_exchange: &str,
        short_exchange: &str,
        long_ask: f64,
        short_bid: f64,
    ) {
        // Calculate spread
        let spread_bps = ((short_bid - long_ask) / long_ask) * 10000.0;
        
        if spread_bps <= self.min_spread_bps {
            return;
        }
        
        // Get funding rates
        let funding_delta = self.get_funding_delta(symbol, long_exchange, short_exchange);
        
        if funding_delta.abs() < self.min_funding_delta {
            return;
        }
        
        // Calculate confidence score
        let confidence = self.calculate_confidence(spread_bps, funding_delta);
        
        if confidence < self.min_confidence {
            return;
        }
        
        // Get order book depths
        let (depth_long, depth_short) = self.get_depths(symbol, long_exchange, short_exchange);
        
        // Calculate fees and profit
        let long_fee = self.get_taker_fee(long_exchange);
        let short_fee = self.get_taker_fee(short_exchange);
        let total_fees = long_fee + short_fee;
        
        let slippage = 3.0; // Estimate
        let funding_cost = 10.0; // Estimate
        
        let projected_profit_bps = spread_bps - total_fees - slippage - funding_cost;
        
        if projected_profit_bps <= 0.0 {
            return;
        }
        
        // Create opportunity
        let opportunity = ArbitrageOpportunity {
            symbol: symbol.to_string(),
            long_exchange: long_exchange.to_string(),
            short_exchange: short_exchange.to_string(),
            long_price: long_ask,
            short_price: short_bid,
            spread_bps,
            funding_delta_8h: funding_delta,
            confidence_score: confidence,
            projected_profit_usd: (projected_profit_bps / 10000.0) * 1000.0,
            projected_profit_after_slippage: projected_profit_bps,
            metrics: self.build_metrics(spread_bps, funding_delta, depth_long, depth_short),
            order_book_depth_long: depth_long,
            order_book_depth_short: depth_short,
            timestamp: Some(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()),
        };
        
        // Push to opportunity queue
        self.opportunity_producer.push(opportunity);
    }
    
    fn get_exchanges_for_symbol(&self, symbol: &str) -> Vec<String> {
        // Query symbol_map for all exchanges that have this symbol
        // This is a simplified version - actual implementation would be more efficient
        vec!["bybit".to_string(), "okx".to_string(), "kucoin".to_string()]
    }
    
    fn get_prices(&self, exchange: &str, symbol: &str) -> Option<(f64, f64)> {
        let symbol_id = self.symbol_map.get_or_insert(exchange, symbol);
        self.market_data_store.get_bid_ask(symbol_id)
    }
    
    fn calculate_confidence(&self, spread_bps: f64, funding_delta: f64) -> u8 {
        let mut score = 0.0;
        
        // Spread component (50%)
        score += (spread_bps / 50.0).min(1.0) * 50.0;
        
        // Funding component (30%)
        score += (funding_delta.abs() / 0.01).min(1.0) * 30.0;
        
        // Base score (20%)
        score += 20.0;
        
        score.min(100.0) as u8
    }
}
```

### 4. OpportunityQueue (Lock-free MPSC)

**Purpose**: Store detected opportunities for multiple consumers

**Location**: `src/strategy/opportunity_queue.rs`

```rust
use crate::strategy::types::ArbitrageOpportunity;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct OpportunityQueue {
    queue: Arc<ArrayQueue<ArbitrageOpportunity>>,
    push_count: AtomicU64,
    pop_count: AtomicU64,
}

impl OpportunityQueue {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }
    
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(capacity)),
            push_count: AtomicU64::new(0),
            pop_count: AtomicU64::new(0),
        }
    }
    
    pub fn producer(&self) -> OpportunityProducer {
        OpportunityProducer {
            queue: self.queue.clone(),
            push_count: &self.push_count,
        }
    }
    
    pub fn consumer(&self) -> OpportunityConsumer {
        OpportunityConsumer {
            queue: self.queue.clone(),
            pop_count: &self.pop_count,
        }
    }
}

pub struct OpportunityProducer {
    queue: Arc<ArrayQueue<ArbitrageOpportunity>>,
    push_count: *const AtomicU64,
}

unsafe impl Send for OpportunityProducer {}
unsafe impl Sync for OpportunityProducer {}

impl OpportunityProducer {
    /// Push opportunity with backpressure (drops oldest if full)
    pub fn push(&self, opportunity: ArbitrageOpportunity) {
        unsafe {
            (*self.push_count).fetch_add(1, Ordering::Relaxed);
        }
        
        if let Err(rejected) = self.queue.push(opportunity) {
            // Queue full - drop oldest and retry
            self.queue.pop();
            let _ = self.queue.push(rejected);
        }
    }
}

pub struct OpportunityConsumer {
    queue: Arc<ArrayQueue<ArbitrageOpportunity>>,
    pop_count: *const AtomicU64,
}

unsafe impl Send for OpportunityConsumer {}
unsafe impl Sync for OpportunityConsumer {}

impl OpportunityConsumer {
    /// Pop opportunity (non-blocking)
    pub fn pop(&self) -> Option<ArbitrageOpportunity> {
        let opp = self.queue.pop();
        if opp.is_some() {
            unsafe {
                (*self.pop_count).fetch_add(1, Ordering::Relaxed);
            }
        }
        opp
    }
    
    /// Pop batch of opportunities
    pub fn pop_batch(&self, max_batch: usize) -> Vec<ArbitrageOpportunity> {
        let mut batch = Vec::with_capacity(max_batch);
        for _ in 0..max_batch {
            if let Some(opp) = self.pop() {
                batch.push(opp);
            } else {
                break;
            }
        }
        batch
    }
}
```

### 5. Strategy Runner Integration

**Purpose**: Consume opportunities and execute trades

**Location**: Modify `src/strategy/runner.rs`

```rust
pub struct StrategyRunner {
    // ... existing fields ...
    opportunity_consumer: Option<OpportunityConsumer>,  // NEW
}

impl StrategyRunner {
    pub fn set_opportunity_consumer(&mut self, consumer: OpportunityConsumer) {
        self.opportunity_consumer = Some(consumer);
    }
    
    pub async fn run_scanning_loop(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Pin thread
        if let Err(e) = crate::strategy::thread_pinning::pin_strategy_thread() {
            eprintln!("[THREAD-PIN] Warning: {}", e);
        }
        
        let consumer = self.opportunity_consumer.as_ref()
            .expect("OpportunityConsumer not set - call set_opportunity_consumer()");
        
        eprintln!("[STRATEGY] Starting STREAMING mode (consuming opportunities)");
        eprintln!("[STRATEGY] Starting capital: ${:.2}", 
            self.portfolio_manager.read().await.get_available_capital().await);
        
        loop {
            // Pop opportunity (non-blocking)
            if let Some(opportunity) = consumer.pop() {
                // Execute trade immediately
                self.execute_opportunity(opportunity).await;
            }
            
            // Small sleep to avoid busy-waiting
            tokio::time::sleep(Duration::from_micros(100)).await;
            
            // Run monitoring tasks
            let (monitor_result, exit_result) = tokio::join!(
                self.monitor_active_positions(),
                self.check_exits()
            );
            
            if let Err(e) = monitor_result {
                eprintln!("Error monitoring positions: {}", e);
            }
            if let Err(e) = exit_result {
                eprintln!("Error checking exits: {}", e);
            }
        }
    }
    
    async fn execute_opportunity(&self, opportunity: ArbitrageOpportunity) {
        // Existing validation and execution logic
        // (from current scan_opportunities method)
        // ...
    }
}
```

### 6. Dashboard Integration

**Purpose**: Display opportunities in real-time

**Location**: Modify `src/bin/dashboard.rs`

```rust
struct AppState {
    // Remove: ticker_data, funding_rates, opportunity calculation logic
    opportunities: BTreeMap<String, ArbitrageOpportunity>,  // From queue
    opportunity_consumer: OpportunityConsumer,  // NEW
    removed_opportunities: VecDeque<RemovedOpportunity>,
    should_quit: bool,
    scroll_offset: usize,
}

impl AppState {
    fn new(opportunity_consumer: OpportunityConsumer) -> Self {
        Self {
            opportunities: BTreeMap::new(),
            opportunity_consumer,
            removed_opportunities: VecDeque::new(),
            should_quit: false,
            scroll_offset: 0,
        }
    }
    
    fn update_from_queue(&mut self) {
        // Pop batch of opportunities
        let batch = self.opportunity_consumer.pop_batch(100);
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Track removals
        for (symbol, old_opp) in &self.opportunities {
            if !batch.iter().any(|o| o.symbol == *symbol) {
                // Opportunity removed
                if self.removed_opportunities.len() >= 10 {
                    self.removed_opportunities.pop_front();
                }
                self.removed_opportunities.push_back(RemovedOpportunity {
                    ticker: symbol.clone(),
                    confidence_score: old_opp.confidence_score,
                    reason: "No longer detected".to_string(),
                });
            }
        }
        
        // Update opportunities
        self.opportunities.clear();
        for opp in batch {
            // Filter stale (older than 5 seconds)
            if let Some(ts) = opp.timestamp {
                if now - ts > 5 {
                    continue;
                }
            }
            self.opportunities.insert(opp.symbol.clone(), opp);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    // ... terminal setup ...
    
    // Get opportunity consumer from shared queue
    let opportunity_consumer = get_opportunity_consumer();  // From main setup
    
    let mut app_state = AppState::new(opportunity_consumer);
    let mut last_update = std::time::Instant::now();
    let update_interval = Duration::from_millis(100);  // 100ms updates
    
    loop {
        // Handle keyboard events
        // ...
        
        // Update from queue periodically
        if last_update.elapsed() >= update_interval {
            app_state.update_from_queue();
            last_update = std::time::Instant::now();
        }
        
        // Draw UI
        terminal.draw(|f| ui(f, &app_state))?;
        
        if app_state.should_quit {
            break;
        }
    }
    
    // ... cleanup ...
    Ok(())
}
```

## Integration Flow

### bybit-synthetic-test.rs

```rust
#[tokio::main]
async fn main() -> Result<(), DynError> {
    // ... existing setup ...
    
    // Create symbol map
    let symbol_map = Arc::new(SymbolMap::new());
    
    // Create market pipeline
    let market_pipeline = Arc::new(MarketPipeline::new());
    let market_consumer = market_pipeline.consumer();
    
    // Create opportunity queue
    let opportunity_queue = Arc::new(OpportunityQueue::new());
    let opportunity_producer = opportunity_queue.producer();
    let opportunity_consumer_strategy = opportunity_queue.consumer();
    let opportunity_consumer_dashboard = opportunity_queue.consumer();
    
    // Modify redis_bridge to push to pipeline
    let symbol_map_bridge = symbol_map.clone();
    let market_pipeline_bridge = market_pipeline.clone();
    let bridge_handle = tokio::spawn(async move {
        redis_bridge(rx, redis_queue_bridge, market_pipeline_bridge, symbol_map_bridge).await;
    });
    
    // Start opportunity detector service
    let mut detector = OpportunityDetector::new(
        market_consumer,
        symbol_map.clone(),
        opportunity_producer,
    );
    let detector_handle = tokio::spawn(async move {
        detector.run().await;
    });
    
    // Start strategy runner with opportunity consumer
    let mut strategy_runner = StrategyRunner::new(/* ... */).await?;
    strategy_runner.set_opportunity_consumer(opportunity_consumer_strategy);
    
    let strategy_handle = tokio::spawn(async move {
        strategy_runner.run_scanning_loop().await
    });
    
    // Start dashboard (separate binary, gets consumer via shared memory or channel)
    // ...
    
    Ok(())
}
```

## Performance Characteristics

### Latency Budget

| Component | Target Latency | Notes |
|-----------|---------------|-------|
| WebSocket → Redis Bridge | < 50μs | JSON parsing + queue push |
| Redis Bridge → Pipeline | < 20μs | Struct conversion + lock-free push |
| Pipeline → Detector | < 10μs | Lock-free pop |
| Detector → Opportunity | < 500μs | Spread calc + validation |
| Opportunity → Strategy | < 10μs | Lock-free pop |
| Strategy → Execution | < 2ms | Order placement |
| **Total End-to-End** | **< 3ms** | WebSocket to order |

### Memory Usage

- **SymbolMap**: ~10KB (100 symbols × 100 bytes)
- **MarketPipeline**: 2MB (32K × 64 bytes)
- **OpportunityQueue**: 512KB (1K × 512 bytes)
- **MarketDataStore**: 1MB (existing)
- **Total Additional**: ~3.5MB

### CPU Usage

- **OpportunityDetector**: 1 dedicated thread, ~5-10% CPU
- **Strategy Runner**: 1 thread (existing), pinned to core 1
- **Dashboard**: 1 thread, ~2-5% CPU

## Testing Strategy

### Unit Tests

1. **SymbolMap**: Test concurrent access, ID generation
2. **OpportunityQueue**: Test push/pop, backpressure
3. **OpportunityDetector**: Test opportunity detection logic
4. **Redis Bridge**: Test MarketUpdate conversion

### Integration Tests

1. **End-to-end latency**: WebSocket → Trade execution
2. **Opportunity consistency**: Dashboard shows same as strategy
3. **Backpressure handling**: Queue full scenarios
4. **Symbol mapping**: Correct ID assignment

### Performance Tests

1. **Throughput**: 10K updates/sec sustained
2. **Latency**: p50 < 1ms, p99 < 5ms
3. **Memory**: No leaks over 24h run
4. **CPU**: < 15% total usage

## Migration Path

### Phase 1: Infrastructure (Week 1)
- Implement SymbolMap
- Implement OpportunityQueue
- Modify redis_bridge to push to pipeline
- Unit tests

### Phase 2: Detector Service (Week 1-2)
- Implement OpportunityDetector
- Integration with MarketPipeline
- Opportunity detection logic
- Integration tests

### Phase 3: Strategy Integration (Week 2)
- Modify StrategyRunner to consume from queue
- Remove scan_opportunities() method
- Remove legacy mode
- End-to-end tests

### Phase 4: Dashboard Integration (Week 2-3)
- Modify dashboard to consume from queue
- Remove Redis polling
- Remove opportunity calculation
- UI testing

### Phase 5: Cleanup & Optimization (Week 3)
- Remove all legacy code
- Performance optimization
- Documentation
- Production deployment

## Rollback Plan

If issues arise, we can:
1. Keep legacy code temporarily with feature flag
2. Run both systems in parallel for validation
3. Gradual rollout: strategy first, then dashboard
4. Monitoring and alerting for latency regressions

## Success Metrics

- ✅ End-to-end latency < 5ms (p99)
- ✅ No Redis polling in any component
- ✅ Dashboard shows real-time opportunities
- ✅ Zero legacy code remaining
- ✅ Memory usage < 5MB additional
- ✅ CPU usage < 15% total
- ✅ 24h stability test passes
