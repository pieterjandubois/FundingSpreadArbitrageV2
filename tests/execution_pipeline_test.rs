/// Integration tests for the execution pipeline (Task 17)
///
/// This test file verifies the lock-free SPSC queue for order execution.
///
/// Requirements: 3.1 (Lock-free queues), 14.3 (Backpressure)

use arbitrage2::strategy::pipeline::ExecutionPipeline;
use arbitrage2::strategy::types::OrderRequest;

#[test]
fn test_execution_pipeline_basic_flow() {
    let pipeline = ExecutionPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Create a market order
    let order = OrderRequest::market(1, 100, 1, 0, 1.5, 1000000);
    
    // Submit order
    producer.submit(order);
    
    // Verify queue has 1 order
    assert_eq!(pipeline.depth(), 1);
    
    // Pop order
    let popped = consumer.pop().unwrap();
    assert_eq!(popped.order_id, 1);
    assert_eq!(popped.symbol_id, 100);
    assert!(popped.is_market());
    assert!(popped.is_buy());
    
    // Queue should be empty
    assert_eq!(pipeline.depth(), 0);
    assert!(consumer.pop().is_none());
}

#[test]
fn test_execution_pipeline_limit_orders() {
    let pipeline = ExecutionPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Create a limit order
    let order = OrderRequest::limit(2, 200, 2, 1, 50000.0, 2.0, 2000000);
    
    producer.submit(order);
    
    let popped = consumer.pop().unwrap();
    assert_eq!(popped.order_id, 2);
    assert_eq!(popped.price, 50000.0);
    assert_eq!(popped.size, 2.0);
    assert!(popped.is_limit());
    assert!(popped.is_sell());
}

#[test]
fn test_execution_pipeline_backpressure() {
    // Small queue to test backpressure
    let pipeline = ExecutionPipeline::with_capacity(3);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Fill the queue
    for i in 1..=3 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, i * 1000));
    }
    
    assert_eq!(pipeline.depth(), 3);
    assert!(pipeline.is_full());
    
    // Submit one more - should drop oldest
    producer.submit(OrderRequest::market(4, 100, 1, 0, 1.0, 4000));
    
    // Still 3 items, but oldest was dropped
    assert_eq!(pipeline.depth(), 3);
    
    // Verify we get orders 2, 3, 4 (1 was dropped)
    assert_eq!(consumer.pop().unwrap().order_id, 2);
    assert_eq!(consumer.pop().unwrap().order_id, 3);
    assert_eq!(consumer.pop().unwrap().order_id, 4);
    assert!(consumer.pop().is_none());
}

#[test]
fn test_execution_pipeline_metrics() {
    let pipeline = ExecutionPipeline::with_capacity(2);
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Submit 3 orders (1 will be dropped due to backpressure)
    for i in 1..=3 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, i * 1000));
    }
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.submit_count, 3);
    assert_eq!(metrics.enqueue_count, 3);
    assert_eq!(metrics.drop_count, 1); // One order was dropped
    assert_eq!(metrics.pop_count, 0);
    assert_eq!(metrics.queue_depth, 2);
    
    // Pop one order
    consumer.pop();
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.pop_count, 1);
    assert_eq!(metrics.queue_depth, 1);
    
    // Check drop rate
    let drop_rate = metrics.drop_rate();
    assert!((drop_rate - 33.333333).abs() < 0.001); // ~33.33% (1/3)
}

#[test]
fn test_execution_pipeline_batch_processing() {
    let pipeline = ExecutionPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Submit 10 orders
    for i in 1..=10 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, i as f64, i * 1000));
    }
    
    assert_eq!(pipeline.depth(), 10);
    
    // Pop batch of 5
    let batch = consumer.pop_batch(5);
    assert_eq!(batch.len(), 5);
    assert_eq!(batch[0].order_id, 1);
    assert_eq!(batch[4].order_id, 5);
    
    assert_eq!(pipeline.depth(), 5);
    
    // Pop remaining
    let batch = consumer.pop_batch(10);
    assert_eq!(batch.len(), 5);
    assert_eq!(batch[0].order_id, 6);
    assert_eq!(batch[4].order_id, 10);
    
    assert_eq!(pipeline.depth(), 0);
}

