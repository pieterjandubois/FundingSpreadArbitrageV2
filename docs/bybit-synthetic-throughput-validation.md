# Throughput Validation Procedure

## Overview

This document describes the procedure for validating that the Bybit Synthetic Test Mode system can handle high-throughput market data without dropping updates or degrading performance.

## Throughput Targets

From Requirements 9.3 and Performance Targets:

- **Target**: Process 1000+ market updates per second
- **Data Loss**: Zero market data loss during normal operation
- **CPU Utilization**: < 50% on strategy thread
- **Memory**: Stable (no leaks)

## Prerequisites

- System running in release mode
- Thread pinning enabled
- Multiple symbols configured (BTCUSDT, ETHUSDT, SOLUSDT, etc.)
- Stable network connection to Bybit demo

## Test Procedure

### 1. Preparation

```bash
# Build in release mode
cargo build --release --bin bybit-synthetic-test

# Configure for high throughput
cat > .env << EOF
BYBIT_DEMO_API_KEY=your_key
BYBIT_DEMO_API_SECRET=your_secret

# Use multiple symbols for higher update rate
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT,ADAUSDT,DOGEUSDT,XRPUSDT,DOTUSDT

SYNTHETIC_SPREAD_BPS=15.0
SYNTHETIC_FUNDING_DELTA=0.0001
ESTIMATED_POSITION_SIZE=100.0
MAX_CONCURRENT_TRADES=3

RUST_LOG=info
EOF
```

### 2. Run Throughput Test

```bash
# Start the system
cargo run --release --bin bybit-synthetic-test \
  > logs/throughput-test-$(date +%Y%m%d-%H%M%S).log 2>&1 &

# Save PID
echo $! > logs/throughput-test.pid

# Monitor for 30 minutes
sleep 1800

# Stop gracefully
kill -INT $(cat logs/throughput-test.pid)
```

### 3. Monitor During Test

Use these commands to monitor throughput:

```bash
# Monitor CPU usage
top -p $(cat logs/throughput-test.pid) -d 1

# Monitor memory usage
watch -n 1 "ps -p $(cat logs/throughput-test.pid) -o pid,vsz,rss,cmd"

# Count updates per second (in another terminal)
watch -n 1 "grep 'market update' logs/throughput-test-*.log | tail -n 1000 | wc -l"

# Monitor queue depth (if logged)
watch -n 1 "grep 'queue depth' logs/throughput-test-*.log | tail -n 1"
```

### 4. Calculate Throughput Metrics

Extract throughput data from logs:

```bash
# Count total market updates
TOTAL_UPDATES=$(grep -c "market update\|MarketUpdate" logs/throughput-test-*.log)

# Calculate test duration (in seconds)
START_TIME=$(head -n 1 logs/throughput-test-*.log | grep -oP '\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}')
END_TIME=$(tail -n 1 logs/throughput-test-*.log | grep -oP '\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}')

# Calculate average throughput
echo "Total Updates: $TOTAL_UPDATES"
echo "Average Throughput: $(($TOTAL_UPDATES / 1800)) updates/sec"

# Check for dropped updates
DROPPED=$(grep -c "dropped\|overflow" logs/throughput-test-*.log)
echo "Dropped Updates: $DROPPED"
```

Create a Python script for detailed analysis:

```python
#!/usr/bin/env python3
import re
from datetime import datetime
from collections import defaultdict

def analyze_throughput(log_file):
    """Analyze throughput from log file"""
    
    updates_per_second = defaultdict(int)
    total_updates = 0
    dropped_updates = 0
    
    with open(log_file, 'r') as f:
        for line in f:
            # Extract timestamp
            match = re.search(r'(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})', line)
            if not match:
                continue
            
            timestamp = match.group(1)
            second = timestamp[:19]  # Truncate to second precision
            
            # Count market updates
            if 'market update' in line.lower() or 'MarketUpdate' in line:
                updates_per_second[second] += 1
                total_updates += 1
            
            # Count dropped updates
            if 'dropped' in line.lower() or 'overflow' in line.lower():
                dropped_updates += 1
    
    # Calculate statistics
    if not updates_per_second:
        print("No throughput data found")
        return
    
    throughputs = list(updates_per_second.values())
    avg_throughput = sum(throughputs) / len(throughputs)
    min_throughput = min(throughputs)
    max_throughput = max(throughputs)
    
    # Calculate percentiles
    sorted_throughputs = sorted(throughputs)
    n = len(sorted_throughputs)
    p50 = sorted_throughputs[int(n * 0.50)]
    p95 = sorted_throughputs[int(n * 0.95)]
    p99 = sorted_throughputs[int(n * 0.99)]
    
    print("=== Throughput Analysis ===\n")
    print(f"Total Updates: {total_updates}")
    print(f"Dropped Updates: {dropped_updates}")
    print(f"Drop Rate: {(dropped_updates / total_updates * 100):.2f}%")
    print()
    print(f"Average Throughput: {avg_throughput:.1f} updates/sec")
    print(f"Min Throughput: {min_throughput} updates/sec")
    print(f"Max Throughput: {max_throughput} updates/sec")
    print()
    print(f"P50 Throughput: {p50} updates/sec")
    print(f"P95 Throughput: {p95} updates/sec")
    print(f"P99 Throughput: {p99} updates/sec")
    print()
    
    # Validation
    target = 1000
    if avg_throughput >= target:
        print(f"✅ PASS: Average throughput ({avg_throughput:.1f}) >= target ({target})")
    else:
        print(f"❌ FAIL: Average throughput ({avg_throughput:.1f}) < target ({target})")
    
    if dropped_updates == 0:
        print("✅ PASS: No dropped updates")
    else:
        print(f"⚠️  WARNING: {dropped_updates} updates dropped")

if __name__ == '__main__':
    import sys
    
    if len(sys.argv) < 2:
        print("Usage: python3 analyze_throughput.py <log_file>")
        sys.exit(1)
    
    analyze_throughput(sys.argv[1])
```

