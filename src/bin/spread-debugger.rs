use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{self, Duration};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), DynError> {
    println!("=== SPREAD CALCULATION DEBUGGER ===");
    println!("Logging to: spread_debug.log");
    println!("Monitoring FUSDT spread calculations...\n");

    let mut interval = time::interval(Duration::from_secs(2));

    loop {
        interval.tick().await;

        if let Err(e) = debug_fusdt_spread().await {
            eprintln!("Error: {}", e);
        }
    }
}

async fn debug_fusdt_spread() -> Result<(), DynError> {
    let client = redis::Client::open("redis://127.0.0.1:6379")?;
    let mut conn = client.get_connection()?;

    let symbol = "FUSDT";
    
    // Check what exchanges have FUSDT data
    let exchanges = vec!["binance", "bybit", "okx", "kucoin", "bitget", "gateio", "hyperliquid", "paradex"];
    
    println!("\n=== Checking FUSDT availability ===");
    for exchange in &exchanges {
        let key = format!("{}:linear:tickers:{}", exchange, symbol);
        let exists: bool = redis::cmd("EXISTS").arg(&key).query(&mut conn).unwrap_or(false);
        println!("{}: {} - {}", exchange, key, if exists { "✓ EXISTS" } else { "✗ NOT FOUND" });
    }
    
    // Now check the actual trade exchanges
    let long_ex = "bitget";
    let short_ex = "bybit";

    println!("\n=== Fetching prices for trade ({}->{})", long_ex, short_ex);

    // Get prices from Redis
    let long_key = format!("{}:linear:tickers:{}", long_ex, symbol);
    let short_key = format!("{}:linear:tickers:{}", short_ex, symbol);

    let long_data_result: Result<String, _> = redis::cmd("GET").arg(&long_key).query(&mut conn);
    let short_data_result: Result<String, _> = redis::cmd("GET").arg(&short_key).query(&mut conn);

    if long_data_result.is_err() {
        println!("✗ {} data NOT FOUND in Redis", long_key);
    }
    if short_data_result.is_err() {
        println!("✗ {} data NOT FOUND in Redis", short_key);
        return Ok(());
    }

    let long_data = long_data_result?;
    let short_data = short_data_result?;

    let long_json: serde_json::Value = serde_json::from_str(&long_data)?;
    let short_json: serde_json::Value = serde_json::from_str(&short_data)?;

    // Use exchange parsers
    let long_parser = arbitrage2::exchange_parser::get_parser(long_ex);
    let short_parser = arbitrage2::exchange_parser::get_parser(short_ex);

    let long_bid_str = long_parser.parse_bid(&long_json).unwrap_or_default();
    let long_ask_str = long_parser.parse_ask(&long_json).unwrap_or_default();
    let short_bid_str = short_parser.parse_bid(&short_json).unwrap_or_default();
    let short_ask_str = short_parser.parse_ask(&short_json).unwrap_or_default();

    let long_bid: f64 = long_bid_str.parse().unwrap_or(0.0);
    let long_ask: f64 = long_ask_str.parse().unwrap_or(0.0);
    let short_bid: f64 = short_bid_str.parse().unwrap_or(0.0);
    let short_ask: f64 = short_ask_str.parse().unwrap_or(0.0);

    // Calculate spread (correct formula)
    let spread_bps = if long_ask > 0.0 {
        ((short_bid - long_ask) / long_ask) * 10000.0
    } else {
        0.0
    };

    // Log to file
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("spread_debug.log")?;

    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let log_line = format!(
        "[{}] {} | Long({}) Bid=${:.6} Ask=${:.6} | Short({}) Bid=${:.6} Ask=${:.6} | Spread={:.2}bps\n",
        timestamp, symbol, long_ex, long_bid, long_ask, short_ex, short_bid, short_ask, spread_bps
    );

    file.write_all(log_line.as_bytes())?;
    print!("{}", log_line);

    Ok(())
}
