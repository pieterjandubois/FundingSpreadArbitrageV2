#[cfg(test)]
mod property_tests {
    use crate::strategy::types::*;
    use crate::strategy::positions::PositionManager;
    use proptest::prelude::*;

    // Property 1: Capital Conservation
    proptest! {
        #[test]
        fn prop_capital_conservation(
            starting_capital in 10000.0f64..100000.0,
            position_sizes in prop::collection::vec(100.0f64..5000.0, 0..10)
        ) {
            let mut portfolio = PortfolioState::new(starting_capital);

            for size in position_sizes {
                if portfolio.available_capital >= size {
                    portfolio.available_capital -= size;
                    portfolio.total_open_positions += size;
                }
            }

            let total = portfolio.available_capital + portfolio.total_open_positions;
            prop_assert!((total - starting_capital).abs() < 0.01);
        }
    }

    // Property 2: Hard Constraints Enforcement
    proptest! {
        #[test]
        fn prop_hard_constraints_enforcement(
            depth_long in 100.0f64..10000.0,
            depth_short in 100.0f64..10000.0,
            position_size in 100.0f64..5000.0,
            latency_ok in prop::bool::ANY,
            funding_delta in -0.001f64..0.001
        ) {
            let constraints = crate::strategy::confluence::ConfluenceCalculator::check_hard_constraints(
                depth_long,
                depth_short,
                position_size,
                latency_ok,
                funding_delta,
            );

            if !constraints.passes_all() {
                let metrics = ConfluenceMetrics {
                    funding_delta,
                    funding_delta_projected: 0.0,
                    obi_ratio: 0.5,
                    oi_current: 1000.0,
                    oi_24h_avg: 1000.0,
                    vwap_deviation: 0.01,
                    atr: 1.0,
                    atr_trend: true,
                    liquidation_cluster_distance: 50.0,
                    hard_constraints: constraints,
                };
                prop_assert_eq!(metrics.calculate_confidence_score(), 0);
            }
        }
    }

    // Property 3: Atomic Execution
    proptest! {
        #[test]
        fn prop_atomic_execution(
            long_filled in prop::bool::ANY,
            short_filled in prop::bool::ANY,
            time_ms in 0u64..1000
        ) {
            let leg_out = PositionManager::detect_leg_out(long_filled, short_filled, time_ms);

            if (long_filled && !short_filled) || (!long_filled && short_filled) {
                prop_assert_eq!(leg_out, time_ms > 500);
            } else {
                prop_assert!(!leg_out);
            }
        }
    }

    // Property 6: Leg-Out Risk Tracking
    proptest! {
        #[test]
        fn prop_leg_out_risk_tracking(
            long_filled in prop::bool::ANY,
            short_filled in prop::bool::ANY,
            time_ms in 0u64..2000
        ) {
            let leg_out = PositionManager::detect_leg_out(long_filled, short_filled, time_ms);

            let expected_leg_out = ((long_filled && !short_filled) || (!long_filled && short_filled)) && time_ms > 500;
            prop_assert_eq!(leg_out, expected_leg_out);
        }
    }

    // Property 7: PnL Accuracy
    proptest! {
        #[test]
        fn prop_pnl_accuracy(
            profits in prop::collection::vec(-1000.0f64..5000.0, 0..20),
            leg_out_losses in prop::collection::vec(0.0f64..500.0, 0..5)
        ) {
            let mut portfolio = PortfolioState::new(20000.0);
            let mut expected_pnl = 0.0;

            for profit in &profits {
                portfolio.cumulative_pnl += profit;
                expected_pnl += profit;
                if *profit > 0.0 {
                    portfolio.win_count += 1;
                } else {
                    portfolio.loss_count += 1;
                }
            }

            for loss in &leg_out_losses {
                portfolio.leg_out_total_loss += loss;
                portfolio.leg_out_count += 1;
            }

            prop_assert!((portfolio.cumulative_pnl - expected_pnl).abs() < 0.01);
        }
    }

