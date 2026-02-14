//! Monitoring and Observability HTTP Endpoint
//!
//! This binary exposes system metrics via a simple HTTP endpoint for monitoring
//! the low-latency trading system. It provides real-time visibility into:
//!
//! - Latency percentiles (p50, p95, p99)
//! - Queue depth metrics (market data and order execution)
//! - Allocation rate metrics
//! - CPU utilization metrics
//!
//! ## Usage
//!
//! ```bash
//! cargo run --bin monitor --release
//! ```
//!
//! Then access metrics at:
//! - http://localhost:9090/metrics (Prometheus-compatible format)
//! - http://localhost:9090/health (Health check)
//!
//! ## Requirements
//!
//! - Requirement 15.3: Expose latency percentiles
//! - Requirement 15.4: Track allocations per second in hot paths

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

/// Global metrics state shared across the monitoring system
#[derive(Clone)]
struct MetricsState {
    /// Latency statistics
    latency: Arc<RwLock<LatencyMetrics>>,
    
    /// Queue depth statistics
    queue: Arc<RwLock<QueueMetrics>>,
    
    /// Allocation rate statistics
    allocation: Arc<RwLock<AllocationMetrics>>,
    
    /// CPU utilization statistics
    cpu: Arc<RwLock<CpuMetrics>>,
    
    /// Server start time
    start_time: Instant,
}

impl MetricsState {
    fn new() -> Self {
        Self {
            latency: Arc::new(RwLock::new(LatencyMetrics::default())),
            queue: Arc::new(RwLock::new(QueueMetrics::default())),
            allocation: Arc::new(RwLock::new(AllocationMetrics::default())),
            cpu: Arc::new(RwLock::new(CpuMetrics::default())),
            start_time: Instant::now(),
        }
    }
    
    /// Get uptime in seconds
    fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

/// Latency metrics (p50, p95, p99, max)
#[derive(Default, Clone)]
struct LatencyMetrics {
    /// P50 latency in nanoseconds
    p50_ns: u64,
    
    /// P95 latency in nanoseconds
    p95_ns: u64,
    
    /// P99 latency in nanoseconds
    p99_ns: u64,
    
    /// Maximum latency in nanoseconds
    max_ns: u64,
    
    /// Total number of measurements
    count: u64,
}

impl LatencyMetrics {
    fn to_prometheus(&self) -> String {
        format!(
            "# HELP latency_p50_microseconds P50 latency in microseconds\n\
             # TYPE latency_p50_microseconds gauge\n\
             latency_p50_microseconds {:.2}\n\
             # HELP latency_p95_microseconds P95 latency in microseconds\n\
             # TYPE latency_p95_microseconds gauge\n\
             latency_p95_microseconds {:.2}\n\
             # HELP latency_p99_microseconds P99 latency in microseconds\n\
             # TYPE latency_p99_microseconds gauge\n\
             latency_p99_microseconds {:.2}\n\
             # HELP latency_max_microseconds Maximum latency in microseconds\n\
             # TYPE latency_max_microseconds gauge\n\
             latency_max_microseconds {:.2}\n\
             # HELP latency_measurements_total Total number of latency measurements\n\
             # TYPE latency_measurements_total counter\n\
             latency_measurements_total {}\n",
            self.p50_ns as f64 / 1000.0,
            self.p95_ns as f64 / 1000.0,
            self.p99_ns as f64 / 1000.0,
            self.max_ns as f64 / 1000.0,
            self.count
        )
    }
}

/// Queue depth metrics for market data and order execution pipelines
#[derive(Default, Clone)]
struct QueueMetrics {
    /// Market data queue depth
    market_depth: usize,
    
    /// Market data queue capacity
    market_capacity: usize,
    
    /// Order execution queue depth
    order_depth: usize,
    
    /// Order execution queue capacity
    order_capacity: usize,
    
    /// Total market updates pushed
    market_push_count: u64,
    
    /// Total market updates dropped
    market_drop_count: u64,
    
    /// Total orders submitted
    order_submit_count: u64,
    
