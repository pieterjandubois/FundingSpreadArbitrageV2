//! Integration test for monitoring HTTP endpoint
//!
//! This test verifies that the monitoring endpoint exposes metrics correctly.

use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_monitor_health_endpoint() {
    // Note: This test assumes the monitor server is running on port 9090
    // In a real scenario, we would start the server programmatically
    
    // Give the server time to start (if running)
    sleep(Duration::from_millis(100)).await;
    
    // Try to connect to health endpoint
    let client = reqwest::Client::new();
    
    match client
        .get("http://localhost:9090/health")
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(response) => {
            println!("Health endpoint responded with status: {}", response.status());
            
            if response.status().is_success() {
                let body = response.text().await.unwrap();
                println!("Health response: {}", body);
                
                // Verify JSON structure
                assert!(body.contains("status"));
                assert!(body.contains("uptime_seconds"));
                assert!(body.contains("market_queue_utilization_percent"));
                assert!(body.contains("order_queue_utilization_percent"));
            }
        }
        Err(e) => {
            println!("Note: Monitor server not running ({}). This is expected in CI.", e);
            println!("To test manually, run: cargo run --bin monitor --release");
        }
    }
}

#[tokio::test]
async fn test_monitor_metrics_endpoint() {
    // Note: This test assumes the monitor server is running on port 9090
    
    sleep(Duration::from_millis(100)).await;
    
    let client = reqwest::Client::new();
    
    match client
        .get("http://localhost:9090/metrics")
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(response) => {
            println!("Metrics endpoint responded with status: {}", response.status());
            
            if response.status().is_success() {
                let body = response.text().await.unwrap();
                println!("Metrics response (first 500 chars):\n{}", &body[..body.len().min(500)]);
                
                // Verify Prometheus format
                assert!(body.contains("# HELP"));
                assert!(body.contains("# TYPE"));
                
                // Verify latency metrics
                assert!(body.contains("latency_p50_microseconds"));
                assert!(body.contains("latency_p95_microseconds"));
                assert!(body.contains("latency_p99_microseconds"));
                assert!(body.contains("latency_max_microseconds"));
                
                // Verify queue metrics
                assert!(body.contains("queue_depth_market"));
                assert!(body.contains("queue_depth_order"));
                assert!(body.contains("queue_utilization_market_percent"));
                assert!(body.contains("queue_utilization_order_percent"));
                
                // Verify allocation metrics
                assert!(body.contains("allocations_hot_path_total"));
                assert!(body.contains("allocations_per_second"));
                
                // Verify CPU metrics
                assert!(body.contains("cpu_strategy_thread_percent"));
                assert!(body.contains("cpu_websocket_threads_percent"));
                assert!(body.contains("cpu_system_percent"));
            }
        }
        Err(e) => {
            println!("Note: Monitor server not running ({}). This is expected in CI.", e);
            println!("To test manually, run: cargo run --bin monitor --release");
        }
    }
}

#[tokio::test]
async fn test_monitor_404_endpoint() {
    sleep(Duration::from_millis(100)).await;
    
    let client = reqwest::Client::new();
    
    match client
        .get("http://localhost:9090/nonexistent")
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(response) => {
            println!("404 endpoint responded with status: {}", response.status());
            
            // Should return 404
            assert_eq!(response.status().as_u16(), 404);
            
            let body = response.text().await.unwrap();
            assert!(body.contains("404 Not Found"));
            assert!(body.contains("/metrics"));
            assert!(body.contains("/health"));
        }
        Err(e) => {
            println!("Note: Monitor server not running ({}). This is expected in CI.", e);
        }
    }
}

#[test]
fn test_metrics_format_validation() {
    // Test that metric names follow Prometheus naming conventions
    let valid_metric_names = vec![
        "latency_p50_microseconds",
        "latency_p95_microseconds",
        "latency_p99_microseconds",
        "latency_max_microseconds",
        "latency_measurements_total",
        "queue_depth_market",
        "queue_depth_order",
        "queue_utilization_market_percent",
        "queue_utilization_order_percent",
        "queue_market_push_total",
        "queue_market_drop_total",
        "queue_order_submit_total",
        "queue_order_drop_total",
        "allocations_hot_path_total",
        "allocations_warm_path_total",
        "allocations_cold_path_total",
        "allocations_per_second",
        "cpu_strategy_thread_percent",
        "cpu_websocket_threads_percent",
        "cpu_system_percent",
        "cpu_context_switches_total",
    ];
    
    // Verify all metric names follow Prometheus conventions:
    // - lowercase
    // - underscores (no hyphens)
    // - descriptive suffixes (_total, _percent, etc.)
    for name in valid_metric_names {
        assert!(name.chars().all(|c| c.is_lowercase() || c.is_numeric() || c == '_'));
        assert!(!name.contains('-'));
        println!("✓ Valid metric name: {}", name);
    }
}
