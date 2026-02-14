# Monitoring and Observability

This document describes the monitoring and observability infrastructure for the low-latency trading system.

## Overview

The monitoring system exposes real-time metrics via HTTP endpoints in Prometheus-compatible format. This enables integration with standard monitoring tools like Prometheus, Grafana, and alerting systems.

## Requirements

- **Requirement 15.3**: Expose latency percentiles (p50, p95, p99)
- **Requirement 15.4**: Track allocations per second in hot paths

## Running the Monitor

Start the monitoring server:

```bash
cargo run --bin monitor --release
```

The server listens on `0.0.0.0:9090` by default.

## Endpoints

### `/metrics` - Prometheus Metrics

Returns all system metrics in Prometheus text format.

**Example:**
```bash
curl http://localhost:9090/metrics
```

**Response:**
```
# Low-Latency Trading System Metrics
# Uptime: 3600 seconds

# HELP latency_p50_microseconds P50 latency in microseconds
# TYPE latency_p50_microseconds gauge
latency_p50_microseconds 2000.00
# HELP latency_p95_microseconds P95 latency in microseconds
# TYPE latency_p95_microseconds gauge
latency_p95_microseconds 5000.00
# HELP latency_p99_microseconds P99 latency in microseconds
# TYPE latency_p99_microseconds gauge
latency_p99_microseconds 10000.00
...
```

### `/health` - Health Check

Returns system health status in JSON format.

**Example:**
```bash
curl http://localhost:9090/health
```

**Response:**
```json
{
  "status": "healthy",
  "uptime_seconds": 3600,
  "market_queue_utilization_percent": 5.00,
  "order_queue_utilization_percent": 5.00
}
```

**Status Codes:**
- `200 OK` - System is healthy (queue utilization < 90%)
- `503 Service Unavailable` - System is degraded (queue utilization >= 90%)

## Metrics Categories

### 1. Latency Metrics

Tracks end-to-end latency from market data ingestion to trade execution.

| Metric | Type | Description |
|--------|------|-------------|
| `latency_p50_microseconds` | gauge | P50 (median) latency in microseconds |
| `latency_p95_microseconds` | gauge | P95 latency in microseconds |
| `latency_p99_microseconds` | gauge | P99 latency in microseconds |
| `latency_max_microseconds` | gauge | Maximum latency in microseconds |
| `latency_measurements_total` | counter | Total number of latency measurements |

**Target Values:**
- P50: < 2,000 µs (2 ms)
- P95: < 5,000 µs (5 ms)
- P99: < 10,000 µs (10 ms)

### 2. Queue Depth Metrics

Monitors the lock-free SPSC queues for market data and order execution.

| Metric | Type | Description |
|--------|------|-------------|
| `queue_depth_market` | gauge | Current market data queue depth |
| `queue_capacity_market` | gauge | Market data queue capacity (10,000) |
| `queue_utilization_market_percent` | gauge | Market queue utilization percentage |
| `queue_depth_order` | gauge | Current order execution queue depth |
| `queue_capacity_order` | gauge | Order execution queue capacity (1,000) |
| `queue_utilization_order_percent` | gauge | Order queue utilization percentage |
| `queue_market_push_total` | counter | Total market updates pushed |
| `queue_market_drop_total` | counter | Total market updates dropped (backpressure) |
| `queue_market_drop_rate_percent` | gauge | Market data drop rate percentage |
| `queue_order_submit_total` | counter | Total orders submitted |
| `queue_order_drop_total` | counter | Total orders dropped (backpressure) |
| `queue_order_drop_rate_percent` | gauge | Order drop rate percentage |

**Target Values:**
- Queue utilization: < 80% (normal), < 90% (warning), >= 90% (critical)
- Drop rate: < 1% (normal), < 5% (warning), >= 5% (critical)

### 3. Allocation Rate Metrics

Tracks memory allocations in different code paths.

| Metric | Type | Description |
|--------|------|-------------|
| `allocations_hot_path_total` | counter | Total allocations in hot path (should be 0) |
| `allocations_warm_path_total` | counter | Total allocations in warm path |
| `allocations_cold_path_total` | counter | Total allocations in cold path |
| `allocations_per_second` | gauge | Allocations per second in hot path |

**Target Values:**
- Hot path allocations: 0 (zero allocations required)
- Allocations per second: 0 (hot path should never allocate)

### 4. CPU Utilization Metrics

Monitors CPU usage across different thread types.

| Metric | Type | Description |
|--------|------|-------------|
| `cpu_strategy_thread_percent` | gauge | Strategy thread CPU utilization (0-100%) |
| `cpu_websocket_threads_percent` | gauge | WebSocket threads CPU utilization (0-100%) |
| `cpu_system_percent` | gauge | Overall system CPU utilization (0-100%) |
| `cpu_context_switches_total` | counter | Total number of context switches |

**Target Values:**
- Strategy thread: < 50% (target from requirements)
- WebSocket threads: < 60%
- Context switches: Minimize (thread pinning should reduce this)

## Integration with Prometheus

Add the following to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'trading-system'
    scrape_interval: 1s  # High-frequency scraping for low-latency monitoring
    static_configs:
      - targets: ['localhost:9090']
```

## Grafana Dashboard

Example Grafana queries:

### Latency Panel
```promql
# P99 latency over time
latency_p99_microseconds / 1000  # Convert to milliseconds