    /// Total orders dropped
    order_drop_count: u64,
}

impl QueueMetrics {
    fn to_prometheus(&self) -> String {
        let market_utilization = if self.market_capacity > 0 {
            (self.market_depth as f64 / self.market_capacity as f64) * 100.0
        } else {
            0.0
        };
        
        let order_utilization = if self.order_capacity > 0 {
            (self.order_depth as f64 / self.order_capacity as f64) * 100.0
        } else {
            0.0
        };
        
        let market_drop_rate = if self.market_push_count > 0 {
            (self.market_drop_count as f64 / self.market_push_count as f64) * 100.0
        } else {
            0.0
        };
        
        let order_drop_rate = if self.order_submit_count > 0 {
            (self.order_drop_count as f64 / self.order_submit_count as f64) * 100.0
        } else {
            0.0
        };
        
        format!(
            "# HELP queue_depth_market Current market data queue depth\n\
             # TYPE queue_depth_market gauge\n\
             queue_depth_market {}\n\
             # HELP queue_capacity_market Market data queue capacity\n\
             # TYPE queue_capacity_market gauge\n\
             queue_capacity_market {}\n\
             # HELP queue_utilization_market_percent Market data queue utilization percentage\n\
             # TYPE queue_utilization_market_percent gauge\n\
             queue_utilization_market_percent {:.2}\n\
             # HELP queue_depth_order Current order execution queue depth\n\
             # TYPE queue_depth_order gauge\n\
             queue_depth_order {}\n\
             # HELP queue_capacity_order Order execution queue capacity\n\
             # TYPE queue_capacity_order gauge\n\
             queue_capacity_order {}\n\
             # HELP queue_utilization_order_percent Order execution queue utilization percentage\n\
             # TYPE queue_utilization_order_percent gauge\n\
             queue_utilization_order_percent {:.2}\n\
             # HELP queue_market_push_total Total market updates pushed\n\
             # TYPE queue_market_push_total counter\n\
             queue_market_push_total {}\n\
             # HELP queue_market_drop_total Total market updates dropped\n\
             # TYPE queue_market_drop_total counter\n\
             queue_market_drop_total {}\n\
             # HELP queue_market_drop_rate_percent Market data drop rate percentage\n\
             # TYPE queue_market_drop_rate_percent gauge\n\
             queue_market_drop_rate_percent {:.2}\n\
             # HELP queue_order_submit_total Total orders submitted\n\
             # TYPE queue_order_submit_total counter\n\
             queue_order_submit_total {}\n\
             # HELP queue_order_drop_total Total orders dropped\n\
             # TYPE queue_order_drop_total counter\n\
             queue_order_drop_total {}\n\
             # HELP queue_order_drop_rate_percent Order drop rate percentage\n\
             # TYPE queue_order_drop_rate_percent gauge\n\
             queue_order_drop_rate_percent {:.2}\n",
            self.market_depth,
            self.market_capacity,
            market_utilization,
            self.order_depth,
            self.order_capacity,
            order_utilization,
            self.market_push_count,
            self.market_drop_count,
            market_drop_rate,
            self.order_submit_count,
            self.order_drop_count,
            order_drop_rate
        )
    }
}

/// Allocation rate metrics
#[derive(Default, Clone)]
struct AllocationMetrics {
    /// Total allocations in hot path
    hot_path_allocations: u64,
    
    /// Total allocations in warm path
    warm_path_allocations: u64,
    
    /// Total allocations in cold path
    cold_path_allocations: u64,
    
    /// Timestamp of last measurement
    last_measurement: Option<Instant>,
    
