// Pre-allocated buffer pool for zero-allocation hot paths
// Requirements: 12.1, 12.2, 12.3

use smallvec::SmallVec;
use std::cell::RefCell;

/// Pre-allocated buffer pool to eliminate heap allocations in hot paths.
/// 
/// This pool maintains:
/// - 100 pre-allocated String buffers (256 bytes each) for formatting
/// - SmallVec support for small collections (<8 items on stack)
/// 
/// Buffers are reused via clear() to maintain heap allocations while
/// resetting content.
pub struct BufferPool {
    /// Pre-allocated String buffers for formatting operations
    pub(crate) format_buffers: Vec<String>,
    /// Index of next available buffer
    next_buffer_idx: usize,
}

impl BufferPool {
    /// Creates a new buffer pool with pre-allocated buffers.
    /// 
    /// Pre-allocates:
    /// - 100 String buffers with 256 bytes capacity each
    /// 
    /// This is a cold-path operation done during initialization.
    pub fn new() -> Self {
        let mut format_buffers = Vec::with_capacity(100);
        
        // Pre-allocate 100 String buffers (Requirement 12.2)
        for _ in 0..100 {
            format_buffers.push(String::with_capacity(256));
        }
        
        Self {
            format_buffers,
            next_buffer_idx: 0,
        }
    }
    
    /// Gets the number of pre-allocated buffers in the pool.
    #[inline(always)]
    pub fn buffer_count(&self) -> usize {
        self.format_buffers.len()
    }
    
    /// Gets the capacity of each buffer in the pool.
    #[inline(always)]
    pub fn buffer_capacity(&self) -> usize {
        self.format_buffers.first().map(|b| b.capacity()).unwrap_or(0)
    }
    
    /// Gets a cleared String buffer from the pool.
    /// 
    /// The buffer is cleared but retains its heap allocation.
    /// Returns a mutable reference that should be returned to the pool
    /// when done (via drop or explicit return).
    /// 
    /// # Hot Path
    /// This function is designed for hot path usage with zero allocations.
    #[inline(always)]
    pub fn get_string_buffer(&mut self) -> &mut String {
        // Round-robin through buffer pool
        let idx = self.next_buffer_idx;
        self.next_buffer_idx = (self.next_buffer_idx + 1) % self.format_buffers.len();
        
        let buffer = &mut self.format_buffers[idx];
        buffer.clear(); // Reuse allocation (Requirement 12.3)
        buffer
    }
    
    /// Creates a SmallVec for small collections.
    /// 
    /// SmallVec stores up to 8 items on the stack, avoiding heap allocation
    /// for small collections (Requirement 12.2).
    /// 
    /// # Hot Path
    /// This function is designed for hot path usage.
    #[inline(always)]
    pub fn create_small_vec<T>() -> SmallVec<[T; 8]> {
        SmallVec::new()
    }
    
    /// Creates a SmallVec with a specific capacity hint.
    /// 
    /// If capacity <= 8, no heap allocation occurs.
    /// If capacity > 8, heap allocation is done once upfront.
    #[inline(always)]
    pub fn create_small_vec_with_capacity<T>(capacity: usize) -> SmallVec<[T; 8]> {
        SmallVec::with_capacity(capacity)
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local buffer pool for zero-contention access in hot paths.
// 
// Each thread gets its own buffer pool, eliminating any synchronization
// overhead. This is safe because hot path threads (strategy thread) are
// single-threaded.
thread_local! {
    static THREAD_BUFFER_POOL: RefCell<BufferPool> = RefCell::new(BufferPool::new());
}

/// Gets a String buffer from the thread-local buffer pool.
/// 
/// # Example
/// ```
/// use arbitrage2::strategy::buffer_pool::with_string_buffer;
/// 
/// with_string_buffer(|buf| {
///     use std::fmt::Write;
///     write!(buf, "Price: {:.2}", 123.45).unwrap();
///     println!("{}", buf);
/// });
/// ```
#[inline(always)]
pub fn with_string_buffer<F, R>(f: F) -> R
where
    F: FnOnce(&mut String) -> R,
{
    THREAD_BUFFER_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        let buffer = pool.get_string_buffer();
        f(buffer)
    })
}

