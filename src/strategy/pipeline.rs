//! Lock-Free SPSC Queue Pipeline for Market Data
//!
//! This module implements Single-Producer Single-Consumer (SPSC) lock-free queues
//! for streaming market data from WebSocket threads to the strategy thread.
//!
//! ## Architecture
//!
//! ```text
//! WebSocket Thread (Producer)     Strategy Thread (Consumer)
//!        │                                 │
//!        ├─ push() ──────────────────────▶ pop()
//!        │         SPSC Queue              │
//!        │         (Lock-Free)             │
//!        │                                 │
//!        └─ Backpressure (drop old)        └─ Process immediately
//! ```
//!
//! ## Why SPSC?
//!
//! - **Lock-Free**: No mutex contention, no context switches
//! - **Cache-Friendly**: Producer and consumer use separate cache lines
//! - **Bounded**: Fixed capacity applies backpressure (prevents memory explosion)
//! - **Fast**: ~10-20ns per operation (vs ~1000ns for Mutex)
//!
//! ## Backpressure Strategy
//!
//! When the queue is full, we drop the OLDEST data (not the newest).
//! This ensures we always process the most recent market data.
//!
//! Requirements: 3.1 (Lock-free queues), 14.3 (Bounded queues), 14.4 (Drop old data)

use crate::strategy::types::{MarketUpdate, OrderRequest};
use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Queue capacity: 10,000 market updates
/// 
/// At 10,000 updates/second, this provides 1 second of buffering.
/// If the strategy thread falls behind by more than 1 second,
/// we start dropping old data to prevent memory explosion.
const MARKET_QUEUE_CAPACITY: usize = 10_000;

/// Market data pipeline with lock-free SPSC queue.
///
/// This structure manages the flow of market data from WebSocket threads
/// to the strategy thread using a lock-free bounded queue.
///
/// # Performance Characteristics
///
/// - **Push**: ~10-20ns (lock-free, non-blocking)
/// - **Pop**: ~10-20ns (lock-free, non-blocking)
/// - **Memory**: 10,000 * 64 bytes = 640KB (fits in L2 cache)
/// - **Throughput**: >1M operations/second
///
/// # Thread Safety
///
/// - **Producer**: Single WebSocket thread (or coordinator)
/// - **Consumer**: Single strategy thread
/// - **Monitoring**: Multiple threads can read metrics (atomic counters)
pub struct MarketPipeline {
    /// Lock-free SPSC queue for market updates
    /// ArrayQueue is bounded and lock-free, perfect for SPSC pattern
    queue: Arc<ArrayQueue<MarketUpdate>>,
    
    /// Metrics: Total number of updates pushed (including dropped)
    push_count: AtomicU64,
    _pad1: [u8; 56],  // Pad to 64 bytes to prevent false sharing
    
    /// Metrics: Total number of updates successfully enqueued
    enqueue_count: AtomicU64,
    _pad2: [u8; 56],  // Pad to 64 bytes to prevent false sharing
    
    /// Metrics: Total number of updates dropped due to backpressure
    drop_count: AtomicU64,
    _pad3: [u8; 56],  // Pad to 64 bytes to prevent false sharing
    
    /// Metrics: Total number of updates consumed
    pop_count: AtomicU64,
    _pad4: [u8; 56],  // Pad to 64 bytes to prevent false sharing
}

