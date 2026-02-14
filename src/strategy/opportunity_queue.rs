use crate::strategy::types::ArbitrageOpportunity;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Lock-free MPSC queue for distributing opportunities to multiple consumers.
///
/// This queue uses crossbeam's ArrayQueue for lock-free operations and supports
/// multiple consumers reading from the same queue independently.
///
/// # Performance Characteristics
///
/// - Lock-free push/pop operations (no mutexes)
/// - Handles 10K+ opportunities/sec
/// - Backpressure handling: drops oldest when full
/// - Multiple consumers work independently
///
/// # Architecture
///
/// ```text
/// OpportunityDetector → OpportunityProducer → ArrayQueue → OpportunityConsumer → Strategy
///                                                        → OpportunityConsumer → Dashboard
/// ```
///
/// Requirements: Streaming Opportunity Detection 1.2
pub struct OpportunityQueue {
    queue: Arc<ArrayQueue<ArbitrageOpportunity>>,
    push_count: Arc<AtomicU64>,
    pop_count: Arc<AtomicU64>,
    drop_count: Arc<AtomicU64>,
}

impl OpportunityQueue {
    /// Create a new opportunity queue with default capacity (1024).
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }
    
    /// Create a new opportunity queue with specified capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of opportunities to store
    ///
    /// # Performance
    ///
    /// - Capacity should be power of 2 for optimal performance
    /// - Recommended: 1024 for most use cases
    /// - Higher capacity reduces drop rate under high load
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(capacity)),
            push_count: Arc::new(AtomicU64::new(0)),
            pop_count: Arc::new(AtomicU64::new(0)),
            drop_count: Arc::new(AtomicU64::new(0)),
        }
    }
    
    /// Get a producer handle for pushing opportunities.
    ///
    /// Multiple producers can be created, but typically only one
    /// (OpportunityDetector) will push to the queue.
    pub fn producer(&self) -> OpportunityProducer {
        OpportunityProducer {
            queue: self.queue.clone(),
            push_count: self.push_count.clone(),
            drop_count: self.drop_count.clone(),
        }
    }
    
    /// Get a consumer handle for popping opportunities.
    ///
    /// Multiple consumers can be created (e.g., strategy runner and dashboard).
    /// Each consumer will compete for opportunities in the queue.
    pub fn consumer(&self) -> OpportunityConsumer {
        OpportunityConsumer {
            queue: self.queue.clone(),
            pop_count: self.pop_count.clone(),
        }
    }
    
    /// Get the total number of opportunities pushed to the queue.
    pub fn push_count(&self) -> u64 {
        self.push_count.load(Ordering::Relaxed)
    }
    
    /// Get the total number of opportunities popped from the queue.
    pub fn pop_count(&self) -> u64 {
        self.pop_count.load(Ordering::Relaxed)
    }
    
    /// Get the total number of opportunities dropped due to backpressure.
    pub fn drop_count(&self) -> u64 {
        self.drop_count.load(Ordering::Relaxed)
    }
    
    /// Get the current number of opportunities in the queue.
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    
    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Default for OpportunityQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Producer handle for pushing opportunities to the queue.
///
/// This handle can be cloned and sent across threads safely.
/// Typically used by OpportunityDetector service.
pub struct OpportunityProducer {
    queue: Arc<ArrayQueue<ArbitrageOpportunity>>,
    push_count: Arc<AtomicU64>,
    drop_count: Arc<AtomicU64>,
}

impl OpportunityProducer {
    /// Push an opportunity to the queue with backpressure handling.
    ///
    /// If the queue is full, this will drop the oldest opportunity
    /// and push the new one. This ensures the queue always contains
    /// the most recent opportunities.
    ///
    /// # Arguments
    ///
    /// * `opportunity` - The opportunity to push
    ///
    /// # Performance
    ///
    /// - Lock-free operation
    /// - O(1) time complexity
    /// - No allocations
    ///
    /// # Backpressure
    ///
    /// When the queue is full:
    /// 1. Pop the oldest opportunity (drop it)
    /// 2. Push the new opportunity
    /// 3. Increment drop counter
    ///
    /// This ensures consumers always see the latest opportunities.
    pub fn push(&self, opportunity: ArbitrageOpportunity) {
        self.push_count.fetch_add(1, Ordering::Relaxed);
        
        if let Err(rejected) = self.queue.push(opportunity) {
            // Queue is full - drop oldest and retry
            self.queue.pop();
            self.drop_count.fetch_add(1, Ordering::Relaxed);
            
            // Retry push (should succeed now)
            let _ = self.queue.push(rejected);
        }
    }
}