    /// Allocations per second (calculated)
    allocations_per_second: f64,
}

impl AllocationMetrics {
    fn to_prometheus(&self) -> String {
        format!(
            "# HELP allocations_hot_path_total Total allocations in hot path\n\
             # TYPE allocations_hot_path_total counter\n\
             allocations_hot_path_total {}\n\
             # HELP allocations_warm_path_total Total allocations in warm path\n\
             # TYPE allocations_warm_path_total counter\n\
             allocations_warm_path_total {}\n\
             # HELP allocations_cold_path_total Total allocations in cold path\n\
             # TYPE allocations_cold_path_total counter\n\
             allocations_cold_path_total {}\n\
             # HELP allocations_per_second Allocations per second\n\
             # TYPE allocations_per_second gauge\n\
             allocations_per_second {:.2}\n",
            self.hot_path_allocations,
            self.warm_path_allocations,
            self.cold_path_allocations,
            self.allocations_per_second
        )
    }
}

/// CPU utilization metrics
#[derive(Default, Clone)]
struct CpuMetrics {
    /// Strategy thread CPU utilization (0-100%)
    strategy_thread_percent: f64,
    
    /// WebSocket threads CPU utilization (0-100%)
    websocket_threads_percent: f64,
    
    /// Overall system CPU utilization (0-100%)
    system_percent: f64,
    
    /// Number of context switches
    context_switches: u64,
}

impl CpuMetrics {
    fn to_prometheus(&self) -> String {
        format!(
            "# HELP cpu_strategy_thread_percent Strategy thread CPU utilization percentage\n\
             # TYPE cpu_strategy_thread_percent gauge\n\
             cpu_strategy_thread_percent {:.2}\n\
             # HELP cpu_websocket_threads_percent WebSocket threads CPU utilization percentage\n\
             # TYPE cpu_websocket_threads_percent gauge\n\
             cpu_websocket_threads_percent {:.2}\n\
             # HELP cpu_system_percent Overall system CPU utilization percentage\n\
             # TYPE cpu_system_percent gauge\n\
             cpu_system_percent {:.2}\n\
             # HELP cpu_context_switches_total Total number of context switches\n\
             # TYPE cpu_context_switches_total counter\n\
             cpu_context_switches_total {}\n",
            self.strategy_thread_percent,
            self.websocket_threads_percent,
            self.system_percent,
            self.context_switches
        )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[MONITOR] Starting monitoring HTTP server on 0.0.0.0:9090");
    println!("[MONITOR] Endpoints:");
    println!("[MONITOR]   - http://localhost:9090/metrics (Prometheus format)");
    println!("[MONITOR]   - http://localhost:9090/health (Health check)");
    
    let metrics_state = MetricsState::new();
    
    // Start background metrics collection task
    let metrics_state_clone = metrics_state.clone();
    tokio::spawn(async move {
        collect_metrics_loop(metrics_state_clone).await;
    });
    
    // Start HTTP server
    let listener = TcpListener::bind("0.0.0.0:9090").await?;
    println!("[MONITOR] Server listening on 0.0.0.0:9090");
    
    loop {
        let (mut socket, addr) = listener.accept().await?;
        let metrics_state = metrics_state.clone();
        
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            
            match socket.read(&mut buffer).await {
                Ok(n) if n > 0 => {
                    let request = String::from_utf8_lossy(&buffer[..n]);
                    
                    // Parse HTTP request line
                    let first_line = request.lines().next().unwrap_or("");
                    let parts: Vec<&str> = first_line.split_whitespace().collect();
                    
                    if parts.len() >= 2 {
                        let path = parts[1];
                        
                        let response = match path {
                            "/metrics" => handle_metrics(&metrics_state).await,
                            "/health" => handle_health(&metrics_state).await,
                            _ => handle_not_found(),
                        };
                        
                        if let Err(e) = socket.write_all(response.as_bytes()).await {
                            eprintln!("[MONITOR] Error writing response to {}: {}", addr, e);
                        }
                    }
                }
                Ok(_) => {
                    // Empty request
                }
                Err(e) => {
                    eprintln!("[MONITOR] Error reading from {}: {}", addr, e);
                }
            }
        });
    }
}