impl MarketPipeline {
    /// Create a new market data pipeline with default capacity.
    ///
    /// # Performance
    ///
    /// - Allocation: One-time cost during initialization (cold path)
    /// - Memory: 640KB for queue + 32 bytes for metrics
    /// - Cache: Queue fits in L2 cache (typical 256KB-1MB)
    ///
    /// Requirement: 3.1 (SPSC queues)
    pub fn new() -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(MARKET_QUEUE_CAPACITY)),
            push_count: AtomicU64::new(0),
            _pad1: [0; 56],
            enqueue_count: AtomicU64::new(0),
            _pad2: [0; 56],
            drop_count: AtomicU64::new(0),
            _pad3: [0; 56],
            pop_count: AtomicU64::new(0),
            _pad4: [0; 56],
        }
    }
    
    /// Create a new market data pipeline with custom capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of market updates to buffer
    ///
    /// # Performance
    ///
    /// Memory usage: capacity * 64 bytes
    ///
    /// Requirement: 14.3 (Bounded queues)
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(capacity)),
            push_count: AtomicU64::new(0),
            _pad1: [0; 56],
            enqueue_count: AtomicU64::new(0),
            _pad2: [0; 56],
            drop_count: AtomicU64::new(0),
            _pad3: [0; 56],
            pop_count: AtomicU64::new(0),
            _pad4: [0; 56],
        }
    }
    
    /// Get a handle for the producer (WebSocket thread).
    ///
    /// This returns a lightweight handle that can be cloned and sent to
    /// the WebSocket thread for pushing market updates.
    pub fn producer(&self) -> MarketProducer {
        MarketProducer {
            queue: Arc::clone(&self.queue),
            push_count: &self.push_count,
            enqueue_count: &self.enqueue_count,
            drop_count: &self.drop_count,
        }
    }
    
    /// Get a handle for the consumer (strategy thread).
    ///
    /// This returns a lightweight handle that can be sent to the
    /// strategy thread for popping market updates.
    pub fn consumer(&self) -> MarketConsumer {
        MarketConsumer {
            queue: Arc::clone(&self.queue),
            pop_count: &self.pop_count,
        }
    }
    
    /// Get the current queue depth (number of items in queue).
    ///
    /// This is useful for monitoring and detecting backpressure.
    ///
    /// # Performance
    ///
    /// - Time: O(1) - just reads a counter
    /// - Accuracy: Approximate (lock-free, may be slightly stale)
    ///
    /// Requirement: Task 7 (Queue depth monitoring)
    #[inline(always)]
    pub fn depth(&self) -> usize {
        self.queue.len()
    }
    
    /// Get the queue capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.queue.capacity()
    }
    
    /// Check if the queue is full.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.queue.is_full()
    }
    
    /// Check if the queue is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    
    /// Get pipeline metrics.
    ///
    /// Returns a snapshot of current metrics for monitoring.
    pub fn metrics(&self) -> PipelineMetrics {
        PipelineMetrics {
            push_count: self.push_count.load(Ordering::Relaxed),
            enqueue_count: self.enqueue_count.load(Ordering::Relaxed),
            drop_count: self.drop_count.load(Ordering::Relaxed),
            pop_count: self.pop_count.load(Ordering::Relaxed),
            queue_depth: self.depth(),
            queue_capacity: self.capacity(),
        }
    }
}

impl Default for MarketPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Producer handle for pushing market updates (WebSocket thread).
///
/// This handle is Send + Sync and can be cloned to share across threads,
/// but should only be used by a single producer thread for optimal performance.
#[derive(Clone)]
pub struct MarketProducer {
    queue: Arc<ArrayQueue<MarketUpdate>>,
    push_count: *const AtomicU64,
    enqueue_count: *const AtomicU64,
    drop_count: *const AtomicU64,
}

// Safety: AtomicU64 is thread-safe, and we only use atomic operations
unsafe impl Send for MarketProducer {}
unsafe impl Sync for MarketProducer {}

