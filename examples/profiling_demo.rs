// examples/profiling_demo.rs
// Demonstration of the profiling infrastructure

use arbitrage2::strategy::latency_tracker::{LatencyStats, measure_latency, measure_and_record};
use std::thread;
use std::time::Duration;

fn main() {
    println!("=== Profiling Infrastructure Demo ===\n");
    
    // Create latency statistics tracker
    let stats = LatencyStats::new();
    
    // Example 1: Measure a single operation
    println!("1. Measuring a single operation:");
    let (result, latency_ns) = measure_latency(|| {
        // Simulate some work
        thread::sleep(Duration::from_micros(100));
        42
    });
    println!("   Result: {}", result);
    println!("   Latency: {} µs\n", latency_ns / 1000);
    
    // Example 2: Measure and record multiple operations
    println!("2. Recording multiple operations:");
    for i in 0..10 {
        measure_and_record(&stats, || {
            // Simulate varying latencies
            thread::sleep(Duration::from_micros(50 + i * 10));
        });
    }
    
    // Get statistics snapshot
    let snapshot = stats.snapshot();
    println!("   Operations recorded: {}", snapshot.count);
    println!("   P50 latency: {:.2} µs", snapshot.p50_us());
    println!("   P95 latency: {:.2} µs", snapshot.p95_us());
    println!("   P99 latency: {:.2} µs", snapshot.p99_us());
    println!("   Max latency: {:.2} µs\n", snapshot.max_us());
    
    // Example 3: Fast operations (nanosecond precision)
    println!("3. Measuring fast operations:");
    stats.reset();
    
    for _ in 0..1000 {
        measure_and_record(&stats, || {
            // Very fast operation
            let _x = (1..10).sum::<i32>();
        });
    }
    
    let snapshot = stats.snapshot();
    println!("   Operations recorded: {}", snapshot.count);
    println!("   P50 latency: {} ns", snapshot.p50_ns);
    println!("   P95 latency: {} ns", snapshot.p95_ns);
    println!("   P99 latency: {} ns", snapshot.p99_ns);
    println!("   Max latency: {} ns\n", snapshot.max_ns);
    
    println!("=== Demo Complete ===");
    println!("\nNext steps:");
    println!("  1. Run benchmarks: cargo test --release -- --ignored --nocapture");
    println!("  2. Profile with flamegraph: cargo flamegraph --release --bin arbitrage2");
    println!("  3. See PROFILING.md for more details");
}
