# Profiling Guide

This guide explains how to use the profiling infrastructure to measure and optimize performance.

## Quick Start

### 1. Build with Optimizations

```bash
# Make the build script executable
chmod +x build-release.sh

# Build with maximum optimizations
./build-release.sh
```

This sets `RUSTFLAGS="-C target-cpu=native -C opt-level=3"` for native CPU optimizations.

### 2. Run Benchmarks

```bash
# Run all benchmarks
cargo test --release -- --ignored --nocapture

# Run specific benchmark
cargo test --release bench_spread_calculation -- --ignored --nocapture
```

### 3. Profile with Flamegraph

```bash
# Install cargo-flamegraph (first time only)
cargo install flamegraph

# Profile the main binary
cargo flamegraph --release --bin arbitrage2

# Profile with specific duration
cargo flamegraph --release --bin arbitrage2 -- --duration 60
```

This generates `flamegraph.svg` showing CPU time distribution.

## Profiling Infrastructure

### Latency Tracking

The `latency_tracker` module provides utilities for measuring hot path latency:

```rust
use arbitrage2::strategy::latency_tracker::{LatencyStats, measure_latency, measure_and_record};

// Create statistics tracker
let stats = LatencyStats::new();

// Measure a single operation
let (result, latency_ns) = measure_latency(|| {
    // Your hot path code here
    calculate_spread(bid, ask)
});
println!("Operation took {} ns", latency_ns);

// Measure and automatically record
let result = measure_and_record(&stats, || {
    // Your hot path code here
    calculate_spread(bid, ask)
});

// Get statistics snapshot
let snapshot = stats.snapshot();
println!("P50: {} µs", snapshot.p50_us());
println!("P95: {} µs", snapshot.p95_us());
println!("P99: {} µs", snapshot.p99_us());
println!("Max: {} µs", snapshot.max_us());
println!("Count: {}", snapshot.count);
```

### Benchmark Framework

The `benches/latency_benchmarks.rs` provides a simple benchmark framework:

```rust
use latency_benchmarks::benchmark;

#[test]
#[ignore]
fn bench_my_function() {
    let result = benchmark("my_function", 100_000, || {
        // Your code to benchmark
        my_hot_path_function();
    });
    
    result.print();
    
    // Assert performance target
    assert!(result.p99_ns < 1000, "Too slow: {} ns", result.p99_ns);
}
```

## Performance Targets

Based on requirements 15.1-15.4, we aim for:

| Metric | Target | Current | Status |
|--------|--------|---------|--------|
| Market data parsing | <100 ns | TBD | 🔄 |
| Spread calculation | <50 ns | TBD | 🔄 |
| Opportunity detection | <1 µs | TBD | 🔄 |
| End-to-end latency (P99) | <10 ms | ~1150 ms | 🔴 |
| Hot path allocations | 0/sec | ~1000/sec | 🔴 |
| Lock contention | 0 futex | ~100/sec | 🔴 |
| L1 cache miss rate | <5% | ~30% | 🔴 |

## Flamegraph Analysis

### What to Look For

1. **Hot Path Allocations** (BAD)
   - Look for `malloc`, `free`, `realloc` in hot paths
   - Look for `Vec::push`, `String::push_str` without pre-allocation
   - Target: ZERO allocations in hot path

2. **Lock Contention** (BAD)
   - Look for `futex`, `pthread_mutex_lock`
   - Look for `Mutex::lock`, `RwLock::read`
   - Target: ZERO lock operations in hot path

3. **Function Call Overhead** (BAD)
   - Deep call stacks in hot path
   - Functions that should be inlined
   - Target: Flat call graph in hot path

4. **Good Patterns** (GOOD)
   - Flat, wide flame graph (parallel work)
   - Minimal system calls
   - Inlined functions (no call overhead)

### Example Analysis

```bash
# Generate flamegraph
cargo flamegraph --release --bin arbitrage2

# Open flamegraph.svg in browser
# Search for "malloc" - should be ZERO in hot path
# Search for "futex" - should be ZERO in hot path
# Search for your hot path functions - should be inlined
```

## Continuous Profiling

### Before Each Commit

```bash
# 1. Run benchmarks
cargo test --release -- --ignored --nocapture

# 2. Check for regressions
# Compare results to baseline in PROFILING.md

# 3. Profile with flamegraph
cargo flamegraph --release --bin arbitrage2

# 4. Verify zero allocations in hot path
# Search flamegraph for "malloc"
```

### Baseline Metrics

Record baseline metrics here after establishing initial measurements:

```
# Baseline (Date: TBD)
- P99 latency: TBD ms
- Hot path allocations: TBD/sec
- Lock contention: TBD futex/sec
- Cache miss rate: TBD%
```

## Advanced Profiling

### CPU Cache Analysis

```bash
# Install perf (Linux only)
sudo apt-get install linux-tools-common linux-tools-generic

# Profile cache misses
perf stat -e cache-references,cache-misses,L1-dcache-loads,L1-dcache-load-misses \
    ./target/release/arbitrage2

# Target: <5% L1 cache miss rate
```

### Memory Profiling

```bash
# Install valgrind
sudo apt-get install valgrind

# Profile memory allocations
valgrind --tool=massif ./target/release/arbitrage2

# Analyze results
ms_print massif.out.*
```

### Thread Profiling

```bash
# Profile thread activity
perf record -e sched:sched_switch ./target/release/arbitrage2
perf script | less

# Look for excessive context switches
```

## Troubleshooting

### Flamegraph Not Generated

```bash
# Ensure perf is available (Linux)
sudo apt-get install linux-tools-common linux-tools-generic

# Or use dtrace (macOS)
# cargo-flamegraph will auto-detect
```

### Benchmarks Too Fast

If benchmarks complete too quickly (<1µs), increase iterations:

```rust
let result = benchmark("my_function", 1_000_000, || {
    // Your code
});
```

### Inconsistent Results

- Run benchmarks multiple times
- Close other applications
- Disable CPU frequency scaling:
  ```bash
  sudo cpupower frequency-set --governor performance
  ```

## References

- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [cargo-flamegraph](https://github.com/flamegraph-rs/flamegraph)
- [Linux perf](https://perf.wiki.kernel.org/)
- Requirements: 15.1, 15.2, 15.3, 15.4