impl MarketProducer {
    /// Push a market update to the queue (non-blocking).
    ///
    /// This is the primary hot path function called by WebSocket threads.
    /// It's marked `#[inline(always)]` to eliminate function call overhead.
    ///
    /// # Backpressure Strategy
    ///
    /// If the queue is full:
    /// 1. Pop the oldest item (drop old data)
    /// 2. Push the new item
    /// 3. Increment drop counter
    ///
    /// This ensures we always process the most recent market data.
    ///
    /// # Performance
    ///
    /// - Time: ~10-20ns (lock-free)
    /// - Allocations: Zero (pre-allocated queue)
    /// - Blocking: Never (non-blocking)
    ///
    /// Requirements: 3.1 (Lock-free), 14.4 (Drop old data)
    #[inline(always)]
    pub fn push(&self, update: MarketUpdate) {
        // Increment push counter (Relaxed ordering is fine for metrics)
        unsafe {
            (*self.push_count).fetch_add(1, Ordering::Relaxed);
        }
        
        // Try to push (non-blocking)
        if self.queue.push(update).is_err() {
            // Queue full - apply backpressure by dropping oldest
            self.queue.pop(); // Drop oldest
            
            // Try again (should succeed now)
            if self.queue.push(update).is_ok() {
                unsafe {
                    (*self.enqueue_count).fetch_add(1, Ordering::Relaxed);
                }
            }
            
            // Increment drop counter
            unsafe {
                (*self.drop_count).fetch_add(1, Ordering::Relaxed);
            }
        } else {
            // Successfully enqueued
            unsafe {
                (*self.enqueue_count).fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    
    /// Try to push a market update without backpressure (returns error if full).
    ///
    /// This variant does NOT drop old data if the queue is full.
    /// Use this if you want to handle backpressure differently.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if successfully pushed
    /// - `Err(update)` if queue is full (returns the update back)
    #[inline(always)]
    pub fn try_push(&self, update: MarketUpdate) -> Result<(), MarketUpdate> {
        unsafe {
            (*self.push_count).fetch_add(1, Ordering::Relaxed);
        }
        
        match self.queue.push(update) {
            Ok(()) => {
                unsafe {
                    (*self.enqueue_count).fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            }
            Err(update) => Err(update),
        }
    }
}

/// Consumer handle for popping market updates (strategy thread).
///
/// This handle is Send + Sync and should only be used by a single
/// consumer thread for optimal performance.
pub struct MarketConsumer {
    queue: Arc<ArrayQueue<MarketUpdate>>,
    pop_count: *const AtomicU64,
}

// Safety: AtomicU64 is thread-safe, and we only use atomic operations
unsafe impl Send for MarketConsumer {}
unsafe impl Sync for MarketConsumer {}

impl MarketConsumer {
    /// Pop a market update from the queue (non-blocking).
    ///
    /// This is the primary hot path function called by the strategy thread.
    /// It's marked `#[inline(always)]` to eliminate function call overhead.
    ///
    /// # Returns
    ///
    /// - `Some(update)` if an update is available
    /// - `None` if the queue is empty
    ///
    /// # Performance
    ///
    /// - Time: ~10-20ns (lock-free)
    /// - Allocations: Zero (returns by value, Copy type)
    /// - Blocking: Never (non-blocking)
    ///
    /// Requirement: 3.1 (Lock-free)
    #[inline(always)]
    pub fn pop(&self) -> Option<MarketUpdate> {
        match self.queue.pop() {
            Some(update) => {
                // Increment pop counter
                unsafe {
                    (*self.pop_count).fetch_add(1, Ordering::Relaxed);
                }
                Some(update)
            }
            None => None,
        }
    }
    
    /// Pop all available updates from the queue.
    ///
    /// This is useful for batch processing when the consumer falls behind.
    /// Returns a vector of all available updates (up to `max_batch` items).
    ///
    /// # Arguments
    ///
    /// * `max_batch` - Maximum number of updates to pop
    ///
    /// # Performance
    ///
    /// - Time: O(n) where n = number of items popped
    /// - Allocations: One Vec allocation (can be reused with clear())
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let consumer = pipeline.consumer();
    /// let updates = consumer.pop_batch(100);
    /// for update in updates {
    ///     process_update(update);
    /// }
    /// ```
    pub fn pop_batch(&self, max_batch: usize) -> Vec<MarketUpdate> {
        let mut batch = Vec::with_capacity(max_batch.min(self.queue.len()));
        
        for _ in 0..max_batch {
            match self.pop() {
                Some(update) => batch.push(update),
                None => break,
            }
        }
        
        batch
    }
}

/// Pipeline metrics for monitoring.
#[derive(Debug, Clone, Copy)]
pub struct PipelineMetrics {
    /// Total number of push attempts
    pub push_count: u64,
    
    /// Total number of successful enqueues
    pub enqueue_count: u64,
    
    /// Total number of dropped updates (backpressure)
    pub drop_count: u64,
    
    /// Total number of consumed updates
    pub pop_count: u64,
    
    /// Current queue depth
    pub queue_depth: usize,
    
    /// Queue capacity
    pub queue_capacity: usize,
}

impl PipelineMetrics {
    /// Calculate the drop rate (percentage of updates dropped).
    pub fn drop_rate(&self) -> f64 {
        if self.push_count == 0 {
            0.0
        } else {
            (self.drop_count as f64 / self.push_count as f64) * 100.0
        }
    }
    
    /// Calculate queue utilization (percentage of capacity used).
    pub fn utilization(&self) -> f64 {
        (self.queue_depth as f64 / self.queue_capacity as f64) * 100.0
    }
    
    /// Check if the pipeline is experiencing backpressure.
    ///
    /// Returns true if:
    /// - Queue is >80% full, OR
    /// - Drop rate is >1%
    pub fn is_backpressure(&self) -> bool {
        self.utilization() > 80.0 || self.drop_rate() > 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_pipeline_creation() {
        let pipeline = MarketPipeline::new();
        assert_eq!(pipeline.capacity(), MARKET_QUEUE_CAPACITY);
        assert_eq!(pipeline.depth(), 0);
        assert!(pipeline.is_empty());
        assert!(!pipeline.is_full());
    }
    
    #[test]
    fn test_push_and_pop() {
        let pipeline = MarketPipeline::new();
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        let update = MarketUpdate::new(1, 50000.0, 50010.0, 1000000);
        
        producer.push(update);
        
        assert_eq!(pipeline.depth(), 1);
        
        let popped = consumer.pop().unwrap();
        assert_eq!(popped.symbol_id, 1);
        assert_eq!(popped.bid, 50000.0);
        assert_eq!(popped.ask, 50010.0);
        
        assert_eq!(pipeline.depth(), 0);
    }
    
    #[test]
    fn test_backpressure_drops_old_data() {
        let pipeline = MarketPipeline::with_capacity(3);
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Fill the queue
        producer.push(MarketUpdate::new(1, 100.0, 101.0, 1000));
        producer.push(MarketUpdate::new(2, 200.0, 201.0, 2000));
        producer.push(MarketUpdate::new(3, 300.0, 301.0, 3000));
        
        assert_eq!(pipeline.depth(), 3);
        assert!(pipeline.is_full());
        
        // Push one more (should drop oldest)
        producer.push(MarketUpdate::new(4, 400.0, 401.0, 4000));
        
        assert_eq!(pipeline.depth(), 3);
        
        // Pop and verify we got 2, 3, 4 (1 was dropped)
        let update2 = consumer.pop().unwrap();
        assert_eq!(update2.symbol_id, 2);
        
        let update3 = consumer.pop().unwrap();
        assert_eq!(update3.symbol_id, 3);
        
        let update4 = consumer.pop().unwrap();
        assert_eq!(update4.symbol_id, 4);
        
        assert!(consumer.pop().is_none());
    }
    
    #[test]
    fn test_try_push_without_backpressure() {
        let pipeline = MarketPipeline::with_capacity(2);
        let producer = pipeline.producer();
        
        let update1 = MarketUpdate::new(1, 100.0, 101.0, 1000);
        let update2 = MarketUpdate::new(2, 200.0, 201.0, 2000);
        let update3 = MarketUpdate::new(3, 300.0, 301.0, 3000);
        
        assert!(producer.try_push(update1).is_ok());
        assert!(producer.try_push(update2).is_ok());
        
        // Queue full - should return error
        let result = producer.try_push(update3);
        assert!(result.is_err());
        
        let returned_update = result.unwrap_err();
        assert_eq!(returned_update.symbol_id, 3);
    }
    
    #[test]
    fn test_pop_batch() {
        let pipeline = MarketPipeline::new();
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Push 5 updates
        for i in 1..=5 {
            producer.push(MarketUpdate::new(i, 100.0 * i as f64, 101.0 * i as f64, i as u64 * 1000));
        }
        
        assert_eq!(pipeline.depth(), 5);
        
        // Pop batch of 3
        let batch = consumer.pop_batch(3);
        assert_eq!(batch.len(), 3);
        assert_eq!(batch[0].symbol_id, 1);
        assert_eq!(batch[1].symbol_id, 2);
        assert_eq!(batch[2].symbol_id, 3);
        
        assert_eq!(pipeline.depth(), 2);
        
        // Pop remaining
        let batch = consumer.pop_batch(10);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].symbol_id, 4);
        assert_eq!(batch[1].symbol_id, 5);
        
        assert_eq!(pipeline.depth(), 0);
    }
    
    #[test]
    fn test_metrics() {
        let pipeline = MarketPipeline::with_capacity(2);
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Push 3 updates (1 will be dropped)
        producer.push(MarketUpdate::new(1, 100.0, 101.0, 1000));
        producer.push(MarketUpdate::new(2, 200.0, 201.0, 2000));
        producer.push(MarketUpdate::new(3, 300.0, 301.0, 3000));
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.push_count, 3);
        assert_eq!(metrics.enqueue_count, 3);
        assert_eq!(metrics.drop_count, 1);
        assert_eq!(metrics.pop_count, 0);
        assert_eq!(metrics.queue_depth, 2);
        
        // Pop 1 update
        consumer.pop();
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.pop_count, 1);
        assert_eq!(metrics.queue_depth, 1);
    }
    
    #[test]
    fn test_metrics_drop_rate() {
        let pipeline = MarketPipeline::with_capacity(2);
        let producer = pipeline.producer();
        
        // Push 4 updates (2 will be dropped)
        for i in 1..=4 {
            producer.push(MarketUpdate::new(i, 100.0, 101.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.drop_count, 2);
        assert_eq!(metrics.push_count, 4);
        assert_eq!(metrics.drop_rate(), 50.0); // 2/4 = 50%
    }
    
    #[test]
    fn test_metrics_utilization() {
        let pipeline = MarketPipeline::with_capacity(10);
        let producer = pipeline.producer();
        
        // Push 5 updates (50% utilization)
        for i in 1..=5 {
            producer.push(MarketUpdate::new(i, 100.0, 101.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.utilization(), 50.0);
    }
    
    #[test]
    fn test_metrics_backpressure_detection() {
        let pipeline = MarketPipeline::with_capacity(10);
        let producer = pipeline.producer();
        
        // Push 7 updates (70% utilization - no backpressure)
        for i in 1..=7 {
            producer.push(MarketUpdate::new(i, 100.0, 101.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert!(!metrics.is_backpressure());
        
        // Push 2 more (90% utilization - backpressure!)
        for i in 8..=9 {
            producer.push(MarketUpdate::new(i, 100.0, 101.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert!(metrics.is_backpressure());
    }
}

/// Queue capacity for order execution: 1,000 orders
/// 
/// This is smaller than the market queue because order execution
/// is typically slower than market data ingestion. If we're generating
/// more than 1,000 pending orders, we have bigger problems.
const ORDER_QUEUE_CAPACITY: usize = 1_000;

/// Order execution pipeline with lock-free SPSC queue.
///
/// This structure manages the flow of order requests from the strategy thread
/// to the execution thread using a lock-free bounded queue.
///
/// # Architecture
///
/// ```text
/// Strategy Thread (Producer)     Execution Thread (Consumer)
///        │                                 │
///        ├─ submit() ────────────────────▶ pop()
///        │         SPSC Queue              │
///        │         (Lock-Free)             │
///        │                                 │
///        └─ Backpressure (drop old)        └─ Execute trade
/// ```
///
/// # Performance Characteristics
///
/// - **Submit**: ~10-20ns (lock-free, non-blocking)
/// - **Pop**: ~10-20ns (lock-free, non-blocking)
/// - **Memory**: 1,000 * 64 bytes = 64KB (fits in L1 cache)
/// - **Throughput**: >1M operations/second
///
/// # Thread Safety
///
/// - **Producer**: Single strategy thread
/// - **Consumer**: Single execution thread
/// - **Monitoring**: Multiple threads can read metrics (atomic counters)
///
/// Requirements: 3.1 (Lock-free queues), 14.3 (Bounded queues)
pub struct ExecutionPipeline {
    /// Lock-free SPSC queue for order requests
    queue: Arc<ArrayQueue<OrderRequest>>,
    
    /// Metrics: Total number of orders submitted (including dropped)
    submit_count: AtomicU64,
    _pad1: [u8; 56],
    
    /// Metrics: Total number of orders successfully enqueued
    enqueue_count: AtomicU64,
    _pad2: [u8; 56],
    
    /// Metrics: Total number of orders dropped due to backpressure
    drop_count: AtomicU64,
    _pad3: [u8; 56],
    
    /// Metrics: Total number of orders consumed
    pop_count: AtomicU64,
    _pad4: [u8; 56],
}

impl ExecutionPipeline {
    /// Create a new order execution pipeline with default capacity.
    ///
    /// # Performance
    ///
    /// - Allocation: One-time cost during initialization (cold path)
    /// - Memory: 64KB for queue + 32 bytes for metrics
    /// - Cache: Queue fits in L1 cache (typical 32KB-64KB)
    ///
    /// Requirement: 3.1 (SPSC queues)
    pub fn new() -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(ORDER_QUEUE_CAPACITY)),
            submit_count: AtomicU64::new(0),
            _pad1: [0; 56],
            enqueue_count: AtomicU64::new(0),
            _pad2: [0; 56],
            drop_count: AtomicU64::new(0),
            _pad3: [0; 56],
            pop_count: AtomicU64::new(0),
            _pad4: [0; 56],
        }
    }
    
    /// Create a new order execution pipeline with custom capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of pending orders to buffer
    ///
    /// # Performance
    ///
    /// Memory usage: capacity * 64 bytes
    ///
    /// Requirement: 14.3 (Bounded queues)
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(capacity)),
            submit_count: AtomicU64::new(0),
            _pad1: [0; 56],
            enqueue_count: AtomicU64::new(0),
            _pad2: [0; 56],
            drop_count: AtomicU64::new(0),
            _pad3: [0; 56],
            pop_count: AtomicU64::new(0),
            _pad4: [0; 56],
        }
    }
    
    /// Get a handle for the producer (strategy thread).
    ///
    /// This returns a lightweight handle that can be cloned and sent to
    /// the strategy thread for submitting order requests.
    pub fn producer(&self) -> OrderProducer {
        OrderProducer {
            queue: Arc::clone(&self.queue),
            submit_count: &self.submit_count,
            enqueue_count: &self.enqueue_count,
            drop_count: &self.drop_count,
        }
    }
    
    /// Get a handle for the consumer (execution thread).
    ///
    /// This returns a lightweight handle that can be sent to the
    /// execution thread for popping order requests.
    pub fn consumer(&self) -> OrderConsumer {
        OrderConsumer {
            queue: Arc::clone(&self.queue),
            pop_count: &self.pop_count,
        }
    }
    
    /// Get the current queue depth (number of pending orders).
    ///
    /// This is useful for monitoring and detecting backpressure.
    ///
    /// # Performance
    ///
    /// - Time: O(1) - just reads a counter
    /// - Accuracy: Approximate (lock-free, may be slightly stale)
    #[inline(always)]
    pub fn depth(&self) -> usize {
        self.queue.len()
    }
    
    /// Get the queue capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.queue.capacity()
    }
    
    /// Check if the queue is full.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.queue.is_full()
    }
    
    /// Check if the queue is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    
    /// Get pipeline metrics.
    ///
    /// Returns a snapshot of current metrics for monitoring.
    pub fn metrics(&self) -> ExecutionMetrics {
        ExecutionMetrics {
            submit_count: self.submit_count.load(Ordering::Relaxed),
            enqueue_count: self.enqueue_count.load(Ordering::Relaxed),
            drop_count: self.drop_count.load(Ordering::Relaxed),
            pop_count: self.pop_count.load(Ordering::Relaxed),
            queue_depth: self.depth(),
            queue_capacity: self.capacity(),
        }
    }
}

impl Default for ExecutionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Producer handle for submitting order requests (strategy thread).
///
/// This handle is Send + Sync and can be cloned to share across threads,
/// but should only be used by a single producer thread for optimal performance.
#[derive(Clone)]
pub struct OrderProducer {
    queue: Arc<ArrayQueue<OrderRequest>>,
    submit_count: *const AtomicU64,
    enqueue_count: *const AtomicU64,
    drop_count: *const AtomicU64,
}

// Safety: AtomicU64 is thread-safe, and we only use atomic operations
unsafe impl Send for OrderProducer {}
unsafe impl Sync for OrderProducer {}

impl OrderProducer {
    /// Submit an order request to the execution queue (non-blocking).
    ///
    /// This is the primary hot path function called by the strategy thread.
    /// It's marked `#[inline(always)]` to eliminate function call overhead.
    ///
    /// # Backpressure Strategy
    ///
    /// If the queue is full:
    /// 1. Pop the oldest order (drop old orders)
    /// 2. Submit the new order
    /// 3. Increment drop counter
    ///
    /// This ensures we always execute the most recent trading decisions.
    ///
    /// # Performance
    ///
    /// - Time: ~10-20ns (lock-free)
    /// - Allocations: Zero (pre-allocated queue)
    /// - Blocking: Never (non-blocking)
    ///
    /// Requirements: 3.1 (Lock-free), 14.3 (Backpressure)
    #[inline(always)]
    pub fn submit(&self, order: OrderRequest) {
        // Increment submit counter (Relaxed ordering is fine for metrics)
        unsafe {
            (*self.submit_count).fetch_add(1, Ordering::Relaxed);
        }
        
        // Try to push (non-blocking)
        if self.queue.push(order).is_err() {
            // Queue full - apply backpressure by dropping oldest
            self.queue.pop(); // Drop oldest order
            
            // Try again (should succeed now)
            if self.queue.push(order).is_ok() {
                unsafe {
                    (*self.enqueue_count).fetch_add(1, Ordering::Relaxed);
                }
            }
            
            // Increment drop counter
            unsafe {
                (*self.drop_count).fetch_add(1, Ordering::Relaxed);
            }
        } else {
            // Successfully enqueued
            unsafe {
                (*self.enqueue_count).fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    
    /// Try to submit an order without backpressure (returns error if full).
    ///
    /// This variant does NOT drop old orders if the queue is full.
    /// Use this if you want to handle backpressure differently.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if successfully submitted
    /// - `Err(order)` if queue is full (returns the order back)
    #[inline(always)]
    pub fn try_submit(
        &self,
        order: OrderRequest,
    ) -> Result<(), OrderRequest> {
        unsafe {
            (*self.submit_count).fetch_add(1, Ordering::Relaxed);
        }
        
        match self.queue.push(order) {
            Ok(()) => {
                unsafe {
                    (*self.enqueue_count).fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            }
            Err(order) => Err(order),
        }
    }
}

/// Consumer handle for popping order requests (execution thread).
///
/// This handle is Send + Sync and should only be used by a single
/// consumer thread for optimal performance.
pub struct OrderConsumer {
    queue: Arc<ArrayQueue<OrderRequest>>,
    pop_count: *const AtomicU64,
}

// Safety: AtomicU64 is thread-safe, and we only use atomic operations
unsafe impl Send for OrderConsumer {}
unsafe impl Sync for OrderConsumer {}

impl OrderConsumer {
    /// Pop an order request from the queue (non-blocking).
    ///
    /// This is the primary hot path function called by the execution thread.
    /// It's marked `#[inline(always)]` to eliminate function call overhead.
    ///
    /// # Returns
    ///
    /// - `Some(order)` if an order is available
    /// - `None` if the queue is empty
    ///
    /// # Performance
    ///
    /// - Time: ~10-20ns (lock-free)
    /// - Allocations: Zero (returns by value, Copy type)
    /// - Blocking: Never (non-blocking)
    ///
    /// Requirement: 3.1 (Lock-free)
    #[inline(always)]
    pub fn pop(&self) -> Option<OrderRequest> {
        match self.queue.pop() {
            Some(order) => {
                // Increment pop counter
                unsafe {
                    (*self.pop_count).fetch_add(1, Ordering::Relaxed);
                }
                Some(order)
            }
            None => None,
        }
    }
    
    /// Pop all available orders from the queue.
    ///
    /// This is useful for batch processing when the consumer falls behind.
    /// Returns a vector of all available orders (up to `max_batch` items).
    ///
    /// # Arguments
    ///
    /// * `max_batch` - Maximum number of orders to pop
    ///
    /// # Performance
    ///
    /// - Time: O(n) where n = number of items popped
    /// - Allocations: One Vec allocation (can be reused with clear())
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let consumer = pipeline.consumer();
    /// let orders = consumer.pop_batch(100);
    /// for order in orders {
    ///     execute_order(order);
    /// }
    /// ```
    pub fn pop_batch(&self, max_batch: usize) -> Vec<OrderRequest> {
        let mut batch = Vec::with_capacity(max_batch.min(self.queue.len()));
        
        for _ in 0..max_batch {
            match self.pop() {
                Some(order) => batch.push(order),
                None => break,
            }
        }
        
        batch
    }
}

/// Execution pipeline metrics for monitoring.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionMetrics {
    /// Total number of submit attempts
    pub submit_count: u64,
    
    /// Total number of successful enqueues
    pub enqueue_count: u64,
    
    /// Total number of dropped orders (backpressure)
    pub drop_count: u64,
    
    /// Total number of consumed orders
    pub pop_count: u64,
    
    /// Current queue depth
    pub queue_depth: usize,
    
    /// Queue capacity
    pub queue_capacity: usize,
}

impl ExecutionMetrics {
    /// Calculate the drop rate (percentage of orders dropped).
    pub fn drop_rate(&self) -> f64 {
        if self.submit_count == 0 {
            0.0
        } else {
            (self.drop_count as f64 / self.submit_count as f64) * 100.0
        }
    }
    
    /// Calculate queue utilization (percentage of capacity used).
    pub fn utilization(&self) -> f64 {
        (self.queue_depth as f64 / self.queue_capacity as f64) * 100.0
    }
    
    /// Check if the pipeline is experiencing backpressure.
    ///
    /// Returns true if:
    /// - Queue is >80% full, OR
    /// - Drop rate is >1%
    pub fn is_backpressure(&self) -> bool {
        self.utilization() > 80.0 || self.drop_rate() > 1.0
    }
}

    // ========================================================================
    // Execution Pipeline Tests
    // ========================================================================
    
    #[test]
    fn test_execution_pipeline_creation() {
        let pipeline = ExecutionPipeline::new();
        assert_eq!(pipeline.capacity(), ORDER_QUEUE_CAPACITY);
        assert_eq!(pipeline.depth(), 0);
        assert!(pipeline.is_empty());
        assert!(!pipeline.is_full());
    }
    
    #[test]
    fn test_order_submit_and_pop() {
        let pipeline = ExecutionPipeline::new();
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        let order = OrderRequest::market(1, 100, 1, 0, 1.5, 1000000);
        
        producer.submit(order);
        
        assert_eq!(pipeline.depth(), 1);
        
        let popped = consumer.pop().unwrap();
        assert_eq!(popped.order_id, 1);
        assert_eq!(popped.symbol_id, 100);
        assert_eq!(popped.exchange_id, 1);
        assert_eq!(popped.side, 0);
        assert_eq!(popped.size, 1.5);
        assert!(popped.is_market());
        assert!(popped.is_buy());
        
        assert_eq!(pipeline.depth(), 0);
    }
    
    #[test]
    fn test_limit_order_creation() {
        let order = OrderRequest::limit(2, 200, 2, 1, 50000.0, 2.0, 2000000);
        
        assert_eq!(order.order_id, 2);
        assert_eq!(order.symbol_id, 200);
        assert_eq!(order.exchange_id, 2);
        assert_eq!(order.side, 1);
        assert_eq!(order.price, 50000.0);
        assert_eq!(order.size, 2.0);
        assert!(order.is_limit());
        assert!(order.is_sell());
    }
    
    #[test]
    fn test_execution_backpressure_drops_old_orders() {
        let pipeline = ExecutionPipeline::with_capacity(3);
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Fill the queue
        producer.submit(OrderRequest::market(1, 100, 1, 0, 1.0, 1000));
        producer.submit(OrderRequest::market(2, 100, 1, 0, 2.0, 2000));
        producer.submit(OrderRequest::market(3, 100, 1, 0, 3.0, 3000));
        
        assert_eq!(pipeline.depth(), 3);
        assert!(pipeline.is_full());
        
        // Submit one more (should drop oldest)
        producer.submit(OrderRequest::market(4, 100, 1, 0, 4.0, 4000));
        
        assert_eq!(pipeline.depth(), 3);
        
        // Pop and verify we got 2, 3, 4 (1 was dropped)
        let order2 = consumer.pop().unwrap();
        assert_eq!(order2.order_id, 2);
        
        let order3 = consumer.pop().unwrap();
        assert_eq!(order3.order_id, 3);
        
        let order4 = consumer.pop().unwrap();
        assert_eq!(order4.order_id, 4);
        
        assert!(consumer.pop().is_none());
    }
    
    #[test]
    fn test_try_submit_without_backpressure() {
        let pipeline = ExecutionPipeline::with_capacity(2);
        let producer = pipeline.producer();
        
        let order1 = OrderRequest::market(1, 100, 1, 0, 1.0, 1000);
        let order2 = OrderRequest::market(2, 100, 1, 0, 2.0, 2000);
        let order3 = OrderRequest::market(3, 100, 1, 0, 3.0, 3000);
        
        assert!(producer.try_submit(order1).is_ok());
        assert!(producer.try_submit(order2).is_ok());
        
        // Queue full - should return error
        let result = producer.try_submit(order3);
        assert!(result.is_err());
        
        let returned_order = result.unwrap_err();
        assert_eq!(returned_order.order_id, 3);
    }
    
    #[test]
    fn test_order_pop_batch() {
        let pipeline = ExecutionPipeline::new();
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Submit 5 orders
        for i in 1..=5 {
            producer.submit(OrderRequest::market(i, 100, 1, 0, i as f64, i * 1000));
        }
        
        assert_eq!(pipeline.depth(), 5);
        
        // Pop batch of 3
        let batch = consumer.pop_batch(3);
        assert_eq!(batch.len(), 3);
        assert_eq!(batch[0].order_id, 1);
        assert_eq!(batch[1].order_id, 2);
        assert_eq!(batch[2].order_id, 3);
        
        assert_eq!(pipeline.depth(), 2);
        
        // Pop remaining
        let batch = consumer.pop_batch(10);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].order_id, 4);
        assert_eq!(batch[1].order_id, 5);
        
        assert_eq!(pipeline.depth(), 0);
    }
    
    #[test]
    fn test_execution_metrics() {
        let pipeline = ExecutionPipeline::with_capacity(2);
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Submit 3 orders (1 will be dropped)
        producer.submit(OrderRequest::market(1, 100, 1, 0, 1.0, 1000));
        producer.submit(OrderRequest::market(2, 100, 1, 0, 2.0, 2000));
        producer.submit(OrderRequest::market(3, 100, 1, 0, 3.0, 3000));
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.submit_count, 3);
        assert_eq!(metrics.enqueue_count, 3);
        assert_eq!(metrics.drop_count, 1);
        assert_eq!(metrics.pop_count, 0);
        assert_eq!(metrics.queue_depth, 2);
        
        // Pop 1 order
        consumer.pop();
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.pop_count, 1);
        assert_eq!(metrics.queue_depth, 1);
    }
    
    #[test]
    fn test_execution_metrics_drop_rate() {
        let pipeline = ExecutionPipeline::with_capacity(2);
        let producer = pipeline.producer();
        
        // Submit 4 orders (2 will be dropped)
        for i in 1..=4 {
            producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.drop_count, 2);
        assert_eq!(metrics.submit_count, 4);
        assert_eq!(metrics.drop_rate(), 50.0); // 2/4 = 50%
    }
    
    #[test]
    fn test_execution_metrics_utilization() {
        let pipeline = ExecutionPipeline::with_capacity(10);
        let producer = pipeline.producer();
        
        // Submit 5 orders (50% utilization)
        for i in 1..=5 {
            producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert_eq!(metrics.utilization(), 50.0);
    }
    
    #[test]
    fn test_execution_metrics_backpressure_detection() {
        let pipeline = ExecutionPipeline::with_capacity(10);
        let producer = pipeline.producer();
        
        // Submit 7 orders (70% utilization - no backpressure)
        for i in 1..=7 {
            producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert!(!metrics.is_backpressure());
        
        // Submit 2 more (90% utilization - backpressure!)
        for i in 8..=9 {
            producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, 1000));
        }
        
        let metrics = pipeline.metrics();
        assert!(metrics.is_backpressure());
    }
    
    #[test]
    fn test_order_request_size() {
        // Verify OrderRequest is exactly 64 bytes (cache line aligned)
        assert_eq!(std::mem::size_of::<OrderRequest>(), 64);
        assert_eq!(std::mem::align_of::<OrderRequest>(), 64);
    }
    
    #[test]
    fn test_mixed_order_types() {
        let pipeline = ExecutionPipeline::new();
        let producer = pipeline.producer();
        let consumer = pipeline.consumer();
        
        // Submit mix of market and limit orders
        producer.submit(OrderRequest::market(1, 100, 1, 0, 1.0, 1000));
        producer.submit(OrderRequest::limit(2, 200, 2, 1, 50000.0, 2.0, 2000));
        producer.submit(OrderRequest::market(3, 300, 3, 1, 3.0, 3000));
        
        // Pop and verify
        let order1 = consumer.pop().unwrap();
        assert!(order1.is_market());
        assert!(order1.is_buy());
        
        let order2 = consumer.pop().unwrap();
        assert!(order2.is_limit());
        assert!(order2.is_sell());
        assert_eq!(order2.price, 50000.0);
        
        let order3 = consumer.pop().unwrap();
        assert!(order3.is_market());
        assert!(order3.is_sell());
    }
