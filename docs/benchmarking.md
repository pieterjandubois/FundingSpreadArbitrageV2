# Performance Benchmarking and Regression Testing

This document describes the performance benchmarking system and regression testing setup for the low-latency arbitrage system.

## Overview

The benchmark suite measures critical hot path performance metrics and automatically detects regressions in CI. Any change that causes >10% latency increase or introduces allocations in the hot path will fail CI.

## Benchmark Categories

### 1. Market Data Benchmarks
- **market_data_update**: Time to update a single market data entry (target: <200ns P99)
- **market_data_spread_calculation**: Time to calculate spread from bid/ask (target: <200ns P99)
- **market_data_sequential_iteration**: Time to iterate through all symbols (target: <10µs P99)
- **market_data_random_access**: Time for random access pattern (cache efficiency test)

### 2. Branchless Validation Benchmarks
- **branchless_opportunity_validation**: Time to validate opportunity without branches (target: <50ns P99)
- **branchless_exit_validation**: Time to check exit conditions (target: <100ns P99)
- **branchless_min_max**: Time for branchless min/max operations (target: <30ns P99)

### 3. JSON Parsing Benchmarks
- **simd_json_parse_binance**: SIMD-accelerated JSON parsing for Binance messages
- **simd_json_parse_bybit**: SIMD-accelerated JSON parsing for Bybit messages
- **simd_json_parse_okx**: SIMD-accelerated JSON parsing for OKX messages
- Comparison benchmarks vs standard serde_json

### 4. SIMD Price Parsing Benchmarks
- **simd_price_parse**: SIMD-accelerated f64 parsing from strings (target: <50ns per price)
- **standard_price_parse**: Standard library parsing for comparison
- Edge case and throughput tests

### 5. Cache Prefetching Benchmarks
- **sequential_with_prefetch**: Sequential access with manual prefetching
- **random_access_with_prefetch**: Random access with prefetching
- Cache miss rate estimation

## Running Benchmarks Locally

### Quick Run (All Benchmarks)
```bash
# Run all benchmarks and compare against baseline
./scripts/run_benchmarks.sh
```

### Run Specific Benchmark Category
```bash
# Market data benchmarks only
cargo test --release -- --ignored --nocapture market_data

# Branchless validation benchmarks
cargo test --release -- --ignored --nocapture branchless

# JSON parsing benchmarks
cargo test --release -- --ignored --nocapture json_parsing

# SIMD price parsing benchmarks
cargo test --release -- --ignored --nocapture simd_price

# Cache prefetching benchmarks
cargo test --release -- --ignored --nocapture cache_prefetching
```

### Run Single Benchmark
```bash
# Run a specific benchmark
cargo test --release -- --ignored --nocapture bench_market_data_update
```

## Establishing a Baseline

When you first run benchmarks or after making intentional performance improvements:

```bash
# Run benchmarks
./scripts/run_benchmarks.sh

# If satisfied with results, update baseline
cp current_metrics.json baseline_metrics.json
```

The baseline is stored in `baseline_metrics.json` at the project root.

## CI Integration

### Automatic Regression Detection

The GitHub Actions workflow (`.github/workflows/benchmark.yml`) automatically:

1. **Runs on every push and PR** to main/master branches
2. **Compares against baseline** stored in the repository
3. **Fails CI if**:
   - Any benchmark shows >10% latency regression (P99)
   - New allocations detected in hot path
4. **Comments on PR** with regression details if detected

### Workflow Jobs

#### 1. Benchmark Job
- Builds release binary with optimizations
- Runs full benchmark suite
- Parses results and extracts key metrics
- Compares against baseline
- Uploads results as artifacts
- Comments on PR if regression detected

#### 2. Allocation Check Job
- Verifies no new allocations in hot path
- Uses cargo-flamegraph for analysis
- Fails if malloc/free detected in hot path

#### 3. Summary Job
- Provides overall test summary
- Lists all tracked metrics and thresholds

## Regression Thresholds

### Latency Regression
- **Threshold**: >10% increase in P99 latency
- **Action**: CI fails, PR comment added
- **Resolution**: Optimize code or revert changes

### Allocation Increase
- **Threshold**: Any new heap allocations in hot path
- **Action**: CI fails
- **Resolution**: Use pre-allocated buffers or stack allocation

## Key Metrics and Targets