#[test]
fn test_execution_pipeline_try_submit() {
    let pipeline = ExecutionPipeline::with_capacity(2);
    let producer = pipeline.producer();
    
    let order1 = OrderRequest::market(1, 100, 1, 0, 1.0, 1000);
    let order2 = OrderRequest::market(2, 100, 1, 0, 2.0, 2000);
    let order3 = OrderRequest::market(3, 100, 1, 0, 3.0, 3000);
    
    // First two should succeed
    assert!(producer.try_submit(order1).is_ok());
    assert!(producer.try_submit(order2).is_ok());
    
    // Third should fail (queue full)
    let result = producer.try_submit(order3);
    assert!(result.is_err());
    
    // Verify we get the order back
    let returned = result.unwrap_err();
    assert_eq!(returned.order_id, 3);
}

#[test]
fn test_order_request_alignment() {
    // Verify OrderRequest is cache-line aligned (64 bytes)
    assert_eq!(std::mem::size_of::<OrderRequest>(), 64);
    assert_eq!(std::mem::align_of::<OrderRequest>(), 64);
}

#[test]
fn test_execution_pipeline_high_throughput() {
    let pipeline = ExecutionPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Submit 1000 orders
    for i in 1..=1000 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, i * 1000));
    }
    
    assert_eq!(pipeline.depth(), 1000);
    
    // Pop all orders
    let mut count = 0;
    while consumer.pop().is_some() {
        count += 1;
    }
    
    assert_eq!(count, 1000);
    assert_eq!(pipeline.depth(), 0);
    
    let metrics = pipeline.metrics();
    assert_eq!(metrics.submit_count, 1000);
    assert_eq!(metrics.pop_count, 1000);
    assert_eq!(metrics.drop_count, 0); // No drops with default capacity
}

#[test]
fn test_execution_metrics_backpressure_detection() {
    let pipeline = ExecutionPipeline::with_capacity(10);
    let producer = pipeline.producer();
    
    // Fill to 70% - no backpressure
    for i in 1..=7 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, i * 1000));
    }
    
    let metrics = pipeline.metrics();
    assert!(!metrics.is_backpressure());
    assert_eq!(metrics.utilization(), 70.0);
    
    // Fill to 90% - backpressure detected
    for i in 8..=9 {
        producer.submit(OrderRequest::market(i, 100, 1, 0, 1.0, i * 1000));
    }
    
    let metrics = pipeline.metrics();
    assert!(metrics.is_backpressure());
    assert_eq!(metrics.utilization(), 90.0);
}

#[test]
fn test_mixed_order_types_in_pipeline() {
    let pipeline = ExecutionPipeline::new();
    let producer = pipeline.producer();
    let consumer = pipeline.consumer();
    
    // Submit mix of market and limit orders
    producer.submit(OrderRequest::market(1, 100, 1, 0, 1.0, 1000));
    producer.submit(OrderRequest::limit(2, 200, 2, 1, 50000.0, 2.0, 2000));
    producer.submit(OrderRequest::market(3, 300, 3, 1, 3.0, 3000));
    producer.submit(OrderRequest::limit(4, 400, 4, 0, 60000.0, 4.0, 4000));
    
    // Pop and verify order types
    let order1 = consumer.pop().unwrap();
    assert!(order1.is_market() && order1.is_buy());
    
    let order2 = consumer.pop().unwrap();
    assert!(order2.is_limit() && order2.is_sell());
    assert_eq!(order2.price, 50000.0);
    
    let order3 = consumer.pop().unwrap();
    assert!(order3.is_market() && order3.is_sell());
    
    let order4 = consumer.pop().unwrap();
    assert!(order4.is_limit() && order4.is_buy());
    assert_eq!(order4.price, 60000.0);
}
