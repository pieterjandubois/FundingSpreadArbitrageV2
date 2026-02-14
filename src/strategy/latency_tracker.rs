// src/strategy/latency_tracker.rs
// Utilities for tracking latency in hot paths

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Cache-aligned latency statistics to prevent false sharing
#[repr(align(64))]
pub struct LatencyStats {
    pub p50_ns: AtomicU64,
    _pad1: [u8; 56],
    
    pub p95_ns: AtomicU64,
    _pad2: [u8; 56],
    
    pub p99_ns: AtomicU64,
    _pad3: [u8; 56],
    
    pub max_ns: AtomicU64,
    _pad4: [u8; 56],
    
    pub count: AtomicU64,
    _pad5: [u8; 56],
}

impl LatencyStats {
    pub fn new() -> Self {
        Self {
            p50_ns: AtomicU64::new(0),
            _pad1: [0; 56],
            p95_ns: AtomicU64::new(0),
            _pad2: [0; 56],
            p99_ns: AtomicU64::new(0),
            _pad3: [0; 56],
            max_ns: AtomicU64::new(0),
            _pad4: [0; 56],
            count: AtomicU64::new(0),
            _pad5: [0; 56],
        }
    }
    
    /// Update statistics with a new latency measurement
    pub fn record(&self, latency_ns: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        
        // Update max if needed
        let mut current_max = self.max_ns.load(Ordering::Relaxed);
        while latency_ns > current_max {
            match self.max_ns.compare_exchange_weak(
                current_max,
                latency_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
        
        // For percentiles, we use a simple exponential moving average
        // This is not perfectly accurate but avoids allocations
        let alpha = 0.1; // Smoothing factor
        
        // Update p50 (median approximation)
        let current_p50 = self.p50_ns.load(Ordering::Relaxed);
        let new_p50 = ((1.0 - alpha) * current_p50 as f64 + alpha * latency_ns as f64) as u64;
        self.p50_ns.store(new_p50, Ordering::Relaxed);
        
        // Update p95 (biased towards higher values)
        if latency_ns > current_p50 {
            let current_p95 = self.p95_ns.load(Ordering::Relaxed);
            let new_p95 = ((1.0 - alpha * 0.5) * current_p95 as f64 + alpha * 0.5 * latency_ns as f64) as u64;
            self.p95_ns.store(new_p95, Ordering::Relaxed);
        }
        
        // Update p99 (biased towards highest values)
        if latency_ns > self.p95_ns.load(Ordering::Relaxed) {
            let current_p99 = self.p99_ns.load(Ordering::Relaxed);
            let new_p99 = ((1.0 - alpha * 0.2) * current_p99 as f64 + alpha * 0.2 * latency_ns as f64) as u64;
            self.p99_ns.store(new_p99, Ordering::Relaxed);
        }
    }
    
    /// Get current statistics snapshot
    pub fn snapshot(&self) -> LatencySnapshot {
        LatencySnapshot {
            p50_ns: self.p50_ns.load(Ordering::Relaxed),
            p95_ns: self.p95_ns.load(Ordering::Relaxed),
            p99_ns: self.p99_ns.load(Ordering::Relaxed),
            max_ns: self.max_ns.load(Ordering::Relaxed),
            count: self.count.load(Ordering::Relaxed),
        }
    }
    
    /// Reset all statistics
    pub fn reset(&self) {
        self.p50_ns.store(0, Ordering::Relaxed);
        self.p95_ns.store(0, Ordering::Relaxed);
        self.p99_ns.store(0, Ordering::Relaxed);
        self.max_ns.store(0, Ordering::Relaxed);
        self.count.store(0, Ordering::Relaxed);
    }
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of latency statistics at a point in time
#[derive(Debug, Clone, Copy)]
pub struct LatencySnapshot {
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub count: u64,
}

impl LatencySnapshot {
    pub fn p50_us(&self) -> f64 {
        self.p50_ns as f64 / 1000.0
    }
    
    pub fn p95_us(&self) -> f64 {
        self.p95_ns as f64 / 1000.0
    }
    
    pub fn p99_us(&self) -> f64 {
        self.p99_ns as f64 / 1000.0
    }
    
    pub fn max_us(&self) -> f64 {
        self.max_ns as f64 / 1000.0
    }
    
    pub fn p50_ms(&self) -> f64 {
        self.p50_ns as f64 / 1_000_000.0
    }
    
    pub fn p95_ms(&self) -> f64 {
        self.p95_ns as f64 / 1_000_000.0
    }
    
    pub fn p99_ms(&self) -> f64 {
        self.p99_ns as f64 / 1_000_000.0
    }
    
    pub fn max_ms(&self) -> f64 {
        self.max_ns as f64 / 1_000_000.0
    }
}

/// Measure latency of a function call
#[inline(always)]
pub fn measure_latency<F, R>(f: F) -> (R, u64)
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = f();
    let elapsed_ns = start.elapsed().as_nanos() as u64;
    (result, elapsed_ns)
}

/// Measure latency and record to stats
#[inline(always)]
pub fn measure_and_record<F, R>(stats: &LatencyStats, f: F) -> R
where
    F: FnOnce() -> R,
{
    let (result, latency_ns) = measure_latency(f);
    stats.record(latency_ns);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_latency_stats_creation() {
        let stats = LatencyStats::new();
        let snapshot = stats.snapshot();
        
        assert_eq!(snapshot.count, 0);
        assert_eq!(snapshot.p50_ns, 0);
        assert_eq!(snapshot.p95_ns, 0);
        assert_eq!(snapshot.p99_ns, 0);
        assert_eq!(snapshot.max_ns, 0);
    }

    #[test]
    fn test_latency_recording() {
        let stats = LatencyStats::new();
        
        stats.record(1000);
        stats.record(2000);
        stats.record(3000);
        
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.count, 3);
        assert!(snapshot.max_ns >= 3000);
    }

    #[test]
    fn test_measure_latency() {
        let (result, latency_ns) = measure_latency(|| {
            thread::sleep(Duration::from_micros(100));
            42
        });
        
        assert_eq!(result, 42);
        assert!(latency_ns >= 100_000); // At least 100 microseconds
    }

    #[test]
    fn test_measure_and_record() {
        let stats = LatencyStats::new();
        
        let result = measure_and_record(&stats, || {
            thread::sleep(Duration::from_micros(50));
            "test"
        });
        
        assert_eq!(result, "test");
        assert_eq!(stats.snapshot().count, 1);
    }

    #[test]
    fn test_snapshot_conversions() {
        let snapshot = LatencySnapshot {
            p50_ns: 1_000_000,
            p95_ns: 5_000_000,
            p99_ns: 10_000_000,
            max_ns: 20_000_000,
            count: 100,
        };
        
        assert_eq!(snapshot.p50_us(), 1000.0);
        assert_eq!(snapshot.p50_ms(), 1.0);
        assert_eq!(snapshot.p95_us(), 5000.0);
        assert_eq!(snapshot.p95_ms(), 5.0);
    }

    #[test]
    fn test_reset() {
        let stats = LatencyStats::new();
        
        stats.record(1000);
        stats.record(2000);
        assert_eq!(stats.snapshot().count, 2);
        
        stats.reset();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.count, 0);
        assert_eq!(snapshot.max_ns, 0);
    }
}
