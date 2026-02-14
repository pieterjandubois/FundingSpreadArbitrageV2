// Integration tests for buffer pool
// Tests Requirements: 12.1, 12.2, 12.3

use arbitrage2::strategy::buffer_pool::{BufferPool, with_string_buffer, create_small_vec};
use std::fmt::Write;

#[test]
fn test_buffer_pool_pre_allocation() {
    // Requirement 12.1: Pre-allocate Vec with with_capacity()
    let pool = BufferPool::new();
    
    // Verify 100 buffers pre-allocated
    assert_eq!(pool.buffer_count(), 100);
    
    // Verify each buffer has 256 bytes capacity (Requirement 12.2)
    assert_eq!(pool.buffer_capacity(), 256);
}

#[test]
fn test_buffer_reuse_with_clear() {
    // Requirement 12.3: Implement buffer reuse with clear()
    let mut pool = BufferPool::new();
    
    // Get first buffer and write to it
    let buf1 = pool.get_string_buffer();
    let ptr1 = buf1.as_ptr();
    buf1.push_str("test data that takes up space");
    
    // Cycle through all 100 buffers
    for _ in 0..99 {
        pool.get_string_buffer();
    }
    
    // Get the first buffer again
    let buf2 = pool.get_string_buffer();
    let ptr2 = buf2.as_ptr();
    
    // Verify same heap allocation (pointer matches)
    assert_eq!(ptr1, ptr2, "Buffer should reuse same heap allocation");
    
    // Verify buffer was cleared
    assert!(buf2.is_empty(), "Buffer should be cleared via clear()");
    
    // Verify capacity maintained
    assert_eq!(buf2.capacity(), 256, "Capacity should be maintained after clear()");
}

#[test]
fn test_small_vec_stack_allocation() {
    // Requirement 12.2: Add SmallVec for small collections
    let mut vec = create_small_vec::<f64>();
    
    // Add 8 items - should stay on stack
    for i in 0..8 {
        vec.push(i as f64);
    }
    
    assert_eq!(vec.len(), 8);
    assert!(!vec.spilled(), "SmallVec should not allocate on heap for <=8 items");
}

#[test]
fn test_small_vec_heap_spill() {
    let mut vec = create_small_vec::<f64>();
    
    // Add 9 items - should spill to heap
    for i in 0..9 {
        vec.push(i as f64);
    }
    
    assert_eq!(vec.len(), 9);
    assert!(vec.spilled(), "SmallVec should spill to heap for >8 items");
}

#[test]
fn test_thread_local_buffer_pool() {
    // Test thread-local access
    let result = with_string_buffer(|buf| {
        write!(buf, "Price: {:.2}", 123.456).unwrap();
        buf.clone()
    });
    
    assert_eq!(result, "Price: 123.46");
    
    // Second call should get a different cleared buffer
    let result2 = with_string_buffer(|buf| {
        write!(buf, "Volume: {:.0}", 1000.0).unwrap();
        buf.clone()
    });
    
    assert_eq!(result2, "Volume: 1000");
}

#[test]
fn test_zero_allocation_hot_path() {
    // Simulate hot path usage
    let mut pool = BufferPool::new();
    
    // First pass - allocations happen during initialization
    for i in 0..100 {
        let buf = pool.get_string_buffer();
        write!(buf, "Order {}: Price {:.2}", i, 100.0 + i as f64).unwrap();
    }
    
    // Second pass - should reuse all buffers (zero allocations)
    for i in 0..100 {
        let buf = pool.get_string_buffer();
        let ptr_before = buf.as_ptr();
        let capacity_before = buf.capacity();
        
        write!(buf, "Order {}: Price {:.2}", i, 200.0 + i as f64).unwrap();
        
        // Verify no reallocation occurred
        assert_eq!(buf.as_ptr(), ptr_before, "No reallocation should occur");
        assert_eq!(buf.capacity(), capacity_before, "Capacity should remain constant");
    }
}

#[test]
fn test_buffer_pool_round_robin() {
    let mut pool = BufferPool::new();
    
    // Get 5 buffers and track their pointers
    let mut pointers = Vec::new();
    for _ in 0..5 {
        let buf = pool.get_string_buffer();
        pointers.push(buf.as_ptr());
    }
    
    // All pointers should be different
    for i in 0..pointers.len() {
        for j in (i+1)..pointers.len() {
            assert_ne!(pointers[i], pointers[j], "Each buffer should be unique");
        }
    }
}

#[test]
fn test_small_vec_with_capacity() {
    // Test pre-allocation hint
    let mut vec = BufferPool::create_small_vec_with_capacity::<u64>(5);
    
    for i in 0..5 {
        vec.push(i);
    }
    
    assert_eq!(vec.len(), 5);
    assert!(!vec.spilled(), "Should not spill for capacity hint <= 8");
}

#[test]
fn test_buffer_pool_default() {
    let pool = BufferPool::default();
    assert_eq!(pool.buffer_count(), 100);
}
