# Bybit Synthetic Test Mode - Deployment Guide

## Overview

This guide describes how to deploy and operate the Bybit Synthetic Test Mode system, from initial setup through production migration.

## Deployment Phases

### Phase 1: Bybit Demo Testing (Current)
- Single exchange (Bybit demo only)
- Synthetic opportunities
- Zero financial risk
- **Duration**: 1-2 weeks

### Phase 2: Dual-Exchange Demo Testing
- Add second exchange (Bitget/OKX demo)
- Real cross-exchange opportunities
- Still zero financial risk
- **Duration**: 1-2 weeks

### Phase 3: Live Testing (Small Positions)
- Real exchanges with real money
- Start with $10-50 position sizes
- Monitor closely
- **Duration**: 2-4 weeks

### Phase 4: Production Scaling
- Gradually increase position sizes
- Scale to target volume
- Full monitoring and alerting
- **Duration**: Ongoing

## Phase 1: Bybit Demo Setup

### 1. Create Bybit Demo Account

1. Go to https://testnet.bybit.com/
2. Click "Sign Up" and create account
3. Verify email address
4. Log in to demo account

### 2. Generate API Keys

1. Go to Settings → API Management
2. Click "Create New Key"
3. Set permissions:
   - ✅ Read
   - ✅ Trade
   - ❌ Withdraw (not needed)
4. Save API Key and Secret securely
5. Whitelist IP addresses (optional but recommended)

### 3. Fund Demo Account

1. Go to Assets → Demo Trading
2. Request demo funds (usually $100,000 USDT)
3. Verify balance appears in account

### 4. Deploy System

```bash
# Clone repository
git clone <repository-url>
cd arbitrage2

# Create environment file
cat > .env << EOF
BYBIT_DEMO_API_KEY=your_api_key_here
BYBIT_DEMO_API_SECRET=your_api_secret_here

SYNTHETIC_SPREAD_BPS=15.0
SYNTHETIC_FUNDING_DELTA=0.0001
ESTIMATED_POSITION_SIZE=100.0
MAX_CONCURRENT_TRADES=3
SYMBOLS_TO_TRADE=BTCUSDT,ETHUSDT,SOLUSDT

RUST_LOG=info
EOF

# Build
cargo build --release --bin bybit-synthetic-test

# Run
cargo run --release --bin bybit-synthetic-test
```

### 5. Validate System

Run all validation procedures:

1. **24-Hour Stability Test**
   - See: `docs/bybit-synthetic-24h-stability-test.md`
   - Target: No crashes, stable memory

2. **Latency Validation**
   - See: `docs/bybit-synthetic-latency-validation.md`
   - Target: P99 < 10ms end-to-end

3. **Throughput Validation**
   - See: `docs/bybit-synthetic-throughput-validation.md`
   - Target: 1000+ updates/sec

4. **Execution Success Rate**
   - See: `docs/bybit-synthetic-execution-success-rate.md`
   - Target: > 95% success rate

### 6. Sign-Off Criteria

Before proceeding to Phase 2:

- ✅ All validation tests pass
- ✅ No critical bugs found
- ✅ Performance meets targets
- ✅ Documentation complete
- ✅ Team trained on system

## Phase 2: Dual-Exchange Demo Testing

### 1. Add Second Exchange

Choose a second demo exchange:
- **Bitget Demo**: https://www.bitget.com/en/demo-trading
- **OKX Demo**: https://www.okx.com/demo-trading

### 2. Modify Configuration

Update the system to use real cross-exchange opportunities:

```rust
// In src/bin/bybit-synthetic-test.rs
// Replace SyntheticOpportunityGenerator with real opportunity detection

// Use existing dashboard logic:
// - Fetch prices from both exchanges
// - Calculate real spreads
// - Apply same qualification logic
// - Execute on both exchanges
```

### 3. Test Scenarios

Test all scenarios with real cross-exchange execution:

1. Happy path (both legs fill)
2. One exchange times out
3. Emergency close on one exchange
4. Partial fills
5. High throughput
6. Exchange disconnection
7. Graceful shutdown

### 4. Validate Performance

- Latency should remain < 10ms
- Success rate should remain > 95%
- No new edge cases

### 5. Sign-Off Criteria

Before proceeding to Phase 3:

- ✅ All scenarios pass with dual exchanges
- ✅ Performance still meets targets
- ✅ No critical bugs
- ✅ Ready for live testing