| Metric | Target (P99) | Requirement |
|--------|--------------|-------------|
| Market data update | <200ns | 2.1, 12.1 |
| Spread calculation | <200ns | 6.2 |
| Branchless validation | <50ns | 7.1, 7.4 |
| Sequential iteration (256 symbols) | <10µs | 5.1 |
| Random access (10 symbols) | <1µs | 5.2 |
| SIMD JSON parsing | <100ns | 8.2 |
| SIMD price parsing | <50ns | 7.2 |

## Interpreting Results

### Good Performance
```
Benchmark: market_data_update
Iterations: 100000
Average:    45 ns
P99:        120 ns
✓ PASSED: Within target
```

### Regression Detected
```
❌ REGRESSION DETECTED:
  market_data_update_p99_ns: +15.5% (120 → 139 ns)
  
Action Required:
  - Profile with cargo flamegraph
  - Identify bottleneck
  - Optimize or revert
```

### Improvement
```
✅ IMPROVEMENT:
  branchless_validation_p99_ns: -22.3% (50 → 39 ns)
  
Consider updating baseline if this is expected.
```

## Profiling for Regressions

If a regression is detected:

### 1. Run Flamegraph
```bash
cargo flamegraph --bin arbitrage2 -- --profile
```

Look for:
- New malloc/free calls in hot path
- Increased function call overhead
- Cache misses (memory access patterns)

### 2. Run Specific Benchmark with Details
```bash
cargo test --release -- --ignored --nocapture bench_market_data_update
```

### 3. Compare Assembly
```bash
# Before changes
cargo asm --release arbitrage2::strategy::market_data::MarketDataStore::update > before.asm

# After changes
cargo asm --release arbitrage2::strategy::market_data::MarketDataStore::update > after.asm

# Compare
diff before.asm after.asm
```

### 4. Use perf for Cache Analysis
```bash
perf stat -e cache-misses,cache-references,L1-dcache-load-misses \
  cargo test --release -- --ignored bench_market_data_update
```

## Best Practices

### When Making Changes

1. **Run benchmarks before changes**
   ```bash
   ./scripts/run_benchmarks.sh
   ```

2. **Make your changes**

3. **Run benchmarks after changes**
   ```bash
   ./scripts/run_benchmarks.sh
   ```

4. **If regression detected**:
   - Profile with flamegraph
   - Identify root cause
   - Optimize or revert
   - Re-run benchmarks

5. **If improvement detected**:
   - Verify it's real (not noise)
   - Update baseline if significant
   - Document the optimization

### Avoiding False Positives

Benchmark results can vary due to:
- CPU frequency scaling
- Background processes
- Thermal throttling
- Cache state

To minimize noise:
```bash
# Disable CPU frequency scaling (Linux)
sudo cpupower frequency-set --governor performance

# Pin to specific cores
taskset -c 0 ./scripts/run_benchmarks.sh

# Run multiple times and average
for i in {1..5}; do ./scripts/run_benchmarks.sh; done
```

### Updating Baselines

Update the baseline when:
- Initial setup (first run)
- After verified performance improvements
- After intentional architectural changes
- When targets are adjusted

```bash
# Update baseline
cp current_metrics.json baseline_metrics.json

# Commit to repository
git add baseline_metrics.json
git commit -m "Update performance baseline after optimization"
```

## Troubleshooting

### Benchmarks Fail to Run
```bash
# Ensure release build works
cargo build --release

# Check for compilation errors
cargo clippy --release

# Run tests without benchmarks first
cargo test --release
```

### Inconsistent Results
```bash
# Use consistent CPU governor
sudo cpupower frequency-set --governor performance

# Increase iterations for more stable results
# Edit benches/latency_benchmarks.rs and increase iteration counts
```

### CI Fails but Local Passes
- CI runs on different hardware (may be slower)
- Adjust thresholds if needed
- Consider using relative comparisons instead of absolute targets

## Related Documentation

- [Requirements](../.kiro/specs/low-latency-optimization/requirements.md) - Performance requirements
- [Design](../.kiro/specs/low-latency-optimization/design.md) - Architecture and optimizations
- [Profiling Guide](../PROFILING.md) - Detailed profiling instructions

## References

- **Requirement 15.1**: Cargo clippy with zero warnings
- **Requirement 15.2**: Profile with cargo flamegraph
- **Requirement 15.3**: Expose latency percentiles (p50, p95, p99)
- **Requirement 15.4**: Track allocations per second in hot paths
- **Task 34**: Create performance regression tests
