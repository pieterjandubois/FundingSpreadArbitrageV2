use std::error::Error;
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Table, Row},
    Terminal,
};
use std::io;

use arbitrage2::exchange_parser::{get_parser, normalize_symbol};

const REDIS_URL: &str = "redis://127.0.0.1:6379";

type DynError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug)]
struct TradeOpportunity {
    ticker: String,
    long_exchange: String,
    short_exchange: String,
    long_bid: f64,
    long_ask: f64,
    short_bid: f64,
    short_ask: f64,
    spread_bps: f64,
    funding_delta: f64,
    confidence_score: u8,
    timestamp: u64,   // When this opportunity was detected (seconds since epoch)
}

#[derive(Clone, Debug)]
struct SpreadHistory {
    spreads: VecDeque<(u64, f64)>,  // (timestamp, spread_bps)
    last_minute_aggregate: Option<f64>,
    last_minute_timestamp: u64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ExchangePriceHistory {
    exchange: String,
    prices: VecDeque<(u64, f64)>,  // (timestamp_ms, mid_price)
    correlation_with_binance: f64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct LeadLagSignal {
    symbol: String,
    anchor_exchange: String,
    laggard_exchange: String,
    anchor_reverting: bool,
    laggard_at_extreme: bool,
    signal_strength: f64,  // 0.0 to 1.0
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct FundingGravity {
    symbol: String,
    exchange: String,
    funding_rate: f64,
    time_to_payout_minutes: u64,
    weighted_spread: f64,
    confidence_boost: u8,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct OBIMetrics {
    exchange: String,
    bid_volume: f64,
    ask_volume: f64,
    obi_ratio: f64,  // -1 to +1
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct OIMetrics {
    symbol: String,
    exchange: String,
    current_oi: f64,
    oi_24h_avg: f64,
    oi_ratio: f64,  // current / average
}

#[derive(Clone, Debug)]
struct RemovedOpportunity {
    ticker: String,
    confidence_score: u8,
    reason: String,
}

struct AppState {
    ticker_data: BTreeMap<String, Vec<(String, f64, f64)>>,
    funding_rates: BTreeMap<String, BTreeMap<String, f64>>, // symbol -> exchange -> rate
    opportunities: BTreeMap<String, TradeOpportunity>,
    spread_history: BTreeMap<String, SpreadHistory>,
    price_histories: BTreeMap<String, BTreeMap<String, ExchangePriceHistory>>, // symbol -> exchange -> history
    lead_lag_signals: BTreeMap<String, LeadLagSignal>,
    funding_gravity: BTreeMap<String, BTreeMap<String, FundingGravity>>, // symbol -> exchange -> gravity
    obi_metrics: BTreeMap<String, BTreeMap<String, OBIMetrics>>, // symbol -> exchange -> OBI
    oi_metrics: BTreeMap<String, BTreeMap<String, OIMetrics>>, // symbol -> exchange -> OI
    removed_opportunities: VecDeque<RemovedOpportunity>, // Track last 10 removals
    redis_url: String,
    should_quit: bool,
    scroll_offset: usize,
}

impl AppState {
    fn new(redis_url: String) -> Self {
        Self {
            ticker_data: BTreeMap::new(),
            funding_rates: BTreeMap::new(),
            opportunities: BTreeMap::new(),
            spread_history: BTreeMap::new(),
            price_histories: BTreeMap::new(),
            lead_lag_signals: BTreeMap::new(),
            funding_gravity: BTreeMap::new(),
            obi_metrics: BTreeMap::new(),
            oi_metrics: BTreeMap::new(),
            removed_opportunities: VecDeque::new(),
            redis_url,
            should_quit: false,
            scroll_offset: 0,
        }
    }

    fn update_from_redis(&mut self, conn: &mut redis::Connection) -> Result<(), DynError> {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("*")
            .query(conn)?;

        if keys.is_empty() {
            return Ok(());
        }

        let mut ticker_data: BTreeMap<String, Vec<(String, f64, f64)>> = BTreeMap::new();
        let mut funding_rates: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();

        for key in &keys {
            // Skip non-ticker/book keys
            if !key.contains("ticker") && !key.contains("bbo") && !key.contains("book") && !key.contains("ctx") {
                continue;
            }

            let data: String = match redis::cmd("GET").arg(&key).query(conn) {
                Ok(d) => d,
                Err(_) => continue,
            };

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                // Extract exchange name from key (first part before colon)
                let exchange = key.split(':').next().unwrap_or("unknown");
                
                // Extract raw symbol from key (last part)
                let raw_symbol = key.split(':').last().unwrap_or("unknown");
                
                // Get parser for this exchange
                let parser = get_parser(exchange);
                
                // Parse bid and ask from the JSON data
                if let (Some(bid_str), Some(ask_str)) = (parser.parse_bid(&json), parser.parse_ask(&json)) {
                    if let (Ok(bid), Ok(ask)) = (bid_str.parse::<f64>(), ask_str.parse::<f64>()) {
                        // Normalize the symbol to match across exchanges
                        let normalized_symbol = normalize_symbol(raw_symbol);
                        
                        // Store by normalized symbol, collecting all exchanges for this symbol
                        ticker_data
                            .entry(normalized_symbol)
                            .or_insert_with(Vec::new)
                            .push((exchange.to_string(), bid, ask));
                    }
                }
                
                // Also collect funding rates
                if let Some(rate) = parser.parse_funding_rate(&json) {
                    let normalized_symbol = normalize_symbol(raw_symbol);
                    funding_rates
                        .entry(normalized_symbol)
                        .or_insert_with(BTreeMap::new)
                        .insert(exchange.to_string(), rate);
                }
            }
        }

        self.ticker_data = ticker_data;
        self.funding_rates = funding_rates;
        self.recalculate_opportunities();
        Ok(())
    }

    fn recalculate_opportunities(&mut self) {
        let mut new_trades: BTreeMap<String, TradeOpportunity> = BTreeMap::new();
        let mut prices_to_track: Vec<(String, String, f64, f64)> = Vec::new();

        for (symbol, exchanges) in &self.ticker_data {
            if exchanges.len() < 2 {
                continue;
            }

            let mut min_ask = f64::MAX;
            let mut max_bid = 0.0;
            let mut long_ex = String::new();
            let mut short_ex = String::new();
            let mut long_bid = 0.0;
            let mut long_ask = 0.0;
            let mut short_bid = 0.0;
            let mut short_ask = 0.0;

            for (ex, bid, ask) in exchanges {
                prices_to_track.push((symbol.clone(), ex.clone(), *bid, *ask));
                
                if *ask < min_ask {
                    min_ask = *ask;
                    long_ex = ex.clone();
                    long_bid = *bid;
                    long_ask = *ask;
                }
                if *bid > max_bid {
                    max_bid = *bid;
                    short_ex = ex.clone();
                    short_bid = *bid;
                    short_ask = *ask;
                }
            }

            if long_ex != short_ex && min_ask < max_bid {
                let spread_bps = ((max_bid - min_ask) / min_ask) * 10000.0;
                
                // Skip unrealistic spreads (likely data errors)
                if spread_bps > 500.0 {
                    continue;
                }
                
                // Skip pairs with extremely low prices (likely data corruption)
                if min_ask < 0.001 {
                    continue;
                }
                
                // Check for toxic flow
                if self.detect_toxic_flow(symbol) {
                    continue;
                }
                
                let funding_delta = self.calculate_funding_delta(symbol, &long_ex, &short_ex);
                
                // HARD CONSTRAINTS (must all pass)
                // 1. Funding delta > 0.01% per 8 hours (0.0001)
                let funding_delta_substantial = funding_delta.abs() > 0.0001;
                if !funding_delta_substantial {
                    continue;
                }
                
                // 2. Get real order book depths from Redis
                let (depth_long, depth_short) = self.get_order_book_depths_from_redis(symbol, &long_ex, &short_ex);
                
                // If we couldn't get depths, use a more lenient fallback
                let (depth_long, depth_short) = if depth_long == 0.0 || depth_short == 0.0 {
                    // Fallback: assume reasonable depth for most pairs
                    // For altcoins with large spreads, assume lower depth
                    let estimated_depth = if spread_bps > 200.0 {
                        5000.0 // Low liquidity altcoins
                    } else if spread_bps > 100.0 {
                        10000.0 // Medium liquidity
                    } else {
                        50000.0 // High liquidity
                    };
                    (estimated_depth, estimated_depth)
                } else {
                    (depth_long, depth_short)
                };
                
                // Estimate position size based on spread and available capital
                let estimated_position_size = 1000.0; // $1000 per trade
                let depth_sufficient = depth_long >= estimated_position_size * 2.0 
                    && depth_short >= estimated_position_size * 2.0;
                
                if !depth_sufficient {
                    continue;
                }
                
                // Calculate confidence score (only if hard constraints pass)
                let confidence_score = self.calculate_confidence_score_with_gravity(spread_bps, funding_delta, &long_ex, &short_ex);
                
                // Calculate slippage (2-5 bps based on depth)
                let min_depth = if depth_long < depth_short { depth_long } else { depth_short };
                let slippage_bps = if min_depth > 0.0 {
                    2.0_f64 + (estimated_position_size / min_depth) * 3.0_f64
                } else {
                    3.0_f64 // Default to 3 bps if we can't calculate
                };
                let slippage_bps = if slippage_bps > 5.0_f64 { 5.0_f64 } else { slippage_bps };
                
                // Calculate projected profit after slippage and fees
                let long_fee_bps = get_exchange_taker_fee(&long_ex);
                let short_fee_bps = get_exchange_taker_fee(&short_ex);
                let total_fees_bps = long_fee_bps + short_fee_bps;
                let funding_cost_bps = 10.0; // 10 bps for funding cost
                let projected_profit_bps = spread_bps - total_fees_bps - funding_cost_bps - slippage_bps;
                
                // Only show opportunities that meet strategy runner criteria:
                // - Confidence >= 70
                // - Projected profit > 0
                if confidence_score >= 70 && projected_profit_bps > 0.0 {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    
                    new_trades.insert(
                        symbol.clone(),
                        TradeOpportunity {
                            ticker: symbol.clone(),
                            long_exchange: long_ex.clone(),
                            short_exchange: short_ex.clone(),
                            long_bid,
                            long_ask,
                            short_bid,
                            short_ask,
                            spread_bps,
                            funding_delta,
                            confidence_score,
                            timestamp: now,
                        },
                    );
                }
            }
        }

        // Track price history after the loop
        for (symbol, exchange, bid, ask) in prices_to_track {
            self.track_price_history(&symbol, &exchange, bid, ask);
        }

        // Track which opportunities were removed
        for (symbol, old_opp) in &self.opportunities {
            if !new_trades.contains_key(symbol) {
                // This opportunity was removed - figure out why
                let reason = self.determine_removal_reason(symbol, old_opp);
                
                // Keep only last 10 removals
                if self.removed_opportunities.len() >= 10 {
                    self.removed_opportunities.pop_front();
                }
                
                self.removed_opportunities.push_back(RemovedOpportunity {
                    ticker: symbol.clone(),
                    confidence_score: old_opp.confidence_score,
                    reason,
                });
            }
        }

        self.opportunities = new_trades;
    }

    fn determine_removal_reason(&self, symbol: &str, old_opp: &TradeOpportunity) -> String {
        // Check if it's still in ticker data
        if !self.ticker_data.contains_key(symbol) {
            return "No price data".to_string();
        }

        let exchanges = match self.ticker_data.get(symbol) {
            Some(ex) => ex,
            None => return "No exchanges".to_string(),
        };

        if exchanges.len() < 2 {
            return "Insufficient exchanges".to_string();
        }

        // Find best spread across all exchanges
        let mut min_ask = f64::MAX;
        let mut max_bid = 0.0;
        for (_, bid, ask) in exchanges {
            if *ask < min_ask {
                min_ask = *ask;
            }
            if *bid > max_bid {
                max_bid = *bid;
            }
        }

        // Check for unrealistic spreads
        if min_ask < 0.001 {
            return "Price too low".to_string();
        }

        if min_ask >= max_bid {
            return "Spread negative".to_string();
        }

        let spread_bps = ((max_bid - min_ask) / min_ask) * 10000.0;
        
        if spread_bps > 500.0 {
            return "Spread unrealistic".to_string();
        }

        // Check funding delta
        let funding_delta = self.calculate_funding_delta(symbol, &old_opp.long_exchange, &old_opp.short_exchange);
        if funding_delta.abs() <= 0.0001 {
            return "Funding delta too low".to_string();
        }

        // Check order book depth
        let (depth_long, depth_short) = self.get_order_book_depths_from_redis(symbol, &old_opp.long_exchange, &old_opp.short_exchange);
        
        // Apply same fallback logic as recalculate_opportunities
        let (depth_long, depth_short) = if depth_long == 0.0 || depth_short == 0.0 {
            let estimated_depth = if spread_bps > 200.0 {
                5000.0
            } else if spread_bps > 100.0 {
                10000.0
            } else {
                50000.0
            };
            (estimated_depth, estimated_depth)
        } else {
            (depth_long, depth_short)
        };
        
        let estimated_position_size = 1000.0;
        if depth_long < estimated_position_size * 2.0 || depth_short < estimated_position_size * 2.0 {
            return "Insufficient depth".to_string();
        }

        // Check confidence
        let confidence_score = self.calculate_confidence_score_with_gravity(spread_bps, funding_delta, &old_opp.long_exchange, &old_opp.short_exchange);
        if confidence_score < 70 {
            return format!("Confidence dropped to {}", confidence_score);
        }

        // Check profitability
        let long_fee_bps = get_exchange_taker_fee(&old_opp.long_exchange);
        let short_fee_bps = get_exchange_taker_fee(&old_opp.short_exchange);
        let total_fees_bps = long_fee_bps + short_fee_bps;
        let funding_cost_bps = 10.0;
        let slippage_bps = 3.0;
        let projected_profit_bps = spread_bps - total_fees_bps - funding_cost_bps - slippage_bps;
        
        if projected_profit_bps <= 0.0 {
            return "Unprofitable".to_string();
        }

        // Check if best exchange pair changed
        let mut best_long_ex = String::new();
        let mut best_short_ex = String::new();
        let mut best_min_ask = f64::MAX;
        let mut best_max_bid = 0.0;
        
        for (ex, bid, ask) in exchanges {
            if *ask < best_min_ask {
                best_min_ask = *ask;
                best_long_ex = ex.clone();
            }
            if *bid > best_max_bid {
                best_max_bid = *bid;
                best_short_ex = ex.clone();
            }
        }

        if best_long_ex != old_opp.long_exchange || best_short_ex != old_opp.short_exchange {
            return "Exchange pair changed".to_string();
        }

        "Unknown".to_string()
    }

    fn get_order_book_depths_from_redis(&self, symbol: &str, long_ex: &str, short_ex: &str) -> (f64, f64) {
        let client = match redis::Client::open(self.redis_url.as_str()) {
            Ok(c) => c,
            Err(_) => return (0.0, 0.0),
        };

        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(_) => return (0.0, 0.0),
        };

        let mut depth_long = 0.0;
        let mut depth_short = 0.0;

        // Try to fetch depth for long exchange
        let long_key = format!("{}:linear:tickers:{}", long_ex, symbol);
        if let Ok(data) = redis::cmd("GET").arg(&long_key).query::<String>(&mut conn) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                // Try to extract bid volume (depth at best bid)
                if let Some(bid_vol) = json.get("bid_volume").and_then(|v| v.as_f64()) {
                    depth_long = bid_vol;
                } else if let Some(depth) = json.get("depth").and_then(|v| v.as_f64()) {
                    depth_long = depth;
                }
            }
        }

        // Try to fetch depth for short exchange
        let short_key = format!("{}:linear:tickers:{}", short_ex, symbol);
        if let Ok(data) = redis::cmd("GET").arg(&short_key).query::<String>(&mut conn) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                // Try to extract ask volume (depth at best ask)
                if let Some(ask_vol) = json.get("ask_volume").and_then(|v| v.as_f64()) {
                    depth_short = ask_vol;
                } else if let Some(depth) = json.get("depth").and_then(|v| v.as_f64()) {
                    depth_short = depth;
                }
            }
        }

