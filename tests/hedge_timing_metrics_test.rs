use arbitrage2::strategy::atomic_execution::HedgeTimingMetrics;
use std::time::Duration;

#[test]
fn test_hedge_timing_metrics_creation() {
    let metrics = HedgeTimingMetrics::new();
    assert!(metrics.other_leg_check_at.is_none());
    assert!(metrics.cancel_initiated_at.is_none());
    assert!(metrics.cancel_completed_at.is_none());
    assert!(metrics.market_order_initiated_at.is_none());
    assert!(metrics.market_order_accepted_at.is_none());
    assert!(metrics.market_order_filled_at.is_none());
    assert!(metrics.total_hedge_duration.is_none());
}

#[test]
fn test_hedge_timing_metrics_record_timestamps() {
    let mut metrics = HedgeTimingMetrics::new();
    
    std::thread::sleep(Duration::from_millis(1));
    metrics.record_other_leg_check();
    assert!(metrics.other_leg_check_at.is_some());
    
    std::thread::sleep(Duration::from_millis(1));
    metrics.record_cancel_initiated();
    assert!(metrics.cancel_initiated_at.is_some());
    
    std::thread::sleep(Duration::from_millis(1));
    metrics.record_cancel_completed();
    assert!(metrics.cancel_completed_at.is_some());
    
    std::thread::sleep(Duration::from_millis(1));
    metrics.record_market_order_initiated();
    assert!(metrics.market_order_initiated_at.is_some());
    
    std::thread::sleep(Duration::from_millis(1));
    metrics.record_market_order_accepted();
    assert!(metrics.market_order_accepted_at.is_some());
    
    std::thread::sleep(Duration::from_millis(1));
    metrics.record_market_order_filled();
    assert!(metrics.market_order_filled_at.is_some());
}

#[test]
fn test_hedge_timing_metrics_duration_calculations() {
    let mut metrics = HedgeTimingMetrics::new();
    
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_other_leg_check();
    
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_cancel_initiated();
    
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_cancel_completed();
    
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_market_order_initiated();
    
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_market_order_accepted();
    
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_market_order_filled();
    
    // Verify durations are calculated correctly
    assert!(metrics.fill_to_other_leg_check().is_some());
    assert!(metrics.fill_to_cancel_initiated().is_some());
    assert!(metrics.cancel_duration().is_some());
    assert!(metrics.cancel_to_market_order().is_some());
    assert!(metrics.market_order_acceptance_duration().is_some());
    assert!(metrics.market_order_fill_duration().is_some());
    
    // Verify durations are positive and reasonable
    let fill_to_check = metrics.fill_to_other_leg_check().unwrap();
    assert!(fill_to_check.as_millis() >= 5);
    
    let cancel_dur = metrics.cancel_duration().unwrap();
    assert!(cancel_dur.as_millis() >= 5);
}

#[test]
fn test_hedge_timing_metrics_finalize() {
    let mut metrics = HedgeTimingMetrics::new();
    
    std::thread::sleep(Duration::from_millis(10));
    metrics.record_market_order_filled();
    
    metrics.finalize();
    
    assert!(metrics.total_hedge_duration.is_some());
    let total = metrics.total_hedge_duration.unwrap();
    assert!(total.as_millis() >= 10);
}

#[test]
fn test_hedge_timing_metrics_finalize_without_fill() {
    let mut metrics = HedgeTimingMetrics::new();
    
    std::thread::sleep(Duration::from_millis(5));
    
    // Finalize without recording market order filled
    metrics.finalize();
    
    assert!(metrics.total_hedge_duration.is_some());
    let total = metrics.total_hedge_duration.unwrap();
    assert!(total.as_millis() >= 5);
}

#[test]
fn test_hedge_timing_metrics_log_summary() {
    let mut metrics = HedgeTimingMetrics::new();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_other_leg_check();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_cancel_initiated();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_cancel_completed();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_market_order_initiated();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_market_order_accepted();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_market_order_filled();
    
    metrics.finalize();
    
    // This should not panic
    metrics.log_summary("binance", "BTCUSDT");
}

#[test]
fn test_hedge_timing_metrics_default() {
    let metrics = HedgeTimingMetrics::default();
    assert!(metrics.other_leg_check_at.is_none());
    assert!(metrics.total_hedge_duration.is_none());
}

