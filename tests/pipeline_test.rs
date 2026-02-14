use arbitrage2::strategy::pipeline::{MarketPipeline, PipelineMetrics};
use arbitrage2::strategy::types::MarketUpdate;

#[test]
fn test_pipeline_creation() {
    let pipeline = MarketPipeline::new();
    assert_eq!(pipeline.capacity(), 10_000);
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