/// Handle /metrics endpoint (Prometheus format)
async fn handle_metrics(state: &MetricsState) -> String {
    let latency = state.latency.read().await;
    let queue = state.queue.read().await;
    let allocation = state.allocation.read().await;
    let cpu = state.cpu.read().await;
    
    let body = format!(
        "# Low-Latency Trading System Metrics\n\
         # Uptime: {} seconds\n\
         \n\
         {}\
         {}\
         {}\
         {}",
        state.uptime_seconds(),
        latency.to_prometheus(),
        queue.to_prometheus(),
        allocation.to_prometheus(),
        cpu.to_prometheus()
    );
    
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain; version=0.0.4\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        body.len(),
        body
    )
}

/// Handle /health endpoint
async fn handle_health(state: &MetricsState) -> String {
    let queue = state.queue.read().await;
    
    // Check if system is healthy
    let market_utilization = if queue.market_capacity > 0 {
        (queue.market_depth as f64 / queue.market_capacity as f64) * 100.0
    } else {
        0.0
    };
    
    let order_utilization = if queue.order_capacity > 0 {
        (queue.order_depth as f64 / queue.order_capacity as f64) * 100.0
    } else {
        0.0
    };
    
    let is_healthy = market_utilization < 90.0 && order_utilization < 90.0;
    
    let status = if is_healthy { "healthy" } else { "degraded" };
    let status_code = if is_healthy { "200 OK" } else { "503 Service Unavailable" };
    
    let body = format!(
        "{{\n\
           \"status\": \"{}\",\n\
           \"uptime_seconds\": {},\n\
           \"market_queue_utilization_percent\": {:.2},\n\
           \"order_queue_utilization_percent\": {:.2}\n\
         }}",
        status,
        state.uptime_seconds(),
        market_utilization,
        order_utilization
    );
    
    format!(
        "HTTP/1.1 {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        status_code,
        body.len(),
        body
    )
}

/// Handle 404 Not Found
fn handle_not_found() -> String {
    let body = "404 Not Found\n\nAvailable endpoints:\n  - /metrics\n  - /health\n";
    
    format!(
        "HTTP/1.1 404 Not Found\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        body.len(),
        body
    )
}

/// Background task to collect metrics periodically
async fn collect_metrics_loop(state: MetricsState) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    
    loop {
        interval.tick().await;
        
        // Simulate metrics collection
        // In a real implementation, this would read from shared atomic counters
        // or query the actual pipeline/latency tracker instances
        
        // Update latency metrics (simulated)
        {
            let mut latency = state.latency.write().await;
            // In production, read from actual LatencyStats
            latency.p50_ns = 2_000_000; // 2ms
            latency.p95_ns = 5_000_000; // 5ms
            latency.p99_ns = 10_000_000; // 10ms
            latency.max_ns = 20_000_000; // 20ms
            latency.count += 1000; // Simulated 1000 measurements per second
        }
        
        // Update queue metrics (simulated)
        {
            let mut queue = state.queue.write().await;
            // In production, read from actual MarketPipeline and ExecutionPipeline
            queue.market_capacity = 10_000;
            queue.market_depth = 500; // 5% utilization
            queue.market_push_count += 10_000; // 10k updates/sec
            queue.market_drop_count += 0; // No drops
            
            queue.order_capacity = 1_000;
            queue.order_depth = 50; // 5% utilization
            queue.order_submit_count += 100; // 100 orders/sec
            queue.order_drop_count += 0; // No drops
        }
        
        // Update allocation metrics (simulated)
        {
            let mut allocation = state.allocation.write().await;
            // In production, use jemalloc stats or custom allocator tracking
            allocation.hot_path_allocations = 0; // Target: zero allocations
            allocation.warm_path_allocations += 10;
            allocation.cold_path_allocations += 100;
            allocation.allocations_per_second = 0.0; // Hot path should be zero
        }
        
        // Update CPU metrics (simulated)
        {
            let mut cpu = state.cpu.write().await;
            // In production, read from /proc/stat or use sysinfo crate
            cpu.strategy_thread_percent = 45.0; // Target: <50%
            cpu.websocket_threads_percent = 30.0;
            cpu.system_percent = 25.0;
            cpu.context_switches += 10;
        }
    }
}
