use std::error::Error;
use std::collections::BTreeMap;
use std::time::Duration;
use arbitrage2::exchange_parser::{get_parser, normalize_symbol};

const REDIS_URL: &str = "redis://127.0.0.1:6379";

type DynError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
struct OpportunityDebug {
    symbol: String,
    long_exchange: String,
    short_exchange: String,
    spread_bps: f64,
    funding_delta: f64,
    confidence_score: u8,
    projected_profit_bps: f64,
    rejection_reason: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    println!("=== OPPORTUNITY FILTER DEBUGGER ===\n");
    println!("This tool shows ALL pairs and why they pass or fail filtering criteria.\n");

    loop {
        match analyze_opportunities().await {
            Ok(results) => {
                print_results(&results);
            }
            Err(e) => {
                eprintln!("Error analyzing opportunities: {}", e);
            }
        }

        // Wait 5 seconds before next scan
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn analyze_opportunities() -> Result<Vec<OpportunityDebug>, DynError> {
    let client = redis::Client::open(REDIS_URL)?;
    let mut conn = client.get_connection()?;

    // Get all ticker data
    let mut ticker_data: BTreeMap<String, Vec<(String, f64, f64)>> = BTreeMap::new();
    let mut funding_rates: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();

    let exchanges = vec!["binance", "bybit", "okx", "kucoin", "bitget", "gateio", "hyperliquid", "paradex"];

    for exchange in &exchanges {
        let pattern = format!("{}:linear:tickers:*USDT", exchange);
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query(&mut conn)?;

        for key in keys {
            if let Ok(data) = redis::cmd("GET").arg(&key).query::<String>(&mut conn) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                    let parser = get_parser(exchange);

                    if let (Some(bid_str), Some(ask_str)) = (parser.parse_bid(&json), parser.parse_ask(&json)) {
                        if let (Ok(bid), Ok(ask)) = (bid_str.parse::<f64>(), ask_str.parse::<f64>()) {
                            let symbol = key.split(':').nth(3).unwrap_or("").to_string();
                            let normalized = normalize_symbol(&symbol);

                            ticker_data
                                .entry(normalized.clone())
                                .or_insert_with(Vec::new)
                                .push((exchange.to_string(), bid, ask));

                            // Get funding rate
                            if let Some(rate) = parser.parse_funding_rate(&json) {
                                funding_rates
                                    .entry(normalized.clone())
                                    .or_insert_with(BTreeMap::new)
                                    .insert(exchange.to_string(), rate);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut results = Vec::new();

    for (symbol, exchanges) in &ticker_data {
        if exchanges.len() < 2 {
            continue;
        }

        let mut min_ask = f64::MAX;
        let mut max_bid = 0.0;
        let mut long_ex = String::new();
        let mut short_ex = String::new();

        for (ex, bid, ask) in exchanges {
            if *ask < min_ask {
                min_ask = *ask;
                long_ex = ex.clone();
            }
            if *bid > max_bid {
                max_bid = *bid;
                short_ex = ex.clone();
            }
        }

        if long_ex == short_ex || min_ask >= max_bid {
            continue;
        }

        let spread_bps = ((max_bid - min_ask) / min_ask) * 10000.0;
        
        // Check all filters
        let mut rejection_reason = None;

        // Filter 1: Unrealistic spreads
        if spread_bps > 500.0 {
            rejection_reason = Some(format!("Spread too wide: {:.2}bps > 500bps (data error)", spread_bps));
        }

        // Filter 2: Low price (data corruption)
        if min_ask < 0.001 && rejection_reason.is_none() {
            rejection_reason = Some(format!("Price too low: ${:.6} < $0.001 (data corruption)", min_ask));
        }

        // Filter 3: Funding delta
        let funding_delta = calculate_funding_delta(&funding_rates, symbol, &long_ex, &short_ex);
        if funding_delta.abs() <= 0.0001 && rejection_reason.is_none() {
            rejection_reason = Some(format!("Funding delta too small: {:.6} <= 0.0001", funding_delta));
        }

        // Filter 4: Order book depth (simplified - assume sufficient for debug)
        let estimated_position_size = 1000.0;
        let depth_long = 50000.0; // Simplified
        let depth_short = 50000.0;
        let depth_sufficient = depth_long >= estimated_position_size * 2.0 
            && depth_short >= estimated_position_size * 2.0;

        if !depth_sufficient && rejection_reason.is_none() {
            rejection_reason = Some(format!("Insufficient depth: long={:.0}, short={:.0}", depth_long, depth_short));
        }

        // Filter 5: Confidence score
        let confidence_score = calculate_confidence_score(spread_bps, funding_delta);

        // Filter 6: Projected profit
        let fees_bps = 20.0;
        let funding_cost_bps = 10.0;
        let slippage_bps = 3.0;
        let projected_profit_bps = spread_bps - fees_bps - funding_cost_bps - slippage_bps;

        if confidence_score < 70 && rejection_reason.is_none() {
            rejection_reason = Some(format!("Confidence too low: {} < 70", confidence_score));
        }

        if projected_profit_bps <= 0.0 && rejection_reason.is_none() {
            rejection_reason = Some(format!("Projected profit negative: {:.2}bps <= 0 (spread={:.2} - fees=20 - funding=10 - slippage=3)", projected_profit_bps, spread_bps));
        }

        results.push(OpportunityDebug {
            symbol: symbol.clone(),
            long_exchange: long_ex,
            short_exchange: short_ex,
            spread_bps,
            funding_delta,
            confidence_score,
            projected_profit_bps,
            rejection_reason,
        });
    }

    // Sort by spread (highest first)
    results.sort_by(|a, b| b.spread_bps.partial_cmp(&a.spread_bps).unwrap_or(std::cmp::Ordering::Equal));

    Ok(results)
}

fn calculate_funding_delta(
    funding_rates: &BTreeMap<String, BTreeMap<String, f64>>,
    symbol: &str,
    long_ex: &str,
    short_ex: &str,
) -> f64 {
    let long_rate = funding_rates
        .get(symbol)
        .and_then(|rates| rates.get(long_ex))
        .copied()
        .unwrap_or(0.0);

    let short_rate = funding_rates
        .get(symbol)
        .and_then(|rates| rates.get(short_ex))
        .copied()
        .unwrap_or(0.0);

    long_rate - short_rate
}

fn calculate_confidence_score(spread_bps: f64, funding_delta: f64) -> u8 {
    let mut score = 0.0;

    // Spread component (weight 50)
    let spread_score = (spread_bps / 50.0).min(1.0) * 100.0;
    score += spread_score * 0.5;

    // Funding delta component (weight 30)
    let funding_score = (funding_delta.abs() / 0.01).min(1.0) * 100.0;
    score += funding_score * 0.3;

    // Base 20% for passing hard constraints
    score += 20.0;

    (score as u8).min(100)
}

fn print_results(results: &[OpportunityDebug]) {
    println!("\n{}", "=".repeat(120));
    println!("{:12} {:8} {:8} {:10} {:12} {:8} {:12} {}", 
        "SYMBOL", "SPREAD", "FUNDING", "CONF", "PROJ_PROFIT", "STATUS", "EXCHANGES", "REASON");
    println!("{}", "=".repeat(120));

    let mut passed_count = 0;
    let mut filtered_count = 0;

    for opp in results {
        let status = if opp.rejection_reason.is_none() {
            passed_count += 1;
            "✓ PASS"
        } else {
            filtered_count += 1;
            "✗ FILTERED"
        };

        let status_color = if opp.rejection_reason.is_none() { "\x1b[32m" } else { "\x1b[31m" };
        let reset_color = "\x1b[0m";

        println!(
            "{:12} {:7.2}bps {:7.4}% {:8} {:11.2}bps {}{:8}{} {:4}->{:4} {}",
            opp.symbol,
            opp.spread_bps,
            opp.funding_delta * 100.0,
            opp.confidence_score,
            opp.projected_profit_bps,
            status_color,
            status,
            reset_color,
            opp.long_exchange,
            opp.short_exchange,
            opp.rejection_reason.as_ref().unwrap_or(&String::new())
        );
    }

    println!("{}", "=".repeat(120));
    println!("\nSUMMARY: {} opportunities PASSED | {} opportunities FILTERED\n", passed_count, filtered_count);
}
