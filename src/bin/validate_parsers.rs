use std::error::Error;
use std::collections::HashMap;

use arbitrage2::exchange_parser::get_parser;

const REDIS_URL: &str = "redis://127.0.0.1:6379";

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), DynError> {
    println!("Validating parsers against actual Redis data...\n");
    
    let client = redis::Client::open(REDIS_URL)?;
    let mut conn = client.get_multiplexed_tokio_connection().await?;

    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("*")
        .query_async(&mut conn)
        .await?;

    let mut exchange_samples: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    let mut bid_ask_data: std::collections::BTreeMap<String, (String, String)> = std::collections::BTreeMap::new();
    let mut funding_rate_data: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();

    // First pass: collect ALL bid/ask and funding rate data
    for key in &keys {
        let data: String = match redis::cmd("GET").arg(&key).query_async(&mut conn).await {
            Ok(d) => d,
            Err(_) => continue,
        };

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
        /* } else if key.starts_with("lighter:") {
            "lighter" */
        } else if key.starts_with("paradex:") {
            "paradex"
        } else {
            continue;
        };

        // Parse and collect bid/ask data
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
            if key.contains(":book:") || key.contains(":tickers:") || key.contains(":bbo:") || key.contains(":tickerV2:") {
                if let Some(ticker) = key.split(':').last() {
                    let parser = get_parser(exchange);
                    if let Some(parsed) = parser.extract_all(exchange, &v) {
                        // For Bybit, collect bid and ask separately (they might come in different messages)
                        if exchange == "bybit" {
                            let key_prefix = format!("{}-{}", exchange, ticker);
                            let entry = bid_ask_data.entry(key_prefix).or_insert_with(|| (String::new(), String::new()));
                            if let Some(bid) = parsed.bid {
                                entry.0 = bid;
                            }
                            if let Some(ask) = parsed.ask {
                                entry.1 = ask;
                            }
                        } else if let (Some(bid), Some(ask)) = (parsed.bid, parsed.ask) {
                            bid_ask_data.insert(format!("{}-{}", exchange, ticker), (bid, ask));
                        }
                    }
                }
            }
            
            // Parse and collect funding rates
            if key.contains(":mark:") || key.contains(":funding:") || key.contains(":funding.") || key.contains(":ctx:") || key.contains(":funding_settlement:") || key.contains(":tickers:") {
                let ticker = if key.contains(":funding_settlement:") {
                    // Extract ticker from funding_settlement key (e.g., "kucoin:futures:funding_settlement:C98USDTM:funding.end" -> "C98USDTM")
                    key.split(':').nth(3).unwrap_or("")
                } else if key.contains(":instrument:") && key.contains(":funding.rate") {
                    // Extract ticker from KuCoin funding.rate key (e.g., "kucoin:futures:instrument:PUFFERUSDTM:funding.rate" -> "PUFFERUSDTM")
                    key.split(':').nth(3).unwrap_or("")
                } else {
                    key.split(':').last().unwrap_or("")
                };
                
                if !ticker.is_empty() {
                    let parser = get_parser(exchange);
                    if let Some(parsed) = parser.extract_all(exchange, &v) {
                        if let Some(rate) = parsed.funding_rate {
                            funding_rate_data.insert(format!("{}-{}", exchange, ticker), rate);
                        }
                    } else if exchange == "kucoin" && key.contains(":funding.rate") {
                        // For KuCoin funding.rate keys, extract directly since parser won't work
                        if let Some(rate) = v.get("data")
                            .and_then(|d| d.get("fundingRate"))
                            .and_then(|r| {
                                if r.is_f64() {
                                    r.as_f64()
                                } else {
                                    r.as_str().and_then(|s| s.parse().ok())
                                }
                            })
                        {
                            funding_rate_data.insert(format!("{}-{}", exchange, ticker), rate);
                        }
                    }
                }
            }
        }
    }

    // Second pass: collect samples for display (up to 3 per exchange)
    // For hyperliquid, we need to merge ctx and bbo data
    let mut hyperliquid_merged: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    // For paradex, we need to merge bbo and funding_payments data
    let mut paradex_merged: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    
    for key in &keys {
        if key.starts_with("hyperliquid:usdc:ctx:") || key.starts_with("hyperliquid:usdc:bbo:") {
            let coin = key.split(':').last().unwrap_or("");
            let data: String = match redis::cmd("GET").arg(&key).query_async(&mut conn).await {
                Ok(d) => d,
                Err(_) => continue,
            };
            
            let entry = hyperliquid_merged.entry(coin.to_string()).or_insert_with(|| (String::new(), String::new()));
            if key.contains(":ctx:") {
                entry.0 = data;
            } else if key.contains(":bbo:") {
                entry.1 = data;
            }
        }
        
        if key.starts_with("paradex:usdt:bbo:") || key.starts_with("paradex:usdt:funding_payments:") {
            let market = key.split(':').last().unwrap_or("");
            let data: String = match redis::cmd("GET").arg(&key).query_async(&mut conn).await {
                Ok(d) => d,
                Err(_) => continue,
            };
            
            let entry = paradex_merged.entry(market.to_string()).or_insert_with(|| (String::new(), String::new()));
            if key.contains(":bbo:") {
                entry.0 = data;
            } else if key.contains(":funding_payments:") {
                entry.1 = data;
            }
        }
    }
    
    for key in &keys {
        let data: String = match redis::cmd("GET").arg(&key).query_async(&mut conn).await {
            Ok(d) => d,
            Err(_) => continue,
        };

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
        /* } else if key.starts_with("lighter:") {
            "lighter" */
        } else if key.starts_with("paradex:") {
            "paradex"
        } else {
            continue;
        };

        // Skip hyperliquid ctx/bbo keys as we'll handle them separately
        if exchange == "hyperliquid" && (key.contains(":ctx:") || key.contains(":bbo:")) {
            continue;
        }
        
        // Skip paradex bbo/funding_payments keys as we'll handle them separately
        if exchange == "paradex" && (key.contains(":bbo:") || key.contains(":funding_payments:")) {
            continue;
        }

        // Only collect samples that have ticker data
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
            let parser = get_parser(exchange);
            if parser.parse_ticker(&v).is_some() {
                let samples = exchange_samples.entry(exchange).or_insert_with(Vec::new);
                if samples.len() < 3 {
                    samples.push((key.clone(), data));
                }
            }
        }
    }
    
    // Add hyperliquid merged samples
    for (coin, (ctx_data, bbo_data)) in &hyperliquid_merged {
        if !ctx_data.is_empty() || !bbo_data.is_empty() {
            let samples = exchange_samples.entry("hyperliquid").or_insert_with(Vec::new);
            if samples.len() < 3 {
                // Use ctx data if available, otherwise bbo
                let data_to_use = if !ctx_data.is_empty() { ctx_data.clone() } else { bbo_data.clone() };
                samples.push((format!("hyperliquid:usdc:data:{}", coin), data_to_use));
            }
        }
    }
    
    // Add paradex merged samples
    for (market, (bbo_data, funding_data)) in &paradex_merged {
        if !bbo_data.is_empty() || !funding_data.is_empty() {
            let samples = exchange_samples.entry("paradex").or_insert_with(Vec::new);
            if samples.len() < 3 {
                // Use bbo data if available, otherwise funding
                let data_to_use = if !bbo_data.is_empty() { bbo_data.clone() } else { funding_data.clone() };
                samples.push((format!("paradex:usdt:data:{}", market), data_to_use));
            }
        }
    }

    // Validate each exchange with merged data
    for exchange in &["binance", "bybit", "okx", "hyperliquid", "kucoin", "bitget", "gateio", /* "lighter", */ "paradex"] {
        println!("╔════════════════════════════════════════════════════════════════╗");
        println!("║ Exchange: {:50} ║", exchange);
        println!("╚════════════════════════════════════════════════════════════════╝");

        if let Some(samples) = exchange_samples.get(exchange) {
            let parser = get_parser(exchange);
            
            for (key, data) in samples {
                println!("\nKey: {}", key);
                
                match serde_json::from_str::<serde_json::Value>(&data) {
                    Ok(v) => {
                        match parser.extract_all(exchange, &v) {
                            Some(mut parsed) => {
                                // Try to merge bid/ask from separate keys
                                if parsed.bid.is_none() || parsed.ask.is_none() {
                                    if let Some(ticker) = if key.contains(":funding_settlement:") {
                                        key.split(':').nth(3)
                                    } else {
                                        key.split(':').last()
                                    } {
                                        if let Some((bid, ask)) = bid_ask_data.get(&format!("{}-{}", exchange, ticker)) {
                                            if !bid.is_empty() && parsed.bid.is_none() {
                                                parsed.bid = Some(bid.clone());
                                            }
                                            if !ask.is_empty() && parsed.ask.is_none() {
                                                parsed.ask = Some(ask.clone());
                                            }
                                        }
                                    }
                                }
                                
                                // Try to merge funding rate from separate keys
                                if parsed.funding_rate.is_none() {
                                    if let Some(ticker) = if key.contains(":funding_settlement:") {
                                        key.split(':').nth(3)
                                    } else {
                                        key.split(':').last()
                                    } {
                                        if let Some(rate) = funding_rate_data.get(&format!("{}-{}", exchange, ticker)) {
                                            parsed.funding_rate = Some(*rate);
                                        }
                                    }
                                }
                                
                                // For hyperliquid, try to merge ctx and bbo data
                                if *exchange == "hyperliquid" {
                                    if let Some(ticker) = key.split(':').last() {
                                        if let Some((ctx_data, bbo_data)) = hyperliquid_merged.get(ticker) {
                                            // Try to parse ctx data for funding rate
                                            if parsed.funding_rate.is_none() && !ctx_data.is_empty() {
                                                if let Ok(ctx_v) = serde_json::from_str::<serde_json::Value>(ctx_data) {
                                                    if let Some(rate) = ctx_v.get("data")
                                                        .and_then(|d| d.get("ctx"))
                                                        .and_then(|c| c.get("funding"))
                                                        .and_then(|v| v.as_str())
                                                        .and_then(|r| r.parse().ok())
                                                    {
                                                        parsed.funding_rate = Some(rate);
                                                    }
                                                }
                                            }
                                            // Try to parse bbo data for bid/ask
                                            if (parsed.bid.is_none() || parsed.ask.is_none()) && !bbo_data.is_empty() {
                                                if let Ok(bbo_v) = serde_json::from_str::<serde_json::Value>(bbo_data) {
                                                    if let Some(bbo_arr) = bbo_v.get("data")
                                                        .and_then(|d| d.get("bbo"))
                                                        .and_then(|b| b.as_array())
                                                    {
                                                        if parsed.bid.is_none() {
                                                            if let Some(bid_px) = bbo_arr.first()
                                                                .and_then(|f| f.get("px"))
                                                                .and_then(|v| v.as_str())
                                                            {
                                                                parsed.bid = Some(bid_px.to_string());
                                                            }
                                                        }
                                                        if parsed.ask.is_none() {
                                                            if let Some(ask_px) = bbo_arr.get(1)
                                                                .and_then(|f| f.get("px"))
                                                                .and_then(|v| v.as_str())
                                                            {
                                                                parsed.ask = Some(ask_px.to_string());
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                // For paradex, try to merge bbo and funding_payments data
                                if *exchange == "paradex" {
                                    if let Some(market) = key.split(':').last() {
                                        if let Some((bbo_data, funding_data)) = paradex_merged.get(market) {
                                            // Try to parse funding_payments data for funding rate
                                            if parsed.funding_rate.is_none() && !funding_data.is_empty() {
                                                if let Ok(funding_v) = serde_json::from_str::<serde_json::Value>(funding_data) {
                                                    if let Some(rate) = funding_v.get("params")
                                                        .and_then(|p| p.get("data"))
                                                        .and_then(|d| d.get("funding_rate"))
                                                        .and_then(|v| v.as_str())
                                                        .and_then(|r| r.parse().ok())
                                                    {
                                                        parsed.funding_rate = Some(rate);
                                                    }
                                                }
                                            }
                                            // Try to parse bbo data for bid/ask
                                            if (parsed.bid.is_none() || parsed.ask.is_none()) && !bbo_data.is_empty() {
                                                if let Ok(bbo_v) = serde_json::from_str::<serde_json::Value>(bbo_data) {
                                                    if let Some(bid) = bbo_v.get("params")
                                                        .and_then(|p| p.get("data"))
                                                        .and_then(|d| d.get("bid"))
                                                        .and_then(|v| v.as_str())
                                                    {
                                                        if parsed.bid.is_none() {
                                                            parsed.bid = Some(bid.to_string());
                                                        }
                                                    }
                                                    if let Some(ask) = bbo_v.get("params")
                                                        .and_then(|p| p.get("data"))
                                                        .and_then(|d| d.get("ask"))
                                                        .and_then(|v| v.as_str())
                                                    {
                                                        if parsed.ask.is_none() {
                                                            parsed.ask = Some(ask.to_string());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                println!("  ✓ Ticker:        {}", parsed.ticker);
                                println!("  {} Funding Rate:  {}", 
                                    if parsed.funding_rate.is_some() { "✓" } else { "✗" },
                                    parsed.funding_rate.map(|r| format!("{}", r)).unwrap_or_else(|| "N/A".to_string())
                                );
                                println!("  {} Bid:           {}", 
                                    if parsed.bid.is_some() { "✓" } else { "✗" },
                                    parsed.bid.as_ref().unwrap_or(&"N/A".to_string())
                                );
                                println!("  {} Ask:           {}", 
                                    if parsed.ask.is_some() { "✓" } else { "✗" },
                                    parsed.ask.as_ref().unwrap_or(&"N/A".to_string())
                                );
                            }
                            None => println!("  ✗ Failed to extract data"),
                        }
                    }
                    Err(e) => println!("  ✗ JSON parse error: {}", e),
                }
            }
        } else {
            println!("  ⚠ No data found for this exchange");
        }
        println!();
    }

    Ok(())
}
