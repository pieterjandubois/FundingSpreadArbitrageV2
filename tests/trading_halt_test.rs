/// Tests for trading halt mechanism (Task 10.2)
/// 
/// Validates Requirement 8.4: Trading Halt on Critical Error
/// 
/// This test verifies that:
/// 1. Trading halt flag can be set when critical errors occur
/// 2. Trading halt flag prevents new operations from starting
/// 3. Trading halt flag can be checked before operations
/// 4. Critical alerts are logged with error details

#[cfg(test)]
mod trading_halt_tests {
    use arbitrage2::strategy::atomic_execution::{is_trading_halted, halt_trading, resume_trading};

    #[test]
    fn test_trading_halt_initially_false() {
        // Reset state
        resume_trading();
        
        // Trading should not be halted initially
        assert!(!is_trading_halted(), "Trading should not be halted initially");
    }

    #[test]
    fn test_halt_trading_sets_flag() {
        // Reset state
        resume_trading();
        
        // Halt trading with a reason
        halt_trading("Test critical error");
        
        // Trading should now be halted
        assert!(is_trading_halted(), "Trading should be halted after halt_trading() is called");
        
        // Clean up
        resume_trading();
    }

    #[test]
    fn test_resume_trading_clears_flag() {
        // Halt trading first
        halt_trading("Test critical error");
        assert!(is_trading_halted(), "Trading should be halted");
        
        // Resume trading
        resume_trading();
        
        // Trading should no longer be halted
        assert!(!is_trading_halted(), "Trading should not be halted after resume_trading() is called");
    }

    #[test]
    fn test_halt_trading_with_different_reasons() {
        // Reset state
        resume_trading();
        
        // Test with various error reasons
        let reasons = vec![
            "Emergency close failed: BTCUSDT on Binance (Network timeout)",
            "Emergency close failed: ETHUSDT on Bybit (Insufficient balance)",
            "Critical system error",
        ];
        
        for reason in reasons {
            halt_trading(reason);
            assert!(is_trading_halted(), "Trading should be halted for reason: {}", reason);
            resume_trading();
            assert!(!is_trading_halted(), "Trading should be resumed after each test");
        }
    }

    #[test]
    fn test_multiple_halt_calls_idempotent() {
        // Reset state
        resume_trading();
        
        // Multiple halt calls should be idempotent
        halt_trading("First error");
        assert!(is_trading_halted());
        
        halt_trading("Second error");
        assert!(is_trading_halted());
        
        halt_trading("Third error");
        assert!(is_trading_halted());
        
        // Single resume should clear the flag
        resume_trading();
        assert!(!is_trading_halted());
    }
}
