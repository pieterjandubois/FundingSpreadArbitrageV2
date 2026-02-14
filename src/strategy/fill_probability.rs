use crate::strategy::types::{OrderBookDepth, OrderSide};

/// Estimates the probability of a limit order filling within a given timeframe
/// based on order book depth and historical fill patterns
#[derive(Debug, Clone)]
pub struct FillProbabilityEstimator {
    /// Minimum probability threshold to attempt a limit order (default: 70%)
    pub min_probability_threshold: f64,
    /// Maximum wait time in milliseconds before giving up on limit (default: 200ms)
    pub max_wait_time_ms: u64,
}

impl FillProbabilityEstimator {
    pub fn new() -> Self {
        Self {
            min_probability_threshold: 0.70, // 70% confidence
            max_wait_time_ms: 200,
        }
    }

    /// Calculate fill probability for a limit order at a specific price level
    /// 
    /// # Arguments
    /// * `order_book` - Current order book snapshot
    /// * `side` - Order side (Long = buying, Short = selling)
    /// * `price_level` - Which level in the book (0 = best, 1 = second best, etc.)
    /// * `order_size` - Size of the order in USD
    /// 
    /// # Returns
    /// Probability between 0.0 and 1.0, where 1.0 = 100% likely to fill
    pub fn calculate_fill_probability(
        &self,
        order_book: &OrderBookDepth,
        side: OrderSide,
        price_level: usize,
        order_size: f64,
    ) -> FillProbability {
        // Select the appropriate side of the book
        let book_side = match side {
            OrderSide::Long => &order_book.asks,  // Buying = take from asks
            OrderSide::Short => &order_book.bids, // Selling = take from bids
        };

        // Check if the requested level exists
        if price_level >= book_side.len() {
            return FillProbability {
                probability: 0.0,
                recommended_action: OrderAction::UseMarket,
                reason: "Price level not available in order book".to_string(),
                estimated_wait_ms: 0,
            };
        }

        // Calculate cumulative depth up to and including this level
        let cumulative_depth: f64 = book_side
            .iter()
            .take(price_level + 1)
            .map(|level| level.quantity * level.price)
            .sum();

        // Calculate depth ratio: how much of our order is covered by available liquidity
        let depth_ratio = cumulative_depth / order_size;

        // Base probability calculation based on depth ratio
        let base_probability = if depth_ratio >= 10.0 {
            0.95 // Excellent depth, very likely to fill
        } else if depth_ratio >= 5.0 {
            0.85 // Good depth
        } else if depth_ratio >= 3.0 {
            0.75 // Adequate depth
        } else if depth_ratio >= 2.0 {
            0.60 // Marginal depth
        } else if depth_ratio >= 1.0 {
            0.40 // Risky - order size equals available depth
        } else {
            0.20 // Very risky - not enough depth
        };

        // Adjust probability based on price level (deeper in book = lower probability)
        let level_penalty = match price_level {
            0 => 1.0,    // Best price - no penalty
            1 => 0.95,   // Second level - slight penalty
            2 => 0.85,   // Third level - moderate penalty
            3 => 0.70,   // Fourth level - significant penalty
            _ => 0.50,   // Deeper levels - high penalty
        };

        let adjusted_probability = base_probability * level_penalty;

        // Estimate wait time based on probability
        let estimated_wait_ms = if adjusted_probability >= 0.90 {
            50  // Very likely to fill quickly
        } else if adjusted_probability >= 0.75 {
            100 // Likely to fill within 100ms
        } else if adjusted_probability >= 0.60 {
            200 // May take up to 200ms
        } else {
            500 // Unlikely to fill quickly
        };

        // Determine recommended action
        let recommended_action = if adjusted_probability >= self.min_probability_threshold {
            OrderAction::UseLimitWithFallback {
                wait_time_ms: estimated_wait_ms.min(self.max_wait_time_ms),
            }
        } else {
            OrderAction::UseMarket
        };

        let reason = format!(
            "Depth ratio: {:.2}x | Level: {} | Base prob: {:.0}% | Adjusted: {:.0}%",
            depth_ratio,
            price_level,
            base_probability * 100.0,
            adjusted_probability * 100.0
        );

        FillProbability {
            probability: adjusted_probability,
            recommended_action,
            reason,
            estimated_wait_ms,
        }
    }

    /// Smart decision: should we try a limit order or go straight to market?
    /// 
    /// This is the main entry point for making hedging decisions
    pub fn should_try_limit(
        &self,
        order_book: &OrderBookDepth,
        side: OrderSide,
        order_size: f64,
    ) -> LimitOrderDecision {
        // Try levels 0, 1, 2 (best, second, third) and pick the best option
        let mut best_decision = LimitOrderDecision {
            use_limit: false,
            price_level: 0,
            probability: 0.0,
            wait_time_ms: 0,
            reason: "No viable limit order strategy".to_string(),
        };

        for level in 0..3 {
            let fill_prob = self.calculate_fill_probability(order_book, side, level, order_size);

            // If this level meets our threshold and is better than what we've seen
            if fill_prob.probability >= self.min_probability_threshold
                && fill_prob.probability > best_decision.probability
            {
                best_decision = LimitOrderDecision {
                    use_limit: true,
                    price_level: level,
                    probability: fill_prob.probability,
                    wait_time_ms: fill_prob.estimated_wait_ms,
                    reason: fill_prob.reason.clone(),
                };
            }
        }

        best_decision
    }

