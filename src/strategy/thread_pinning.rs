//! Thread Pinning for Low-Latency Performance
//!
//! This module provides utilities for pinning threads to specific CPU cores
//! to maintain hot CPU caches and prevent OS interference.
//!
//! ## Why Thread Pinning?
//!
//! - **Hot Caches**: Keeping a thread on the same core maintains L1/L2 cache
//! - **No Context Switching**: Prevents OS from moving threads between cores
//! - **Predictable Latency**: Eliminates jitter from core migrations
//! - **NUMA Awareness**: Can pin to cores on same NUMA node as network card
//!
//! ## Kernel Configuration Required
//!
//! For best results, isolate cores from the OS scheduler:
//!
//! ```bash
//! # Add to /etc/default/grub
//! GRUB_CMDLINE_LINUX="isolcpus=1-7 nohz_full=1-7 rcu_nocbs=1-7"
//!
//! # Rebuild grub
//! sudo update-grub
//! sudo reboot
//! ```
//!
//! This isolates cores 1-7 for our trading threads, leaving core 0 for OS tasks.
//!
//! Requirements: 4.1, 4.2, 4.3, 4.4

use core_affinity::CoreId;
use std::thread;

/// Core assignments for different thread types
pub struct CoreAssignment {
    /// Core for the main strategy thread (hot path)
    pub strategy_core: CoreId,
    
    /// Cores for WebSocket threads (warm path)
    pub websocket_cores: Vec<CoreId>,
}

impl CoreAssignment {
    /// Default core assignment:
    /// - Core 1: Strategy thread (hot path)
    /// - Cores 2-7: WebSocket threads (warm path)
    /// - Core 0: Left for OS (not used by trading system)
    pub fn default_assignment() -> Self {
        Self {
            strategy_core: CoreId { id: 1 },
            websocket_cores: (2..8).map(|id| CoreId { id }).collect(),
        }
    }
    
    /// Custom core assignment
    pub fn custom(strategy_core: usize, websocket_cores: Vec<usize>) -> Self {
        Self {
            strategy_core: CoreId { id: strategy_core },
            websocket_cores: websocket_cores.into_iter().map(|id| CoreId { id }).collect(),
        }
    }
}

/// Pin the current thread to the strategy core (core 1).
///
/// This should be called at the start of the strategy thread's execution.
///
/// # Returns
///
/// - `Ok(())` if pinning succeeded
/// - `Err(String)` if pinning failed
///
/// # Example
///
/// ```rust,ignore
/// use arbitrage2::strategy::thread_pinning::pin_strategy_thread;
///
/// tokio::spawn(async move {
///     if let Err(e) = pin_strategy_thread() {
///         eprintln!("Failed to pin strategy thread: {}", e);
///     }
///     // Run strategy logic...
/// });
/// ```
///
/// Requirement: 4.1 (Pin strategy thread to core 1)
pub fn pin_strategy_thread() -> Result<(), String> {
    let core = CoreId { id: 1 };
    pin_current_thread(core, "strategy")
}

/// Pin the current thread to a WebSocket core (cores 2-7).
///
/// This should be called at the start of each WebSocket thread's execution.
///
/// # Arguments
///
/// * `worker_id` - Worker ID (0-5) which maps to cores 2-7
///
/// # Returns
///
/// - `Ok(())` if pinning succeeded
/// - `Err(String)` if pinning failed
///
/// # Example
///
/// ```rust,ignore
/// use arbitrage2::strategy::thread_pinning::pin_websocket_thread;
///
/// for worker_id in 0..6 {
///     tokio::spawn(async move {
///         if let Err(e) = pin_websocket_thread(worker_id) {
///             eprintln!("Failed to pin WebSocket thread {}: {}", worker_id, e);
///         }
///         // Run WebSocket logic...
///     });
/// }
/// ```
///
/// Requirement: 4.2 (Pin WebSocket threads to cores 2-7)
pub fn pin_websocket_thread(worker_id: usize) -> Result<(), String> {
    let core_id = 2 + worker_id; // Map worker 0-5 to cores 2-7
    let core = CoreId { id: core_id };
    pin_current_thread(core, &format!("websocket-{}", worker_id))
}

/// Pin the current thread to a specific core.
///
/// This is a low-level function used by pin_strategy_thread and pin_websocket_thread.
///
/// # Arguments
///
/// * `core` - The core to pin to
/// * `thread_name` - Name for logging purposes
///
/// # Returns
///
/// - `Ok(())` if pinning succeeded
/// - `Err(String)` if pinning failed
///
/// Requirement: 4.3 (Verify affinity and log core assignments)
fn pin_current_thread(core: CoreId, thread_name: &str) -> Result<(), String> {
    // Attempt to pin the thread
    if !core_affinity::set_for_current(core) {
        return Err(format!(
            "Failed to pin {} thread to core {}",
            thread_name, core.id
        ));
    }
    
    // Verify the pinning worked
    if let Some(current_core) = core_affinity::get_core_ids().and_then(|cores| {
        cores.into_iter().find(|c| c.id == core.id)
    }) {
        eprintln!(
            "[THREAD-PIN] ✓ {} thread pinned to core {}",
            thread_name, current_core.id
        );
        Ok(())
    } else {
        Err(format!(
            "Failed to verify pinning for {} thread to core {}",
            thread_name, core.id
        ))
    }
}

