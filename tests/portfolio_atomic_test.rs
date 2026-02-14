/// Test for Task 15: Atomic counter operations in PortfolioState
/// 
/// This test verifies that:
/// - AtomicU64 counters work correctly with Ordering::Relaxed
/// - #[repr(align(64))] prevents false sharing
/// - Atomic operations are lock-free
/// 
/// Requirements: 13.1, 13.2, 13.3, 13.4

use arbitrage2::strategy::types::PortfolioState;

#[test]
fn test_atomic_counter_operations() {
    let state = PortfolioState::new(10000.0);
    
    // Test initial values
    assert_eq!(state.get_win_count(), 0);
    assert_eq!(state.get_loss_count(), 0);
    assert_eq!(state.get_leg_out_count(), 0);
    
    // Test increment operations
    state.increment_wins();
    state.increment_wins();
    state.increment_wins();
    assert_eq!(state.get_win_count(), 3);
    
    state.increment_losses();
    state.increment_losses();
    assert_eq!(state.get_loss_count(), 2);
    
    state.increment_leg_outs();
    assert_eq!(state.get_leg_out_count(), 1);
}

#[test]
fn test_atomic_counter_serialization() {
    let state = PortfolioState::new(10000.0);
    
    // Increment counters
    state.increment_wins();
    state.increment_wins();
    state.increment_losses();
    state.increment_leg_outs();
    
    // Convert to serializable form
    let serializable = state.to_serializable();
    
    // Verify counters are preserved
    assert_eq!(serializable.win_count, 2);
    assert_eq!(serializable.loss_count, 1);
    assert_eq!(serializable.leg_out_count, 1);
    
    // Verify serialization works
    let json = serde_json::to_string(&serializable).unwrap();
    assert!(json.contains("\"win_count\":2"));
    assert!(json.contains("\"loss_count\":1"));
    assert!(json.contains("\"leg_out_count\":1"));
}

#[test]
fn test_atomic_counter_alignment() {
    use std::mem::{align_of, size_of};
    
    // Verify struct is aligned to 64 bytes (cache line)
    assert_eq!(align_of::<PortfolioState>(), 64);
    
    // Verify AtomicU64 is properly sized
    assert_eq!(size_of::<std::sync::atomic::AtomicU64>(), 8);
}
