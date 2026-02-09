use std::error::Error;
use std::time::Duration;
use std::collections::BTreeMap;

use arbitrage2::exchange_parser::get_parser;

const REDIS_URL: &str = "redis://127.0.0.1:6379";

type DynError = Box<dyn Error + Send + Sync>;

// Map exchanges to their Redis key patterns for bid/ask data
fn get_bid_ask_key_patterns(exchange: &str) -> Vec<&'static str> {
    match exchange {
        "binance" => vec!["binance:usdm:book:"],
        "okx" => vec!["okx:usdt:tickers:"],
        "hyperliquid" => vec!["hyperliquid:usdc:bbo:"],
        "kucoin" => vec!["kucoin:futures:tickerV2:"],
        "paradex" => vec!["paradex:usdt:bbo:"],
        _ => vec![],
    }
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    println!("Starting funding rate monitor...\n");
    
    let client = redis::Client::open(REDIS_URL)?;
    let mut conn = client.get_multiplexed_tokio_connection().await?;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("*")
            .query_async(&mut conn)
            .await?;

        if keys.is_empty() {
            println!("Waiting for data from exchanges...");
            continue;
        }

        let mut bid_ask_data: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut funding_rates: BTreeMap<String, (f64, String, String, String)> = BTreeMap::new();

        // First pass: collect bid/ask data from all exchanges using their patterns
        for key in &keys {
            let data: String = match redis::cmd("GET").arg(&key).query_async(&mut conn).await {
                Ok(d) => d,
                Err(_) => continue,
            };

            let v: serde_json::Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Check each exchange's bid/ask patterns
            for exchange in &["binance", "okx", "hyperliquid", "kucoin", "paradex"] {
                for pattern in get_bid_ask_key_patterns(exchange) {
                    if key.starts_with(pattern) {
                        if let Some(ticker) = key.strip_prefix(pattern) {
                            let parser = get_parser(exchange);
                            if let Some(data) = parser.extract_all(exchange, &v) {
                                if let (Some(bid), Some(ask)) = (data.bid, data.ask) {
                                    bid_ask_data.insert(format!("{}-{}", exchange, ticker), (bid, ask));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Second pass: collect funding rates with bid/ask
        for key in &keys {
            let data: String = match redis::cmd("GET").arg(&key).query_async(&mut conn).await {
                Ok(d) => d,
                Err(_) => continue,
            };

            let v: serde_json::Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract exchange name from key
            let exchange = if key.starts_with("binance:") {
                "binance"
            } else if key.starts_with("bybit:") {
                "bybit"
            } else if key.starts_with("okx:") {
                "okx"
            } else if key.starts_with("hyperliquid:") {
                "hyperliquid"
            } else if key.starts_with("kucoin:") {
                "kucoin"
            } else if key.starts_with("bitget:") {
                "bitget"
            } else if key.starts_with("gateio:") {
                "gateio"
            } else if key.starts_with("lighter:") {
                "lighter"
            } else if key.starts_with("paradex:") {
                "paradex"
            } else {
                continue;
            };

            let parser = get_parser(exchange);
            if let Some(data) = parser.extract_all(exchange, &v) {
                if let Some(rate) = data.funding_rate {
                    if rate != 0.0 {
                        // Try to get bid/ask from bid_ask_data map, fallback to parser data
                        let (bid, ask) = bid_ask_data.get(&format!("{}-{}", exchange, data.ticker))
                            .map(|(b, a)| (b.clone(), a.clone()))
                            .unwrap_or_else(|| (
                                data.bid.unwrap_or_else(|| "N/A".to_string()),
                                data.ask.unwrap_or_else(|| "N/A".to_string()),
                            ));

                        funding_rates.insert(
                            format!("{}-{}", exchange.to_uppercase(), data.ticker.clone()),
                            (rate.abs(), data.ticker, bid, ask),
                        );
                    }
                }
            }
        }

        if !funding_rates.is_empty() {
            println!("\n╔════════════════════════════════════════════════════════════════════════════════╗");
            println!("║                    TOP 20 HIGHEST FUNDING RATES                                ║");
            println!("╚════════════════════════════════════════════════════════════════════════════════╝");
            
            let mut sorted: Vec<_> = funding_rates.iter().collect();
            sorted.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap_or(std::cmp::Ordering::Equal));

            for (i, (key, (rate, ticker, bid, ask))) in sorted.iter().take(20).enumerate() {
                let exchange = key.split('-').next().unwrap_or("UNKNOWN");
                println!(
                    "{:2}. {:12} {:15} | Rate: {:>10.4}% | Bid: {:>12} | Ask: {:>12}",
                    i + 1,
                    exchange,
                    ticker,
                    rate * 100.0,
                    bid,
                    ask
                );
            }
            println!();
        } else {
            println!("No funding rate data available yet. Checking exchange data availability...");
            
            let keys: Vec<String> = redis::cmd("KEYS")
                .arg("*")
                .query_async(&mut conn)
                .await?;
            
            let mut exchanges_found = std::collections::HashSet::new();
            for key in keys {
                if key.starts_with("binance:") {
                    exchanges_found.insert("BINANCE");
                } else if key.starts_with("bybit:") {
                    exchanges_found.insert("BYBIT");
                } else if key.starts_with("okx:") {
                    exchanges_found.insert("OKX");
                } else if key.starts_with("hyperliquid:") {
                    exchanges_found.insert("HYPERLIQUID");
                } else if key.starts_with("kucoin:") {
                    exchanges_found.insert("KUCOIN");
                } else if key.starts_with("bitget:") {
                    exchanges_found.insert("BITGET");
                } else if key.starts_with("gateio:") {
                    exchanges_found.insert("GATEIO");
                } else if key.starts_with("paradex:") {
                    exchanges_found.insert("PARADEX");
                } else if key.starts_with("lighter:") {
                    exchanges_found.insert("LIGHTER");
                }
            }
            
            if !exchanges_found.is_empty() {
                println!("Exchanges with data: {:?}", exchanges_found);
            }
        }
    }
}
