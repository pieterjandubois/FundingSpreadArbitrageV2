use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type OkxWrite = futures_util::stream::SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>;

use crate::DynError;
use crate::utils;
use crate::strategy::pipeline::MarketProducer;
use crate::strategy::types::{MarketUpdate, symbol_to_id};

const OKX_BASE_URL: &str = "https://www.okx.com";
const OKX_WS_PUBLIC_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";

const INSTRUMENTS_PER_CONNECTION: usize = 80;
const SUBSCRIBE_INSTRUMENTS_PER_MSG: usize = 25;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 50;

pub struct OkxUsdtSwapConnector;

#[derive(Debug, Deserialize)]
struct OkxResponse<T> {
    code: String,
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct OkxInstrument {
    #[serde(rename = "instId")]
    inst_id: String,
    state: String,
    #[serde(rename = "settleCcy")]
    settle_ccy: String,
}

impl OkxUsdtSwapConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let url = format!("{}/api/v5/public/time", OKX_BASE_URL);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            eprintln!("OKX Futures connection check failed: {}", response.status());
            std::process::exit(1);
        }
        println!("OKX Futures connection check OK");
        Ok(())
    }

    pub async fn run(
        client: &reqwest::Client, 
        tx: mpsc::Sender<(String, String)>,
        market_producer: Option<MarketProducer>,
    ) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let inst_ids = fetch_valid_usdt_swap_instruments(client).await?;
        println!("Valid OKX USDT SWAP instruments (live): {}", inst_ids.len());

        let batches = utils::chunk_vec(&inst_ids, INSTRUMENTS_PER_CONNECTION);
        println!("Starting OKX websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_okx_ws_worker(
                worker_id, 
                Arc::new(batch), 
                tx.clone(),
                market_producer.clone(),
            ));
        }

        Ok(())
    }
}

async fn fetch_valid_usdt_swap_instruments(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let url = format!("{}/api/v5/public/instruments?instType=SWAP", OKX_BASE_URL);
    let resp = client.get(url).send().await?.json::<OkxResponse<OkxInstrument>>().await?;

    if resp.code != "0" {
        return Err(format!("OKX instruments returned code={}", resp.code).into());
    }

    let mut inst_ids: Vec<String> = resp
        .data
        .into_iter()
        .filter(|i| i.state == "live")
        .filter(|i| i.settle_ccy == "USDT")
        .map(|i| i.inst_id)
        .collect();

    inst_ids.sort();
    inst_ids.dedup();
    Ok(inst_ids)
}

async fn subscribe_channel(
    write: &mut OkxWrite,
    channel: &str,
    inst_ids: &[String],
) -> Result<(), DynError> {
    let mut args = Vec::with_capacity(inst_ids.len());
    for inst_id in inst_ids {
        args.push(serde_json::json!({"channel": channel, "instId": inst_id}));
    }

    let subscribe = serde_json::json!({
        "op": "subscribe",
        "args": args
    });

    write
        .send(tokio_tungstenite::tungstenite::Message::Text(subscribe.to_string()))
        .await?;

    Ok(())
}

