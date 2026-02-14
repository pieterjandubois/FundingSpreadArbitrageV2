# Bybit Synthetic Test Mode - User Guide

## Overview

The Bybit Synthetic Test Mode is a safe testing environment for validating the low-latency arbitrage system using only Bybit demo accounts. It generates synthetic arbitrage opportunities from real market data and executes real orders on Bybit demo to test all execution logic without financial risk.

## What is Synthetic Test Mode?

Instead of requiring two exchanges for arbitrage testing, this mode:
- Connects to Bybit demo WebSocket for real market data
- Generates synthetic "long" and "short" prices by applying a spread
- Executes both legs on Bybit demo (at different prices)
- Tests all execution logic (atomic execution, emergency close, etc.)
- Provides zero financial risk (demo account only)

## Prerequisites

### 1. Bybit Demo Account

Create a Bybit demo account:
1. Go to https://testnet.bybit.com/
2. Sign up for a demo account
3. Generate API keys (Settings → API Management)
4. Save your API key and secret

### 2. System Requirements

- **OS**: Linux, macOS, or Windows
- **CPU**: 4+ cores (for thread pinning)
- **Memory**: 2GB+ available
- **Disk**: 1GB+ for logs
- **Network**: Stable connection with <100ms latency to Bybit

### 3. Rust Toolchain

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify installation
rustc --version
cargo --version
```

## Installation

### 1. Clone the Repository

```bash
git clone <repository-url>
cd arbitrage2
```

### 2. Build the Binary

```bash
# Build in release mode for best performance
cargo build --release --bin bybit-synthetic-test

# Verify the binary was created
ls -lh target/release/bybit-synthetic-test
```

## Configuration

### Environment Variables

Create a `.env` file in the project root:

```bash
# Bybit Demo Credentials (REQUIRED)
BYBIT_DEMO_API_KEY=your_api_key_here
BYBIT_DEMO_API_SECRET=your_api_secret_here

# Synthetic Test Configuration
SYNTHETIC_SPREAD_BPS=15.0              # Spread in basis points (default: 15)
SYNTHETIC_FUNDING_DELTA=0.0001         # Funding delta per 8h (default: 0.01%)
ESTIMATED_POSITION_SIZE=100.0          # Position size in USD (default: $100)
MAX_CONCURRENT_TRADES=3                # Max concurrent trades (default: 3)

# Symbols to Trade (comma-separated)
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT

# Logging Level
RUST_LOG=info                          # Options: error, warn, info, debug, trace
```

### Configuration Parameters

| Parameter | Description | Default | Range |
|-----------|-------------|---------|-------|
| `SYNTHETIC_SPREAD_BPS` | Synthetic spread in basis points | 15.0 | 10-50 |
| `SYNTHETIC_FUNDING_DELTA` | Funding rate delta per 8 hours | 0.0001 | 0.0001-0.01 |
| `ESTIMATED_POSITION_SIZE` | Position size in USD | 100.0 | 10-1000 |
| `MAX_CONCURRENT_TRADES` | Maximum concurrent trades | 3 | 1-10 |
| `SYMBOLS_TO_TRADE` | Trading symbols | BTCUSDT,ETHUSDT | Any Bybit perpetuals |

### Recommended Settings

**Conservative (Learning)**:
```bash
SYNTHETIC_SPREAD_BPS=20.0
ESTIMATED_POSITION_SIZE=50.0
MAX_CONCURRENT_TRADES=2
SYMBOLS_TO_TRADE=BTCUSDT
```

**Moderate (Testing)**:
```bash
SYNTHETIC_SPREAD_BPS=15.0
ESTIMATED_POSITION_SIZE=100.0
MAX_CONCURRENT_TRADES=3
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT
```

**Aggressive (Stress Testing)**:
```bash
SYNTHETIC_SPREAD_BPS=12.0
ESTIMATED_POSITION_SIZE=200.0
MAX_CONCURRENT_TRADES=5
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT,ADAUSDT
```

## Running the System

### Basic Usage

```bash
# Run with default configuration
cargo run --release --bin bybit-synthetic-test

# Run with custom log level
RUST_LOG=debug cargo run --release --bin bybit-synthetic-test

# Run in background with log file
cargo run --release --bin bybit-synthetic-test \
  > logs/test-$(date +%Y%m%d-%H%M%S).log 2>&1 &
