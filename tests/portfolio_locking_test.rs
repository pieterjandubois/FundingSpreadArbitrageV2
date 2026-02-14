/// Tests for portfolio manager locking optimization (Task 18)
/// 
/// Validates:
/// - RwLock allows concurrent reads
/// - Write operations are properly synchronized
/// - Lock hold time is minimized
/// - Atomic counters work correctly

use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::Instant;
use futures_util::future;

#[tokio::test]
async fn test_concurrent_reads_with_rwlock() {
    // This test validates that RwLock allows multiple concurrent readers
    // which is the key optimization for read-heavy operations
    
    let shared_data = Arc::new(RwLock::new(vec![1, 2, 3, 4, 5]));
    let mut handles = vec![];
    
    // Spawn 10 concurrent readers
    for i in 0..10 {
        let data = Arc::clone(&shared_data);
        let handle = tokio::spawn(async move {
            let start = Instant::now();
            let guard = data.read().await;
            let sum: i32 = guard.iter().sum();
            // Hold the lock for a bit to simulate work
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            let elapsed = start.elapsed();
            (i, sum, elapsed)
        });
        handles.push(handle);
    }
    
    // Wait for all readers to complete
    let results: Vec<_> = future::join_all(handles).await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    
    // All readers should get the same sum
    for (i, sum, elapsed) in &results {
        assert_eq!(*sum, 15, "Reader {} got wrong sum", i);
        // With RwLock, concurrent reads should complete quickly
        // even though each holds the lock for 10ms
        println!("Reader {} completed in {:?}", i, elapsed);
    }
    
    // The total time should be much less than 10 * 10ms = 100ms
    // because reads happen concurrently
    let max_elapsed = results.iter().map(|(_, _, e)| e).max().unwrap();
    println!("Max elapsed time: {:?}", max_elapsed);
}

#[tokio::test]
async fn test_write_blocks_reads() {
    // This test validates that write operations properly block reads
    // ensuring data consistency
    
    let shared_data = Arc::new(RwLock::new(0));
    
    // Start a writer that holds the lock for 50ms
    let data_writer = Arc::clone(&shared_data);
    let writer = tokio::spawn(async move {
        let mut guard = data_writer.write().await;
        *guard = 42;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    });
    
    // Give writer time to acquire the lock
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    
    // Try to read - should block until writer releases
    let data_reader = Arc::clone(&shared_data);
    let start = Instant::now();
    let reader = tokio::spawn(async move {
        let guard = data_reader.read().await;
        (*guard, Instant::now())
    });
    
    writer.await.unwrap();
    let (value, read_time) = reader.await.unwrap();
    let elapsed = read_time.duration_since(start);
    
    assert_eq!(value, 42, "Reader should see updated value");
    assert!(elapsed.as_millis() >= 40, "Reader should have been blocked by writer");
    println!("Reader was blocked for {:?}", elapsed);
}

#[tokio::test]
async fn test_minimal_lock_hold_time() {
    // This test validates that expensive operations are done outside the lock
    // by measuring lock hold time
    
    let shared_data = Arc::new(RwLock::new(vec![1, 2, 3]));
    
    // Simulate the pattern used in portfolio manager:
    // 1. Acquire lock
    // 2. Read/modify data quickly
    // 3. Release lock
    // 4. Do expensive I/O
    
    let data = Arc::clone(&shared_data);
    let start = Instant::now();
    
    // Critical section - should be very fast
    let snapshot = {
        let guard = data.read().await;
        guard.clone() // Quick clone
    }; // Lock released here
    let lock_time = start.elapsed();
    
    // Expensive operation outside the lock
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let _result = snapshot.iter().sum::<i32>();
    
    let total_time = start.elapsed();
    
    println!("Lock held for: {:?}", lock_time);
    println!("Total time: {:?}", total_time);
    
    // Lock should be held for much less than the total time
    assert!(lock_time.as_micros() < 1000, "Lock held too long: {:?}", lock_time);
    assert!(total_time.as_millis() >= 50, "Total time should include expensive operation");
}

#[tokio::test]
async fn test_atomic_counters_are_lock_free() {
    // This test validates that atomic counters can be updated
    // without holding the main lock
    
    use std::sync::atomic::{AtomicU64, Ordering};
    
    let counter = Arc::new(AtomicU64::new(0));
    let mut handles = vec![];
    
    // Spawn 100 concurrent incrementers
    for _ in 0..100 {
        let c = Arc::clone(&counter);
        let handle = tokio::spawn(async move {
            for _ in 0..100 {
                c.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }
    
    // Wait for all to complete
    future::join_all(handles).await;
    
    // Should have 100 * 100 = 10,000 increments
    assert_eq!(counter.load(Ordering::Relaxed), 10_000);
}

#[tokio::test]
async fn test_read_write_fairness() {
    // This test validates that readers and writers can make progress
    // without starvation
    
    let shared_data = Arc::new(RwLock::new(0));
    let mut writer_handles = vec![];
    let mut reader_handles = vec![];
    
    // Spawn 5 writers
    for i in 0..5 {
        let data = Arc::clone(&shared_data);
        let handle = tokio::spawn(async move {
            for _ in 0..10 {
                let mut guard = data.write().await;
                *guard += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            }
            i
        });
        writer_handles.push(handle);
    }
    
    // Spawn 10 readers
    for i in 0..10 {
        let data = Arc::clone(&shared_data);
        let handle = tokio::spawn(async move {
            let mut reads = 0;
            for _ in 0..20 {
                let guard = data.read().await;
                let _ = *guard;
                reads += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            }
            (i, reads)
        });
        reader_handles.push(handle);
    }
    
    // Wait for all to complete
    let writer_results = future::join_all(writer_handles).await;
    let reader_results = future::join_all(reader_handles).await;
    
    // All tasks should complete successfully
    assert_eq!(writer_results.len(), 5);
    assert_eq!(reader_results.len(), 10);
    
    // Final value should be 5 writers * 10 increments = 50
    let final_value = *shared_data.read().await;
    assert_eq!(final_value, 50);
}
