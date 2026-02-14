use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::utils;
use crate::DynError;

const HYPERLIQUID_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const HYPERLIQUID_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const COINS_PER_CONNECTION: usize = 50;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 20;

pub struct HyperliquidPerpsConnector;

#[derive(Debug, Deserialize)]
struct HyperMetaResponse {
    universe: Vec<HyperUniverseItem>,
}

#[derive(Debug, Deserialize)]
struct HyperUniverseItem {
    name: String,
}

impl HyperliquidPerpsConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let resp = client
            .post(HYPERLIQUID_INFO_URL)
            .json(&serde_json::json!({"type": "meta"}))
            .send()
            .await?;

        if !resp.status().is_success() {
            eprintln!("Hyperliquid connection check failed: {}", resp.status());
            std::process::exit(1);
        }

        println!("Hyperliquid connection check OK");
        Ok(())
    }

    pub async fn run(
        client: &reqwest::Client, 
        tx: mpsc::Sender<(String, String)>,
        _market_producer: Option<crate::strategy::pipeline::MarketProducer>,
    ) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let coins = fetch_perp_coins(client).await?;
        println!("Valid Hyperliquid perp coins: {}", coins.len());

        let batches = utils::chunk_vec(&coins, COINS_PER_CONNECTION);
        println!("Starting Hyperliquid websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_perp_coins(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let resp = client
        .post(HYPERLIQUID_INFO_URL)
        .json(&serde_json::json!({"type": "meta"}))
        .send()
        .await?
        .json::<HyperMetaResponse>()
        .await?;

    let mut coins: Vec<String> = resp.universe.into_iter().map(|u| u.name).collect();
    coins.sort();
    coins.dedup();
    Ok(coins)
}

async fn run_worker(worker_id: usize, coins: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    // Pin WebSocket thread to cores 2-7 for optimal cache performance
    // Requirement: 4.2 (Pin WebSocket threads to cores 2-7)
    if let Err(e) = crate::strategy::thread_pinning::pin_websocket_thread(worker_id) {
        eprintln!("[THREAD-PIN] Warning: Failed to pin Hyperliquid worker {}: {}", worker_id, e);
        eprintln!("[THREAD-PIN] Continuing without thread pinning (performance may be degraded)");
    }
    
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_batch(worker_id, &coins[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Hyperliquid ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Hyperliquid ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }
        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}

async fn run_batch(worker_id: usize, coins: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(HYPERLIQUID_WS_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("Hyperliquid ws[{}] connected", worker_id);

    for coin in coins {
        let msg = serde_json::json!({"method": "subscribe", "subscription": {"type": "activeAssetCtx", "coin": coin}});
        write.send(Message::Text(msg.to_string())).await?;
        let msg = serde_json::json!({"method": "subscribe", "subscription": {"type": "bbo", "coin": coin}});
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

                // Zero-copy WebSocket message handling (Requirement 8.1, 8.3, 8.4)
                // Work directly with bytes, avoiding String allocation
                let bytes = match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        // Convert String to bytes (unavoidable with tungstenite's API)
                        text.into_bytes()
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        // Binary messages can be used directly
                        bytes
                    }
                    _ => continue,
                };
                
                // SIMD-accelerated JSON parsing (Requirement 8.2)
                // Parse directly from bytes without intermediate String allocation
                let mut bytes_mut = bytes;
                let v: serde_json::Value = match simd_json::serde::from_slice(&mut bytes_mut) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let channel = v.get("channel").and_then(|c| c.as_str()).unwrap_or("");
                if channel != "activeAssetCtx" && channel != "bbo" {
                    continue;
                }

                if !first_data_logged {
                    first_data_logged = true;
                    println!("Hyperliquid ws[{}] first data message received", worker_id);
                }

                let coin = match v.get("data").and_then(|d| d.get("coin")).and_then(|c| c.as_str()) {
                    Some(c) => c,
                    None => continue,
                };

                let payload = serde_json::to_string(&v)?;

                // Publish to separate keys based on channel to avoid overwriting
                if channel == "activeAssetCtx" {
                    let key = format!("hyperliquid:usdc:ctx:{}", coin);
                    if tx.send((key, payload)).await.is_err() {
                        break;
                    }
                } else if channel == "bbo" {
                    let key = format!("hyperliquid:usdc:bbo:{}", coin);
                    if tx.send((key, payload)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
