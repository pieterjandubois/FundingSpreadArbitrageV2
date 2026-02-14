# 24-Hour Stability Test Procedure

## Overview

This document describes the procedure for running a 24-hour stability test of the Bybit Synthetic Test Mode system. The test validates that the system can run continuously without crashes, memory leaks, or performance degradation.

## Prerequisites

- Bybit demo account with API credentials
- Environment variables configured (see `.env.example`)
- System with at least 4 CPU cores (for thread pinning)
- Minimum 2GB RAM available
- Stable internet connection

## Test Configuration

### Environment Variables

```bash
# Bybit demo credentials
BYBIT_DEMO_API_KEY=your_key_here
BYBIT_DEMO_API_SECRET=your_secret_here

# Synthetic test configuration
SYNTHETIC_SPREAD_BPS=15.0
SYNTHETIC_FUNDING_DELTA=0.0001
ESTIMATED_POSITION_SIZE=100.0
MAX_CONCURRENT_TRADES=3
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT

# Logging
RUST_LOG=info
```

### System Requirements

- **CPU**: 4+ cores (cores 1 and 2 will be pinned)
- **Memory**: 2GB+ available
- **Disk**: 1GB+ for logs
- **Network**: Stable connection with <100ms latency to Bybit

## Test Procedure

### 1. Pre-Test Checks

Before starting the 24-hour test:

```bash
# Verify environment variables
cat .env

# Check system resources
free -h
df -h

# Verify Bybit demo connectivity
curl -I https://api-demo.bybit.com/v5/market/time

# Build the binary in release mode
cargo build --release --bin bybit-synthetic-test
```

### 2. Start the Test

```bash
# Create log directory
mkdir -p logs

# Start the test with output redirection
RUST_LOG=info cargo run --release --bin bybit-synthetic-test \
  > logs/stability-test-$(date +%Y%m%d-%H%M%S).log 2>&1 &

# Save the process ID
echo $! > logs/test.pid

# Verify it's running
ps aux | grep bybit-synthetic-test
```

### 3. Monitoring During Test

Monitor the system every 4 hours:

```bash
# Check process is still running
ps -p $(cat logs/test.pid)

# Check memory usage
ps -p $(cat logs/test.pid) -o pid,vsz,rss,cmd

# Check CPU usage
top -p $(cat logs/test.pid) -n 1

# Check log file size
ls -lh logs/*.log

# Tail recent logs
tail -n 50 logs/*.log
```

### 4. Metrics to Track

Create a monitoring spreadsheet with these columns:

| Time | Status | Memory (MB) | CPU (%) | Opportunities | Trades | Success Rate | Errors |
|------|--------|-------------|---------|---------------|--------|--------------|--------|
| 0h   | Running| 45          | 12      | 0             | 0      | N/A          | 0      |
| 4h   | Running| 48          | 15      | 1250          | 45     | 95.5%        | 2      |
| 8h   | Running| 47          | 14      | 2480          | 89     | 96.2%        | 3      |
| ...  | ...    | ...         | ...     | ...           | ...    | ...          | ...    |

### 5. Success Criteria

The test passes if ALL of the following are true:

#### Stability Criteria
- ✅ Process runs for full 24 hours without crashes
- ✅ No panics or fatal errors in logs
- ✅ System responds to monitoring commands throughout

#### Memory Criteria
- ✅ Memory usage remains < 100MB throughout test
- ✅ No memory leaks (memory should stabilize after initial ramp-up)
- ✅ RSS (Resident Set Size) doesn't grow continuously

#### Performance Criteria
- ✅ CPU usage remains < 50% on strategy thread
- ✅ Latency metrics remain within targets (P99 < 10ms)
- ✅ Throughput remains stable (1000+ updates/sec)

#### Functional Criteria
- ✅ Opportunities continue to be generated throughout test
- ✅ Trades continue to execute successfully
- ✅ Success rate remains > 95%
- ✅ WebSocket reconnections work (if disconnects occur)

### 6. Post-Test Analysis

After 24 hours, stop the test gracefully:

