# Performance Benchmark Quick Reference

## Quick Start

### Run All Benchmarks
```bash
# Linux/Mac
./scripts/run_benchmarks.sh

# Windows
.\scripts\run_benchmarks.ps1
```

### Run Specific Category
```bash
cargo test --release -- --ignored --nocapture market_data
cargo test --release -- --ignored --nocapture branchless
cargo test --release -- --ignored --nocapture json_parsing
cargo test --release -- --ignored --nocapture simd_price
cargo test --release -- --ignored --nocapture cache_prefetching
```

## CI Behavior

### When CI Runs
- Every push to main/master
- Every pull request
- Manual workflow dispatch

### What CI Checks
1. **Latency Regression**: >10% increase in P99 latency
2. **Allocation Increase**: New heap allocations in hot path

### If CI Fails
1. Download `benchmark-results` artifact
2. Review regression details
3. Profile with `cargo flamegraph`
4. Optimize or revert changes

## Key Metrics

| Metric | Target (P99) | Threshold |
|--------|--------------|-----------|
| market_data_update | <200ns | 10% |
| spread_calculation | <200ns | 10% |
| branchless_validation | <50ns | 10% |
| sequential_access | <10µs | 10% |
| random_access | <1µs | 10% |

## Updating Baseline

After verified improvements:
```bash
# Linux/Mac
cp current_metrics.json baseline_metrics.json

# Windows
Copy-Item current_metrics.json baseline_metrics.json

# Commit
git add baseline_metrics.json
git commit -m "Update performance baseline"
```

## Profiling Regressions

```bash
# Flamegraph
cargo flamegraph --bin arbitrage2

# Specific benchmark
cargo test --release -- --ignored --nocapture bench_market_data_update

# Cache analysis (Linux)
perf stat -e cache-misses,cache-references cargo test --release -- --ignored bench_market_data_update
```

## Common Issues

### Inconsistent Results
- Disable CPU frequency scaling
- Close background applications
- Run multiple times and average

### CI Fails but Local Passes
- CI hardware may be slower
- Check for thermal throttling
- Consider adjusting thresholds

## Documentation

Full documentation: [docs/benchmarking.md](../docs/benchmarking.md)
