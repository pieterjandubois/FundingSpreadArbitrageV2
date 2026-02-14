# Latency Validation Procedure

## Overview

This document describes the procedure for validating that the Bybit Synthetic Test Mode system meets all latency requirements specified in the design.

## Latency Targets

From Requirements 9.1-9.5:

| Stage | Target | Requirement |
|-------|--------|-------------|
| WebSocket → Queue | < 0.5ms | P99 |
| Queue → Strategy | < 0.1ms | P99 |
| Opportunity Detection | < 2ms | P99 |
| Order Placement | < 5ms | P99 |
| End-to-End | < 10ms | P99 |

## Prerequisites

- System running in release mode (`cargo build --release`)
- Thread pinning enabled (cores 1 and 2)
- Minimal system load (close unnecessary applications)
- Stable network connection to Bybit demo

## Test Procedure

### 1. Preparation

```bash
# Build in release mode with optimizations
cargo build --release --bin bybit-synthetic-test

# Verify CPU governor is set to performance
cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# If not, set to performance mode
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Disable CPU frequency scaling (optional, for most accurate results)
sudo cpupower frequency-set --governor performance

# Check system load
uptime
```

### 2. Run Latency Collection Test

```bash
# Start the system with detailed logging
RUST_LOG=debug cargo run --release --bin bybit-synthetic-test \
  > logs/latency-test-$(date +%Y%m%d-%H%M%S).log 2>&1 &

# Save PID
echo $! > logs/latency-test.pid

# Let it run for 1 hour to collect sufficient samples
sleep 3600

# Stop gracefully
kill -INT $(cat logs/latency-test.pid)
```

### 3. Extract Latency Metrics

The system reports latency percentiles every 60 seconds. Extract them:

```bash
# Extract all latency reports
grep "Latency Percentiles" logs/latency-test-*.log -A 10 > latency-results.txt

# Or use a more detailed extraction script
cat > extract_latencies.sh << 'EOF'
#!/bin/bash

LOG_FILE=$1

echo "=== WebSocket → Queue Latency ==="
grep "WebSocket.*P50\|WebSocket.*P95\|WebSocket.*P99" "$LOG_FILE"

echo ""
echo "=== Queue → Strategy Latency ==="
grep "Queue.*P50\|Queue.*P95\|Queue.*P99" "$LOG_FILE"

echo ""
echo "=== Opportunity Detection Latency ==="
grep "Opportunity.*P50\|Opportunity.*P95\|Opportunity.*P99" "$LOG_FILE"

echo ""
echo "=== Order Placement Latency ==="
grep "Order.*P50\|Order.*P95\|Order.*P99" "$LOG_FILE"

echo ""
echo "=== End-to-End Latency ==="
grep "End-to-End.*P50\|End-to-End.*P95\|End-to-End.*P99" "$LOG_FILE"
EOF

chmod +x extract_latencies.sh
./extract_latencies.sh logs/latency-test-*.log
```

### 4. Calculate Statistics

Create a Python script to analyze the latency data:

```python
#!/usr/bin/env python3
import re
import statistics

def parse_latency_log(log_file):
    """Parse latency metrics from log file"""
    
    metrics = {
        'websocket_to_queue': [],
        'queue_to_strategy': [],
        'opportunity_detection': [],
        'order_placement': [],
        'end_to_end': []
    }
    
    with open(log_file, 'r') as f:
        for line in f:
            # Parse P99 latencies (most important)
            if 'WebSocket → Queue P99:' in line:
                match = re.search(r'(\d+\.?\d*)([μm]s)', line)
                if match:
                    value = float(match.group(1))
                    unit = match.group(2)
                    # Convert to ms
                    if unit == 'μs':
                        value /= 1000
                    metrics['websocket_to_queue'].append(value)
            
            # Similar parsing for other stages...
    
    return metrics

def calculate_percentiles(values):
    """Calculate P50, P95, P99 from list of values"""
    if not values:
        return None, None, None
    
    sorted_values = sorted(values)
    n = len(sorted_values)
    
    p50 = sorted_values[int(n * 0.50)]
    p95 = sorted_values[int(n * 0.95)]
    p99 = sorted_values[int(n * 0.99)]
    
    return p50, p95, p99

def main():
    import sys
    
    if len(sys.argv) < 2:
        print("Usage: python3 analyze_latency.py <log_file>")
        sys.exit(1)
    
    log_file = sys.argv[1]
    metrics = parse_latency_log(log_file)
    
    print("=== Latency Analysis ===\n")
    
    targets = {
        'websocket_to_queue': 0.5,
        'queue_to_strategy': 0.1,
        'opportunity_detection': 2.0,
        'order_placement': 5.0,
        'end_to_end': 10.0
    }
    
    for stage, values in metrics.items():
        if not values:
            print(f"{stage}: No data")
            continue
        
        p50, p95, p99 = calculate_percentiles(values)
        target = targets[stage]
        
        status = "✅ PASS" if p99 <= target else "❌ FAIL"
        
        print(f"{stage}:")
        print(f"  P50: {p50:.3f}ms")
        print(f"  P95: {p95:.3f}ms")
        print(f"  P99: {p99:.3f}ms (target: {target}ms) {status}")
        print(f"  Samples: {len(values)}")
        print()

if __name__ == '__main__':
    main()
```

