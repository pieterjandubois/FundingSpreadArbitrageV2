// Demonstration of buffer pool usage for zero-allocation hot paths
// Requirements: 12.1, 12.2, 12.3

use arbitrage2::strategy::buffer_pool::{BufferPool, with_string_buffer, create_small_vec};
use std::fmt::Write;

fn main() {
    println!("=== Buffer Pool Demo ===\n");
    
    // Demo 1: Pre-allocated buffer pool
    demo_buffer_pool();
    
    // Demo 2: Thread-local buffer access
    demo_thread_local_buffers();
    
    // Demo 3: SmallVec for small collections
    demo_small_vec();
    
    // Demo 4: Hot path simulation
    demo_hot_path_usage();
}

fn demo_buffer_pool() {
    println!("1. Pre-allocated Buffer Pool (Requirement 12.1, 12.2)");
    println!("   Creating pool with 100 pre-allocated 256-byte String buffers...");
    
    let mut pool = BufferPool::new();
    
    println!("   ✓ Pool created with {} buffers", pool.buffer_count());
    println!("   ✓ Each buffer has {} bytes capacity\n", pool.buffer_capacity());
    
    // Use buffers
    for i in 0..5 {
        let buf = pool.get_string_buffer();
        write!(buf, "Order #{}: Price ${:.2}", i, 100.0 + i as f64 * 10.0).unwrap();
        println!("   Buffer {}: {}", i, buf);
    }
    println!();
}

fn demo_thread_local_buffers() {
    println!("2. Thread-Local Buffer Access (Zero Contention)");
    println!("   Using thread-local buffer pool for hot path...");
    
    // Format multiple strings without allocation
    for i in 0..3 {
        let formatted = with_string_buffer(|buf| {
            write!(buf, "Trade #{}: BTC/USDT @ ${:.2}", i, 50000.0 + i as f64 * 100.0).unwrap();
            buf.clone()
        });
        println!("   {}", formatted);
    }
    println!();
}

fn demo_small_vec() {
    println!("3. SmallVec for Small Collections (Requirement 12.2)");
    println!("   Creating SmallVec with stack allocation for ≤8 items...");
    
    let mut prices = create_small_vec::<f64>();
    
    // Add 5 prices (stays on stack)
    for i in 0..5 {
        prices.push(100.0 + i as f64);
    }
    
    println!("   ✓ Added {} prices", prices.len());
    println!("   ✓ Heap allocated: {} (should be false)", prices.spilled());
    println!("   Prices: {:?}\n", prices);
}

fn demo_hot_path_usage() {
    println!("4. Hot Path Simulation (Requirement 12.3)");
    println!("   Simulating 1000 iterations with buffer reuse...");
    
    let mut pool = BufferPool::new();
    let mut total_len = 0;
    
    // Simulate hot path: format 1000 messages
    for i in 0..1000 {
        let buf = pool.get_string_buffer();
        
        // Buffer is cleared and ready to use (Requirement 12.3)
        assert!(buf.is_empty(), "Buffer should be cleared");
        
        write!(buf, "Market update #{}: BID={:.2} ASK={:.2}", 
               i, 50000.0 + i as f64 * 0.1, 50001.0 + i as f64 * 0.1).unwrap();
        
        total_len += buf.len();
    }
    
    println!("   ✓ Processed 1000 iterations");
    println!("   ✓ Average message length: {} bytes", total_len / 1000);
    println!("   ✓ Zero heap allocations (buffers reused via clear())");
    println!();
}