# Latency percentiles comparison
latency_p50_microseconds / 1000
latency_p95_microseconds / 1000
latency_p99_microseconds / 1000
```

### Queue Utilization Panel
```promql
# Market queue utilization
queue_utilization_market_percent

# Order queue utilization
queue_utilization_order_percent
```

### Drop Rate Panel
```promql
# Market data drop rate
queue_market_drop_rate_percent

# Order drop rate
queue_order_drop_rate_percent
```

### Allocation Rate Panel
```promql
# Hot path allocations (should be 0)
allocations_per_second

# Total allocations by path
allocations_hot_path_total
allocations_warm_path_total
allocations_cold_path_total
```

## Alerting Rules

Example Prometheus alerting rules:

```yaml
groups:
  - name: trading_system
    interval: 1s
    rules:
      # Latency alerts
      - alert: HighP99Latency
        expr: latency_p99_microseconds > 10000
        for: 10s
        labels:
          severity: warning
        annotations:
          summary: "P99 latency exceeds 10ms"
          description: "P99 latency is {{ $value }}µs (target: <10,000µs)"
      
      # Queue utilization alerts
      - alert: HighQueueUtilization
        expr: queue_utilization_market_percent > 80 or queue_utilization_order_percent > 80
        for: 5s
        labels:
          severity: warning
        annotations:
          summary: "Queue utilization exceeds 80%"
          description: "Queue utilization is high, potential backpressure"
      
      - alert: CriticalQueueUtilization
        expr: queue_utilization_market_percent > 90 or queue_utilization_order_percent > 90
        for: 5s
        labels:
          severity: critical
        annotations:
          summary: "Queue utilization exceeds 90%"
          description: "Queue utilization is critical, system degraded"
      
      # Drop rate alerts
      - alert: HighDropRate
        expr: queue_market_drop_rate_percent > 1 or queue_order_drop_rate_percent > 1
        for: 10s
        labels:
          severity: warning
        annotations:
          summary: "Drop rate exceeds 1%"
          description: "System is dropping data due to backpressure"
      
      # Allocation alerts
      - alert: HotPathAllocations
        expr: allocations_per_second > 0
        for: 10s
        labels:
          severity: critical
        annotations:
          summary: "Hot path is allocating memory"
          description: "Hot path should have zero allocations (found {{ $value }}/sec)"
      
      # CPU alerts
      - alert: HighCPUUtilization
        expr: cpu_strategy_thread_percent > 50
        for: 30s
        labels:
          severity: warning
        annotations:
          summary: "Strategy thread CPU exceeds 50%"
          description: "Strategy thread CPU is {{ $value }}% (target: <50%)"
```

## Implementation Notes

### Current Implementation

The current implementation uses **simulated metrics** for demonstration purposes. In production, the metrics should be collected from:

1. **Latency Metrics**: Read from `LatencyStats` instances in `src/strategy/latency_tracker.rs`
2. **Queue Metrics**: Read from `MarketPipeline` and `ExecutionPipeline` in `src/strategy/pipeline.rs`
3. **Allocation Metrics**: Use jemalloc stats or custom allocator tracking
4. **CPU Metrics**: Read from `/proc/stat` (Linux) or use the `sysinfo` crate

### Integration Points

To integrate real metrics, modify `collect_metrics_loop()` in `src/bin/monitor.rs`:

```rust
// Example: Read from actual pipeline
let market_pipeline = /* get reference to MarketPipeline */;
let metrics = market_pipeline.metrics();

queue.market_depth = metrics.queue_depth;
queue.market_capacity = metrics.queue_capacity;
queue.market_push_count = metrics.push_count;
queue.market_drop_count = metrics.drop_count;
```

### Performance Considerations

- Metrics collection runs every 1 second (configurable)
- HTTP server uses async I/O (tokio) for minimal overhead
- Metrics are stored in atomic counters (lock-free)
- No allocations in metrics collection hot path

## Testing

Run the monitoring endpoint tests:

```bash
# Test metric format validation
cargo test --test monitor_endpoint_test test_metrics_format_validation

# Test endpoints (requires monitor server running)
cargo run --bin monitor --release &
cargo test --test monitor_endpoint_test
```

## Troubleshooting

### Port Already in Use

If port 9090 is already in use, modify the bind address in `src/bin/monitor.rs`:

```rust
let listener = TcpListener::bind("0.0.0.0:9091").await?;
```

### Metrics Not Updating

1. Verify the monitor server is running: `curl http://localhost:9090/health`
2. Check that metrics collection loop is running (should see periodic updates)
3. Verify integration with actual pipeline instances

### High Memory Usage

The monitoring server uses minimal memory (~1MB). If memory usage is high:
1. Check for memory leaks in metrics collection
2. Verify metrics are not accumulating unbounded data
3. Use `valgrind` or `heaptrack` to profile memory usage

## Future Enhancements

1. **Histogram Metrics**: Add histogram support for more accurate percentile calculations
2. **Custom Metrics**: Allow applications to register custom metrics
3. **Push Gateway**: Support pushing metrics to Prometheus Push Gateway
4. **OpenTelemetry**: Add OpenTelemetry support for distributed tracing
5. **Real-time Dashboards**: Built-in web dashboard for real-time monitoring