## Phase 3: Live Testing (Small Positions)

### 1. Create Live Exchange Accounts

**Bybit Live**:
1. Go to https://www.bybit.com/
2. Create account and complete KYC
3. Generate API keys (same permissions as demo)
4. Fund account with $500-1000 USDT

**Second Exchange** (Bitget/OKX):
1. Create account and complete KYC
2. Generate API keys
3. Fund account with $500-1000 USDT

### 2. Update Configuration

```bash
# Use live API endpoints
BYBIT_API_KEY=your_live_key
BYBIT_API_SECRET=your_live_secret

BITGET_API_KEY=your_live_key
BITGET_API_SECRET=your_live_secret

# Start with small positions
ESTIMATED_POSITION_SIZE=10.0  # $10 per trade
MAX_CONCURRENT_TRADES=2

# Conservative spread
MINIMUM_SPREAD_BPS=20.0

# Enable all safety features
ENABLE_EMERGENCY_STOP=true
MAX_DAILY_LOSS=50.0  # $50 max loss per day
```

### 3. Safety Limits

**Critical Safety Features**:

1. **Position Size Limits**
   - Start: $10 per trade
   - Week 1: $10-20
   - Week 2: $20-50
   - Week 3+: Gradually increase

2. **Daily Loss Limits**
   - Start: $50 per day
   - Adjust based on success rate
   - Auto-stop if limit reached

3. **Emergency Stop Conditions**
   - 3 failures in 60 seconds
   - Success rate < 90%
   - Latency > 50ms
   - Exchange API errors

4. **Manual Monitoring**
   - Check system every 4 hours
   - Review all trades daily
   - Analyze failures weekly

### 4. Monitoring Setup

**Real-Time Monitoring**:

```bash
# Set up monitoring dashboard
# - Grafana for metrics
# - Prometheus for data collection
# - Alertmanager for alerts

# Key metrics to monitor:
# - Success rate (alert if < 95%)
# - Latency (alert if P99 > 20ms)
# - Position sizes (alert if > limit)
# - Daily P&L (alert if loss > limit)
# - Error rate (alert if > 5%)
```

**Alerts**:

1. **Critical** (immediate action):
   - System crash
   - Emergency stop triggered
   - Daily loss limit reached
   - Success rate < 90%

2. **Warning** (review within 1 hour):
   - Success rate < 95%
   - Latency > 15ms
   - Error rate > 3%
   - Unusual trade volume

3. **Info** (review daily):
   - Success rate 95-98%
   - Latency 10-15ms
   - Normal operation

### 5. Risk Management

**Position Sizing**:
- Never risk more than 1% of account per trade
- Start with 0.1% ($10 on $10,000 account)
- Increase gradually based on success

**Diversification**:
- Trade multiple symbols
- Use multiple exchanges
- Don't concentrate risk

**Stop Loss**:
- Daily loss limit
- Weekly loss limit
- Monthly loss limit

### 6. Performance Tracking

Track these metrics daily:

| Metric | Target | Actual | Status |
|--------|--------|--------|--------|
| Success Rate | > 95% | 96.5% | ✅ |
| Average Latency | < 10ms | 8.2ms | ✅ |
| Daily P&L | > $0 | +$12.50 | ✅ |
| Trades Executed | 20-50 | 35 | ✅ |
| Errors | < 5% | 2.1% | ✅ |

### 7. Sign-Off Criteria

Before proceeding to Phase 4:

- ✅ 2+ weeks of profitable operation
- ✅ Success rate consistently > 95%
- ✅ No critical bugs in live environment
- ✅ All safety features working
- ✅ Monitoring and alerts working
- ✅ Team comfortable with live operation

## Phase 4: Production Scaling

### 1. Gradual Position Size Increase

**Week 1-2**: $10 per trade
- Monitor closely
- Verify all systems working
- Build confidence

**Week 3-4**: $20-50 per trade
- Increase if success rate > 95%
- Monitor for any issues
- Adjust if needed

**Week 5-8**: $50-100 per trade
- Continue gradual increase
- Monitor performance
- Optimize as needed

**Week 9+**: $100-500 per trade
- Scale to target volume
- Maintain safety limits
- Continuous optimization

### 2. Optimization

**Latency Optimization**:
- Profile hot paths
- Optimize allocations
- Tune thread pinning
- Optimize network calls

**Success Rate Optimization**:
- Analyze failure patterns
- Adjust timeout values
- Optimize order prices
- Improve retry logic