/// Creates a SmallVec from the thread-local buffer pool.
/// 
/// # Example
/// ```
/// use arbitrage2::strategy::buffer_pool::create_small_vec;
/// 
/// let mut vec = create_small_vec::<f64>();
/// vec.push(1.0);
/// vec.push(2.0);
/// // No heap allocation if <= 8 items
/// ```
#[inline(always)]
pub fn create_small_vec<T>() -> SmallVec<[T; 8]> {
    BufferPool::create_small_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn test_buffer_pool_creation() {
        let pool = BufferPool::new();
        assert_eq!(pool.format_buffers.len(), 100);
        assert_eq!(pool.format_buffers[0].capacity(), 256);
    }

    #[test]
    fn test_get_string_buffer_clears() {
        let mut pool = BufferPool::new();
        
        // Get a buffer and write to it
        let buf1 = pool.get_string_buffer();
        buf1.push_str("test data");
        
        // Get another buffer (should be different)
        let buf2 = pool.get_string_buffer();
        assert!(buf2.is_empty(), "Buffer should be cleared");
    }

    #[test]
    fn test_buffer_reuse() {
        let mut pool = BufferPool::new();
        
        // Fill first buffer
        let buf = pool.get_string_buffer();
        let ptr1 = buf.as_ptr();
        buf.push_str("test");
        
        // Cycle through all buffers
        for _ in 0..99 {
            pool.get_string_buffer();
        }
        
        // Should get back to first buffer
        let buf = pool.get_string_buffer();
        let ptr2 = buf.as_ptr();
        
        // Same heap allocation (pointer should match)
        assert_eq!(ptr1, ptr2, "Buffer should be reused");
        assert!(buf.is_empty(), "Buffer should be cleared");
    }

    #[test]
    fn test_small_vec_no_heap_allocation() {
        let mut vec = BufferPool::create_small_vec::<u64>();
        
        // Add 8 items (should stay on stack)
        for i in 0..8 {
            vec.push(i);
        }
        
        assert_eq!(vec.len(), 8);
        assert!(!vec.spilled(), "SmallVec should not spill to heap for 8 items");
    }

    #[test]
    fn test_small_vec_heap_allocation_when_needed() {
        let mut vec = BufferPool::create_small_vec::<u64>();
        
        // Add 9 items (should spill to heap)
        for i in 0..9 {
            vec.push(i);
        }
        
        assert_eq!(vec.len(), 9);
        assert!(vec.spilled(), "SmallVec should spill to heap for 9 items");
    }

    #[test]
    fn test_with_string_buffer() {
        let result = with_string_buffer(|buf| {
            write!(buf, "Price: {:.2}", 123.45).unwrap();
            buf.clone()
        });
        
        assert_eq!(result, "Price: 123.45");
    }

    #[test]
    fn test_thread_local_buffer_pool() {
        // Each call should get a cleared buffer
        let result1 = with_string_buffer(|buf| {
            buf.push_str("first");
            buf.clone()
        });
        
        let result2 = with_string_buffer(|buf| {
            buf.push_str("second");
            buf.clone()
        });
        
        assert_eq!(result1, "first");
        assert_eq!(result2, "second");
    }

    #[test]
    fn test_create_small_vec_function() {
        let mut vec = create_small_vec::<f64>();
        vec.push(1.0);
        vec.push(2.0);
        vec.push(3.0);
        
        assert_eq!(vec.len(), 3);
        assert!(!vec.spilled());
    }

    #[test]
    fn test_buffer_capacity_maintained() {
        let mut pool = BufferPool::new();
        
        let buf = pool.get_string_buffer();
        let initial_capacity = buf.capacity();
        
        // Write less than capacity
        buf.push_str("short string");
        
        // Get same buffer again (after cycling)
        for _ in 0..99 {
            pool.get_string_buffer();
        }
        
        let buf = pool.get_string_buffer();
        assert_eq!(buf.capacity(), initial_capacity, "Capacity should be maintained");
    }
}