// ============================================================================
// Enhanced Timing Metrics Tests (Phase 4)
// ============================================================================

#[test]
fn test_enhanced_metrics_initialization() {
    let metrics = HedgeTimingMetrics::new();
    
    // Verify new fields are initialized correctly
    assert!(metrics.depth_check_initiated_at.is_none());
    assert!(metrics.depth_check_completed_at.is_none());
    assert!(metrics.first_reprice_at.is_none());
    assert!(metrics.last_reprice_at.is_none());
    assert_eq!(metrics.total_reprice_time.as_secs(), 0);
}

#[test]
fn test_depth_check_timing_calculations() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Record depth check initiated
    metrics.record_depth_check_initiated();
    assert!(metrics.depth_check_initiated_at.is_some());
    
    // Wait a bit
    std::thread::sleep(Duration::from_millis(10));
    
    // Record depth check completed
    metrics.record_depth_check_completed();
    assert!(metrics.depth_check_completed_at.is_some());
    
    // Verify duration calculation
    let duration = metrics.depth_check_duration();
    assert!(duration.is_some());
    let dur = duration.unwrap();
    assert!(dur.as_millis() >= 10, "Expected at least 10ms, got {}ms", dur.as_millis());
    assert!(dur.as_millis() < 100, "Expected less than 100ms, got {}ms", dur.as_millis());
}

#[test]
fn test_depth_check_duration_without_completion() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Only record initiation
    metrics.record_depth_check_initiated();
    
    // Duration should be None without completion
    assert!(metrics.depth_check_duration().is_none());
}

#[test]
fn test_repricing_timing_calculations() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Record first reprice
    let reprice_duration_1 = Duration::from_millis(50);
    metrics.record_reprice(reprice_duration_1);
    
    assert!(metrics.first_reprice_at.is_some());
    assert!(metrics.last_reprice_at.is_some());
    assert_eq!(metrics.total_reprice_time.as_millis(), 50);
    
    // Wait a bit
    std::thread::sleep(Duration::from_millis(5));
    
    // Record second reprice
    let reprice_duration_2 = Duration::from_millis(30);
    metrics.record_reprice(reprice_duration_2);
    
    // First reprice timestamp should not change
    let first = metrics.first_reprice_at.unwrap();
    let last = metrics.last_reprice_at.unwrap();
    
    // Last should be after first
    assert!(last > first);
    
    // Total reprice time should be cumulative
    assert_eq!(metrics.total_reprice_time.as_millis(), 80);
}

#[test]
fn test_multiple_reprices_accumulate() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Record multiple reprices
    for i in 1..=5 {
        metrics.record_reprice(Duration::from_millis(i * 10));
        std::thread::sleep(Duration::from_millis(2));
    }
    
    // Total should be 10 + 20 + 30 + 40 + 50 = 150ms
    assert_eq!(metrics.total_reprice_time.as_millis(), 150);
    
    // First and last should be different
    assert!(metrics.first_reprice_at.is_some());
    assert!(metrics.last_reprice_at.is_some());
    let first = metrics.first_reprice_at.unwrap();
    let last = metrics.last_reprice_at.unwrap();
    assert!(last > first);
}

#[test]
fn test_enhanced_metrics_log_summary() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Record depth check
    metrics.record_depth_check_initiated();
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_depth_check_completed();
    
    // Record repricing
    metrics.record_reprice(Duration::from_millis(25));
    std::thread::sleep(Duration::from_millis(3));
    metrics.record_reprice(Duration::from_millis(30));
    
    // Record other timing events
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_other_leg_check();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_cancel_initiated();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_cancel_completed();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_market_order_initiated();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_market_order_accepted();
    
    std::thread::sleep(Duration::from_millis(2));
    metrics.record_market_order_filled();
    
    metrics.finalize();
    
    // This should not panic and should include enhanced metrics
    metrics.log_summary("binance", "BTCUSDT");
    
    // Verify enhanced metrics are present
    assert!(metrics.depth_check_duration().is_some());
    assert_eq!(metrics.total_reprice_time.as_millis(), 55);
}