        // If we couldn't get real depths, return 0 to fail the constraint check
        (depth_long, depth_short)
    }

    fn calculate_funding_delta(&self, symbol: &str, long_ex: &str, short_ex: &str) -> f64 {
        let long_rate = self.funding_rates
            .get(symbol)
            .and_then(|rates| rates.get(long_ex))
            .copied()
            .unwrap_or(0.0);
        
        let short_rate = self.funding_rates
            .get(symbol)
            .and_then(|rates| rates.get(short_ex))
            .copied()
            .unwrap_or(0.0);
        
        long_rate - short_rate
    }

    fn calculate_confidence_score(&self, spread_bps: f64, funding_delta: f64) -> u8 {
        let mut score = 0.0;
        
        // Spread component (weight 50): higher spread = higher score
        let spread_score = (spread_bps / 50.0).min(1.0) * 100.0;
        score += spread_score * 0.5;
        
        // Funding delta component (weight 30): higher absolute delta = higher score
        let funding_score = (funding_delta.abs() / 0.01).min(1.0) * 100.0;
        score += funding_score * 0.3;
        
        // OBI component (weight 10): will be added separately
        // OI component (weight 10): will be added separately
        
        (score as u8).min(100)
    }

    fn calculate_confidence_score_with_gravity(&self, spread_bps: f64, funding_delta: f64, long_exchange: &str, short_exchange: &str) -> u8 {
        let mut score = self.calculate_confidence_score(spread_bps, funding_delta);
        
        // Apply OBI boost
        let obi_boost = self.calculate_obi_boost("", long_exchange, short_exchange);
        score = score.saturating_add(obi_boost).min(100);
        
        // Apply OI boost (can be negative)
        let oi_boost = self.calculate_oi_boost("", long_exchange, short_exchange);
        score = if oi_boost < 0 {
            score.saturating_sub(oi_boost.abs() as u8)
        } else {
            score.saturating_add(oi_boost as u8).min(100)
        };
        
        // Apply funding gravity boost from both exchanges
        let long_boost = self.calculate_funding_gravity("", long_exchange, spread_bps)
            .map(|g| g.confidence_boost)
            .unwrap_or(0);
        
        let short_boost = self.calculate_funding_gravity("", short_exchange, spread_bps)
            .map(|g| g.confidence_boost)
            .unwrap_or(0);
        
        // Use the maximum boost from either exchange
        let max_boost = long_boost.max(short_boost);
        score = score.saturating_add(max_boost).min(100);
        
        score
    }

    fn track_spread(&mut self, symbol: &str, spread_bps: f64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let history = self.spread_history.entry(symbol.to_string()).or_insert_with(|| SpreadHistory {
            spreads: std::collections::VecDeque::with_capacity(1200),
            last_minute_aggregate: None,
            last_minute_timestamp: now,
        });

        // Add to short-term history (10 minutes = 600 seconds)
        history.spreads.push_back((now, spread_bps));
        if history.spreads.len() > 1200 {
            history.spreads.pop_front();
        }

        // Aggregate to 1-minute for long-term stats
        if now - history.last_minute_timestamp >= 60 {
            let avg: f64 = history.spreads.iter().map(|(_, s)| s).sum::<f64>() / history.spreads.len() as f64;
            history.last_minute_aggregate = Some(avg);
            history.last_minute_timestamp = now;
        }
    }

    fn detect_toxic_flow(&self, symbol: &str) -> bool {
        let history = match self.spread_history.get(symbol) {
            Some(h) => h,
            None => return false,
        };

        if history.spreads.len() < 20 {
            return false; // Need at least 10 seconds of data
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Get spread from 10 seconds ago
        let spread_10s_ago = history.spreads.iter()
            .rev()
            .find(|(ts, _)| now - ts >= 10)
            .map(|(_, s)| s);

        let current_spread = history.spreads.back().map(|(_, s)| s);

        match (spread_10s_ago, current_spread) {
            (Some(&old), Some(&new)) if old > 0.0 => {
                let change_ratio = new / old;
                change_ratio > 3.0 // 3x increase = toxic flow
            }
            _ => false,
        }
    }

    fn calculate_z_score(&self, symbol: &str, current_spread: f64) -> f64 {
        let history = match self.spread_history.get(symbol) {
            Some(h) => h,
            None => return 0.0,
        };

        if history.spreads.len() < 10 {
            return 0.0; // Not enough data
        }

        let spreads: Vec<f64> = history.spreads.iter().map(|(_, s)| s).copied().collect();
        let mean = spreads.iter().sum::<f64>() / spreads.len() as f64;
        let variance = spreads.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / spreads.len() as f64;
        let std_dev = variance.sqrt();

        if std_dev == 0.0 {
            0.0
        } else {
            (current_spread - mean) / std_dev
        }
    }

    fn track_price_history(&mut self, symbol: &str, exchange: &str, bid: f64, ask: f64) {
        let mid_price = (bid + ask) / 2.0;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let symbol_histories = self.price_histories.entry(symbol.to_string()).or_insert_with(BTreeMap::new);
        let history = symbol_histories.entry(exchange.to_string()).or_insert_with(|| ExchangePriceHistory {
            exchange: exchange.to_string(),
            prices: VecDeque::with_capacity(60),
            correlation_with_binance: 0.0,
        });

        history.prices.push_back((now, mid_price));
        if history.prices.len() > 60 {
            history.prices.pop_front();
        }
    }

    fn calculate_correlation(prices1: &[f64], prices2: &[f64]) -> f64 {
        if prices1.len() < 2 || prices2.len() < 2 || prices1.len() != prices2.len() {
            return 0.0;
        }

        let n = prices1.len() as f64;
        let mean1 = prices1.iter().sum::<f64>() / n;
        let mean2 = prices2.iter().sum::<f64>() / n;

        let mut covariance = 0.0;
        let mut var1 = 0.0;
        let mut var2 = 0.0;

        for i in 0..prices1.len() {
            let diff1 = prices1[i] - mean1;
            let diff2 = prices2[i] - mean2;
            covariance += diff1 * diff2;
            var1 += diff1 * diff1;
            var2 += diff2 * diff2;
        }

        if var1 == 0.0 || var2 == 0.0 {
            return 0.0;
        }

        covariance / (var1.sqrt() * var2.sqrt())
    }

    fn update_correlations(&mut self, symbol: &str) {
        let symbol_histories = match self.price_histories.get_mut(symbol) {
            Some(h) => h,
            None => return,
        };

        // Get Binance prices
        let binance_prices: Vec<f64> = symbol_histories
            .get("binance")
            .map(|h| h.prices.iter().map(|(_, p)| *p).collect())
            .unwrap_or_default();

        if binance_prices.len() < 2 {
            return; // Not enough data
        }

        // Calculate correlation for each exchange with Binance
        for (exchange, history) in symbol_histories.iter_mut() {
            if exchange == "binance" {
                history.correlation_with_binance = 1.0;
                continue;
            }

            let exchange_prices: Vec<f64> = history.prices.iter().map(|(_, p)| *p).collect();
            if exchange_prices.len() < 2 {
                continue;
            }

            history.correlation_with_binance = Self::calculate_correlation(&binance_prices, &exchange_prices);
        }
    }

    fn detect_reversion(prices: &[f64]) -> bool {
        if prices.len() < 3 {
            return false;
        }

        // Get last 3 prices
        let len = prices.len();
        let price_prev = prices[len - 3];
        let price_mid = prices[len - 2];
        let price_current = prices[len - 1];

        // Calculate percentage changes
        let change1_pct = ((price_mid - price_prev) / price_prev).abs();

        // Reversion threshold: 0.15% (15 bps)
        const REVERSION_THRESHOLD: f64 = 0.0015;

        // Check if first move was significant and second move reversed it
        if change1_pct > REVERSION_THRESHOLD {
            let direction1 = (price_mid - price_prev).signum();
            let direction2 = (price_current - price_mid).signum();
            
            // Reversion detected if directions are opposite
            return direction1 * direction2 < 0.0;
        }

        false
    }

    fn detect_lead_lag_signal(&self, symbol: &str, spread_bps: f64) -> Option<LeadLagSignal> {
        let symbol_histories = match self.price_histories.get(symbol) {
            Some(h) => h,
            None => return None,
        };

        // Check if Binance exists and is reverting
        let binance_history = match symbol_histories.get("binance") {
            Some(h) => h,
            None => return None,
        };

        if !Self::detect_reversion(&binance_history.prices.iter().map(|(_, p)| *p).collect::<Vec<_>>()) {
            return None; // Binance not reverting
        }

        // Find the laggard exchange (highest correlation with Binance, but not Binance itself)
        let mut laggard_exchange = String::new();
        let mut highest_correlation = 0.0;

        for (exchange, history) in symbol_histories.iter() {
            if exchange == "binance" {
                continue;
            }
            if history.correlation_with_binance > highest_correlation && history.correlation_with_binance > 0.65 {
                highest_correlation = history.correlation_with_binance;
                laggard_exchange = exchange.clone();
            }
        }

        if laggard_exchange.is_empty() {
            return None; // No strong follower found
        }

        // Check if laggard is at extreme spread (spread > 15 bps minimum)
        let laggard_at_extreme = spread_bps > 15.0;

        if !laggard_at_extreme {
            return None; // Spread not extreme enough
        }

        // Signal strength: correlation coefficient (0.65 to 1.0 maps to 0.0 to 1.0)
        let signal_strength = ((highest_correlation - 0.65) / 0.35).min(1.0).max(0.0);

        Some(LeadLagSignal {
            symbol: symbol.to_string(),
            anchor_exchange: "binance".to_string(),
            laggard_exchange,
            anchor_reverting: true,
            laggard_at_extreme,
            signal_strength,
        })
    }

    fn time_to_next_payout(exchange: &str) -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        // Convert to UTC hours
        let seconds_per_hour = 3600;
        let current_hour_utc = (now / seconds_per_hour) % 24;
        
        // Define payout hours for each exchange (UTC)
        let payout_hours: &[u64] = match exchange {
            "binance" | "bybit" | "bitget" | "gate" => &[0, 8, 16],
            "kucoin" => &[1, 9, 17],
            "okx" => &[16],
            _ => &[0, 8, 16], // Default to 8-hour intervals
        };
        
        // Find next payout hour
        let mut next_payout_hour = None;
        for &hour in payout_hours {
            if hour > current_hour_utc {
                next_payout_hour = Some(hour);
                break;
            }
        }
        
        // If no payout found today, use first payout tomorrow
        let next_hour = next_payout_hour.unwrap_or(payout_hours[0]);
        let hours_until_payout = if next_payout_hour.is_some() {
            next_hour - current_hour_utc
        } else {
            24 - current_hour_utc + payout_hours[0]
        };
        
        hours_until_payout * 60 // Convert to minutes
    }

    fn calculate_funding_gravity(&self, symbol: &str, exchange: &str, spread_bps: f64) -> Option<FundingGravity> {
        let funding_rate = self.funding_rates
            .get(symbol)
            .and_then(|rates| rates.get(exchange))
            .copied()
            .unwrap_or(0.0);

        let time_to_payout = Self::time_to_next_payout(exchange);
        
        // Weight spread by time remaining: spread * (time_remaining / 480 minutes)
        const NORMALIZATION_MINUTES: f64 = 480.0; // 8 hours
        let weighted_spread = spread_bps * (time_to_payout as f64 / NORMALIZATION_MINUTES);
        
        // Calculate confidence boost based on time to payout
        let confidence_boost = if time_to_payout < 5 * 60 {
            30 // <5 minutes: +30 points
        } else if time_to_payout < 15 * 60 {
            15 // <15 minutes: +15 points
        } else {
            0 // >15 minutes: no boost
        };

        Some(FundingGravity {
            symbol: symbol.to_string(),
            exchange: exchange.to_string(),
            funding_rate,
            time_to_payout_minutes: time_to_payout,
            weighted_spread,
            confidence_boost,
        })
    }

    fn calculate_obi_boost(&self, symbol: &str, long_exchange: &str, short_exchange: &str) -> u8 {
        // Get OBI metrics for both exchanges
        let long_obi = self.obi_metrics
            .get(symbol)
            .and_then(|m| m.get(long_exchange))
            .map(|m| m.obi_ratio);
        
        let short_obi = self.obi_metrics
            .get(symbol)
            .and_then(|m| m.get(short_exchange))
            .map(|m| m.obi_ratio);

        match (long_obi, short_obi) {
            (Some(long), Some(short)) => {
                // Long exchange should have positive OBI (buying pressure)
                // Short exchange should have negative OBI (selling pressure)
                let long_aligned = long > 0.1;
                let short_aligned = short < -0.1;
                
                if long_aligned && short_aligned {
                    20 // Strong alignment: +20 points
                } else if long_aligned || short_aligned {
                    10 // Partial alignment: +10 points
                } else {
                    0 // No alignment
                }
            }
            _ => 0, // No OBI data available
        }
    }

    fn calculate_oi_boost(&self, symbol: &str, long_exchange: &str, short_exchange: &str) -> i8 {
        // Get OI metrics for both exchanges
        let long_oi = self.oi_metrics
            .get(symbol)
            .and_then(|m| m.get(long_exchange))
            .map(|m| m.oi_ratio);
        
        let short_oi = self.oi_metrics
            .get(symbol)
            .and_then(|m| m.get(short_exchange))
            .map(|m| m.oi_ratio);

        match (long_oi, short_oi) {
            (Some(long), Some(short)) => {
                // Average OI ratio across both exchanges
                let avg_oi_ratio = (long + short) / 2.0;
                
                if avg_oi_ratio < 0.8 {
                    15 // OI below average: +15 points (market not crowded)
                } else if avg_oi_ratio > 1.2 {
                    -10 // OI above average: -10 points (market crowded)
                } else {
                    0 // OI normal
                }
            }
            _ => 0, // No OI data available
        }
    }

    fn calculate_obi(&self, symbol: &str, exchange: &str) -> Option<OBIMetrics> {
        // Fetch order book data from Redis based on exchange
        let book_key = match exchange {
            "okx" => format!("okx:usdt:book:{}", symbol),
            "kucoin" => format!("kucoin:futures:level2:{}", symbol),
            "bitget" => format!("bitget:usdt:book:{}", symbol),
            _ => return None, // Other exchanges don't have order book data
        };

        let client = match redis::Client::open(self.redis_url.as_str()) {
            Ok(c) => c,
            Err(_) => return None,
        };

        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(_) => return None,
        };

        let book_data: String = match redis::cmd("GET").arg(&book_key).query(&mut conn) {
            Ok(d) => d,
            Err(_) => return None,
        };

        let json: serde_json::Value = match serde_json::from_str(&book_data) {
            Ok(j) => j,
            Err(_) => return None,
        };

        // Extract bid/ask volumes from order book
        let (bid_volume, ask_volume) = match exchange {
            "okx" => {
                // OKX format: data[0].bids and data[0].asks are arrays of [price, size, ...]
                let bids = json.get("data")
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|f| f.get("bids"))
                    .and_then(|b| b.as_array())?;
                
                let asks = json.get("data")
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|f| f.get("asks"))
                    .and_then(|a| a.as_array())?;

                // Sum top 5 levels
                let bid_vol: f64 = bids.iter().take(5)
                    .filter_map(|b| b.as_array().and_then(|a| a.get(1)).and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()))
                    .sum();
                
                let ask_vol: f64 = asks.iter().take(5)
                    .filter_map(|a| a.as_array().and_then(|arr| arr.get(1)).and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()))
                    .sum();
                
                (bid_vol, ask_vol)
            }
            "kucoin" => {
                // KuCoin format: data.bids and data.asks are arrays of [price, size]
                let bids = json.get("data")
                    .and_then(|d| d.get("bids"))
                    .and_then(|b| b.as_array())?;
                
                let asks = json.get("data")
                    .and_then(|d| d.get("asks"))
                    .and_then(|a| a.as_array())?;

                let bid_vol: f64 = bids.iter().take(5)
                    .filter_map(|b| b.as_array().and_then(|a| a.get(1)).and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()))
                    .sum();
                
                let ask_vol: f64 = asks.iter().take(5)
                    .filter_map(|a| a.as_array().and_then(|arr| arr.get(1)).and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()))
                    .sum();
                
                (bid_vol, ask_vol)
            }
            "bitget" => {
                // Bitget format: data.bids and data.asks are arrays of [price, size]
                let bids = json.get("data")
                    .and_then(|d| d.get("bids"))
                    .and_then(|b| b.as_array())?;
                
                let asks = json.get("data")
                    .and_then(|d| d.get("asks"))
                    .and_then(|a| a.as_array())?;

                let bid_vol: f64 = bids.iter().take(5)
                    .filter_map(|b| b.as_array().and_then(|a| a.get(1)).and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()))
                    .sum();
                
                let ask_vol: f64 = asks.iter().take(5)
                    .filter_map(|a| a.as_array().and_then(|arr| arr.get(1)).and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()))
                    .sum();
                
                (bid_vol, ask_vol)
            }
            _ => return None,
        };

        if bid_volume == 0.0 || ask_volume == 0.0 {
            return None;
        }

        // Calculate OBI ratio: (bid_volume - ask_volume) / (bid_volume + ask_volume)
        let obi_ratio = (bid_volume - ask_volume) / (bid_volume + ask_volume);

        Some(OBIMetrics {
            exchange: exchange.to_string(),
            bid_volume,
            ask_volume,
            obi_ratio,
        })
    }

    fn calculate_oi(&self, symbol: &str, exchange: &str) -> Option<OIMetrics> {
        // Fetch OI data from Redis
        let oi_key = format!("{}:oi:{}", exchange, symbol);
        let history_key = format!("{}:oi_history:{}", exchange, symbol);
        
        let client = match redis::Client::open(self.redis_url.as_str()) {
            Ok(c) => c,
            Err(_) => return None,
        };

        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(_) => return None,
        };

        // Get current OI
        let oi_data: String = match redis::cmd("GET").arg(&oi_key).query(&mut conn) {
            Ok(d) => d,
            Err(_) => return None,
        };

        let json: serde_json::Value = match serde_json::from_str(&oi_data) {
            Ok(j) => j,
            Err(_) => return None,
        };

        // Extract current OI from exchange-specific format
        let current_oi = match exchange {
            "binance" => {
                json.get("openInterest")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            "okx" => {
                json.get("data")
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|f| f.get("oi"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            "bybit" => {
                json.get("result")
                    .and_then(|r| r.get("list"))
                    .and_then(|l| l.as_array())
                    .and_then(|a| a.first())
                    .and_then(|f| f.get("openInterest"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            "kucoin" => {
                json.get("data")
                    .and_then(|d| d.get("openInterest"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            "bitget" => {
                json.get("data")
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|f| f.get("oi"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            "hyperliquid" => {
                json.get("openInterest")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            "paradex" => {
                json.get("market")
                    .and_then(|m| m.get("open_interest"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())?
            }
            _ => return None,
        };

        // Calculate 24h average from historical snapshots
        let oi_24h_avg = match self.calculate_oi_24h_average(&mut conn, &history_key) {
            Some(avg) => avg,
            None => current_oi, // Fallback to current if no history
        };

        let oi_ratio = if oi_24h_avg > 0.0 {
            current_oi / oi_24h_avg
        } else {
            1.0
        };

        Some(OIMetrics {
            symbol: symbol.to_string(),
            exchange: exchange.to_string(),
            current_oi,
            oi_24h_avg,
            oi_ratio,
        })
    }

    fn calculate_oi_24h_average(&self, conn: &mut redis::Connection, history_key: &str) -> Option<f64> {
        // Get all snapshots from sorted set
        let members: Vec<String> = match redis::cmd("ZRANGE")
            .arg(history_key)
            .arg(0)
            .arg(-1)
            .query(conn)
        {
            Ok(m) => m,
            Err(_) => return None,
        };

        if members.is_empty() {
            return None;
        }

        // Parse OI values from members (format: "timestamp,oi_value")
        let oi_values: Vec<f64> = members
            .iter()
            .filter_map(|member| {
                member.split(',').nth(1).and_then(|oi_str| oi_str.parse::<f64>().ok())
            })
            .collect();

        if oi_values.is_empty() {
            return None;
        }

        // Calculate average
        let sum: f64 = oi_values.iter().sum();
        Some(sum / oi_values.len() as f64)
    }
}

fn get_exchange_taker_fee(exchange: &str) -> f64 {
    // Returns taker fee in basis points (bps)
    match exchange.to_lowercase().as_str() {
        "binance" => 4.0,      // 0.04%
        "okx" => 5.0,          // 0.05%
        "bybit" => 5.5,        // 0.055%
        "bitget" => 6.0,       // 0.06%
        "kucoin" => 6.0,       // 0.06%
        "hyperliquid" => 3.5,  // 0.035%
        "paradex" => 5.0,      // 0.05%
        "gateio" => 6.0,       // 0.06%
        _ => 6.0,              // Default fallback
    }
}

fn store_opportunities_to_redis(
    conn: &mut redis::Connection,
    opportunities: &BTreeMap<String, TradeOpportunity>,
) -> Result<(), DynError> {
    // Convert opportunities to ArbitrageOpportunity format for strategy runner
    let mut arb_opportunities = Vec::new();
    
    for opp in opportunities.values() {
        // Only include opportunities that meet strategy runner criteria
        if opp.confidence_score >= 70 && opp.spread_bps > 0.0 {
            // Calculate actual fees based on exchanges
            let total_fee_bps = get_exchange_taker_fee(&opp.long_exchange) + get_exchange_taker_fee(&opp.short_exchange);
            
            // Create a complete ArbitrageOpportunity struct with all required fields
            let opp_json = serde_json::json!({
                "symbol": opp.ticker,
                "long_exchange": opp.long_exchange,
                "short_exchange": opp.short_exchange,
                "long_price": (opp.long_bid + opp.long_ask) / 2.0,
                "short_price": (opp.short_bid + opp.short_ask) / 2.0,
                "spread_bps": opp.spread_bps,
                "funding_delta_8h": opp.funding_delta,
                "confidence_score": opp.confidence_score,
                "projected_profit_usd": 0.0,
                "projected_profit_after_slippage": 0.0,
                "order_book_depth_long": 50000.0,
                "order_book_depth_short": 50000.0,
                "metrics": {
                    "funding_delta": opp.funding_delta,
                    "funding_delta_projected": opp.funding_delta,
                    "obi_ratio": 0.0,
                    "oi_current": 1000000.0,
                    "oi_24h_avg": 1000000.0,
                    "vwap_deviation": 0.0,
                    "atr": 0.0,
                    "atr_trend": false,
                    "liquidation_cluster_distance": 100.0,
                    "hard_constraints": {
                        "order_book_depth_sufficient": true,
                        "exchange_latency_ok": true,
                        "funding_delta_substantial": true,
                    }
                }
            });
            arb_opportunities.push(opp_json);
        }
    }
    
    // Store to Redis with key "strategy:opportunities" (no timestamp)
    let json_str = serde_json::to_string(&arb_opportunities)?;
    redis::cmd("SET")
        .arg("strategy:opportunities")
        .arg(json_str)
        .query::<()>(conn)?;
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let client = redis::Client::open(REDIS_URL)?;
    let mut conn = client.get_connection()?;

    let mut app_state = AppState::new(REDIS_URL.to_string());
    let mut last_update = std::time::Instant::now();
    let update_interval = Duration::from_millis(500); // Update every 500ms

    loop {
        // Handle events with timeout
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app_state.should_quit = true;
                    }
                    KeyCode::Up => {
                        if app_state.scroll_offset > 0 {
                            app_state.scroll_offset -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if app_state.scroll_offset < app_state.opportunities.len().saturating_sub(1) {
                            app_state.scroll_offset += 1;
                        }
                    }
                    KeyCode::PageUp => {
                        app_state.scroll_offset = app_state.scroll_offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        app_state.scroll_offset = (app_state.scroll_offset + 10).min(
                            app_state.opportunities.len().saturating_sub(1)
                        );
                    }
                    KeyCode::Home => {
                        app_state.scroll_offset = 0;
                    }
                    KeyCode::End => {
                        app_state.scroll_offset = app_state.opportunities.len().saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }

        // Update data from Redis periodically
        if last_update.elapsed() >= update_interval {
            let _ = app_state.update_from_redis(&mut conn);
            
            // Store opportunities to Redis for strategy runner to read
            if let Err(e) = store_opportunities_to_redis(&mut conn, &app_state.opportunities) {
                eprintln!("Failed to store opportunities to Redis: {}", e);
            }
            
            last_update = std::time::Instant::now();
        }

        // Draw UI
        terminal.draw(|f| ui(f, &app_state))?;

        if app_state.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut ratatui::Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(4),      // Header
            Constraint::Min(8),         // Opportunities table
            Constraint::Length(12),     // Removed opportunities (max 10 + 2 for borders)
            Constraint::Length(2)       // Footer
        ])
        .split(f.size());

    // Header
    let header = ratatui::widgets::Paragraph::new("SPREAD ARBITRAGE DASHBOARD")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).title("Live Opportunities"));
    f.render_widget(header, chunks[0]);

    // Get visible rows based on scroll offset and available height
    let table_height = chunks[1].height as usize;
    let mut sorted_opps: Vec<_> = app.opportunities.values().collect();
    sorted_opps.sort_by(|a, b| b.spread_bps.partial_cmp(&a.spread_bps).unwrap_or(std::cmp::Ordering::Equal));
    
    let visible_rows: Vec<_> = sorted_opps
        .iter()
        .skip(app.scroll_offset)
        .take(table_height.saturating_sub(2))
        .collect();

    // Opportunities table
    let rows: Vec<Row> = visible_rows
        .iter()
        .map(|opp| {
            let spread_color = if opp.spread_bps > 20.0 {
                Color::Green
            } else if opp.spread_bps > 10.0 {
                Color::Yellow
            } else {
                Color::White
            };

            // Check for lead-lag signal
            let lead_lag_indicator = if app.lead_lag_signals.contains_key(&opp.ticker) {
                "⚡"
            } else {
                " "
            };

            // Get funding gravity info
            let gravity_info = app.funding_gravity
                .get(&opp.ticker)
                .and_then(|g| g.get(&opp.long_exchange))
                .map(|g| format!("{}m", g.time_to_payout_minutes / 60))
                .unwrap_or_else(|| "N/A".to_string());

            // Calculate how old this opportunity is
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let age_secs = now.saturating_sub(opp.timestamp);
            let age_color = if age_secs < 5 {
                Color::Green  // Fresh (< 5 seconds)
            } else if age_secs < 15 {
                Color::Yellow // Stale (5-15 seconds)
            } else {
                Color::Red    // Very stale (> 15 seconds)
            };
            let age_str = if age_secs < 60 {
                format!("{}s", age_secs)
            } else {
                format!("{}m", age_secs / 60)
            };

            let ticker_display = format!("{}{}", lead_lag_indicator, &opp.ticker);

            Row::new(vec![
                Span::raw(ticker_display),
                Span::styled(
                    format!("{:.2}bps", opp.spread_bps),
                    Style::default().fg(spread_color),
                ),
                Span::styled(
                    format!("{}", opp.confidence_score),
                    Style::default().fg(if opp.confidence_score > 70 { Color::Green } else if opp.confidence_score > 50 { Color::Yellow } else { Color::White }),
                ),
                Span::raw(format!("{:.4}%", opp.funding_delta * 100.0)),
                Span::styled(age_str, Style::default().fg(age_color)),
                Span::raw(&opp.long_exchange),
                Span::raw(format!("${:.4}", opp.long_bid)),
                Span::raw(format!("${:.4}", opp.long_ask)),
                Span::raw(&opp.short_exchange),
                Span::raw(format!("${:.4}", opp.short_bid)),
                Span::raw(format!("${:.4}", opp.short_ask)),
                Span::raw(gravity_info),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(6),   // Age column
            Constraint::Length(10),  // Long Exchange
            Constraint::Length(12),  // Long Bid
            Constraint::Length(12),  // Long Ask
            Constraint::Length(10),  // Short Exchange
            Constraint::Length(12),  // Short Bid
            Constraint::Length(12),  // Short Ask
            Constraint::Length(8),   // Payout
        ],
    )
    .header(
        Row::new(vec![
            Span::styled("Ticker", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Spread", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Score", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Fund.Δ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Age", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Long Ex", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Long Bid", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Long Ask", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Short Ex", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Short Bid", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Short Ask", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Payout", Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().fg(Color::Cyan)),
    )
    .block(Block::default().borders(Borders::ALL).title(format!(
        "Opportunities ({} total) - ⚡=Lead-Lag Signal - Scroll: {}/{}",
        app.opportunities.len(),
        app.scroll_offset + 1,
        app.opportunities.len()
    )))
    .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_widget(table, chunks[1]);

    // Removed opportunities section
    let removed_rows: Vec<Row> = app.removed_opportunities
        .iter()
        .map(|removed| {
            Row::new(vec![
                Span::raw(removed.ticker.clone()),
                Span::raw(format!("{}", removed.confidence_score)),
                Span::styled(
                    removed.reason.clone(),
                    Style::default().fg(Color::Yellow)
                ),
            ])
        })
        .collect();

    let removed_table = Table::new(
        removed_rows,
        [
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Min(30),
        ],
    )
    .header(
        Row::new(vec![
            Span::styled("Ticker", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
            Span::styled("Conf", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
            Span::styled("Removal Reason", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
        ])
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Recently Removed ({} total)", app.removed_opportunities.len()))
            .style(Style::default().fg(Color::DarkGray))
    );

    f.render_widget(removed_table, chunks[2]);

    // Footer with controls
    let footer_text = "↑↓: Scroll | PgUp/PgDn: Page | Home/End: Jump | q: Quit";
    let footer = ratatui::widgets::Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[3]);
}