**Profitability Optimization**:
- Adjust spread thresholds
- Optimize fee calculations
- Improve opportunity detection
- Reduce slippage

### 3. Monitoring and Maintenance

**Daily Tasks**:
- Review overnight trades
- Check error logs
- Verify balances
- Update metrics dashboard

**Weekly Tasks**:
- Analyze performance trends
- Review failure patterns
- Optimize parameters
- Update documentation

**Monthly Tasks**:
- Performance review
- Risk assessment
- System upgrades
- Team training

### 4. Incident Response

**Incident Levels**:

**Level 1 (Critical)**:
- System crash
- Large loss (> daily limit)
- Security breach
- **Response**: Stop immediately, investigate, fix

**Level 2 (High)**:
- Success rate < 90%
- Multiple failures
- Exchange API issues
- **Response**: Reduce position sizes, investigate

**Level 3 (Medium)**:
- Success rate 90-95%
- Latency increase
- Minor errors
- **Response**: Monitor closely, optimize

**Level 4 (Low)**:
- Success rate 95-98%
- Normal operation
- Minor issues
- **Response**: Log and review

## Environment Configuration

### Development
```bash
ENVIRONMENT=development
BYBIT_API_URL=https://api-demo.bybit.com
ENABLE_DEBUG_LOGGING=true
POSITION_SIZE=10.0
```

### Staging
```bash
ENVIRONMENT=staging
BYBIT_API_URL=https://api-demo.bybit.com
ENABLE_DEBUG_LOGGING=false
POSITION_SIZE=50.0
```

### Production
```bash
ENVIRONMENT=production
BYBIT_API_URL=https://api.bybit.com
ENABLE_DEBUG_LOGGING=false
POSITION_SIZE=100.0
ENABLE_ALL_SAFETY_FEATURES=true
```

## Metrics Interpretation

### Success Rate
- **> 98%**: Excellent, consider increasing position sizes
- **95-98%**: Good, maintain current settings
- **90-95%**: Acceptable, monitor closely
- **< 90%**: Poor, reduce position sizes or stop

### Latency
- **< 5ms**: Excellent
- **5-10ms**: Good
- **10-20ms**: Acceptable
- **> 20ms**: Poor, investigate

### Daily P&L
- **Positive**: Good, system is profitable
- **Break-even**: Acceptable, optimize
- **Negative**: Review trades, adjust parameters

## Migration Checklist

### Pre-Migration
- [ ] All validation tests pass
- [ ] Documentation complete
- [ ] Team trained
- [ ] Monitoring set up
- [ ] Alerts configured
- [ ] Safety limits defined
- [ ] Risk management plan approved

### Migration
- [ ] Create live exchange accounts
- [ ] Generate API keys
- [ ] Fund accounts
- [ ] Update configuration
- [ ] Deploy to production
- [ ] Verify connectivity
- [ ] Start with small positions

### Post-Migration
- [ ] Monitor first 24 hours closely
- [ ] Review all trades
- [ ] Verify metrics
- [ ] Adjust parameters if needed
- [ ] Document any issues
- [ ] Plan next steps

## Rollback Plan

If issues occur in production:

1. **Stop Trading**
   - Send Ctrl+C to system
   - Wait for graceful shutdown
   - Verify all positions closed

2. **Assess Damage**
   - Check account balances
   - Review error logs
   - Identify root cause

3. **Fix Issues**
   - Fix bugs if found
   - Adjust configuration
   - Test in demo environment

4. **Resume Trading**
   - Start with smaller positions
   - Monitor closely
   - Gradually increase

## Support and Escalation

### Level 1 Support (Team)
- Monitor system
- Handle routine issues
- Adjust parameters
- Review daily metrics

### Level 2 Support (Engineering)
- Investigate bugs
- Optimize performance
- Deploy fixes
- Update documentation

### Level 3 Support (Architecture)
- Major system changes
- Architecture decisions
- Risk assessment
- Strategic planning

## Conclusion

This deployment guide provides a safe, gradual path from demo testing to production operation. Follow each phase carefully, validate thoroughly, and never skip safety checks.

**Key Principles**:
1. Start small
2. Validate thoroughly
3. Monitor continuously
4. Scale gradually
5. Maintain safety limits

**Success Criteria**:
- Consistent profitability
- High success rate (> 95%)
- Low latency (< 10ms)
- Stable operation
- Effective risk management

Good luck with your deployment!