#[test]
fn test_metrics_serialization_with_enhanced_fields() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Populate all fields including enhanced ones
    metrics.record_depth_check_initiated();
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_depth_check_completed();
    
    metrics.record_reprice(Duration::from_millis(20));
    metrics.record_reprice(Duration::from_millis(15));
    
    metrics.record_other_leg_check();
    metrics.record_cancel_initiated();
    metrics.record_cancel_completed();
    metrics.record_market_order_initiated();
    metrics.record_market_order_accepted();
    metrics.record_market_order_filled();
    
    metrics.finalize();
    
    // Verify all enhanced fields are accessible
    assert!(metrics.depth_check_initiated_at.is_some());
    assert!(metrics.depth_check_completed_at.is_some());
    assert!(metrics.first_reprice_at.is_some());
    assert!(metrics.last_reprice_at.is_some());
    assert_eq!(metrics.total_reprice_time.as_millis(), 35);
    
    // Verify duration calculations work
    assert!(metrics.depth_check_duration().is_some());
    assert!(metrics.total_hedge_duration.is_some());
}

#[test]
fn test_concurrent_metric_recording() {
    use std::sync::{Arc, Mutex};
    use std::thread;
    
    let metrics = Arc::new(Mutex::new(HedgeTimingMetrics::new()));
    let mut handles = vec![];
    
    // Spawn multiple threads to record reprices concurrently
    for i in 0..5 {
        let metrics_clone = Arc::clone(&metrics);
        let handle = thread::spawn(move || {
            let mut m = metrics_clone.lock().unwrap();
            m.record_reprice(Duration::from_millis(10 * (i + 1)));
        });
        handles.push(handle);
    }
    
    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }
    
    // Verify total reprice time is correct
    let final_metrics = metrics.lock().unwrap();
    // Total should be 10 + 20 + 30 + 40 + 50 = 150ms
    assert_eq!(final_metrics.total_reprice_time.as_millis(), 150);
    assert!(final_metrics.first_reprice_at.is_some());
    assert!(final_metrics.last_reprice_at.is_some());
}

#[test]
fn test_depth_check_without_initiation() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Only record completion without initiation
    metrics.record_depth_check_completed();
    
    // Duration should be None
    assert!(metrics.depth_check_duration().is_none());
}

#[test]
fn test_zero_reprices() {
    let metrics = HedgeTimingMetrics::new();
    
    // No reprices recorded
    assert!(metrics.first_reprice_at.is_none());
    assert!(metrics.last_reprice_at.is_none());
    assert_eq!(metrics.total_reprice_time.as_secs(), 0);
}

#[test]
fn test_single_reprice() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Record single reprice
    metrics.record_reprice(Duration::from_millis(42));
    
    // First and last should be set
    assert!(metrics.first_reprice_at.is_some());
    assert!(metrics.last_reprice_at.is_some());
    
    // They should be approximately equal (same instant)
    let first = metrics.first_reprice_at.unwrap();
    let last = metrics.last_reprice_at.unwrap();
    let diff = if last > first {
        last.duration_since(first)
    } else {
        first.duration_since(last)
    };
    assert!(diff.as_millis() < 10, "First and last reprice should be very close for single reprice");
    
    // Total time should match
    assert_eq!(metrics.total_reprice_time.as_millis(), 42);
}

#[test]
fn test_api_response_times_with_enhanced_metrics() {
    let mut metrics = HedgeTimingMetrics::new();
    
    // Record API response times
    metrics.record_api_response("depth_check".to_string(), 15);
    metrics.record_api_response("cancel_order".to_string(), 25);
    metrics.record_api_response("place_order".to_string(), 30);
    
    // Record enhanced metrics
    metrics.record_depth_check_initiated();
    std::thread::sleep(Duration::from_millis(5));
    metrics.record_depth_check_completed();
    
    metrics.record_reprice(Duration::from_millis(20));
    
    // Verify API response times are tracked
    assert_eq!(metrics.api_response_times.len(), 3);
    assert_eq!(metrics.api_response_times[0].0, "depth_check");
    assert_eq!(metrics.api_response_times[0].1, 15);
    
    // Verify enhanced metrics are independent
    assert!(metrics.depth_check_duration().is_some());
    assert_eq!(metrics.total_reprice_time.as_millis(), 20);
}
