use arbitrage2::strategy::thread_pinning::*;

#[test]
fn test_core_assignment_default() {
    let assignment = CoreAssignment::default_assignment();
    assert_eq!(assignment.strategy_core.id, 1);
    assert_eq!(assignment.websocket_cores.len(), 6);
    assert_eq!(assignment.websocket_cores[0].id, 2);
    assert_eq!(assignment.websocket_cores[5].id, 7);
}

#[test]
fn test_core_assignment_custom() {
    let assignment = CoreAssignment::custom(3, vec![4, 5, 6]);
    assert_eq!(assignment.strategy_core.id, 3);
    assert_eq!(assignment.websocket_cores.len(), 3);
    assert_eq!(assignment.websocket_cores[0].id, 4);
    assert_eq!(assignment.websocket_cores[2].id, 6);
}

#[test]
fn test_get_core_count() {
    let count = get_core_count();
    // Should have at least 1 core
    assert!(count > 0);
    println!("System has {} CPU cores", count);
}

#[test]
fn test_has_sufficient_cores() {
    let sufficient = has_sufficient_cores();
    let count = get_core_count();
    assert_eq!(sufficient, count >= 8);
    
    if sufficient {
        println!("✓ System has sufficient cores ({}) for thread pinning", count);
    } else {
        println!("⚠ System has insufficient cores ({}) - need at least 8", count);
    }
}

#[test]
fn test_print_core_assignment_info() {
    // Just verify it doesn't panic
    print_core_assignment_info();
}

#[test]
fn test_pin_strategy_thread() {
    // This test may fail on systems without proper permissions or core isolation
    // We just verify it doesn't panic
    match pin_strategy_thread() {
        Ok(()) => println!("✓ Successfully pinned strategy thread to core 1"),
        Err(e) => println!("⚠ Failed to pin strategy thread: {}", e),
    }
}

#[test]
fn test_pin_websocket_thread() {
    // Test pinning to core 2 (worker_id = 0)
    match pin_websocket_thread(0) {
        Ok(()) => println!("✓ Successfully pinned websocket thread 0 to core 2"),
        Err(e) => println!("⚠ Failed to pin websocket thread: {}", e),
    }
}

#[test]
fn test_spawn_pinned_thread() {
    use core_affinity::CoreId;
    use std::sync::mpsc;
    
    let (tx, rx) = mpsc::channel();
    
    let handle = spawn_pinned_thread(
        CoreId { id: 1 },
        "test-thread",
        move || {
            tx.send("Hello from pinned thread!").unwrap();
            42
        }
    );
    
    let message = rx.recv().unwrap();
    assert_eq!(message, "Hello from pinned thread!");
    
    let result = handle.join().unwrap();
    assert_eq!(result, 42);
}
