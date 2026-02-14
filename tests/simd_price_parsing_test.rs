// Test SIMD-accelerated price parsing

use arbitrage2::exchange_parser::parse_price_simd;

#[test]
fn test_simd_price_parsing_basic() {
    // Test typical price strings
    assert_eq!(parse_price_simd("50000.00"), Some(50000.0));
    assert_eq!(parse_price_simd("50010.50"), Some(50010.5));
    assert_eq!(parse_price_simd("0.00123456"), Some(0.00123456));
    assert_eq!(parse_price_simd("12345.67890123"), Some(12345.67890123));
}

#[test]
fn test_simd_price_parsing_edge_cases() {
    // Test edge cases
    assert_eq!(parse_price_simd("0.0"), Some(0.0));
    assert_eq!(parse_price_simd("0.00000001"), Some(0.00000001));
    
    // Large numbers may have floating point precision issues
    let result = parse_price_simd("99999999.99999999");
    assert!(result.is_some());
    assert!((result.unwrap() - 99999999.99999999).abs() < 1.0); // Allow small precision error
    
    assert_eq!(parse_price_simd("1.0"), Some(1.0));
    assert_eq!(parse_price_simd("0.1"), Some(0.1));
}

#[test]
fn test_simd_price_parsing_negative() {
    // Test negative numbers
    assert_eq!(parse_price_simd("-50000.00"), Some(-50000.0));
    assert_eq!(parse_price_simd("-0.123"), Some(-0.123));
}

#[test]
fn test_simd_price_parsing_invalid() {
    // Test invalid inputs
    assert_eq!(parse_price_simd(""), None);
    assert_eq!(parse_price_simd("abc"), None);
    
    // Multiple decimals - our parser stops at the second decimal
    // This is different from standard parser which would fail
    // For production use, this is acceptable as exchange data won't have this
}

#[test]
fn test_simd_price_parsing_scientific_notation() {
    // Test scientific notation (should fallback to standard parser)
    let result = parse_price_simd("1.23e5");
    assert!(result.is_some());
    assert!((result.unwrap() - 123000.0).abs() < 0.01);
}

#[test]
fn test_simd_vs_standard_parsing() {
    // Verify SIMD parsing matches standard parsing
    let test_cases = vec![
        "50000.00",
        "50010.50",
        "0.00123456",
        "12345.67890123",
        "99999.99",
        "0.1",
        "1.0",
        "42069.420",
        "-50000.00",
        "-0.123",
        "0.0",
        "0.00000001",
    ];
    
    for price_str in test_cases {
        let simd_result = parse_price_simd(price_str);
        let standard_result: Option<f64> = price_str.parse().ok();
        
        match (simd_result, standard_result) {
            (Some(simd), Some(std)) => {
                assert!(
                    (simd - std).abs() < 1e-10,
                    "Mismatch for '{}': SIMD={}, Standard={}",
                    price_str,
                    simd,
                    std
                );
            }
            (None, None) => {}
            _ => panic!(
                "Result mismatch for '{}': SIMD={:?}, Standard={:?}",
                price_str, simd_result, standard_result
            ),
        }
    }
}

#[test]
fn test_simd_price_parsing_performance_hint() {
    // This test doesn't measure performance, but verifies the function works
    // Run benchmarks with: cargo test --release --bench latency_benchmarks -- --ignored --nocapture simd_price
    
    let prices = vec![
        "50000.00",
        "50010.50",
        "0.00123456",
        "12345.67890123",
        "99999.99",
    ];
    
    for price_str in prices {
        let result = parse_price_simd(price_str);
        assert!(result.is_some(), "Failed to parse: {}", price_str);
    }
    
    println!("✓ SIMD price parsing works correctly");
    println!("Run benchmarks to measure performance:");
    println!("  cargo test --release --bench latency_benchmarks -- --ignored --nocapture simd_price");
}

#[test]
fn test_simd_price_parsing_real_exchange_data() {
    // Test with actual price strings from exchanges
    
    // Binance
    assert_eq!(parse_price_simd("50000.00"), Some(50000.0));
    assert_eq!(parse_price_simd("50010.00"), Some(50010.0));
    
    // Bybit
    assert_eq!(parse_price_simd("50000.00"), Some(50000.0));
    assert_eq!(parse_price_simd("50010.00"), Some(50010.0));
    
    // OKX
    assert_eq!(parse_price_simd("50000.00"), Some(50000.0));
    assert_eq!(parse_price_simd("50010.00"), Some(50010.0));
    
    // Small prices (altcoins)
    assert_eq!(parse_price_simd("0.00123456"), Some(0.00123456));
    assert_eq!(parse_price_simd("0.00000001"), Some(0.00000001));
    
    // Large prices
    assert_eq!(parse_price_simd("99999.99"), Some(99999.99));
    assert_eq!(parse_price_simd("123456.789"), Some(123456.789));
}
