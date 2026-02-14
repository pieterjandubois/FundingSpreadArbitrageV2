/// Demonstration of type-safe order state machine using the typestate pattern
/// 
/// This example shows how the typestate pattern prevents illegal state transitions
/// at compile time, eliminating entire classes of runtime errors.
/// 
/// Requirements: 10.1, 10.2, 10.3, 10.4
/// 
/// Run with: cargo run --example order_typestate_demo

use std::marker::PhantomData;

// State types (zero-sized)
struct Pending;
struct Filled;
struct Cancelled;

// Order with typestate pattern
#[derive(Debug, Clone)]
struct Order<S> {
    id: u64,
    symbol_id: u32,
    price: f64,
    size: f64,
    _state: PhantomData<S>,
}

impl Order<Pending> {
    fn new(id: u64, symbol_id: u32, price: f64, size: f64) -> Self {
        Self {
            id,
            symbol_id,
            price,
            size,
            _state: PhantomData,
        }
    }
    
    fn fill(self, _fill_price: f64, _fill_time: u64) -> Order<Filled> {
        println!("  ✓ Order {} filled", self.id);
        Order {
            id: self.id,
            symbol_id: self.symbol_id,
            price: self.price,
            size: self.size,
            _state: PhantomData,
        }
    }
    
    fn cancel(self, _reason: &'static str) -> Order<Cancelled> {
        println!("  ✗ Order {} cancelled", self.id);
        Order {
            id: self.id,
            symbol_id: self.symbol_id,
            price: self.price,
            size: self.size,
            _state: PhantomData,
        }
    }
}

impl Order<Filled> {
    fn get_fill_price(&self) -> f64 {
        self.price
    }
}

impl Order<Cancelled> {
    fn get_reason(&self) -> &'static str {
        "Order cancelled"
    }
}

fn main() {
    println!("=== Type-Safe Order State Machine Demo ===\n");
    
    // Example 1: Fill an order
    println!("Example 1: Filling an order");
    let order1 = Order::<Pending>::new(1, 1, 100.0, 1.0);
    println!("  Created pending order: ID={}, Price={}", order1.id, order1.price);
    let filled_order = order1.fill(100.5, 1234567890);
    println!("  Fill price: {}", filled_order.get_fill_price());
    
    // Example 2: Cancel an order
    println!("\nExample 2: Cancelling an order");
    let order2 = Order::<Pending>::new(2, 2, 200.0, 2.0);
    println!("  Created pending order: ID={}, Price={}", order2.id, order2.price);
    let cancelled_order = order2.cancel("Insufficient funds");
    println!("  Cancellation reason: {}", cancelled_order.get_reason());
    
    // Example 3: Compile-time safety
    println!("\nExample 3: Compile-time safety guarantees");
    println!("  The following operations would NOT compile:");
    println!("  - filled_order.fill(...)     // ERROR: no method 'fill' on Order<Filled>");
    println!("  - filled_order.cancel(...)   // ERROR: no method 'cancel' on Order<Filled>");
    println!("  - pending_order.get_fill_price() // ERROR: no method 'get_fill_price' on Order<Pending>");
    
    // Example 4: Zero runtime overhead
    println!("\nExample 4: Zero runtime overhead");
    use std::mem::size_of;
    println!("  Size of Order<Pending>:   {} bytes", size_of::<Order<Pending>>());
    println!("  Size of Order<Filled>:    {} bytes", size_of::<Order<Filled>>());
    println!("  Size of Order<Cancelled>: {} bytes", size_of::<Order<Cancelled>>());
    println!("  All states have the same size - PhantomData is zero-sized!");
    
    println!("\n=== Benefits ===");
    println!("✓ Illegal states are unrepresentable at compile time");
    println!("✓ State transitions are enforced by the type system");
    println!("✓ Zero runtime overhead (PhantomData is zero-sized)");
    println!("✓ No boolean flags needed (is_filled, is_cancelled, etc.)");
    println!("✓ Eliminates entire classes of runtime errors");
}