```bash
# Send SIGINT (Ctrl+C equivalent)
kill -INT $(cat logs/test.pid)

# Wait for graceful shutdown (up to 30 seconds)
sleep 30

# Verify process stopped
ps -p $(cat logs/test.pid) || echo "Process stopped successfully"
```

Analyze the results:

```bash
# Extract final metrics from logs
grep "Final Metrics Summary" logs/*.log -A 20

# Count opportunities generated
grep "Generated opportunity" logs/*.log | wc -l

# Count successful trades
grep "Trade executed successfully" logs/*.log | wc -l

# Count errors
grep "ERROR" logs/*.log | wc -l

# Check for memory leaks
grep "memory" logs/*.log -i

# Check for panics
grep "panic" logs/*.log -i
```

### 7. Report Generation

Create a test report with:

1. **Test Configuration**
   - Start time and end time
   - Environment variables used
   - System specifications

2. **Stability Results**
   - Total runtime (should be 24 hours)
   - Number of crashes (should be 0)
   - Number of restarts (should be 0)

3. **Performance Results**
   - Memory usage: min, max, average
   - CPU usage: min, max, average
   - Latency percentiles: P50, P95, P99
   - Throughput: average updates/sec

4. **Functional Results**
   - Total opportunities generated
   - Total trades executed
   - Success rate (%)
   - Error count and types

5. **Issues Encountered**
   - List any anomalies or unexpected behavior
   - WebSocket disconnections and reconnections
   - API errors or rate limits

6. **Conclusion**
   - Pass/Fail determination
   - Recommendations for production deployment

## Example Report Template

```markdown
# 24-Hour Stability Test Report

**Test Date**: 2024-01-15
**Start Time**: 2024-01-15 00:00:00 UTC
**End Time**: 2024-01-16 00:00:00 UTC
**Duration**: 24 hours

## Configuration
- Spread: 15 bps
- Position Size: $100
- Max Concurrent Trades: 3
- Symbols: BTCUSDT, ETHUSDT, SOLUSDT

## Results

### Stability
- ✅ No crashes
- ✅ No panics
- ✅ Ran for full 24 hours

### Memory
- Min: 42 MB
- Max: 52 MB
- Average: 47 MB
- ✅ No memory leaks detected

### Performance
- CPU Average: 18%
- P99 Latency: 8.2ms
- Throughput: 1250 updates/sec
- ✅ All targets met

### Functional
- Opportunities Generated: 12,450
- Trades Executed: 487
- Success Rate: 96.8%
- Errors: 15 (all recoverable)
- ✅ All targets met

## Conclusion
**PASS** - System is stable and ready for extended testing.
```

## Troubleshooting

### Process Crashes

If the process crashes during the test:

1. Check the logs for panic messages
2. Check system resources (OOM killer?)
3. Verify Bybit API credentials are valid
4. Check network connectivity
5. Review error messages before crash

### Memory Leaks

If memory grows continuously:

1. Check for unbounded collections (Vec, HashMap)
2. Review latency tracking (are vectors growing unbounded?)
3. Check for circular references (Arc cycles)
4. Use `valgrind` or `heaptrack` for detailed analysis

### Performance Degradation

If latency increases over time:

1. Check CPU usage (thermal throttling?)
2. Check disk I/O (log file too large?)
3. Check network latency to Bybit
4. Review system load (other processes?)

### WebSocket Issues

If WebSocket disconnects frequently:

1. Check network stability
2. Review Bybit API status
3. Verify reconnection logic works
4. Check for rate limiting

## Next Steps

After successful 24-hour test:

1. ✅ Document results in test report
2. ✅ Archive logs for future reference
3. ✅ Proceed to latency validation (Task 31)
4. ✅ Proceed to throughput validation (Task 32)
5. ✅ Proceed to execution success rate validation (Task 33)

## References

- Bybit Demo API: https://testnet.bybit.com/
- Requirements: `.kiro/specs/bybit-synthetic-test-mode/requirements.md`
- Design: `.kiro/specs/bybit-synthetic-test-mode/design.md`