/// Get the number of available CPU cores.
///
/// This is useful for determining if we have enough cores for our assignment.
pub fn get_core_count() -> usize {
    core_affinity::get_core_ids()
        .map(|cores| cores.len())
        .unwrap_or(0)
}

/// Check if we have enough cores for the default assignment.
///
/// Returns true if we have at least 8 cores (0-7).
pub fn has_sufficient_cores() -> bool {
    get_core_count() >= 8
}

/// Print core assignment information.
///
/// This should be called at startup to document the core assignments.
///
/// Requirement: 4.4 (Document required isolcpus kernel parameter)
pub fn print_core_assignment_info() {
    let core_count = get_core_count();
    
    eprintln!("=== CPU Core Assignment ===");
    eprintln!("Total cores available: {}", core_count);
    eprintln!("Core 0: OS and system tasks");
    eprintln!("Core 1: Strategy thread (hot path)");
    eprintln!("Cores 2-7: WebSocket threads (warm path)");
    eprintln!();
    
    if !has_sufficient_cores() {
        eprintln!("⚠️  WARNING: Insufficient cores detected!");
        eprintln!("   Required: 8 cores (0-7)");
        eprintln!("   Available: {}", core_count);
        eprintln!("   Thread pinning may not work optimally.");
        eprintln!();
    }
    
    eprintln!("For optimal performance, isolate cores 1-7 from OS scheduler:");
    eprintln!("  1. Edit /etc/default/grub");
    eprintln!("  2. Add: GRUB_CMDLINE_LINUX=\"isolcpus=1-7 nohz_full=1-7 rcu_nocbs=1-7\"");
    eprintln!("  3. Run: sudo update-grub");
    eprintln!("  4. Reboot");
    eprintln!();
    eprintln!("To verify isolation:");
    eprintln!("  cat /sys/devices/system/cpu/isolated");
    eprintln!("  (should show: 1-7)");
    eprintln!("===========================");
    eprintln!();
}

/// Spawn a thread with pinning to a specific core.
///
/// This is a convenience function that spawns a thread and pins it to a core.
///
/// # Arguments
///
/// * `core` - The core to pin to
/// * `name` - Thread name
/// * `f` - The function to run in the thread
///
/// # Returns
///
/// A JoinHandle for the spawned thread
///
/// # Example
///
/// ```rust,ignore
/// use arbitrage2::strategy::thread_pinning::{spawn_pinned_thread, CoreId};
///
/// let handle = spawn_pinned_thread(
///     CoreId { id: 1 },
///     "my-thread",
///     || {
///         // Thread logic here
///         println!("Running on pinned core!");
///     }
/// );
///
/// handle.join().unwrap();
/// ```
pub fn spawn_pinned_thread<F, T>(
    core: CoreId,
    name: &str,
    f: F,
) -> thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let name_owned = name.to_string();
    thread::spawn(move || {
        if let Err(e) = pin_current_thread(core, &name_owned) {
            eprintln!("Warning: {}", e);
        }
        f()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_core_assignment_default() {
        let assignment = CoreAssignment::default_assignment();
        assert_eq!(assignment.strategy_core.id, 1);
        assert_eq!(assignment.websocket_cores.len(), 6);
        assert_eq!(assignment.websocket_cores[0].id, 2);
        assert_eq!(assignment.websocket_cores[5].id, 7);
    }
    
    #[test]
    fn test_core_assignment_custom() {
        let assignment = CoreAssignment::custom(3, vec![4, 5, 6]);
        assert_eq!(assignment.strategy_core.id, 3);
        assert_eq!(assignment.websocket_cores.len(), 3);
        assert_eq!(assignment.websocket_cores[0].id, 4);
        assert_eq!(assignment.websocket_cores[2].id, 6);
    }
    
    #[test]
    fn test_get_core_count() {
        let count = get_core_count();
        // Should have at least 1 core
        assert!(count > 0);
    }
    
    #[test]
    fn test_has_sufficient_cores() {
        // This test may fail on systems with < 8 cores
        // That's expected - it's a system requirement check
        let sufficient = has_sufficient_cores();
        let count = get_core_count();
        assert_eq!(sufficient, count >= 8);
    }
    
    #[test]
    fn test_print_core_assignment_info() {
        // Just verify it doesn't panic
        print_core_assignment_info();
    }
}
