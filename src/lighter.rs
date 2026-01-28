use futures_util::SinkExt;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::utils;
use crate::DynError;

const LIGHTER_WS_URL: &str = "wss://mainnet.zklighter.elliot.ai/stream";
const MARKETS_PER_CONNECTION: usize = 20;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 25;

pub struct LighterPerpsConnector;

impl LighterPerpsConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        match tokio_tungstenite::connect_async(LIGHTER_WS_URL).await {
            Ok(_) => {
                println!("Lighter connection check OK");
                Ok(())
            }
            Err(e) => {
                eprintln!("Lighter connection check failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    pub async fn run(client: &reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let markets = fetch_markets(client).await?;
        println!("Valid Lighter perp markets: {}", markets.len());

        let batches = utils::chunk_vec(&markets, MARKETS_PER_CONNECTION);
        println!("Starting Lighter websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_markets(_client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    // Lighter uses market indices (0, 1, 2, etc.)
    // Since the orderBooks endpoint doesn't return data, use a reasonable default range
    let markets: Vec<String> = (0..100).map(|i| i.to_string()).collect();
    Ok(markets)
}

async fn run_worker(worker_id: usize, markets: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_batch(worker_id, &markets[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Lighter ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Lighter ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }
        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}

async fn run_batch(worker_id: usize, markets: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(LIGHTER_WS_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("Lighter ws[{}] connected", worker_id);

    for market in markets {
        let msg = serde_json::json!({"type": "subscribe", "channel": format!("market_stats/{}", market)});
        write.send(Message::Text(msg.to_string())).await?;
        let msg = serde_json::json!({"type": "subscribe", "channel": format!("order_book/{}", market)});
        write.send(Message::Text(msg.to_string())).await?;
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

                let channel = v.get("channel").and_then(|c| c.as_str()).unwrap_or("");
                if !channel.starts_with("market_stats/") && !channel.starts_with("order_book/") {
                    continue;
                }

                if !first_data_logged {
                    first_data_logged = true;
                    println!("Lighter ws[{}] first data message received", worker_id);
                    println!("Lighter ws[{}] sample message: {}", worker_id, &text[..std::cmp::min(500, text.len())]);
                }

                let market_id = if channel.starts_with("market_stats/") {
                    channel.trim_start_matches("market_stats/").to_string()
                } else if channel.starts_with("order_book/") {
                    channel.trim_start_matches("order_book/").to_string()
                } else {
                    continue;
                };

                let key = format!("lighter:usdt:data:{}", market_id);
                let payload = serde_json::to_string(&v)?;

                if tx.send((key, payload)).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}
