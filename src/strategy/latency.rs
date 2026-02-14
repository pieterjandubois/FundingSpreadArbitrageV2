use std::collections::HashMap;

pub struct LatencyMonitor {
    latencies: HashMap<String, Vec<u64>>,
}

impl Default for LatencyMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyMonitor {
    pub fn new() -> Self {
        Self {
            latencies: HashMap::new(),
        }
    }

    pub fn any_stale(&self, threshold_ms: u64) -> bool {
        self.latencies.values().any(|latencies| {
            if latencies.is_empty() {
                false
            } else {
                let avg = latencies.iter().sum::<u64>() / latencies.len() as u64;
                avg > threshold_ms
            }
        })
    }
}