```

### Expected Output

When the system starts, you should see:

```
=== Bybit Synthetic Test Mode ===
Starting initialization...
Configuration loaded: SyntheticConfig { ... }
Initializing Bybit demo backend...
Backend initialized and time synced
Setting up streaming pipeline...
Pipeline created
Starting WebSocket thread on core 2...
Starting strategy thread on core 1...
Strategy thread initialized, entering main loop...
System running. Press Ctrl+C to shutdown gracefully...
```

During operation:

```
Generated opportunity: BTCUSDT spread=15.02bps confidence=78 profit=2.45bps
Trade executed successfully: PaperTrade { symbol: "BTCUSDT", ... }
Generated opportunity: ETHUSDT spread=15.01bps confidence=75 profit=2.12bps
Trade executed successfully: PaperTrade { symbol: "ETHUSDT", ... }

=== Periodic Metrics (60s) ===
Opportunities Generated: 45
Trades Executed: 12
Success Rate: 96.7%
Latency P99: 8.2ms
```

### Graceful Shutdown

Press `Ctrl+C` to stop the system gracefully:

```
^C
=== Graceful Shutdown Initiated ===
Waiting for threads to complete...
Closing 2 active trades...
Closing trade: PaperTrade { ... }
Closing trade: PaperTrade { ... }

=== Final Metrics Summary ===
Opportunities Generated: 487
Trades Executed: 123
Success Rate: 96.8%
Average Latency: 6.5ms
P99 Latency: 8.9ms