### 5. CPU and Memory Analysis

```bash
# Extract CPU usage samples
grep "CPU" logs/throughput-test-*.log > cpu_usage.txt

# Calculate average CPU usage
awk '{sum+=$2; count++} END {print "Average CPU:", sum/count "%"}' cpu_usage.txt

# Extract memory usage samples
grep "Memory" logs/throughput-test-*.log > memory_usage.txt

# Check for memory growth
head -n 1 memory_usage.txt
tail -n 1 memory_usage.txt
```

### 6. Validation Criteria

#### Throughput
- **Target**: ≥ 1000 updates/sec average
- **Measurement**: Count market updates per second
- **Pass Criteria**: Average ≥ 1000 updates/sec

#### Data Loss
- **Target**: 0 dropped updates
- **Measurement**: Count dropped/overflow messages
- **Pass Criteria**: 0 dropped updates

#### CPU Utilization
- **Target**: < 50% on strategy thread
- **Measurement**: Monitor CPU usage during test
- **Pass Criteria**: Average CPU < 50%

#### Memory Stability
- **Target**: No memory leaks
- **Measurement**: Compare memory at start vs end
- **Pass Criteria**: Memory growth < 10MB over 30 minutes

### 7. Report Generation

Create a throughput validation report:

```markdown
# Throughput Validation Report

**Test Date**: 2024-01-15
**Duration**: 30 minutes
**Symbols**: 8 (BTCUSDT, ETHUSDT, SOLUSDT, BNBUSDT, ADAUSDT, DOGEUSDT, XRPUSDT, DOTUSDT)

## Results

### Throughput
- Total Updates: 1,845,230
- Average: 1,025 updates/sec
- Min: 850 updates/sec
- Max: 1,250 updates/sec
- P50: 1,020 updates/sec
- P95: 1,180 updates/sec
- P99: 1,220 updates/sec
- **Status**: ✅ PASS (target: 1000+ updates/sec)

### Data Loss
- Dropped Updates: 0
- Drop Rate: 0.00%
- **Status**: ✅ PASS (target: 0 drops)

### CPU Utilization
- Strategy Thread Average: 38%
- Strategy Thread Peak: 47%
- WebSocket Thread Average: 12%
- **Status**: ✅ PASS (target: <50%)

### Memory
- Start: 45 MB
- End: 48 MB
- Growth: 3 MB
- **Status**: ✅ PASS (target: <10MB growth)

## Analysis

### Throughput Performance
The system consistently processed over 1000 updates/sec with comfortable headroom. Peak throughput reached 1,250 updates/sec during high-volatility periods.

### Zero Data Loss
No updates were dropped during the 30-minute test, demonstrating the lock-free queue and backpressure handling work correctly.

### CPU Efficiency
Strategy thread utilization remained well below 50%, indicating the system can handle even higher throughput if needed. Thread pinning is working effectively.

### Memory Stability
Memory growth of 3MB over 30 minutes is within normal bounds and likely due to log buffering. No memory leaks detected.

## Conclusion

**PASS** - System meets all throughput requirements with comfortable margins.

## Recommendations

1. System can handle production load (1000+ updates/sec)
2. Consider increasing to 10+ symbols for stress testing
3. Monitor CPU usage in production, alert if >40%
4. Set up throughput monitoring dashboard
```

## Troubleshooting

### Low Throughput

If average throughput < 1000 updates/sec:

1. Check Bybit WebSocket connection (is it receiving data?)
2. Verify multiple symbols are configured
3. Check network latency to Bybit
4. Review market hours (low volume during off-hours)
5. Check for CPU throttling

### Dropped Updates

If updates are being dropped:

1. Increase queue capacity in MarketPipeline
2. Check strategy thread CPU usage (is it overloaded?)
3. Review opportunity generation logic (too slow?)
4. Check for blocking operations in hot path
5. Profile the strategy thread

### High CPU Usage

If CPU > 50% on strategy thread:

1. Profile to find hot spots
2. Check for unnecessary allocations
3. Review opportunity generation frequency
4. Optimize confidence score calculation
5. Consider reducing number of symbols

### Memory Growth

If memory grows continuously:

1. Check for unbounded collections
2. Review latency tracking (vectors growing?)
3. Check log buffering
4. Use memory profiler (heaptrack, valgrind)
5. Look for Arc cycles

## Stress Testing

For extreme throughput testing:

```bash
# Configure 20+ symbols
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT,ADAUSDT,DOGEUSDT,XRPUSDT,DOTUSDT,MATICUSDT,LINKUSDT,AVAXUSDT,ATOMUSDT,NEARUSDT,APTUSDT,ARBUSDT,OPUSDT,LDOUSDT,INJUSDT,SUIUSDT,SEIUSDT

# Run for 1 hour
cargo run --release --bin bybit-synthetic-test

# Monitor for queue overflow, CPU saturation, memory issues
```

## Next Steps

After successful throughput validation:

1. ✅ Document results in validation report
2. ✅ Archive throughput data for baseline
3. ✅ Proceed to execution success rate validation (Task 33)
4. ✅ Prepare for production deployment

## References

- Requirements: `.kiro/specs/bybit-synthetic-test-mode/requirements.md` (Performance Targets)
- Design: `.kiro/specs/bybit-synthetic-test-mode/design.md`
- Pipeline: `src/strategy/pipeline.rs`
