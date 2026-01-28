use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::utils;
use crate::DynError;

const PARADEX_API_URL: &str = "https://api.prod.paradex.trade/v1";
const PARADEX_WS_URL: &str = "wss://ws.api.prod.paradex.trade/v1";
const MARKETS_PER_CONNECTION: usize = 15;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 30;

pub struct ParadexPerpsConnector;

#[derive(Debug, Deserialize)]
struct ParadexMarket {
    symbol: String,
    asset_kind: String,
    quote_currency: String,
}

#[derive(Debug, Deserialize)]
struct ParadexMarketsResponse {
    results: Vec<ParadexMarket>,
}

impl ParadexPerpsConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let resp = client.get(&format!("{}/markets", PARADEX_API_URL)).send().await?;
        if !resp.status().is_success() {
            eprintln!("Paradex connection check failed: {}", resp.status());
            std::process::exit(1);
        }
        println!("Paradex connection check OK");
        Ok(())
    }

    pub async fn run(client: &reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let markets = fetch_perp_markets(client).await?;
        println!("Valid Paradex perp markets (USDT): {}", markets.len());

        let batches = utils::chunk_vec(&markets, MARKETS_PER_CONNECTION);
        println!("Starting Paradex websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_perp_markets(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let resp = client
        .get(&format!("{}/markets", PARADEX_API_URL))
        .send()
        .await?
        .json::<ParadexMarketsResponse>()
        .await?;

    let mut markets: Vec<String> = resp
        .results
        .into_iter()
        .filter(|m| m.asset_kind == "PERP" && m.quote_currency == "USD")
        .map(|m| m.symbol)
        .collect();
    markets.sort();
    markets.dedup();
    Ok(markets)
}

async fn run_worker(worker_id: usize, markets: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_batch(worker_id, &markets[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Paradex ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Paradex ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }
        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}

async fn run_batch(worker_id: usize, markets: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(PARADEX_WS_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("Paradex ws[{}] connected", worker_id);

    for market in markets {
        for channel_type in &["bbo", "funding_data"] {
            let msg = serde_json::json!({
                "id": 1,
                "jsonrpc": "2.0",
                "method": "subscribe",
                "params": {"channel": format!("{}.{}", channel_type, market)}
            });
            write.send(Message::Text(msg.to_string())).await?;
        }
        if SUBSCRIBE_BATCH_DELAY_MS > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;
        }
    }

    let mut first_data_logged = false;
    let mut ping_tick = utils::interval_secs(20);

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                if write.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            msg = read.next() => {
                let msg = match msg {
                    Some(m) => m?,
                    None => break,
                };

                if !msg.is_text() {
                    continue;
                }

                let text = msg.into_text()?;
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if v.get("method").and_then(|m| m.as_str()).unwrap_or("") != "subscription" {
                    continue;
                }

                if !first_data_logged {
                    first_data_logged = true;
                    println!("Paradex ws[{}] first data message received", worker_id);
                }

                let channel = v.get("params")
                    .and_then(|p| p.get("channel"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");

                let market = match v.get("params")
                    .and_then(|p| p.get("data"))
                    .and_then(|d| d.get("market"))
                    .and_then(|m| m.as_str()) {
                    Some(m) => m,
                    None => continue,
                };

                let channel_type = channel.split('.').next().unwrap_or("");
                let key = format!("paradex:usdt:{}:{}", channel_type, market);
                let payload = serde_json::to_string(&v)?;

                if tx.send((key, payload)).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}