### 5. Validation Criteria

For each stage, verify that P99 latency meets the target:

#### WebSocket → Queue
- **Target**: P99 < 0.5ms
- **Measurement**: Time from WebSocket message received to pushed to queue
- **Pass Criteria**: P99 ≤ 0.5ms

#### Queue → Strategy
- **Target**: P99 < 0.1ms
- **Measurement**: Time from queue pop to strategy processing
- **Pass Criteria**: P99 ≤ 0.1ms

#### Opportunity Detection
- **Target**: P99 < 2ms
- **Measurement**: Time from market update to opportunity generated
- **Pass Criteria**: P99 ≤ 2ms

#### Order Placement
- **Target**: P99 < 5ms
- **Measurement**: Time from opportunity to orders placed on exchange
- **Pass Criteria**: P99 ≤ 5ms

#### End-to-End
- **Target**: P99 < 10ms
- **Measurement**: Time from WebSocket data to orders placed
- **Pass Criteria**: P99 ≤ 10ms

### 6. Report Generation

Create a latency validation report:

```markdown
# Latency Validation Report

**Test Date**: 2024-01-15
**Duration**: 1 hour
**Samples Collected**: 3,600+

## Results

| Stage | P50 | P95 | P99 | Target | Status |
|-------|-----|-----|-----|--------|--------|
| WebSocket → Queue | 0.12ms | 0.28ms | 0.42ms | <0.5ms | ✅ PASS |
| Queue → Strategy | 0.03ms | 0.06ms | 0.08ms | <0.1ms | ✅ PASS |
| Opportunity Detection | 0.45ms | 1.2ms | 1.8ms | <2ms | ✅ PASS |
| Order Placement | 2.1ms | 3.8ms | 4.5ms | <5ms | ✅ PASS |
| End-to-End | 3.2ms | 6.5ms | 8.9ms | <10ms | ✅ PASS |

## Analysis

### WebSocket → Queue (✅ PASS)
- P99: 0.42ms (target: 0.5ms)
- Margin: 0.08ms (16%)
- Excellent performance, well within target

### Queue → Strategy (✅ PASS)
- P99: 0.08ms (target: 0.1ms)
- Margin: 0.02ms (20%)
- Lock-free queue performing as expected

### Opportunity Detection (✅ PASS)
- P99: 1.8ms (target: 2ms)
- Margin: 0.2ms (10%)
- Dashboard logic replication is efficient

### Order Placement (✅ PASS)
- P99: 4.5ms (target: 5ms)
- Margin: 0.5ms (10%)
- Network latency to Bybit demo is acceptable

### End-to-End (✅ PASS)
- P99: 8.9ms (target: 10ms)
- Margin: 1.1ms (11%)
- Overall pipeline meets low-latency requirements

## Conclusion

**PASS** - All latency targets met with comfortable margins.

## Recommendations

1. Continue monitoring latency in production
2. Set up alerts if P99 exceeds 80% of target
3. Profile if latency degrades over time
```

## Troubleshooting

### High WebSocket → Queue Latency

If P99 > 0.5ms:

1. Check CPU frequency scaling (should be "performance")
2. Verify thread pinning is working (core 2)
3. Check network latency to Bybit
4. Review system load (other processes?)
5. Check for CPU thermal throttling

### High Queue → Strategy Latency

If P99 > 0.1ms:

1. Verify lock-free queue implementation
2. Check thread pinning (core 1)
3. Review CPU cache effects
4. Check for context switches

### High Opportunity Detection Latency

If P99 > 2ms:

1. Profile the generator code
2. Check for unnecessary allocations
3. Review confidence score calculation
4. Optimize hot paths

### High Order Placement Latency

If P99 > 5ms:

1. Check network latency to Bybit
2. Review API request serialization
3. Check for DNS resolution delays
4. Verify HTTP connection pooling

### High End-to-End Latency

If P99 > 10ms:

1. Identify which stage is the bottleneck
2. Profile the entire pipeline
3. Check for unexpected blocking operations
4. Review async/await usage

## Advanced Profiling

For detailed latency analysis:

```bash
# Use perf to profile
sudo perf record -F 99 -p $(cat logs/latency-test.pid) sleep 60
sudo perf report

# Use flamegraph
cargo flamegraph --bin bybit-synthetic-test

# Use tokio-console for async profiling
RUSTFLAGS="--cfg tokio_unstable" cargo run --release --bin bybit-synthetic-test
```

## Next Steps

After successful latency validation:

1. ✅ Document results in validation report
2. ✅ Archive latency data for baseline
3. ✅ Proceed to throughput validation (Task 32)
4. ✅ Proceed to execution success rate validation (Task 33)

## References

- Requirements: `.kiro/specs/bybit-synthetic-test-mode/requirements.md` (Section 9)
- Design: `.kiro/specs/bybit-synthetic-test-mode/design.md`
- Latency Tracker: `src/strategy/latency_tracker.rs`
