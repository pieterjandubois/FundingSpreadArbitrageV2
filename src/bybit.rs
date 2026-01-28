use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::DynError;
use crate::utils;

const BYBIT_BASE_URL: &str = "https://api.bybit.com";
const BYBIT_LINEAR_WS_PUBLIC_URL: &str = "wss://stream.bybit.com/v5/public/linear";

const TOPICS_PER_CONNECTION: usize = 100;
const SUBSCRIBE_BATCH_SIZE: usize = 10;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 100;

pub struct BybitLinearConnector;

#[derive(Debug, Deserialize)]
struct InstrumentsInfoResponse {
    #[serde(rename = "retCode")]
    ret_code: i64,
    result: Option<InstrumentsInfoResult>,
}

#[derive(Debug, Deserialize)]
struct InstrumentsInfoResult {
    list: Vec<InstrumentInfo>,
    #[serde(rename = "nextPageCursor")]
    next_page_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InstrumentInfo {
    symbol: String,
    status: String,
    #[serde(rename = "quoteCoin")]
    quote_coin: Option<String>,
}

impl BybitLinearConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let url = format!("{}/v5/market/time", BYBIT_BASE_URL);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            eprintln!("Bybit Futures connection check failed: {}", response.status());
            std::process::exit(1);
        }
        println!("Bybit Futures connection check OK");
        Ok(())
    }

    pub async fn run(client: &reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let symbols = fetch_valid_linear_symbols(client).await?;
        println!("Valid BYBIT linear symbols (TRADING): {}", symbols.len());

        let mut topics: Vec<String> = symbols.iter().map(|s| format!("tickers.{}", s)).collect();
        topics.extend(symbols.into_iter().map(|s| format!("funding.{}", s)));
        
        let batches = utils::chunk_vec(&topics, TOPICS_PER_CONNECTION);
        println!("Starting Bybit websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_bybit_linear_ws_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_valid_linear_symbols(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let mut cursor: Option<String> = None;
    let mut symbols: Vec<String> = Vec::new();

    loop {
        let url = format!("{}/v5/market/instruments-info", BYBIT_BASE_URL);
        let mut req = client.get(url).query(&[("category", "linear"), ("limit", "1000")]);
        if let Some(c) = cursor.as_ref() {
            req = req.query(&[("cursor", c.as_str())]);
        }

        let resp = req.send().await?.json::<InstrumentsInfoResponse>().await?;
        if resp.ret_code != 0 {
            return Err(format!("Bybit instruments-info returned retCode={}", resp.ret_code).into());
        }

        let result = match resp.result {
            Some(r) => r,
            None => break,
        };

        for i in result.list {
            if i.status != "Trading" {
                continue;
            }
            if i.quote_coin.as_deref() != Some("USDT") {
                continue;
            }
            symbols.push(i.symbol);
        }

        cursor = result.next_page_cursor.and_then(|c| if c.is_empty() { None } else { Some(c) });
        if cursor.is_none() {
            break;
        }
    }

    symbols.sort();
    symbols.dedup();
    Ok(symbols)
}

async fn run_bybit_linear_ws_batch(worker_id: usize, topics: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(BYBIT_LINEAR_WS_PUBLIC_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("Bybit ws[{}] connected", worker_id);

    let mut first_data_logged = false;
    let mut ticker_state: std::collections::HashMap<String, Value> = std::collections::HashMap::new();

    utils::subscribe_in_batches(&mut write, topics, SUBSCRIBE_BATCH_SIZE, SUBSCRIBE_BATCH_DELAY_MS, |w, chunk| {
        Box::pin(async move {
            let subscribe = json!({
                "op": "subscribe",
                "args": chunk
            });
            w
                .send(tokio_tungstenite::tungstenite::Message::Text(subscribe.to_string()))
                .await?;
            Ok(())
        })
    }).await?;

    let mut ping_tick = utils::interval_secs(20);

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                let ping = json!({"op": "ping"});
                if write.send(tokio_tungstenite::tungstenite::Message::Text(ping.to_string())).await.is_err() {
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
                let v: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let topic = match v.get("topic").and_then(|t| t.as_str()) {
                    Some(t) => t,
                    None => continue,
                };
                
                let (key_type, symbol) = if topic.starts_with("tickers.") {
                    ("tickers", topic.trim_start_matches("tickers."))
                } else if topic.starts_with("funding.") {
                    ("funding", topic.trim_start_matches("funding."))
                } else {
                    continue
                };

                if !first_data_logged {
                    first_data_logged = true;
                    println!("Bybit ws[{}] first data message received", worker_id);
                }

                let key = format!("bybit:linear:{}:{}", key_type, symbol);
                
                // For tickers, merge delta updates into state and publish incrementally
                if key_type == "tickers" {
                    if let Some(data) = v.get("data") {
                        let state = ticker_state.entry(symbol.to_string()).or_insert_with(|| v.clone());
                        
                        // Merge data fields
                        if let Some(state_data) = state.get_mut("data") {
                            if let Some(obj) = state_data.as_object_mut() {
                                if let Some(new_data) = data.as_object() {
                                    for (k, v) in new_data {
                                        obj.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                        }
                        
                        // Update top-level fields from new message
                        if let Some(obj) = state.as_object_mut() {
                            for (k, v) in v.as_object().unwrap_or(&Default::default()) {
                                if k != "data" {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        
                        // Publish merged state incrementally to Redis
                        let payload = serde_json::to_string(&state)?;
                        if tx.send((key, payload)).await.is_err() {
                            break;
                        }
                    }
                } else {
                    // For funding, publish as-is
                    let payload = serde_json::to_string(&v)?;
                    if tx.send((key, payload)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_bybit_linear_ws_worker(worker_id: usize, topics: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_bybit_linear_ws_batch(worker_id, &topics[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Bybit ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Bybit ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }
        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}
