# Buffer Pool Usage Guide

## Overview

The `BufferPool` module provides pre-allocated buffers to eliminate heap allocations in hot paths, implementing Requirements 12.1, 12.2, and 12.3 from the low-latency optimization spec.

## Features

### 1. Pre-Allocated String Buffers (Requirement 12.1, 12.2)

The buffer pool pre-allocates 100 String buffers with 256 bytes capacity each during initialization:

```rust
use arbitrage2::strategy::buffer_pool::BufferPool;

let mut pool = BufferPool::new();
assert_eq!(pool.buffer_count(), 100);
assert_eq!(pool.buffer_capacity(), 256);
```

### 2. Buffer Reuse with clear() (Requirement 12.3)

Buffers are reused via `clear()` which maintains the heap allocation while resetting content:

```rust
use std::fmt::Write;

let mut pool = BufferPool::new();

// Get a buffer and use it
let buf = pool.get_string_buffer();
write!(buf, "Price: {:.2}", 123.45).unwrap();

// Buffer is automatically cleared on next get
let buf2 = pool.get_string_buffer();
assert!(buf2.is_empty()); // Cleared, but heap allocation maintained
```

### 3. SmallVec for Small Collections (Requirement 12.2)

SmallVec stores up to 8 items on the stack, avoiding heap allocation:

```rust
use arbitrage2::strategy::buffer_pool::create_small_vec;

let mut prices = create_small_vec::<f64>();

// Add up to 8 items - stays on stack
for i in 0..8 {
    prices.push(100.0 + i as f64);
}

assert!(!prices.spilled()); // No heap allocation
```

### 4. Thread-Local Buffer Pool

For zero-contention access in hot paths, use the thread-local buffer pool:

```rust
use arbitrage2::strategy::buffer_pool::with_string_buffer;
use std::fmt::Write;

let formatted = with_string_buffer(|buf| {
    write!(buf, "Order #{}: {:.2}", 123, 456.78).unwrap();
    buf.clone()
});
```

## Hot Path Usage Examples

### Example 1: Market Data Formatting

```rust
use arbitrage2::strategy::buffer_pool::with_string_buffer;
use std::fmt::Write;

fn format_market_update(symbol: &str, bid: f64, ask: f64) -> String {
    with_string_buffer(|buf| {
        write!(buf, "{}: BID={:.2} ASK={:.2}", symbol, bid, ask).unwrap();
        buf.clone()
    })
}
```

### Example 2: Order Message Construction

```rust
use arbitrage2::strategy::buffer_pool::BufferPool;
use std::fmt::Write;

fn build_order_message(pool: &mut BufferPool, order_id: u64, price: f64, qty: f64) -> String {
    let buf = pool.get_string_buffer();
    write!(buf, "{{\"order_id\":{},\"price\":{:.2},\"qty\":{:.4}}}", 
           order_id, price, qty).unwrap();
    buf.clone()
}
```

### Example 3: Small Collections

```rust
use arbitrage2::strategy::buffer_pool::create_small_vec;

fn collect_top_opportunities(opportunities: &[Opportunity]) -> Vec<Opportunity> {
    let mut top = create_small_vec::<Opportunity>();
    
    for opp in opportunities.iter().take(5) {
        top.push(opp.clone());
    }
    
    top.into_vec() // Convert to Vec if needed
}
```

## Performance Characteristics

### Zero Allocations in Hot Path

After initialization, the buffer pool performs zero heap allocations:

```rust
let mut pool = BufferPool::new();

// First pass - uses pre-allocated buffers
for i in 0..100 {
    let buf = pool.get_string_buffer();
    write!(buf, "Message {}", i).unwrap();
}

// Second pass - reuses same buffers (ZERO allocations)
for i in 0..100 {
    let buf = pool.get_string_buffer();
    write!(buf, "Message {}", i).unwrap();
}
```

### Round-Robin Buffer Access

Buffers are accessed in round-robin fashion:

```rust
let mut pool = BufferPool::new();

// Get 5 different buffers
for i in 0..5 {
    let buf = pool.get_string_buffer();
    // Each buffer is unique
}

// After 100 gets, we cycle back to the first buffer
```

## Integration with Existing Code

### Replacing String::new()

**Before:**
```rust
let mut message = String::new();
write!(message, "Price: {:.2}", price).unwrap();
```

**After:**
```rust
let message = with_string_buffer(|buf| {
    write!(buf, "Price: {:.2}", price).unwrap();
    buf.clone()
});
```

### Replacing Vec::new()

**Before:**
```rust
let mut items = Vec::new();
items.push(1);
items.push(2);
```

**After:**
```rust
let mut items = create_small_vec::<i32>();
items.push(1);
items.push(2);
// No heap allocation if ≤8 items
```

## Testing

Run the buffer pool tests:

```bash
cargo test buffer_pool
```

Run the demo:

```bash
cargo run --example buffer_pool_demo
```

## Benchmarking

To verify zero allocations in hot paths, use cargo-flamegraph:

```bash
cargo flamegraph --example buffer_pool_demo
```

Look for:
- Zero `malloc` calls in hot path
- Zero `realloc` calls in hot path
- Consistent memory usage across iterations

## Requirements Mapping

| Requirement | Implementation | Verification |
|-------------|----------------|--------------|
| 12.1 | Pre-allocate Vec with `with_capacity()` | `BufferPool::new()` pre-allocates 100 buffers |
| 12.2 | Pre-allocate String buffers + SmallVec | 256-byte String buffers + SmallVec<[T; 8]> |
| 12.3 | Buffer reuse with `clear()` | `get_string_buffer()` calls `clear()` before returning |

## Best Practices

1. **Use thread-local pool for single-threaded hot paths**: Eliminates synchronization overhead
2. **Use SmallVec for collections with known small size**: Avoids heap allocation for ≤8 items
3. **Clone only when necessary**: If you need to keep the string, clone it; otherwise use it in-place
4. **Monitor buffer pool size**: If you run out of buffers, increase the pool size in `BufferPool::new()`
5. **Profile regularly**: Use flamegraph to verify zero allocations

## Limitations

1. **Buffer size**: Fixed at 256 bytes. Larger strings will cause reallocation.
2. **Pool size**: Fixed at 100 buffers. More concurrent uses will cycle through buffers.
3. **SmallVec size**: Fixed at 8 items. More items will spill to heap.

To adjust these limits, modify the constants in `BufferPool::new()`.

## Future Enhancements

- [ ] Configurable buffer size and pool size
- [ ] Per-thread pool statistics
- [ ] Automatic pool size adjustment based on usage
- [ ] Support for different SmallVec sizes