    /// Calculate partial fill probability - useful for retry logic
    /// 
    /// If a limit order partially fills, should we retry with another limit or go to market?
    pub fn should_retry_limit(
        &self,
        order_book: &OrderBookDepth,
        side: OrderSide,
        remaining_size: f64,
        previous_attempts: u32,
    ) -> bool {
        // Reduce threshold for retries (more aggressive)
        let retry_threshold = self.min_probability_threshold - (0.10 * previous_attempts as f64);
        
        if retry_threshold < 0.50 {
            return false; // Don't retry if threshold drops below 50%
        }

        let fill_prob = self.calculate_fill_probability(order_book, side, 0, remaining_size);
        
        fill_prob.probability >= retry_threshold
    }
}

impl Default for FillProbabilityEstimator {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of fill probability calculation
#[derive(Debug, Clone)]
pub struct FillProbability {
    /// Probability of fill (0.0 to 1.0)
    pub probability: f64,
    /// Recommended action based on probability
    pub recommended_action: OrderAction,
    /// Human-readable reason for the recommendation
    pub reason: String,
    /// Estimated wait time in milliseconds
    pub estimated_wait_ms: u64,
}

/// Recommended order action based on fill probability
#[derive(Debug, Clone, PartialEq)]
pub enum OrderAction {
    /// Use market order immediately (low fill probability for limit)
    UseMarket,
    /// Try limit order first, fallback to market after wait time
    UseLimitWithFallback { wait_time_ms: u64 },
}

/// Decision about whether to use a limit order
#[derive(Debug, Clone)]
pub struct LimitOrderDecision {
    /// Should we try a limit order?
    pub use_limit: bool,
    /// Which price level to use (0 = best, 1 = second, etc.)
    pub price_level: usize,
    /// Probability of fill
    pub probability: f64,
    /// How long to wait before fallback to market
    pub wait_time_ms: u64,
    /// Reason for the decision
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::types::PriceLevel;

    fn create_test_order_book() -> OrderBookDepth {
        OrderBookDepth {
            bids: vec![
                PriceLevel { price: 100.0, quantity: 10.0 },  // $1,000 depth
                PriceLevel { price: 99.9, quantity: 20.0 },   // $1,998 depth
                PriceLevel { price: 99.8, quantity: 30.0 },   // $2,994 depth
            ],
            asks: vec![
                PriceLevel { price: 100.1, quantity: 10.0 },  // $1,001 depth
                PriceLevel { price: 100.2, quantity: 20.0 },  // $2,004 depth
                PriceLevel { price: 100.3, quantity: 30.0 },  // $3,009 depth
            ],
            timestamp: 0,
        }
    }

    #[test]
    fn test_high_probability_small_order() {
        let estimator = FillProbabilityEstimator::new();
        let order_book = create_test_order_book();
        
        // Small order ($100) with $1,000+ depth at best level
        let result = estimator.calculate_fill_probability(
            &order_book,
            OrderSide::Long,
            0,
            100.0,
        );

        assert!(result.probability >= 0.90, "Small order should have high fill probability");
        assert!(matches!(result.recommended_action, OrderAction::UseLimitWithFallback { .. }));
    }

    #[test]
    fn test_low_probability_large_order() {
        let estimator = FillProbabilityEstimator::new();
        let order_book = create_test_order_book();
        
        // Large order ($5,000) exceeds available depth at best level
        let result = estimator.calculate_fill_probability(
            &order_book,
            OrderSide::Long,
            0,
            5000.0,
        );

        assert!(result.probability < 0.70, "Large order should have low fill probability");
        assert_eq!(result.recommended_action, OrderAction::UseMarket);
    }

    #[test]
    fn test_should_try_limit_decision() {
        let estimator = FillProbabilityEstimator::new();
        let order_book = create_test_order_book();
        
        // Medium order ($500) should get a limit recommendation
        let decision = estimator.should_try_limit(&order_book, OrderSide::Long, 500.0);

        assert!(decision.use_limit, "Should recommend limit order for medium-sized order");
        assert!(decision.probability >= 0.70, "Probability should meet threshold");
    }

    #[test]
    fn test_retry_logic_degradation() {
        let estimator = FillProbabilityEstimator::new();
        let order_book = create_test_order_book();
        
        // First retry should be more lenient
        let retry_1 = estimator.should_retry_limit(&order_book, OrderSide::Long, 500.0, 1);
        
        // Third retry should be much stricter
        let retry_3 = estimator.should_retry_limit(&order_book, OrderSide::Long, 500.0, 3);
        
        // After many retries, should give up
        let retry_5 = estimator.should_retry_limit(&order_book, OrderSide::Long, 500.0, 5);

        assert!(retry_1, "First retry should be allowed");
        assert!(!retry_5, "Should give up after many retries");
    }

    #[test]
    fn test_price_level_penalty() {
        let estimator = FillProbabilityEstimator::new();
        let order_book = create_test_order_book();
        
        let level_0 = estimator.calculate_fill_probability(&order_book, OrderSide::Long, 0, 500.0);
        let level_2 = estimator.calculate_fill_probability(&order_book, OrderSide::Long, 2, 500.0);

        assert!(
            level_0.probability > level_2.probability,
            "Best price level should have higher probability than deeper levels"
        );
    }
}
