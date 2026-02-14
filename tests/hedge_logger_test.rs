use arbitrage2::strategy::atomic_execution::{HedgeLogger, LogLevel, CancellationResult, HedgeTimingMetrics};

#[test]
fn test_hedge_logger_creation() {
    let logger = HedgeLogger::new(LogLevel::Info);
    // Should not panic
    drop(logger);
}

#[test]
fn test_hedge_logger_default() {
    let logger = HedgeLogger::default_level();
    // Should not panic
    drop(logger);
}

#[test]
fn test_hedge_logger_all_log_methods() {
    let logger = HedgeLogger::new(LogLevel::Debug);
    
    // Test all logging methods - should not panic
    logger.log_fill_detected("binance", "order123", 1.5, 100);
    logger.log_cancel_initiated("bybit", "order456");
    logger.log_cancel_result("binance", &CancellationResult::Cancelled, 50);
    logger.log_cancel_result("binance", &CancellationResult::AlreadyFilled, 50);
    logger.log_cancel_result("binance", &CancellationResult::Failed("API error".to_string()), 50);
    logger.log_market_order_placed("binance", "order_placed", 2.0);
    logger.log_race_condition_detected("BTCUSDT", 1.5, 1.5);
}

#[test]
fn test_hedge_logger_with_timing_metrics() {
    let logger = HedgeLogger::new(LogLevel::Info);
    let mut metrics = HedgeTimingMetrics::new();
    
    std::thread::sleep(std::time::Duration::from_millis(2));
    metrics.record_other_leg_check();
    metrics.record_cancel_initiated();
    metrics.record_cancel_completed();
    metrics.record_market_order_initiated();
    metrics.record_market_order_accepted();
    metrics.record_market_order_filled();
    metrics.finalize();
    
    // Should not panic
    logger.log_timing_summary(&metrics, "binance", "BTCUSDT");
}

#[test]
fn test_log_level_filtering() {
    // Error level logger should only log errors
    let error_logger = HedgeLogger::new(LogLevel::Error);
    
    // These should not produce output (but shouldn't panic)
    error_logger.log_fill_detected("binance", "order123", 1.5, 100);
    error_logger.log_cancel_initiated("bybit", "order456");
}

#[test]
fn test_cancellation_result_variants() {
    let cancelled = CancellationResult::Cancelled;
    let filled = CancellationResult::AlreadyFilled;
    let failed = CancellationResult::Failed("Error".to_string());
    
    // Test that variants can be created and matched
    match cancelled {
        CancellationResult::Cancelled => {},
        _ => panic!("Wrong variant"),
    }
    
    match filled {
        CancellationResult::AlreadyFilled => {},
        _ => panic!("Wrong variant"),
    }
    
    match failed {
        CancellationResult::Failed(err) => assert_eq!(err, "Error"),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_log_level_variants() {
    // Test that all log level variants can be created
    let _debug = LogLevel::Debug;
    let _info = LogLevel::Info;
    let _warn = LogLevel::Warn;
    let _error = LogLevel::Error;
    
    // Test that loggers can be created with each level
    let _logger1 = HedgeLogger::new(LogLevel::Debug);
    let _logger2 = HedgeLogger::new(LogLevel::Info);
    let _logger3 = HedgeLogger::new(LogLevel::Warn);
    let _logger4 = HedgeLogger::new(LogLevel::Error);
}