impl Clone for OpportunityProducer {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            push_count: self.push_count.clone(),
            drop_count: self.drop_count.clone(),
        }
    }
}

/// Consumer handle for popping opportunities from the queue.
///
/// This handle can be cloned and sent across threads safely.
/// Multiple consumers will compete for opportunities (MPSC pattern).
pub struct OpportunityConsumer {
    queue: Arc<ArrayQueue<ArbitrageOpportunity>>,
    pop_count: Arc<AtomicU64>,
}

impl OpportunityConsumer {
    /// Pop a single opportunity from the queue (non-blocking).
    ///
    /// Returns `None` if the queue is empty.
    ///
    /// # Performance
    ///
    /// - Lock-free operation
    /// - O(1) time complexity
    /// - No allocations
    ///
    /// # Example
    ///
    /// ```ignore
    /// let consumer = queue.consumer();
    /// if let Some(opportunity) = consumer.pop() {
    ///     // Process opportunity
    /// }
    /// ```
    pub fn pop(&self) -> Option<ArbitrageOpportunity> {
        let opp = self.queue.pop();
        if opp.is_some() {
            self.pop_count.fetch_add(1, Ordering::Relaxed);
        }
        opp
    }
    
    /// Pop a batch of opportunities from the queue (non-blocking).
    ///
    /// Returns a vector of up to `max_batch` opportunities.
    /// If fewer opportunities are available, returns what's available.
    ///
    /// # Arguments
    ///
    /// * `max_batch` - Maximum number of opportunities to pop
    ///
    /// # Performance
    ///
    /// - Lock-free operations
    /// - O(n) where n = min(max_batch, queue.len())
    /// - Single allocation for the vector
    ///
    /// # Example
    ///
    /// ```ignore
    /// let consumer = queue.consumer();
    /// let batch = consumer.pop_batch(100);
    /// for opportunity in batch {
    ///     // Process opportunity
    /// }
    /// ```
    pub fn pop_batch(&self, max_batch: usize) -> Vec<ArbitrageOpportunity> {
        let mut batch = Vec::with_capacity(max_batch);
        for _ in 0..max_batch {
            if let Some(opp) = self.pop() {
                batch.push(opp);
            } else {
                break;
            }
        }
        batch
    }
}

impl Clone for OpportunityConsumer {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            pop_count: self.pop_count.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::types::{ConfluenceMetrics, HardConstraints};
    
    fn create_test_opportunity(symbol: &str, spread_bps: f64) -> ArbitrageOpportunity {
        ArbitrageOpportunity {
            symbol: symbol.to_string(),
            long_exchange: "bybit".to_string(),
            short_exchange: "okx".to_string(),
            long_price: 50000.0,
            short_price: 50100.0,
            spread_bps,
            funding_delta_8h: 0.0001,
            confidence_score: 80,
            projected_profit_usd: 10.0,
            projected_profit_after_slippage: 8.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0001,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000000.0,
                oi_24h_avg: 900000.0,
                vwap_deviation: 0.5,
                atr: 100.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 10000.0,
            order_book_depth_short: 10000.0,
            timestamp: Some(1234567890),
        }
    }
    
    #[test]
    fn test_push_and_pop() {
        let queue = OpportunityQueue::new();
        let producer = queue.producer();
        let consumer = queue.consumer();
        
        // Push an opportunity
        let opp = create_test_opportunity("BTCUSDT", 15.0);
        producer.push(opp.clone());
        
        // Verify metrics
        assert_eq!(queue.push_count(), 1);
        assert_eq!(queue.len(), 1);
        
        // Pop the opportunity
        let popped = consumer.pop().expect("Should pop opportunity");
        assert_eq!(popped.symbol, "BTCUSDT");
        assert_eq!(popped.spread_bps, 15.0);
        
        // Verify metrics
        assert_eq!(queue.pop_count(), 1);
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }
    
    #[test]
    fn test_backpressure_drops_oldest() {
        let queue = OpportunityQueue::with_capacity(2);
        let producer = queue.producer();
        let consumer = queue.consumer();
        
        // Fill the queue
        producer.push(create_test_opportunity("BTC1", 10.0));
        producer.push(create_test_opportunity("BTC2", 20.0));
        
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.drop_count(), 0);
        
        // Push a third opportunity (should drop oldest)
        producer.push(create_test_opportunity("BTC3", 30.0));
        
