use std::error::Error;
use std::time::Duration;

mod binance;
mod bitget;
mod bybit;
mod exchange_parser;
mod hyperliquid;
mod kucoin;
//mod lighter;
mod okx;
mod paradex;
mod utils;
mod strategy;

use tokio::sync::mpsc;
use tokio::time;
use strategy::runner::StrategyRunner;

const REDIS_URL: &str = "redis://127.0.0.1:6379";

const REDIS_FLUSH_MAX_ITEMS: usize = 512;
const REDIS_FLUSH_INTERVAL_MS: u64 = 50;

pub type DynError = Box<dyn Error + Send + Sync>;

async fn redis_writer(mut rx: mpsc::Receiver<(String, String)>) -> Result<(), DynError> {
    let client = redis::Client::open(REDIS_URL)?;
    let mut conn = client.get_multiplexed_tokio_connection().await?;

    let mut buffer: Vec<(String, String)> = Vec::with_capacity(REDIS_FLUSH_MAX_ITEMS);
    let mut tick = time::interval(Duration::from_millis(REDIS_FLUSH_INTERVAL_MS));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                if !buffer.is_empty() {
                    let mut pipe = redis::pipe();
                    for (k, v) in buffer.iter() {
                        pipe.cmd("SET").arg(k).arg(v).ignore();
                        pipe.publish(k, v).ignore();
                    }
                    let _: () = pipe.query_async(&mut conn).await?;
                    buffer.clear();
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(item) => {
                        buffer.push(item);
                        if buffer.len() >= REDIS_FLUSH_MAX_ITEMS {
                            let mut pipe = redis::pipe();
                            for (k, v) in buffer.iter() {
                                pipe.cmd("SET").arg(k).arg(v).ignore();
                                pipe.publish(k, v).ignore();
                            }
                            let _: () = pipe.query_async(&mut conn).await?;
                            buffer.clear();
                        }
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}

async fn oi_poller(client: reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    // Poll OI data from exchanges every 5 minutes
    let mut interval = time::interval(Duration::from_secs(300));
    let redis_client = redis::Client::open(REDIS_URL)?;
    let mut redis_conn = redis_client.get_connection()?;
    
    loop {
        interval.tick().await;
        
        // Collect OI data from all exchanges
        let symbols = vec!["BTCUSDT", "ETHUSDT"];
        let exchanges = vec!["binance", "okx", "bybit", "kucoin", "bitget", "hyperliquid", "paradex"];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        for symbol in &symbols {
            for exchange in &exchanges {
                let oi_data = match fetch_oi_data(&client, exchange, symbol).await {
                    Ok(data) => data,
                    Err(e) => {
                        eprintln!("Failed to fetch OI from {} for {}: {}", exchange, symbol, e);
                        continue;
                    }
                };
                
                // Extract OI value from JSON
                if let Ok(oi_value) = extract_oi_value(&oi_data, exchange) {
                    // Store current OI
                    let key = format!("{}:oi:{}", exchange, symbol);
                    if tx.send((key.clone(), oi_data.clone())).await.is_err() {
                        return Ok(());
                    }
                    
                    // Store OI snapshot with timestamp in sorted set for 24h average calculation
                    let snapshot_key = format!("{}:oi_history:{}", exchange, symbol);
                    let score = now as f64;
                    let member = format!("{},{}", now, oi_value);
                    
                    // Add to sorted set
                    let _: () = redis::cmd("ZADD")
                        .arg(&snapshot_key)
                        .arg(score)
                        .arg(&member)
                        .query(&mut redis_conn)
                        .unwrap_or_default();
                    
                    // Remove snapshots older than 24 hours (86400 seconds)
                    let cutoff_time = now - 86400;
                    let _: () = redis::cmd("ZREMRANGEBYSCORE")
                        .arg(&snapshot_key)
                        .arg("-inf")
                        .arg(cutoff_time as f64)
                        .query(&mut redis_conn)
                        .unwrap_or_default();
                    
                    // Set expiry on the sorted set (25 hours to be safe)
                    let _: () = redis::cmd("EXPIRE")
                        .arg(&snapshot_key)
                        .arg(90000)
                        .query(&mut redis_conn)
                        .unwrap_or_default();
                }
            }
        }
    }
}

fn extract_oi_value(oi_data: &str, exchange: &str) -> Result<f64, DynError> {
    let json: serde_json::Value = serde_json::from_str(oi_data)?;
    
    let oi = match exchange {
        "binance" => {
            json.get("openInterest")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "okx" => {
            json.get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("oi"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "bybit" => {
            json.get("result")
                .and_then(|r| r.get("list"))
                .and_then(|l| l.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("openInterest"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "kucoin" => {
            json.get("data")
                .and_then(|d| d.get("openInterest"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "bitget" => {
            json.get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|f| f.get("oi"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "hyperliquid" => {
            json.get("openInterest")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        "paradex" => {
            json.get("market")
                .and_then(|m| m.get("open_interest"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        }
        _ => None,
    };
    
    oi.ok_or_else(|| "Failed to extract OI value".into())
}

async fn fetch_oi_data(client: &reqwest::Client, exchange: &str, symbol: &str) -> Result<String, DynError> {
    match exchange {
        "binance" => {
            let url = format!("https://fapi.binance.com/fapi/v1/openInterest?symbol={}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "okx" => {
            let inst_id = format!("{}-USDT-SWAP", symbol.trim_end_matches("USDT"));
            let url = format!("https://www.okx.com/api/v5/public/open-interest?instId={}", inst_id);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "bybit" => {
            let url = format!("https://api.bybit.com/v5/market/open-interest?category=linear&symbol={}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "kucoin" => {
            let symbol_kc = format!("{}M", symbol.trim_end_matches("USDT"));
            let url = format!("https://api-futures.kucoin.com/api/v1/contracts/{}/open-interest", symbol_kc);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "bitget" => {
            let url = format!("https://api.bitget.com/api/v2/mix/market/open-interest?productType=usdt-futures&symbol={}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        "hyperliquid" => {
            // Hyperliquid: POST /info with type=openInterest
            let coin = symbol.trim_end_matches("USDT");
            let payload = serde_json::json!({
                "type": "openInterest",
                "coin": coin
            });
            let resp = client
                .post("https://api.hyperliquid.xyz/info")
                .json(&payload)
                .send()
                .await?;
            Ok(resp.text().await?)
        }
        "paradex" => {
            // Paradex: GET /markets/{symbol}
            let url = format!("https://api.prod.paradex.trade/v1/markets/{}", symbol);
            let resp = client.get(&url).send().await?;
            Ok(resp.text().await?)
        }
        _ => Err("Unknown exchange".into()),
    }
}

macro_rules! spawn_connector {
    ($client:expr, $tx:expr, $connector:ty) => {
        {
            let c = $client.clone();
            let t = $tx.clone();
            tokio::spawn(async move {
                let _ = <$connector>::run(&c, t).await;
            });
        }
    };
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let (tx, rx) = mpsc::channel::<(String, String)>(32_768);

    let _redis_handle = tokio::spawn(async move { redis_writer(rx).await });

    spawn_connector!(client, tx, binance::BinanceUsdmConnector);
    spawn_connector!(client, tx, bybit::BybitLinearConnector);
    spawn_connector!(client, tx, bitget::BitgetUsdtFuturesConnector);
    spawn_connector!(client, tx, kucoin::KucoinFuturesConnector);
    spawn_connector!(client, tx, okx::OkxUsdtSwapConnector);
    spawn_connector!(client, tx, hyperliquid::HyperliquidPerpsConnector);

    /* let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = lighter::LighterPerpsConnector::run(&client_clone, tx_clone).await;
    }); */

    spawn_connector!(client, tx, paradex::ParadexPerpsConnector);

    // Spawn OI poller
    let client_oi = client.clone();
    let tx_oi = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = oi_poller(client_oi, tx_oi).await {
            eprintln!("OI poller error: {}", e);
        }
    });

    drop(tx);

    // Start strategy runner with real data from Redis
    println!("Starting spread arbitrage strategy runner...");
    let redis_client = redis::Client::open(REDIS_URL)?;
    let redis_conn = redis_client.get_multiplexed_tokio_connection().await?;
    
    let mut strategy_runner = StrategyRunner::new(redis_conn, 20000.0).await?;
    println!("Strategy runner initialized with $20,000 capital");
    
    tokio::spawn(async move {
        if let Err(e) = strategy_runner.run_scanning_loop().await {
            eprintln!("Strategy runner error: {}", e);
        }
    });

    // Keep main thread alive indefinitely
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