=== Shutdown Complete ===
```

## Monitoring

### Real-Time Monitoring

While the system is running, monitor it with:

```bash
# Watch log file
tail -f logs/*.log

# Monitor CPU and memory
top -p $(pgrep bybit-synthetic)

# Count opportunities generated
watch -n 5 "grep 'Generated opportunity' logs/*.log | wc -l"

# Count successful trades
watch -n 5 "grep 'Trade executed successfully' logs/*.log | wc -l"
```

### Metrics Interpretation

#### Opportunities Generated
- **Good**: 10-50 per minute
- **Low**: < 5 per minute (check spread settings)
- **High**: > 100 per minute (may overwhelm system)

#### Success Rate
- **Excellent**: > 95%
- **Good**: 90-95%
- **Poor**: < 90% (investigate failures)

#### Latency (P99)
- **Excellent**: < 5ms
- **Good**: 5-10ms
- **Acceptable**: 10-20ms
- **Poor**: > 20ms (investigate bottlenecks)

## Troubleshooting

### Common Issues

#### 1. "Failed to sync server time"

**Cause**: Cannot connect to Bybit demo API

**Solutions**:
- Check internet connection
- Verify Bybit demo is accessible: `curl https://api-demo.bybit.com/v5/market/time`
- Check API credentials are correct
- Verify API keys have trading permissions

#### 2. "WebSocket connection failed"

**Cause**: Cannot connect to Bybit WebSocket

**Solutions**:
- Check firewall settings (allow WSS connections)
- Verify network stability
- Check Bybit demo status page
- System will auto-reconnect with exponential backoff

#### 3. "No opportunities generated"

**Cause**: Spread too narrow or constraints too strict

**Solutions**:
- Increase `SYNTHETIC_SPREAD_BPS` (try 20-30)
- Check market is active (not weekend/low volume)
- Verify symbols are correct
- Check logs for constraint failures

#### 4. "Trade execution failed: Insufficient balance"

**Cause**: Demo account balance too low

**Solutions**:
- Log into Bybit demo web interface
- Request more demo funds (usually available in settings)
- Reduce `ESTIMATED_POSITION_SIZE`

#### 5. "High CPU usage"

**Cause**: Too many symbols or opportunities

**Solutions**:
- Reduce number of symbols
- Increase `SYNTHETIC_SPREAD_BPS` (fewer opportunities)
- Check for other processes using CPU
- Verify thread pinning is working

#### 6. "Memory growing continuously"

**Cause**: Possible memory leak

**Solutions**:
- Check logs for unbounded growth
- Restart system periodically
- Report issue with logs
- Monitor with `valgrind` or `heaptrack`

### Debug Mode

For detailed debugging:

```bash
# Run with trace logging
RUST_LOG=trace cargo run --release --bin bybit-synthetic-test

# Enable tokio console (requires rebuild)
RUSTFLAGS="--cfg tokio_unstable" cargo run --release --bin bybit-synthetic-test
```

### Log Analysis

```bash
# Extract errors
grep ERROR logs/*.log

# Extract warnings
grep WARN logs/*.log

# Count opportunities by symbol
grep "Generated opportunity" logs/*.log | grep -oP "BTCUSDT|ETHUSDT|SOLUSDT" | sort | uniq -c

# Calculate average success rate
SUCCESS=$(grep -c "Trade executed successfully" logs/*.log)
FAILURES=$(grep -c "Trade execution failed" logs/*.log)
echo "Success Rate: $(echo "scale=2; $SUCCESS * 100 / ($SUCCESS + $FAILURES)" | bc)%"
```

## Safety Features

### Position Limits
- Maximum position size enforced per trade
- Maximum concurrent trades enforced
- Prevents runaway execution

### Emergency Stop
- System halts trading after 3 failures in 60 seconds
- Requires manual restart
- Prevents cascading failures

### Atomic Execution
- Both legs must fill or neither fills
- No unhedged positions
- Automatic cancellation on timeout

### Graceful Shutdown
- Ctrl+C closes all active positions
- Waits for cleanup before exit
- Prints final metrics

## Performance Tips

### 1. Thread Pinning

For best performance, ensure thread pinning works:

```bash
# Check CPU cores
lscpu

# Verify cores 1 and 2 are available
# System will pin strategy to core 1, WebSocket to core 2
```

### 2. CPU Governor

Set CPU governor to performance mode:

```bash
# Check current governor
cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Set to performance (requires sudo)
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
```

### 3. Network Optimization

- Use wired connection (not WiFi)
- Close bandwidth-heavy applications
- Check latency: `ping api-demo.bybit.com`

### 4. System Load

- Close unnecessary applications
- Monitor with `htop` or `top`
- Ensure sufficient RAM available

## Next Steps

After successfully running the synthetic test mode:

1. **Validate Performance**
   - Run 24-hour stability test
   - Validate latency requirements
   - Validate throughput requirements
   - Validate execution success rate

2. **Analyze Results**
   - Review metrics and logs
   - Identify any issues or anomalies
   - Document findings

3. **Production Planning**
   - Add second exchange (Bitget/OKX demo)
   - Test real cross-exchange arbitrage
   - Gradually increase position sizes
   - Deploy to production

## Support

### Documentation
- Requirements: `.kiro/specs/bybit-synthetic-test-mode/requirements.md`
- Design: `.kiro/specs/bybit-synthetic-test-mode/design.md`
- Tasks: `.kiro/specs/bybit-synthetic-test-mode/tasks.md`

### Validation Procedures
- 24-Hour Stability Test: `docs/bybit-synthetic-24h-stability-test.md`
- Latency Validation: `docs/bybit-synthetic-latency-validation.md`
- Throughput Validation: `docs/bybit-synthetic-throughput-validation.md`
- Execution Success Rate: `docs/bybit-synthetic-execution-success-rate.md`

### Getting Help

If you encounter issues:
1. Check this user guide
2. Review troubleshooting section
3. Check logs for error messages
4. Consult design documentation
5. Report issues with logs and configuration

## Appendix: Example Session

```bash
# 1. Set up environment
cat > .env << EOF
BYBIT_DEMO_API_KEY=your_key
BYBIT_DEMO_API_SECRET=your_secret
SYNTHETIC_SPREAD_BPS=15.0
ESTIMATED_POSITION_SIZE=100.0
MAX_CONCURRENT_TRADES=3
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT
RUST_LOG=info
EOF

# 2. Build and run
cargo build --release --bin bybit-synthetic-test
cargo run --release --bin bybit-synthetic-test

# 3. Monitor (in another terminal)
tail -f logs/*.log

# 4. Stop gracefully (Ctrl+C in main terminal)
^C

# 5. Review results
grep "Final Metrics Summary" logs/*.log -A 10
```

Expected output:
```
=== Final Metrics Summary ===
Opportunities Generated: 487
Trades Executed: 123
Success Rate: 96.8%
Average Latency: 6.5ms
P99 Latency: 8.9ms
Emergency Closes: 2
Partial Fills: 5
Cancellations: 3
```

Success! The system is working correctly.
