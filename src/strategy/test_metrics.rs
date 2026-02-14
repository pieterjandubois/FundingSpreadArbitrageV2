//! Test Metrics Collector
//!
//! This module tracks performance metrics and validates requirements for the
//! synthetic test mode. It measures latency at each stage of the pipeline,
//! tracks execution success rates, and reports comprehensive statistics.
//!
//! ## Metrics Tracked
//!
//! - **Latency Metrics**: WebSocket→Queue, Queue→Strategy, Opportunity Detection, Order Placement
//! - **Execution Metrics**: Opportunities generated, trades executed, success/failure rates
//! - **Edge Case Metrics**: Partial fills, cancellations, timeouts, emergency closes
//!
//! ## Performance Targets (from requirements)
//!
//! - WebSocket → Queue: <0.5ms (P99)
//! - Queue → Strategy: <0.1ms (P99)
//! - Opportunity Detection: <2ms (P99)
//! - Order Placement: <5ms (P99)
//! - End-to-End: <10ms (P99)
//!
//! Requirements: 5.1-5.5, 9.1-9.5 (Logging and performance validation)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Test Metrics Collector
///
/// Thread-safe metrics collector for tracking performance and execution statistics
/// during synthetic test mode operation.
#[derive(Clone)]
pub struct TestMetricsCollector {
    // Execution counters (atomic for lock-free updates)
    opportunities_generated: Arc<AtomicU64>,
    trades_executed: Arc<AtomicU64>,
    trades_successful: Arc<AtomicU64>,
    trades_failed: Arc<AtomicU64>,
    
    // Edge case counters
    partial_fills: Arc<AtomicU64>,
    cancellations: Arc<AtomicU64>,
    timeouts: Arc<AtomicU64>,
    emergency_closes: Arc<AtomicU64>,
    
    // Latency tracking (mutex-protected vectors for percentile calculation)
    websocket_to_queue_latencies: Arc<Mutex<Vec<Duration>>>,
    queue_to_strategy_latencies: Arc<Mutex<Vec<Duration>>>,
    opportunity_detection_latencies: Arc<Mutex<Vec<Duration>>>,
    order_placement_latencies: Arc<Mutex<Vec<Duration>>>,
    end_to_end_latencies: Arc<Mutex<Vec<Duration>>>,
}

