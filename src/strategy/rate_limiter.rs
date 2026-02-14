use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use std::collections::VecDeque;

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    requests_per_second: u32,
    burst_capacity: u32,
}

struct RateLimiterState {
    tokens: u32,
    last_refill: Instant,
    request_timestamps: VecDeque<Instant>,
}

impl RateLimiter {
    pub fn new(requests_per_second: u32, burst_capacity: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimiterState {
                tokens: burst_capacity,
                last_refill: Instant::now(),
                request_timestamps: VecDeque::with_capacity(requests_per_second as usize * 2),
            })),
            requests_per_second,
            burst_capacity,
        }
    }

    pub async fn acquire(&self) {
        loop {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(state.last_refill);
            let tokens_to_add = (elapsed.as_secs_f64() * self.requests_per_second as f64) as u32;
            
            if tokens_to_add > 0 {
                state.tokens = (state.tokens + tokens_to_add).min(self.burst_capacity);
                state.last_refill = now;
            }
            
            while let Some(&timestamp) = state.request_timestamps.front() {
                if now.duration_since(timestamp) > Duration::from_secs(1) {
                    state.request_timestamps.pop_front();
                } else {
                    break;
                }
            }
            
            if state.tokens > 0 {
                state.tokens -= 1;
                state.request_timestamps.push_back(now);
                
                if state.request_timestamps.len() % 10 == 0 {
                    let requests_last_second = state.request_timestamps.len();
                    eprintln!("[RATE LIMITER] Tokens: {} | Requests in last 1s: {} | Limit: {}/s", 
                        state.tokens, requests_last_second, self.requests_per_second);
                }
                
                drop(state);
                return;
            }
            
            let wait_time = if state.tokens == 0 {
                Duration::from_secs_f64(1.0 / self.requests_per_second as f64)
            } else {
                Duration::from_millis(10)
            };
            
            drop(state);
            eprintln!("[RATE LIMITER] ⏳ Rate limit reached, waiting {:?}", wait_time);
            tokio::time::sleep(wait_time).await;
        }
    }

    pub async fn stats(&self) -> RateLimiterStats {
        let state = self.state.lock().await;
        let now = Instant::now();
        let requests_last_second = state.request_timestamps.iter()
            .filter(|&&ts| now.duration_since(ts) <= Duration::from_secs(1))
            .count();
        
        RateLimiterStats {
            available_tokens: state.tokens,
            requests_last_second: requests_last_second as u32,
            limit_per_second: self.requests_per_second,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RateLimiterStats {
    pub available_tokens: u32,
    pub requests_last_second: u32,
    pub limit_per_second: u32,
}
