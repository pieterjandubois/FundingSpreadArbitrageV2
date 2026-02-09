use crate::strategy::types::HardConstraints;
use std::collections::VecDeque;

pub struct ConfluenceCalculator {
    oi_history: VecDeque<f64>,
    vwap_history: VecDeque<(f64, f64)>,
    atr_history: VecDeque<f64>,
}

impl ConfluenceCalculator {
    pub fn new() -> Self {
        Self {
            oi_history: VecDeque::with_capacity(24),
            vwap_history: VecDeque::with_capacity(3600),
            atr_history: VecDeque::with_capacity(14),
        }
    }

    pub fn check_hard_constraints(
        order_book_depth_long: f64,
        order_book_depth_short: f64,
        position_size: f64,
        exchange_latency_ok: bool,
        funding_delta: f64,
    ) -> HardConstraints {
        HardConstraints {
            order_book_depth_sufficient: order_book_depth_long >= position_size * 2.0
                && order_book_depth_short >= position_size * 2.0,
            exchange_latency_ok,
            funding_delta_substantial: funding_delta.abs() > 0.0001,
        }
    }

    pub fn calculate_obi(bid_volume: f64, ask_volume: f64) -> f64 {
        if bid_volume + ask_volume == 0.0 {
            0.0
        } else {
            (bid_volume - ask_volume) / (bid_volume + ask_volume)
        }
    }

    pub fn calculate_vwap_deviation(current_price: f64, vwap: f64) -> f64 {
        if vwap == 0.0 {
            0.0
        } else {
            (current_price - vwap) / vwap
        }
    }

    pub fn calculate_atr(high: f64, low: f64, _close: f64, prev_close: f64) -> f64 {
        let tr1 = high - low;
        let tr2 = (high - prev_close).abs();
        let tr3 = (low - prev_close).abs();
        tr1.max(tr2).max(tr3)
    }

    pub fn update_oi_history(&mut self, oi: f64) {
        self.oi_history.push_back(oi);
        if self.oi_history.len() > 24 {
            self.oi_history.pop_front();
        }
    }

    pub fn update_vwap_history(&mut self, price: f64, volume: f64) {
        self.vwap_history.push_back((price, volume));
        if self.vwap_history.len() > 3600 {
            self.vwap_history.pop_front();
        }
    }

    pub fn update_atr_history(&mut self, atr: f64) {
        self.atr_history.push_back(atr);
        if self.atr_history.len() > 14 {
            self.atr_history.pop_front();
        }
    }

    pub fn get_atr_trend(&self) -> bool {
        if self.atr_history.len() < 2 {
            return false;
        }
        let recent_atr = self.atr_history.back().copied().unwrap_or(0.0);
        let prev_atr = self.atr_history.get(self.atr_history.len() - 2).copied().unwrap_or(0.0);
        recent_atr < prev_atr
    }

    pub fn identify_liquidation_clusters(
        liquidation_prices: &[f64],
        current_price: f64,
    ) -> f64 {
        if liquidation_prices.is_empty() {
            return 100.0;
        }

        let mut min_distance = f64::MAX;
        for &price in liquidation_prices {
            let distance = (price - current_price).abs();
            if distance < min_distance {
                min_distance = distance;
            }
        }

        if current_price == 0.0 {
            100.0
        } else {
            (min_distance / current_price) * 100.0
        }
    }
}