impl TestMetricsCollector {
    /// Create a new test metrics collector
    pub fn new() -> Self {
        Self {
            opportunities_generated: Arc::new(AtomicU64::new(0)),
            trades_executed: Arc::new(AtomicU64::new(0)),
            trades_successful: Arc::new(AtomicU64::new(0)),
            trades_failed: Arc::new(AtomicU64::new(0)),
            partial_fills: Arc::new(AtomicU64::new(0)),
            cancellations: Arc::new(AtomicU64::new(0)),
            timeouts: Arc::new(AtomicU64::new(0)),
            emergency_closes: Arc::new(AtomicU64::new(0)),
            websocket_to_queue_latencies: Arc::new(Mutex::new(Vec::new())),
            queue_to_strategy_latencies: Arc::new(Mutex::new(Vec::new())),
            opportunity_detection_latencies: Arc::new(Mutex::new(Vec::new())),
            order_placement_latencies: Arc::new(Mutex::new(Vec::new())),
            end_to_end_latencies: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    // === Execution Tracking ===
    
    /// Record an opportunity generated
    ///
    /// Requirement 5.1: Log opportunity generation
    pub fn record_opportunity(&self) {
        self.opportunities_generated.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a trade execution attempt
    pub fn record_trade_executed(&self) {
        self.trades_executed.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a successful trade
    ///
    /// Requirement 5.3: Log successful fills
    pub fn record_success(&self) {
        self.trades_successful.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a failed trade
    ///
    /// Requirement 5.3: Log failures
    pub fn record_failure(&self) {
        self.trades_failed.fetch_add(1, Ordering::Relaxed);
    }
    
    // === Edge Case Tracking ===
    
    /// Record a partial fill
    pub fn record_partial_fill(&self) {
        self.partial_fills.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a cancellation
    pub fn record_cancellation(&self) {
        self.cancellations.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a timeout
    pub fn record_timeout(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record an emergency close
    pub fn record_emergency_close(&self) {
        self.emergency_closes.fetch_add(1, Ordering::Relaxed);
    }
    
    // === Latency Tracking ===
    
    /// Record WebSocket → Queue latency
    ///
    /// Requirement 9.1: Measure WebSocket → Queue latency (target: <0.5ms)
    pub fn record_websocket_latency(&self, latency: Duration) {
        if let Ok(mut latencies) = self.websocket_to_queue_latencies.lock() {
            latencies.push(latency);
        }
    }
    
    /// Record Queue → Strategy latency
    ///
    /// Requirement 9.2: Measure Queue → Strategy latency (target: <0.1ms)
    pub fn record_queue_latency(&self, latency: Duration) {
        if let Ok(mut latencies) = self.queue_to_strategy_latencies.lock() {
            latencies.push(latency);
        }
    }
    
    /// Record opportunity detection latency
    ///
    /// Requirement 9.3: Measure opportunity detection latency (target: <2ms)
    pub fn record_opportunity_detection_latency(&self, latency: Duration) {
        if let Ok(mut latencies) = self.opportunity_detection_latencies.lock() {
            latencies.push(latency);
        }
    }
    
    /// Record order placement latency
    ///
    /// Requirement 9.4: Measure order placement latency (target: <5ms)
    pub fn record_order_placement_latency(&self, latency: Duration) {
        if let Ok(mut latencies) = self.order_placement_latencies.lock() {
            latencies.push(latency);
        }
    }
    
    /// Record end-to-end latency
    pub fn record_end_to_end_latency(&self, latency: Duration) {
        if let Ok(mut latencies) = self.end_to_end_latencies.lock() {
            latencies.push(latency);
        }
    }
    
    // === Statistics Calculation ===
    
    /// Calculate success rate
    pub fn calculate_success_rate(&self) -> f64 {
        let total = self.trades_executed.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let successful = self.trades_successful.load(Ordering::Relaxed);
        (successful as f64 / total as f64) * 100.0
    }
    
    /// Calculate percentile from a sorted vector
    fn calculate_percentile(sorted_values: &[Duration], percentile: f64) -> Option<Duration> {
        if sorted_values.is_empty() {
            return None;
        }
        let index = ((percentile / 100.0) * sorted_values.len() as f64) as usize;
        let index = index.min(sorted_values.len() - 1);
        Some(sorted_values[index])
    }
    
    /// Calculate P50, P95, P99 for a latency vector
    fn calculate_latency_percentiles(latencies: &[Duration]) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        if latencies.is_empty() {
            return (None, None, None);
        }
        
        let mut sorted = latencies.to_vec();
        sorted.sort();
        
        let p50 = Self::calculate_percentile(&sorted, 50.0);
        let p95 = Self::calculate_percentile(&sorted, 95.0);
        let p99 = Self::calculate_percentile(&sorted, 99.0);
        
        (p50, p95, p99)
    }
    
    /// Get WebSocket → Queue latency percentiles
    pub fn get_websocket_latency_percentiles(&self) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        if let Ok(latencies) = self.websocket_to_queue_latencies.lock() {
            Self::calculate_latency_percentiles(&latencies)
        } else {
            (None, None, None)
        }
    }
    
    /// Get Queue → Strategy latency percentiles
    pub fn get_queue_latency_percentiles(&self) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        if let Ok(latencies) = self.queue_to_strategy_latencies.lock() {
            Self::calculate_latency_percentiles(&latencies)
        } else {
            (None, None, None)
        }
    }
    
    /// Get opportunity detection latency percentiles
    pub fn get_opportunity_detection_percentiles(&self) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        if let Ok(latencies) = self.opportunity_detection_latencies.lock() {
            Self::calculate_latency_percentiles(&latencies)
        } else {
            (None, None, None)
        }
    }
    
    /// Get order placement latency percentiles
    pub fn get_order_placement_percentiles(&self) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        if let Ok(latencies) = self.order_placement_latencies.lock() {
            Self::calculate_latency_percentiles(&latencies)
        } else {
            (None, None, None)
        }
    }
    
    /// Get end-to-end latency percentiles
    pub fn get_end_to_end_percentiles(&self) -> (Option<Duration>, Option<Duration>, Option<Duration>) {
        if let Ok(latencies) = self.end_to_end_latencies.lock() {
            Self::calculate_latency_percentiles(&latencies)
        } else {
            (None, None, None)
        }
    }
    
    // === Reporting ===
    
    /// Report comprehensive metrics summary
    ///
    /// Requirements: 5.5, 9.5 (Metrics reporting)
    pub fn report_summary(&self) {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║          SYNTHETIC TEST MODE - METRICS SUMMARY                 ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        
        // Execution metrics
        println!("\n📊 EXECUTION METRICS:");
        println!("  Opportunities Generated:  {}", self.opportunities_generated.load(Ordering::Relaxed));
        println!("  Trades Executed:          {}", self.trades_executed.load(Ordering::Relaxed));
        println!("  Trades Successful:        {}", self.trades_successful.load(Ordering::Relaxed));
        println!("  Trades Failed:            {}", self.trades_failed.load(Ordering::Relaxed));
        println!("  Success Rate:             {:.2}%", self.calculate_success_rate());
        
        // Edge case metrics
        println!("\n⚠️  EDGE CASE METRICS:");
        println!("  Partial Fills:            {}", self.partial_fills.load(Ordering::Relaxed));
        println!("  Cancellations:            {}", self.cancellations.load(Ordering::Relaxed));
        println!("  Timeouts:                 {}", self.timeouts.load(Ordering::Relaxed));
        println!("  Emergency Closes:         {}", self.emergency_closes.load(Ordering::Relaxed));
        
        // Latency metrics
        println!("\n⚡ LATENCY METRICS:");
        
        let (ws_p50, ws_p95, ws_p99) = self.get_websocket_latency_percentiles();
        println!("  WebSocket → Queue:");
        println!("    P50: {:?} | P95: {:?} | P99: {:?} (target: <0.5ms)", 
            ws_p50.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            ws_p95.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            ws_p99.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()));
        
        let (q_p50, q_p95, q_p99) = self.get_queue_latency_percentiles();
        println!("  Queue → Strategy:");
        println!("    P50: {:?} | P95: {:?} | P99: {:?} (target: <0.1ms)", 
            q_p50.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            q_p95.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            q_p99.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()));
        
        let (opp_p50, opp_p95, opp_p99) = self.get_opportunity_detection_percentiles();
        println!("  Opportunity Detection:");
        println!("    P50: {:?} | P95: {:?} | P99: {:?} (target: <2ms)", 
            opp_p50.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            opp_p95.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            opp_p99.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()));
        
        let (ord_p50, ord_p95, ord_p99) = self.get_order_placement_percentiles();
        println!("  Order Placement:");
        println!("    P50: {:?} | P95: {:?} | P99: {:?} (target: <5ms)", 
            ord_p50.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            ord_p95.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            ord_p99.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()));
        