    // Property 8: Latency Impact
    proptest! {
        #[test]
        fn prop_latency_impact(
            latency_ms in 0u64..500,
            _base_score in 0u8..100
        ) {
            let latency_ok = latency_ms <= 200;

            let constraints = HardConstraints {
                order_book_depth_sufficient: true,
                exchange_latency_ok: latency_ok,
                funding_delta_substantial: true,
            };

            let metrics = ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: constraints,
            };

            let score = metrics.calculate_confidence_score();

            if latency_ms > 200 {
                prop_assert_eq!(score, 0);
            }
        }
    }

    // Property 9: Position Sizing Constraints
    // **Validates: Requirements 3.3, 3.6, 5.1, 5.2**
    proptest! {
        #[test]
        fn prop_position_sizing_constraints(
            spread_bps in 5.0f64..500.0,
            available_capital in 1000.0f64..100000.0,
            fees_bps in 0.0f64..100.0,
            funding_cost_bps in 0.0f64..100.0
        ) {
            use crate::strategy::entry::EntryExecutor;

            let position_size = EntryExecutor::calculate_position_size(
                spread_bps,
                available_capital,
                fees_bps,
                funding_cost_bps,
            );

            // Position size must never exceed 50% of available capital
            prop_assert!(position_size <= available_capital * 0.5);

            // Position size must be at least $100 or 0 (if unprofitable)
            prop_assert!(position_size >= 100.0 || position_size == 0.0);

            // If net profit is positive, position size should be > 0
            let net_profit = spread_bps - fees_bps - funding_cost_bps;
            if net_profit > 0.0 && spread_bps > 0.0 {
                prop_assert!(position_size > 0.0);
            }

            // If net profit is non-positive, position size should be 0
            if net_profit <= 0.0 || spread_bps <= 0.0 {
                prop_assert_eq!(position_size, 0.0);
            }
        }
    }

    // Property 10: Harder Leg Identification Determinism
    // **Validates: Requirements 3.9**
    proptest! {
        #[test]
        fn prop_harder_leg_identification_deterministic(
            long_exchange in "binance|bybit|okx|deribit|bitget|kucoin|gate|hyperliquid|paradex|lighter",
            short_exchange in "binance|bybit|okx|deribit|bitget|kucoin|gate|hyperliquid|paradex|lighter"
        ) {
            use crate::strategy::entry::identify_harder_leg;

            let result1 = identify_harder_leg(&long_exchange, &short_exchange);
            let result2 = identify_harder_leg(&long_exchange, &short_exchange);
            let result3 = identify_harder_leg(&long_exchange, &short_exchange);

            // Function must be deterministic
            prop_assert_eq!(&result1, &result2);
            prop_assert_eq!(&result2, &result3);

            // Result must always be either "long" or "short"
            prop_assert!(result1 == "long" || result1 == "short");
        }
    }

    // Property 11: Harder Leg Consistency
    // **Validates: Requirements 3.9**
    proptest! {
        #[test]
        fn prop_harder_leg_consistency(
            long_exchange in "binance|bybit|okx|deribit|bitget|kucoin|gate|hyperliquid|paradex|lighter",
            short_exchange in "binance|bybit|okx|deribit|bitget|kucoin|gate|hyperliquid|paradex|lighter"
        ) {
            use crate::strategy::entry::identify_harder_leg;

            let result = identify_harder_leg(&long_exchange, &short_exchange);

            // If we swap exchanges, the harder leg should swap too (unless they're the same)
            let swapped_result = identify_harder_leg(&short_exchange, &long_exchange);

            if long_exchange.to_lowercase() == short_exchange.to_lowercase() {
                // If both exchanges are the same, result should always be "long"
                prop_assert_eq!(result, "long");
                prop_assert_eq!(swapped_result, "long");
            } else if result == "long" {
                prop_assert_eq!(swapped_result, "short");
            } else {
                prop_assert_eq!(swapped_result, "long");
            }
        }
    }

    // Property 12: Harder Leg Case Insensitivity
    // **Validates: Requirements 3.9**
    proptest! {
        #[test]
        fn prop_harder_leg_case_insensitive(
            long_exchange in "binance|bybit|okx|deribit|bitget|kucoin|gate|hyperliquid|paradex|lighter",
            short_exchange in "binance|bybit|okx|deribit|bitget|kucoin|gate|hyperliquid|paradex|lighter"
        ) {
            use crate::strategy::entry::identify_harder_leg;

            let result_lower = identify_harder_leg(&long_exchange, &short_exchange);
            let result_upper = identify_harder_leg(&long_exchange.to_uppercase(), &short_exchange.to_uppercase());
            let result_mixed = identify_harder_leg(&long_exchange.to_uppercase(), &short_exchange);

            // All case variations should produce the same result
            prop_assert_eq!(&result_lower, &result_upper);
            prop_assert_eq!(&result_lower, &result_mixed);
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use crate::strategy::positions::PositionManager;
    use crate::strategy::atomic_execution::NegativeFundingTracker;
    use crate::strategy::types::{TradeStatus, OrderStatus, OrderType};

    #[test]
    fn test_unrealized_pnl() {
        let pnl = PositionManager::calculate_unrealized_pnl(100.0, 101.0, 101.0, 100.0, 1000.0);
        assert!(pnl > 0.0);
    }

    #[test]
    fn test_negative_funding_tracker_basic() {
        let tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        assert_eq!(tracker.consecutive_negative_cycles, 0);
        assert!(!tracker.should_exit());
    }

    #[test]
    fn test_negative_funding_single_cycle() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        let should_exit = tracker.update_funding(-0.001);
        assert!(!should_exit);
        assert_eq!(tracker.consecutive_negative_cycles, 1);
    }

    #[test]
    fn test_negative_funding_two_cycles_triggers_exit() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        tracker.update_funding(-0.001);
        let should_exit = tracker.update_funding(-0.001);
        assert!(should_exit);
        assert_eq!(tracker.consecutive_negative_cycles, 2);
    }

    #[test]
    fn test_negative_funding_positive_resets() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        tracker.update_funding(-0.001);
        tracker.update_funding(-0.001);
        assert!(tracker.should_exit());
        
        let should_exit = tracker.update_funding(0.001);
        assert!(!should_exit);
        assert_eq!(tracker.consecutive_negative_cycles, 0);
    }

    #[test]
    fn test_negative_funding_tracker_reset() {
        let mut tracker = NegativeFundingTracker::new("ETHUSDT".to_string());
        tracker.update_funding(-0.001);
        tracker.update_funding(-0.001);
        assert!(tracker.should_exit());

        tracker.reset();
        assert!(!tracker.should_exit());
        assert_eq!(tracker.consecutive_negative_cycles, 0);
        assert_eq!(tracker.last_funding_rate, 0.0);
    }

    #[test]
    fn test_negative_funding_tracker_symbol() {
        let tracker = NegativeFundingTracker::new("SOLUSDT".to_string());
        assert_eq!(tracker.symbol, "SOLUSDT");
    }

    #[test]
    fn test_negative_funding_alternating_pattern() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        
        assert!(!tracker.update_funding(-0.001));
        assert!(!tracker.update_funding(0.001));
        assert!(!tracker.update_funding(-0.001));
        assert!(tracker.update_funding(-0.001));
        
        assert!(tracker.should_exit());
    }

    #[test]
    fn test_negative_funding_large_negative_values() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        
        assert!(!tracker.update_funding(-0.1));
        assert!(tracker.update_funding(-0.05));
        assert!(tracker.should_exit());
    }

    #[test]
    fn test_negative_funding_zero_funding_rate() {
        let mut tracker = NegativeFundingTracker::new("BTCUSDT".to_string());
        
        assert!(!tracker.update_funding(0.0));
        assert_eq!(tracker.consecutive_negative_cycles, 0);
    }

    // Position Sizing Tests
    use crate::strategy::entry::EntryExecutor;

    #[test]
    fn test_position_sizing_basic() {
        // spread_bps=100, available_capital=10000, fees=20, funding_cost=10
        // base_size = (100 - 20 - 10) / 100 * 10000 = 0.7 * 10000 = 7000
        // capped_size = min(7000, 10000 * 0.5) = min(7000, 5000) = 5000
        // final_size = max(5000, 100) = 5000
        let size = EntryExecutor::calculate_position_size(100.0, 10000.0, 20.0, 10.0);
        assert_eq!(size, 5000.0);
    }

    #[test]
    fn test_position_sizing_minimum_100() {
        // spread_bps=10, available_capital=1000, fees=5, funding_cost=3
        // base_size = (10 - 5 - 3) / 10 * 1000 = 0.2 * 1000 = 200
        // capped_size = min(200, 1000 * 0.5) = min(200, 500) = 200
        // final_size = max(200, 100) = 200
        let size = EntryExecutor::calculate_position_size(10.0, 1000.0, 5.0, 3.0);
        assert_eq!(size, 200.0);
    }

    #[test]
    fn test_position_sizing_hits_minimum() {
        // spread_bps=10, available_capital=1000, fees=8, funding_cost=1
        // base_size = (10 - 8 - 1) / 10 * 1000 = 0.1 * 1000 = 100
        // capped_size = min(100, 1000 * 0.5) = min(100, 500) = 100
        // final_size = max(100, 100) = 100
        let size = EntryExecutor::calculate_position_size(10.0, 1000.0, 8.0, 1.0);
        assert_eq!(size, 100.0);
    }

    #[test]
    fn test_position_sizing_below_minimum() {
        // spread_bps=10, available_capital=1000, fees=8, funding_cost=2
        // base_size = (10 - 8 - 2) / 10 * 1000 = 0.0 * 1000 = 0
        // Returns 0 because net_profit_bps <= 0
        let size = EntryExecutor::calculate_position_size(10.0, 1000.0, 8.0, 2.0);
        assert_eq!(size, 0.0);
    }

    #[test]
    fn test_position_sizing_negative_net_profit() {
        // spread_bps=50, available_capital=10000, fees=30, funding_cost=25
        // net_profit = 50 - 30 - 25 = -5 (negative)
        // Returns 0 because net_profit_bps <= 0
        let size = EntryExecutor::calculate_position_size(50.0, 10000.0, 30.0, 25.0);
        assert_eq!(size, 0.0);
    }

    #[test]
    fn test_position_sizing_zero_spread() {
        // spread_bps=0 (invalid)
        // Returns 0 because spread_bps <= 0
        let size = EntryExecutor::calculate_position_size(0.0, 10000.0, 20.0, 10.0);
        assert_eq!(size, 0.0);
    }

    #[test]
    fn test_position_sizing_negative_spread() {
        // spread_bps=-10 (invalid)
        // Returns 0 because spread_bps <= 0
        let size = EntryExecutor::calculate_position_size(-10.0, 10000.0, 20.0, 10.0);
        assert_eq!(size, 0.0);
    }

    #[test]
    fn test_position_sizing_50_percent_cap() {
        // spread_bps=200, available_capital=10000, fees=10, funding_cost=10
        // base_size = (200 - 10 - 10) / 200 * 10000 = 0.9 * 10000 = 9000
        // capped_size = min(9000, 10000 * 0.5) = min(9000, 5000) = 5000
        // final_size = max(5000, 100) = 5000
        let size = EntryExecutor::calculate_position_size(200.0, 10000.0, 10.0, 10.0);
        assert_eq!(size, 5000.0);
    }

    #[test]
    fn test_position_sizing_large_capital() {
        // spread_bps=100, available_capital=100000, fees=20, funding_cost=10
        // base_size = (100 - 20 - 10) / 100 * 100000 = 0.7 * 100000 = 70000
        // capped_size = min(70000, 100000 * 0.5) = min(70000, 50000) = 50000
        // final_size = max(50000, 100) = 50000
        let size = EntryExecutor::calculate_position_size(100.0, 100000.0, 20.0, 10.0);
        assert_eq!(size, 50000.0);
    }

    #[test]
    fn test_position_sizing_small_capital() {
        // spread_bps=100, available_capital=500, fees=20, funding_cost=10
        // base_size = (100 - 20 - 10) / 100 * 500 = 0.7 * 500 = 350
        // capped_size = min(350, 500 * 0.5) = min(350, 250) = 250
        // final_size = max(250, 100) = 250
        let size = EntryExecutor::calculate_position_size(100.0, 500.0, 20.0, 10.0);
        assert_eq!(size, 250.0);
    }

    #[test]
    fn test_position_sizing_high_fees() {
        // spread_bps=100, available_capital=10000, fees=80, funding_cost=15
        // base_size = (100 - 80 - 15) / 100 * 10000 = 0.05 * 10000 = 500
        // capped_size = min(500, 10000 * 0.5) = min(500, 5000) = 500
        // final_size = max(500, 100) = 500
        let size = EntryExecutor::calculate_position_size(100.0, 10000.0, 80.0, 15.0);
        assert_eq!(size, 500.0);
    }

    #[test]
    fn test_position_sizing_zero_fees_and_funding() {
        // spread_bps=100, available_capital=10000, fees=0, funding_cost=0
        // base_size = (100 - 0 - 0) / 100 * 10000 = 1.0 * 10000 = 10000
        // capped_size = min(10000, 10000 * 0.5) = min(10000, 5000) = 5000
        // final_size = max(5000, 100) = 5000
        let size = EntryExecutor::calculate_position_size(100.0, 10000.0, 0.0, 0.0);
        assert_eq!(size, 5000.0);
    }

    #[test]
    fn test_position_sizing_exactly_at_minimum() {
        // spread_bps=100, available_capital=1000, fees=50, funding_cost=40
        // base_size = (100 - 50 - 40) / 100 * 1000 = 0.1 * 1000 = 100
        // capped_size = min(100, 1000 * 0.5) = min(100, 500) = 100
        // final_size = max(100, 100) = 100
        let size = EntryExecutor::calculate_position_size(100.0, 1000.0, 50.0, 40.0);
        assert_eq!(size, 100.0);
    }

    #[test]
    fn test_position_sizing_just_below_minimum() {
        // spread_bps=100, available_capital=1000, fees=50, funding_cost=41
        // base_size = (100 - 50 - 41) / 100 * 1000 = 0.09 * 1000 = 90
        // capped_size = min(90, 1000 * 0.5) = min(90, 500) = 90
        // final_size = max(90, 100) = 100 (bumped to minimum)
        let size = EntryExecutor::calculate_position_size(100.0, 1000.0, 50.0, 41.0);
        assert_eq!(size, 100.0);
    }

    // Harder Leg Identification Tests
    use crate::strategy::entry::identify_harder_leg;

    #[test]
    fn test_identify_harder_leg_tier1_vs_tier2() {
        // Binance (Tier 1) vs Bitget (Tier 2)
        // Bitget has lower liquidity, so if it's the short leg, short is harder
        let result = identify_harder_leg("binance", "bitget");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_identify_harder_leg_tier1_vs_tier3() {
        // Binance (Tier 1) vs Hyperliquid (Tier 3)
        // Hyperliquid has lower liquidity, so if it's the long leg, long is harder
        let result = identify_harder_leg("hyperliquid", "binance");
        assert_eq!(result, "long");
    }

    #[test]
    fn test_identify_harder_leg_tier2_vs_tier3() {
        // KuCoin (Tier 2) vs Paradex (Tier 3)
        // Paradex has lower liquidity, so if it's the short leg, short is harder
        let result = identify_harder_leg("kucoin", "paradex");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_identify_harder_leg_same_tier_alphabetical() {
        // Both Tier 1: Binance vs Bybit
        // Same tier, so use alphabetical order: "binance" < "bybit"
        let result = identify_harder_leg("binance", "bybit");
        assert_eq!(result, "long");
    }

    #[test]
    fn test_identify_harder_leg_same_tier_alphabetical_reverse() {
        // Both Tier 1: Bybit vs Binance
        // Same tier, so use alphabetical order: "binance" < "bybit"
        let result = identify_harder_leg("bybit", "binance");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_identify_harder_leg_tier2_same_tier() {
        // Both Tier 2: Gate vs KuCoin
        // Same tier, so use alphabetical order: "gate" < "kucoin"
        let result = identify_harder_leg("gate", "kucoin");
        assert_eq!(result, "long");
    }

    #[test]
    fn test_identify_harder_leg_tier3_same_tier() {
        // Both Tier 3: Hyperliquid vs Paradex
        // Same tier, so use alphabetical order: "hyperliquid" < "paradex"
        let result = identify_harder_leg("hyperliquid", "paradex");
        assert_eq!(result, "long");
    }

    #[test]
    fn test_identify_harder_leg_case_insensitive() {
        // Test case insensitivity: "BINANCE" should be treated as "binance"
        let result1 = identify_harder_leg("BINANCE", "bitget");
        let result2 = identify_harder_leg("binance", "bitget");
        assert_eq!(result1, result2);
        assert_eq!(result1, "short");
    }

    #[test]
    fn test_identify_harder_leg_mixed_case() {
        // Test mixed case: "BiNaNcE" should be treated as "binance"
        let result = identify_harder_leg("BiNaNcE", "BITGET");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_identify_harder_leg_all_tier1_exchanges() {
        // Test all Tier 1 exchanges against each other
        let tier1_exchanges = vec!["binance", "bybit", "okx", "deribit"];
        
        for i in 0..tier1_exchanges.len() {
            for j in 0..tier1_exchanges.len() {
                if i != j {
                    let result = identify_harder_leg(tier1_exchanges[i], tier1_exchanges[j]);
                    // Should use alphabetical order
                    if tier1_exchanges[i] < tier1_exchanges[j] {
                        assert_eq!(result, "long");
                    } else {
                        assert_eq!(result, "short");
                    }
                }
            }
        }
    }

    #[test]
    fn test_identify_harder_leg_all_tier2_exchanges() {
        // Test all Tier 2 exchanges against each other
        let tier2_exchanges = vec!["bitget", "kucoin", "gate"];
        
        for i in 0..tier2_exchanges.len() {
            for j in 0..tier2_exchanges.len() {
                if i != j {
                    let result = identify_harder_leg(tier2_exchanges[i], tier2_exchanges[j]);
                    // Should use alphabetical order
                    if tier2_exchanges[i] < tier2_exchanges[j] {
                        assert_eq!(result, "long");
                    } else {
                        assert_eq!(result, "short");
                    }
                }
            }
        }
    }

    #[test]
    fn test_identify_harder_leg_all_tier3_exchanges() {
        // Test all Tier 3 exchanges against each other
        let tier3_exchanges = vec!["hyperliquid", "paradex", "lighter"];
        
        for i in 0..tier3_exchanges.len() {
            for j in 0..tier3_exchanges.len() {
                if i != j {
                    let result = identify_harder_leg(tier3_exchanges[i], tier3_exchanges[j]);
                    // Should use alphabetical order
                    if tier3_exchanges[i] < tier3_exchanges[j] {
                        assert_eq!(result, "long");
                    } else {
                        assert_eq!(result, "short");
                    }
                }
            }
        }
    }

    #[test]
    fn test_identify_harder_leg_unknown_exchange_treated_as_tier3() {
        // Unknown exchange should be treated as Tier 3 (conservative)
        // "unknown" (Tier 3) vs "binance" (Tier 1)
        let result = identify_harder_leg("unknown", "binance");
        assert_eq!(result, "long");
    }

    #[test]
    fn test_identify_harder_leg_unknown_vs_tier2() {
        // Unknown exchange (Tier 3) vs Tier 2
        let result = identify_harder_leg("bitget", "unknown_exchange");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_identify_harder_leg_deterministic() {
        // Test that the function is deterministic (same inputs always produce same output)
        let result1 = identify_harder_leg("binance", "hyperliquid");
        let result2 = identify_harder_leg("binance", "hyperliquid");
        let result3 = identify_harder_leg("binance", "hyperliquid");
        
        assert_eq!(result1, result2);
        assert_eq!(result2, result3);
        assert_eq!(result1, "short");
    }

    #[test]
    fn test_identify_harder_leg_gateio_variants() {
        // Test different variants of Gate.io name
        let result1 = identify_harder_leg("binance", "gate");
        let result2 = identify_harder_leg("binance", "gateio");
        let result3 = identify_harder_leg("binance", "gate.io");
        
        // All should identify short as harder (Gate is Tier 2)
        assert_eq!(result1, "short");
        assert_eq!(result2, "short");
        assert_eq!(result3, "short");
    }

    #[test]
    fn test_identify_harder_leg_returns_string() {
        // Verify that the function returns either "long" or "short"
        let result = identify_harder_leg("binance", "hyperliquid");
        assert!(result == "long" || result == "short");
    }

    #[test]
    fn test_identify_harder_leg_tier1_vs_unknown() {
        // Tier 1 vs unknown (treated as Tier 3)
        let result = identify_harder_leg("binance", "some_random_exchange");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_identify_harder_leg_consistency_across_calls() {
        // Verify consistency: calling with same exchanges multiple times gives same result
        let exchanges = vec![
            ("binance", "hyperliquid"),
            ("bybit", "paradex"),
            ("okx", "lighter"),
            ("kucoin", "paradex"),
        ];
        
        for (long_ex, short_ex) in exchanges {
            let result1 = identify_harder_leg(long_ex, short_ex);
            let result2 = identify_harder_leg(long_ex, short_ex);
            let result3 = identify_harder_leg(long_ex, short_ex);
            
            assert_eq!(result1, result2);
            assert_eq!(result2, result3);
        }
    }

    // Slippage Calculation Tests
    // **Validates: Requirements 3.8, 5.4**
    #[test]
    fn test_slippage_base_only() {
        // When position_size is very small relative to order_book_depth,
        // slippage should be just the base (2 bps = 0.0002)
        let slippage = EntryExecutor::calculate_slippage(100.0, 1000000.0);
        // base_slippage = 0.0002
        // depth_ratio = 100 / 1000000 = 0.0001
        // additional_slippage = 0.0001 * 0.0003 = 0.00000003
        // total = 0.0002 + 0.00000003 = 0.00020003
        assert!((slippage - 0.0002).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_with_additional() {
        // When position_size is significant relative to order_book_depth,
        // slippage should include additional component
        let slippage = EntryExecutor::calculate_slippage(10000.0, 100000.0);
        // base_slippage = 0.0002
        // depth_ratio = 10000 / 100000 = 0.1
        // additional_slippage = 0.1 * 0.0003 = 0.00003
        // total = 0.0002 + 0.00003 = 0.00023
        assert!((slippage - 0.00023).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_capped_at_5_bps() {
        // When position_size is very large relative to order_book_depth,
        // slippage should be capped at 5 bps (0.0005)
        let slippage = EntryExecutor::calculate_slippage(100000.0, 100000.0);
        // base_slippage = 0.0002
        // depth_ratio = 100000 / 100000 = 1.0
        // additional_slippage = 1.0 * 0.0003 = 0.0003
        // total = 0.0002 + 0.0003 = 0.0005 (exactly at cap)
        assert!((slippage - 0.0005).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_exceeds_cap() {
        // When position_size is extremely large relative to order_book_depth,
        // slippage should still be capped at 5 bps (0.0005)
        let slippage = EntryExecutor::calculate_slippage(1000000.0, 100000.0);
        // base_slippage = 0.0002
        // depth_ratio = 1000000 / 100000 = 10.0
        // additional_slippage = 10.0 * 0.0003 = 0.003
        // total = 0.0002 + 0.003 = 0.0032, but capped at 0.0005
        assert!((slippage - 0.0005).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_minimum_2_bps() {
        // Slippage should never be less than 2 bps (0.0002)
        let slippage = EntryExecutor::calculate_slippage(1.0, 1000000.0);
        // base_slippage = 0.0002
        // depth_ratio = 1 / 1000000 = 0.000001
        // additional_slippage = 0.000001 * 0.0003 = 0.0000000003
        // total = 0.0002 + 0.0000000003 ≈ 0.0002
        assert!(slippage >= 0.0002);
        assert!((slippage - 0.0002).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_realistic_scenario_1() {
        // Realistic scenario: $5000 position, $50k order book depth
        let slippage = EntryExecutor::calculate_slippage(5000.0, 50000.0);
        // base_slippage = 0.0002
        // depth_ratio = 5000 / 50000 = 0.1
        // additional_slippage = 0.1 * 0.0003 = 0.00003
        // total = 0.0002 + 0.00003 = 0.00023
        assert!((slippage - 0.00023).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_realistic_scenario_2() {
        // Realistic scenario: $10000 position, $100k order book depth
        let slippage = EntryExecutor::calculate_slippage(10000.0, 100000.0);
        // base_slippage = 0.0002
        // depth_ratio = 10000 / 100000 = 0.1
        // additional_slippage = 0.1 * 0.0003 = 0.00003
        // total = 0.0002 + 0.00003 = 0.00023
        assert!((slippage - 0.00023).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_realistic_scenario_3() {
        // Realistic scenario: $1000 position, $10k order book depth
        let slippage = EntryExecutor::calculate_slippage(1000.0, 10000.0);
        // base_slippage = 0.0002
        // depth_ratio = 1000 / 10000 = 0.1
        // additional_slippage = 0.1 * 0.0003 = 0.00003
        // total = 0.0002 + 0.00003 = 0.00023
        assert!((slippage - 0.00023).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_high_depth_ratio() {
        // High depth ratio: $50k position, $100k order book depth
        let slippage = EntryExecutor::calculate_slippage(50000.0, 100000.0);
        // base_slippage = 0.0002
        // depth_ratio = 50000 / 100000 = 0.5
        // additional_slippage = 0.5 * 0.0003 = 0.00015
        // total = 0.0002 + 0.00015 = 0.00035
        assert!((slippage - 0.00035).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_zero_position_size() {
        // Edge case: zero position size
        let slippage = EntryExecutor::calculate_slippage(0.0, 100000.0);
        // base_slippage = 0.0002
        // depth_ratio = 0 / 100000 = 0
        // additional_slippage = 0 * 0.0003 = 0
        // total = 0.0002
        assert!((slippage - 0.0002).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_very_small_order_book() {
        // Edge case: very small order book depth
        let slippage = EntryExecutor::calculate_slippage(1000.0, 100.0);
        // base_slippage = 0.0002
        // depth_ratio = 1000 / 100 = 10.0
        // additional_slippage = 10.0 * 0.0003 = 0.003
        // total = 0.0002 + 0.003 = 0.0032, but capped at 0.0005
        assert!((slippage - 0.0005).abs() < 0.00001);
    }

    #[test]
    fn test_slippage_always_between_2_and_5_bps() {
        // Property: slippage should always be between 2 bps and 5 bps
        let test_cases = vec![
            (100.0, 1000000.0),
            (1000.0, 100000.0),
            (5000.0, 50000.0),
            (10000.0, 100000.0),
            (50000.0, 100000.0),
            (100000.0, 100000.0),
            (1000000.0, 100000.0),
        ];

        for (position_size, order_book_depth) in test_cases {
            let slippage = EntryExecutor::calculate_slippage(position_size, order_book_depth);
            assert!(slippage >= 0.0002, "Slippage {} is below 2 bps minimum", slippage);
            assert!(slippage <= 0.0005, "Slippage {} exceeds 5 bps maximum", slippage);
        }
    }

    // Atomic Entry Execution Tests
    // **Validates: Requirements 3.7, 3.10, 4.4**
    use crate::strategy::types::{ArbitrageOpportunity, ConfluenceMetrics, HardConstraints};

    #[test]
    fn test_execute_atomic_entry_both_legs_fill() {
        // Create a test opportunity
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        assert_eq!(trade.symbol, "BTCUSDT");
        assert_eq!(trade.position_size_usd, 5000.0);
        assert_eq!(trade.status, TradeStatus::Active);
        assert!(trade.long_order.status == OrderStatus::Filled);
        assert!(trade.short_order.status == OrderStatus::Filled);
    }

    #[test]
    fn test_execute_atomic_entry_position_size_exceeds_capital() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 1000.0, 5000.0);
        
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds available capital"));
    }

    #[test]
    fn test_execute_atomic_entry_zero_position_size() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 0.0);
        
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be positive"));
    }

    #[test]
    fn test_execute_atomic_entry_negative_position_size() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, -1000.0);
        
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be positive"));
    }

    #[test]
    fn test_execute_atomic_entry_creates_limit_orders() {
        let opportunity = ArbitrageOpportunity {
            symbol: "ETHUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "hyperliquid".to_string(),
            long_price: 2000.0,
            short_price: 2010.0,
            spread_bps: 50.0,
            funding_delta_8h: 0.0001,
            confidence_score: 80,
            projected_profit_usd: 300.0,
            projected_profit_after_slippage: 280.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0001,
                funding_delta_projected: 0.0001,
                obi_ratio: 0.3,
                oi_current: 500.0,
                oi_24h_avg: 500.0,
                vwap_deviation: 0.02,
                atr: 0.5,
                atr_trend: true,
                liquidation_cluster_distance: 30.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 500.0,
            order_book_depth_short: 300.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 5000.0, 2000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // Verify orders are limit orders
        assert_eq!(trade.long_order.order_type, OrderType::Limit);
        assert_eq!(trade.short_order.order_type, OrderType::Limit);
        
        // Verify queue positions are set
        assert!(trade.long_order.queue_position.is_some());
        assert!(trade.short_order.queue_position.is_some());
    }

    #[test]
    fn test_execute_atomic_entry_queue_position_tracking() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // Verify queue positions have correct fill threshold
        if let Some(queue_pos) = &trade.long_order.queue_position {
            assert_eq!(queue_pos.fill_threshold_pct, 0.20);
            assert_eq!(queue_pos.resting_depth_at_entry, 1000.0);
        }
        
        if let Some(queue_pos) = &trade.short_order.queue_position {
            assert_eq!(queue_pos.fill_threshold_pct, 0.20);
            assert_eq!(queue_pos.resting_depth_at_entry, 1000.0);
        }
    }

    #[test]
    fn test_execute_atomic_entry_sets_trade_status_active() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        assert_eq!(trade.status, TradeStatus::Active);
    }

    #[test]
    fn test_execute_atomic_entry_calculates_entry_spread() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // Entry spread should be calculated as (short_price - long_price) / long_price * 10000
        // (101 - 100) / 100 * 10000 = 100 bps
        assert!((trade.entry_spread_bps - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_execute_atomic_entry_preserves_opportunity_data() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        assert_eq!(trade.symbol, opportunity.symbol);
        assert_eq!(trade.long_exchange, opportunity.long_exchange);
        assert_eq!(trade.short_exchange, opportunity.short_exchange);
        assert_eq!(trade.entry_long_price, opportunity.long_price);
        assert_eq!(trade.entry_short_price, opportunity.short_price);
        assert_eq!(trade.funding_delta_entry, opportunity.funding_delta_8h);
        assert_eq!(trade.projected_profit_usd, opportunity.projected_profit_after_slippage);
    }

    #[test]
    fn test_execute_atomic_entry_generates_unique_trade_ids() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result1 = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        let result2 = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result1.is_ok());
        assert!(result2.is_ok());
        
        let trade1 = result1.unwrap();
        let trade2 = result2.unwrap();
        
        // Trade IDs should be unique
        assert_ne!(trade1.id, trade2.id);
    }

    #[test]
    fn test_execute_atomic_entry_harder_leg_identification() {
        // Test with Binance (Tier 1) vs Hyperliquid (Tier 3)
        // Hyperliquid should be the harder leg
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "hyperliquid".to_string(),
            short_exchange: "binance".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // Both orders should be filled (simulated)
        assert_eq!(trade.long_order.status, OrderStatus::Filled);
        assert_eq!(trade.short_order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_execute_atomic_entry_sets_entry_time() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // Entry time should be between before and after
        assert!(trade.entry_time >= before);
        assert!(trade.entry_time <= after + 1);
    }

    #[test]
    fn test_execute_atomic_entry_initializes_profit_fields() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // Projected profit should be set from opportunity
        assert_eq!(trade.projected_profit_usd, 450.0);
        
        // Actual profit should be initialized to 0
        assert_eq!(trade.actual_profit_usd, 0.0);
        
        // Exit reason and time should be None
        assert!(trade.exit_reason.is_none());
        assert!(trade.exit_time.is_none());
    }

    #[test]
    fn test_execute_atomic_entry_no_leg_out_event_on_success() {
        let opportunity = ArbitrageOpportunity {
            symbol: "BTCUSDT".to_string(),
            long_exchange: "binance".to_string(),
            short_exchange: "bybit".to_string(),
            long_price: 100.0,
            short_price: 101.0,
            spread_bps: 100.0,
            funding_delta_8h: 0.0002,
            confidence_score: 75,
            projected_profit_usd: 500.0,
            projected_profit_after_slippage: 450.0,
            metrics: ConfluenceMetrics {
                funding_delta: 0.0002,
                funding_delta_projected: 0.0002,
                obi_ratio: 0.5,
                oi_current: 1000.0,
                oi_24h_avg: 1000.0,
                vwap_deviation: 0.01,
                atr: 1.0,
                atr_trend: true,
                liquidation_cluster_distance: 50.0,
                hard_constraints: HardConstraints {
                    order_book_depth_sufficient: true,
                    exchange_latency_ok: true,
                    funding_delta_substantial: true,
                },
            },
            order_book_depth_long: 1000.0,
            order_book_depth_short: 1000.0,
        };

        let result = EntryExecutor::execute_atomic_entry(&opportunity, 10000.0, 5000.0);
        
        assert!(result.is_ok());
        let trade = result.unwrap();
        
        // No leg-out event should occur on successful execution
        assert!(trade.leg_out_event.is_none());
    }
}
