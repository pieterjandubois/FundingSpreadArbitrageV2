use arbitrage2::strategy::atomic_execution::RaceConditionGuard;

#[test]
fn test_cleanup_lock_basic() {
    let guard = RaceConditionGuard::new();
    let symbol = "BTCUSDT";
    
    // Should be able to acquire cleanup lock
    let lock = guard.try_acquire_cleanup_lock(symbol);
    assert!(lock.is_ok(), "Should acquire cleanup lock");
    
    // Should not be able to acquire another cleanup lock for same symbol
    let lock2 = guard.try_acquire_cleanup_lock(symbol);
    assert!(lock2.is_err(), "Should not acquire duplicate cleanup lock");
    
    // Drop the first lock
    drop(lock);
    
    // Should be able to acquire cleanup lock again after dropping
    let lock3 = guard.try_acquire_cleanup_lock(symbol);
    assert!(lock3.is_ok(), "Should acquire cleanup lock after drop");
}

#[test]
fn test_cleanup_lock_is_locked() {
    let guard = RaceConditionGuard::new();
    let symbol = "ETHUSDT";
    
    // Initially not locked
    assert!(!guard.is_cleanup_locked(symbol), "Should not be locked initially");
    
    // Acquire lock
    let _lock = guard.try_acquire_cleanup_lock(symbol).unwrap();
    
    // Should be locked now
    assert!(guard.is_cleanup_locked(symbol), "Should be locked after acquiring");
    
    // Drop lock
    drop(_lock);
    
    // Should not be locked anymore
    assert!(!guard.is_cleanup_locked(symbol), "Should not be locked after drop");
}

#[test]
fn test_cleanup_lock_different_symbols() {
    let guard = RaceConditionGuard::new();
    
    // Should be able to acquire locks for different symbols simultaneously
    let lock1 = guard.try_acquire_cleanup_lock("BTCUSDT");
    let lock2 = guard.try_acquire_cleanup_lock("ETHUSDT");
    let lock3 = guard.try_acquire_cleanup_lock("SOLUSDT");
    
    assert!(lock1.is_ok(), "Should acquire lock for BTC");
    assert!(lock2.is_ok(), "Should acquire lock for ETH");
    assert!(lock3.is_ok(), "Should acquire lock for SOL");
    
    // All should be locked
    assert!(guard.is_cleanup_locked("BTCUSDT"));
    assert!(guard.is_cleanup_locked("ETHUSDT"));
    assert!(guard.is_cleanup_locked("SOLUSDT"));
}

#[test]
fn test_cleanup_lock_and_hedge_lock_independent() {
    let guard = RaceConditionGuard::new();
    let symbol = "BTCUSDT";
    
    // Should be able to acquire both cleanup and hedge lock for same symbol
    // (they serve different purposes and don't conflict)
    let cleanup_lock = guard.try_acquire_cleanup_lock(symbol);
    let hedge_lock = guard.try_acquire_hedge_lock(symbol);
    
    assert!(cleanup_lock.is_ok(), "Should acquire cleanup lock");
    assert!(hedge_lock.is_ok(), "Should acquire hedge lock");
    
    // Both should be active
    assert!(guard.is_cleanup_locked(symbol));
}