        let (e2e_p50, e2e_p95, e2e_p99) = self.get_end_to_end_percentiles();
        println!("  End-to-End:");
        println!("    P50: {:?} | P95: {:?} | P99: {:?} (target: <10ms)", 
            e2e_p50.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            e2e_p95.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()),
            e2e_p99.map(|d| format!("{:.3}ms", d.as_secs_f64() * 1000.0)).unwrap_or("N/A".to_string()));
        
        println!("\n╚════════════════════════════════════════════════════════════════╝\n");
    }
    
    /// Report periodic summary (called every 60 seconds)
    ///
    /// Requirement 5.5: Periodic reporting
    pub fn report_periodic(&self) {
        let opportunities = self.opportunities_generated.load(Ordering::Relaxed);
        let executed = self.trades_executed.load(Ordering::Relaxed);
        let success_rate = self.calculate_success_rate();
        
        println!("[METRICS] Opportunities: {} | Executed: {} | Success Rate: {:.1}%", 
            opportunities, executed, success_rate);
    }
}

impl Default for TestMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    
    #[test]
    fn test_metrics_collector_creation() {
        let metrics = TestMetricsCollector::new();
        assert_eq!(metrics.opportunities_generated.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.trades_executed.load(Ordering::Relaxed), 0);
    }
    
    #[test]
    fn test_record_opportunity() {
        let metrics = TestMetricsCollector::new();
        metrics.record_opportunity();
        metrics.record_opportunity();
        assert_eq!(metrics.opportunities_generated.load(Ordering::Relaxed), 2);
    }
    
    #[test]
    fn test_success_rate_calculation() {
        let metrics = TestMetricsCollector::new();
        
        // No trades yet
        assert_eq!(metrics.calculate_success_rate(), 0.0);
        
        // 3 successful out of 5
        metrics.record_trade_executed();
        metrics.record_success();
        metrics.record_trade_executed();
        metrics.record_success();
        metrics.record_trade_executed();
        metrics.record_success();
        metrics.record_trade_executed();
        metrics.record_failure();
        metrics.record_trade_executed();
        metrics.record_failure();
        
        assert_eq!(metrics.calculate_success_rate(), 60.0);
    }
    
    #[test]
    fn test_latency_tracking() {
        let metrics = TestMetricsCollector::new();
        
        // Record some latencies
        metrics.record_websocket_latency(Duration::from_micros(100));
        metrics.record_websocket_latency(Duration::from_micros(200));
        metrics.record_websocket_latency(Duration::from_micros(300));
        
        let (p50, p95, p99) = metrics.get_websocket_latency_percentiles();
        
        assert!(p50.is_some());
        assert!(p95.is_some());
        assert!(p99.is_some());
    }
    
    #[test]
    fn test_percentile_calculation() {
        let latencies = vec![
            Duration::from_micros(100),
            Duration::from_micros(200),
            Duration::from_micros(300),
            Duration::from_micros(400),
            Duration::from_micros(500),
        ];
        
        let (p50, p95, p99) = TestMetricsCollector::calculate_latency_percentiles(&latencies);
        
        assert_eq!(p50, Some(Duration::from_micros(300))); // Middle value
        assert_eq!(p95, Some(Duration::from_micros(500))); // Near end
        assert_eq!(p99, Some(Duration::from_micros(500))); // Last value
    }
    
    #[test]
    fn test_edge_case_tracking() {
        let metrics = TestMetricsCollector::new();
        
        metrics.record_partial_fill();
        metrics.record_cancellation();
        metrics.record_timeout();
        metrics.record_emergency_close();
        
        assert_eq!(metrics.partial_fills.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.cancellations.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.timeouts.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.emergency_closes.load(Ordering::Relaxed), 1);
    }
}
