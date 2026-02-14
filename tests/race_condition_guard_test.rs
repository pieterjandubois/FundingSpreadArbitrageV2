use arbitrage2::strategy::atomic_execution::{RaceConditionGuard};

#[test]
fn test_race_condition_guard_creation() {
    let guard = RaceConditionGuard::new();
    // Guard should be created successfully
    // We can't directly inspect the internal HashSet, but we can test behavior
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_ok());
}

#[test]
fn test_race_condition_guard_default() {
    let guard = RaceConditionGuard::default();
    // Default should work the same as new()
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_ok());
}

#[test]
fn test_acquire_hedge_lock_success() {
    let guard = RaceConditionGuard::new();
    let result = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(result.is_ok(), "First lock acquisition should succeed");
}

#[test]
fn test_acquire_hedge_lock_concurrent_failure() {
    let guard = RaceConditionGuard::new();
    
    // First lock should succeed
    let lock1 = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(lock1.is_ok(), "First lock should succeed");
    
    // Second lock on same symbol should fail
    let lock2 = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(lock2.is_err(), "Second lock on same symbol should fail");
    
    let error_msg = lock2.unwrap_err();
    assert!(error_msg.contains("already in progress"), 
            "Error message should indicate hedge already in progress");
    assert!(error_msg.contains("BTCUSDT"), 
            "Error message should include the symbol name");
}

#[test]
fn test_acquire_hedge_lock_different_symbols() {
    let guard = RaceConditionGuard::new();
    
    // Lock for first symbol
    let lock1 = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(lock1.is_ok(), "Lock for BTCUSDT should succeed");
    
    // Lock for different symbol should succeed
    let lock2 = guard.try_acquire_hedge_lock("ETHUSDT");
    assert!(lock2.is_ok(), "Lock for ETHUSDT should succeed");
    
    // Both locks should be held simultaneously
    // Trying to acquire BTCUSDT again should fail
    let lock3 = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(lock3.is_err(), "BTCUSDT should still be locked");
    
    // Trying to acquire ETHUSDT again should fail
    let lock4 = guard.try_acquire_hedge_lock("ETHUSDT");
    assert!(lock4.is_err(), "ETHUSDT should still be locked");
}

#[test]
fn test_hedge_lock_guard_raii_cleanup() {
    let guard = RaceConditionGuard::new();
    
    {
        // Acquire lock in inner scope
        let _lock = guard.try_acquire_hedge_lock("BTCUSDT").unwrap();
        
        // Verify symbol is locked by trying to acquire again
        let lock2 = guard.try_acquire_hedge_lock("BTCUSDT");
        assert!(lock2.is_err(), "Symbol should be locked");
    } // Lock guard dropped here
    
    // Verify symbol is unlocked after guard is dropped
    let lock3 = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(lock3.is_ok(), "Symbol should be unlocked after guard dropped");
}

#[test]
fn test_hedge_lock_guard_reacquire_after_drop() {
    let guard = RaceConditionGuard::new();
    
    {
        let _lock1 = guard.try_acquire_hedge_lock("BTCUSDT").unwrap();
        // Verify it's locked
        assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_err());
    } // First lock dropped
    
    // Should be able to acquire lock again after drop
    let lock2 = guard.try_acquire_hedge_lock("BTCUSDT");
    assert!(lock2.is_ok(), "Should be able to reacquire lock after drop");
}

#[test]
fn test_multiple_symbols_concurrent_locks() {
    let guard = RaceConditionGuard::new();
    
    let _lock1 = guard.try_acquire_hedge_lock("BTCUSDT").unwrap();
    let _lock2 = guard.try_acquire_hedge_lock("ETHUSDT").unwrap();
    let _lock3 = guard.try_acquire_hedge_lock("SOLUSDT").unwrap();
    
    // Verify all three symbols are locked by trying to acquire them again
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_err());
    assert!(guard.try_acquire_hedge_lock("ETHUSDT").is_err());
    assert!(guard.try_acquire_hedge_lock("SOLUSDT").is_err());
    
    // A fourth different symbol should still be acquirable
    let lock4 = guard.try_acquire_hedge_lock("ADAUSDT");
    assert!(lock4.is_ok(), "Different symbol should be acquirable");
}

#[test]
fn test_hedge_lock_guard_selective_drop() {
    let guard = RaceConditionGuard::new();
    
    let lock1 = guard.try_acquire_hedge_lock("BTCUSDT").unwrap();
    let _lock2 = guard.try_acquire_hedge_lock("ETHUSDT").unwrap();
    
    // Both should be locked
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_err());
    assert!(guard.try_acquire_hedge_lock("ETHUSDT").is_err());
    
    // Drop only the first lock
    drop(lock1);
    
    // BTCUSDT should be unlocked, ETHUSDT should still be locked
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_ok(), 
            "BTCUSDT should be unlocked after dropping lock1");
    assert!(guard.try_acquire_hedge_lock("ETHUSDT").is_err(), 
            "ETHUSDT should still be locked");
}

#[test]
fn test_lock_guard_explicit_drop() {
    let guard = RaceConditionGuard::new();
    
    let lock = guard.try_acquire_hedge_lock("BTCUSDT").unwrap();
    
    // Verify it's locked
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_err());
    
    // Explicitly drop the lock
    drop(lock);
    
    // Should be unlocked now
    assert!(guard.try_acquire_hedge_lock("BTCUSDT").is_ok());
}

#[test]
fn test_case_sensitive_symbol_names() {
    let guard = RaceConditionGuard::new();
    
    let _lock1 = guard.try_acquire_hedge_lock("BTCUSDT").unwrap();
    
    // Different case should be treated as different symbol
    let lock2 = guard.try_acquire_hedge_lock("btcusdt");
    assert!(lock2.is_ok(), "Symbol names should be case-sensitive");
}

#[test]
fn test_empty_symbol_name() {
    let guard = RaceConditionGuard::new();
    
    // Empty string should be a valid symbol name (edge case)
    let lock = guard.try_acquire_hedge_lock("");
    assert!(lock.is_ok(), "Empty string should be a valid symbol");
    
    // Trying to acquire again should fail
    assert!(guard.try_acquire_hedge_lock("").is_err());
}

#[test]
fn test_special_characters_in_symbol() {
    let guard = RaceConditionGuard::new();
    
    // Symbols with special characters
    let _lock1 = guard.try_acquire_hedge_lock("BTC-USDT").unwrap();
    let _lock2 = guard.try_acquire_hedge_lock("BTC/USDT").unwrap();
    let _lock3 = guard.try_acquire_hedge_lock("BTC_USDT").unwrap();
    
    // All should be treated as different symbols
    assert!(guard.try_acquire_hedge_lock("BTC-USDT").is_err());
    assert!(guard.try_acquire_hedge_lock("BTC/USDT").is_err());
    assert!(guard.try_acquire_hedge_lock("BTC_USDT").is_err());
}