        // Verify drop count
        assert_eq!(queue.drop_count(), 1);
        assert_eq!(queue.len(), 2);
        
        // Pop and verify we get BTC2 and BTC3 (BTC1 was dropped)
        let opp1 = consumer.pop().unwrap();
        let opp2 = consumer.pop().unwrap();
        
        assert_eq!(opp1.symbol, "BTC2");
        assert_eq!(opp2.symbol, "BTC3");
        assert!(consumer.pop().is_none());
    }
    
    #[test]
    fn test_multiple_consumers() {
        let queue = OpportunityQueue::new();
        let producer = queue.producer();
        let consumer1 = queue.consumer();
        let consumer2 = queue.consumer();
        
        // Push multiple opportunities
        for i in 0..10 {
            producer.push(create_test_opportunity(&format!("BTC{}", i), 10.0 + i as f64));
        }
        
        assert_eq!(queue.len(), 10);
        
        // Both consumers can pop independently
        let opp1 = consumer1.pop().expect("Consumer 1 should pop");
        let opp2 = consumer2.pop().expect("Consumer 2 should pop");
        
        // They should get different opportunities (competing for the queue)
        assert_ne!(opp1.symbol, opp2.symbol);
        
        // Total pops should be 2
        assert_eq!(queue.pop_count(), 2);
        assert_eq!(queue.len(), 8);
    }
    
    #[test]
    fn test_pop_batch() {
        let queue = OpportunityQueue::new();
        let producer = queue.producer();
        let consumer = queue.consumer();
        
        // Push 5 opportunities
        for i in 0..5 {
            producer.push(create_test_opportunity(&format!("BTC{}", i), 10.0 + i as f64));
        }
        
        // Pop batch of 3
        let batch = consumer.pop_batch(3);
        assert_eq!(batch.len(), 3);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop_count(), 3);
        
        // Pop batch of 10 (should only get 2)
        let batch2 = consumer.pop_batch(10);
        assert_eq!(batch2.len(), 2);
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.pop_count(), 5);
    }
    
    #[test]
    fn test_metrics_accuracy() {
        let queue = OpportunityQueue::with_capacity(2);
        let producer = queue.producer();
        let consumer = queue.consumer();
        
        // Push 5 opportunities (capacity is 2, so 3 should be dropped)
        for i in 0..5 {
            producer.push(create_test_opportunity(&format!("BTC{}", i), 10.0));
        }
        
        assert_eq!(queue.push_count(), 5);
        assert_eq!(queue.drop_count(), 3);
        assert_eq!(queue.len(), 2);
        
        // Pop all
        consumer.pop();
        consumer.pop();
        
        assert_eq!(queue.pop_count(), 2);
        assert_eq!(queue.len(), 0);
        
        // Final metrics
        assert_eq!(queue.push_count(), 5);
        assert_eq!(queue.pop_count(), 2);
        assert_eq!(queue.drop_count(), 3);
    }
    
    #[test]
    #[ignore] // Run with --ignored flag for performance testing
    fn test_throughput_10k_per_second() {
        use std::time::Instant;
        
        let queue = OpportunityQueue::with_capacity(10000);
        let producer = queue.producer();
        let consumer = queue.consumer();
        
        // Test push throughput
        let push_count = 20000;
        let start = Instant::now();
        
        for i in 0..push_count {
            let opp = create_test_opportunity(&format!("BTC{}", i % 100), 10.0 + (i % 50) as f64);
            producer.push(opp);
        }
        
        let push_duration = start.elapsed();
        let push_per_sec = (push_count as f64 / push_duration.as_secs_f64()) as u64;
        
        println!("Push throughput: {} ops/sec", push_per_sec);
        println!("Push duration: {:?} for {} operations", push_duration, push_count);
        
        // Verify we can handle 10K+ per second
        assert!(push_per_sec > 10_000, "Push throughput {} is below 10K/sec", push_per_sec);
        
        // Test pop throughput
        let start = Instant::now();
        let mut pop_count = 0;
        
        while consumer.pop().is_some() {
            pop_count += 1;
        }
        
        let pop_duration = start.elapsed();
        let pop_per_sec = (pop_count as f64 / pop_duration.as_secs_f64()) as u64;
        
        println!("Pop throughput: {} ops/sec", pop_per_sec);
        println!("Pop duration: {:?} for {} operations", pop_duration, pop_count);
        
        // Verify we can handle 10K+ per second
        assert!(pop_per_sec > 10_000, "Pop throughput {} is below 10K/sec", pop_per_sec);
    }
}