async fn run_okx_ws_batch(
    worker_id: usize, 
    inst_ids: &[String], 
    tx: mpsc::Sender<(String, String)>,
    market_producer: Option<MarketProducer>,
) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(OKX_WS_PUBLIC_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("OKX ws[{}] connected", worker_id);

    let mut first_data_logged = false;

    utils::subscribe_in_batches(&mut write, inst_ids, SUBSCRIBE_INSTRUMENTS_PER_MSG, 0, |w, chunk| {
        Box::pin(async move {
            subscribe_channel(w, "tickers", chunk).await?;
            time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;

            subscribe_channel(w, "funding-rate", chunk).await?;
            time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;

            subscribe_channel(w, "books5", chunk).await?;
            time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;
            Ok(())
        })
    }).await?;

    let mut ping_tick = utils::interval_secs(20);

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                if write.send(tokio_tungstenite::tungstenite::Message::Text("ping".to_string())).await.is_err() {
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

                // Skip pong messages (OKX specific)
                if bytes == b"pong" {
                    continue;
                }

                // SIMD-accelerated JSON parsing (Requirement 8.2)
                // Parse directly from bytes without intermediate String allocation
                let mut bytes_mut = bytes;
                let v: serde_json::Value = match simd_json::serde::from_slice(&mut bytes_mut) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if !first_data_logged {
                    if v.get("data").is_some() {
                        first_data_logged = true;
                        println!("OKX ws[{}] first data message received", worker_id);
                    }
                }

                let arg = match v.get("arg") {
                    Some(a) => a,
                    None => continue,
                };
                let channel = match arg.get("channel").and_then(|c| c.as_str()) {
                    Some(c) => c,
                    None => continue,
                };
                let inst_id = match arg.get("instId").and_then(|i| i.as_str()) {
                    Some(i) => i,
                    None => continue,
                };

                // HOT PATH: Push to queue if this is ticker or book data and we have a producer
                if channel == "tickers" || channel == "books5" {
                    if let Some(ref producer) = market_producer {
                        if let Some(data_array) = v.get("data").and_then(|d| d.as_array()) {
                            if let Some(data) = data_array.first() {
                                let (bid, ask) = if channel == "tickers" {
                                    // Tickers have bidPx/askPx
                                    (
                                        data.get("bidPx").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()),
                                        data.get("askPx").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()),
                                    )
                                } else {
                                    // books5 has bids/asks arrays
                                    let bid = data.get("bids")
                                        .and_then(|b| b.as_array())
                                        .and_then(|arr| arr.first())
                                        .and_then(|level| level.as_array())
                                        .and_then(|level| level.first())
                                        .and_then(|p| p.as_str())
                                        .and_then(|s| s.parse::<f64>().ok());
                                    let ask = data.get("asks")
                                        .and_then(|a| a.as_array())
                                        .and_then(|arr| arr.first())
                                        .and_then(|level| level.as_array())
                                        .and_then(|level| level.first())
                                        .and_then(|p| p.as_str())
                                        .and_then(|s| s.parse::<f64>().ok());
                                    (bid, ask)
                                };

                                if let (Some(bid), Some(ask)) = (bid, ask) {
                                    // Map symbol to ID (OKX uses BTC-USDT format, convert to BTCUSDT)
                                    let symbol = inst_id.replace("-", "");
                                    if let Some(symbol_id) = symbol_to_id(&symbol) {
                                        let timestamp_us = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap()
                                            .as_micros() as u64;
                                        
                                        let update = MarketUpdate::new(symbol_id, bid, ask, timestamp_us);
                                        producer.push(update);
                                    }
                                }
                            }
                        }
                    }
                }

                let key = match channel {
                    "tickers" => format!("okx:usdt:tickers:{}", inst_id),
                    "funding-rate" => format!("okx:usdt:funding:{}", inst_id),
                    "books5" => format!("okx:usdt:book:{}", inst_id),
                    _ => continue,
                };

                // COLD PATH: Still write to Redis for monitoring/dashboard
                let payload = serde_json::to_string(&v)?;
                if tx.send((key, payload)).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn run_okx_ws_worker(
    worker_id: usize, 
    inst_ids: Arc<Vec<String>>, 
    tx: mpsc::Sender<(String, String)>,
    market_producer: Option<MarketProducer>,
) {
    // Pin WebSocket thread to cores 2-7 for optimal cache performance
    // Requirement: 4.2 (Pin WebSocket threads to cores 2-7)
    if let Err(e) = crate::strategy::thread_pinning::pin_websocket_thread(worker_id) {
        eprintln!("[THREAD-PIN] Warning: Failed to pin OKX worker {}: {}", worker_id, e);
        eprintln!("[THREAD-PIN] Continuing without thread pinning (performance may be degraded)");
    }
    
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_okx_ws_batch(worker_id, &inst_ids[..], tx.clone(), market_producer.clone()).await;
        match &res {
            Ok(()) => println!("[{}] OKX ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] OKX ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }

        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}
