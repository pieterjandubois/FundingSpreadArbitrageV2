use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time;

use crate::DynError;
use crate::utils;

const WS_TOKEN_URL: &str = "https://api-futures.kucoin.com/api/v1/bullet-public";
const CONTRACTS_ACTIVE_URL: &str = "https://api-futures.kucoin.com/api/v1/contracts/active";

const SYMBOLS_PER_CONNECTION: usize = 50;
const SUBSCRIBE_SYMBOLS_PER_MSG: usize = 20;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 50;

pub struct KucoinFuturesConnector;

#[derive(Debug, Deserialize)]
struct KucoinApiResponse<T> {
    code: String,
    data: T,
}

#[derive(Debug, Deserialize)]
struct ContractInfo {
    symbol: String,
    status: String,
    #[serde(rename = "quoteCurrency")]
    quote_currency: String,
    #[serde(rename = "settleCurrency")]
    settle_currency: String,
}

#[derive(Debug, Deserialize)]
struct WsTokenData {
    token: String,
    #[serde(rename = "instanceServers")]
    instance_servers: Vec<WsInstanceServer>,
}

#[derive(Debug, Deserialize)]
struct WsInstanceServer {
    endpoint: String,
    #[serde(rename = "pingInterval")]
    ping_interval_ms: u64,
}

impl KucoinFuturesConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let response = client.get(CONTRACTS_ACTIVE_URL).send().await?;
        if !response.status().is_success() {
            eprintln!("KuCoin Futures connection check failed: {}", response.status());
            std::process::exit(1);
        }
        println!("KuCoin Futures connection check OK");
        Ok(())
    }

    pub async fn run(
        client: &reqwest::Client, 
        tx: mpsc::Sender<(String, String)>,
        _market_producer: Option<crate::strategy::pipeline::MarketProducer>,
    ) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let symbols = fetch_valid_contract_symbols(client).await?;
        println!("Valid KUCOIN futures symbols (Open): {}", symbols.len());

        let batches = utils::chunk_vec(&symbols, SYMBOLS_PER_CONNECTION);
        println!("Starting KuCoin websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_ws_worker(worker_id, Arc::new(batch), client.clone(), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_valid_contract_symbols(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let resp = client
        .get(CONTRACTS_ACTIVE_URL)
        .send()
        .await?
        .json::<KucoinApiResponse<Vec<ContractInfo>>>()
        .await?;

    if resp.code != "200000" {
        return Err(format!("KuCoin contracts/active returned code={}", resp.code).into());
    }

    let mut symbols: Vec<String> = resp
        .data
        .into_iter()
        .filter(|c| c.status == "Open")
        .filter(|c| c.quote_currency == "USDT")
        .filter(|c| c.settle_currency == "USDT")
        .map(|c| c.symbol)
        .collect();

    symbols.sort();
    symbols.dedup();
    Ok(symbols)
}

async fn fetch_ws_endpoint_and_token(client: &reqwest::Client) -> Result<(String, u64), DynError> {
    let resp = client
        .post(WS_TOKEN_URL)
        .send()
        .await?
        .json::<KucoinApiResponse<WsTokenData>>()
        .await?;

    if resp.code != "200000" {
        return Err(format!("KuCoin bullet-public returned code={}", resp.code).into());
    }

    let server = resp
        .data
        .instance_servers
        .into_iter()
        .next()
        .ok_or_else(|| "KuCoin bullet-public returned empty instanceServers".to_string())?;

    let connect_id = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );

    let endpoint = format!("{}?token={}&connectId={}", server.endpoint, resp.data.token, connect_id);
    Ok((endpoint, server.ping_interval_ms))
}

async fn run_ws_batch(
    worker_id: usize,
    symbols: &[String],
    client: reqwest::Client,
    tx: mpsc::Sender<(String, String)>,
) -> Result<(), DynError> {
    let (ws_url, ping_interval_ms) = fetch_ws_endpoint_and_token(&client).await?;
    let (ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
    let (mut write, mut read) = ws.split();

    println!("KuCoin ws[{}] connected", worker_id);
    let mut first_data_logged = false;
    let mut symbol_state: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();

    let mut sub_id: u64 = 1;
    for chunk in symbols.chunks(SUBSCRIBE_SYMBOLS_PER_MSG) {
        let joined = chunk.join(",");
        let topics = [
            format!("/contractMarket/tickerV2:{}", joined),
            format!("/contractMarket/level2:{}", joined),
            format!("/contract/instrument:{}", joined),
        ];

        for topic in topics {
            sub_id += 1;
            let subscribe = serde_json::json!({
                "id": sub_id.to_string(),
                "type": "subscribe",
                "topic": topic,
                "response": true
            });
            write
                .send(tokio_tungstenite::tungstenite::Message::Text(subscribe.to_string()))
                .await?;
            time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;
        }
    }

    sub_id += 1;
    let subscribe_announcement = serde_json::json!({
        "id": sub_id.to_string(),
        "type": "subscribe",
        "topic": "/contract/announcement",
        "response": false
    });
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(subscribe_announcement.to_string()))
        .await?;

    let mut ping_tick = time::interval(std::time::Duration::from_millis(ping_interval_ms.max(1000)));

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                let ping_id = format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                );
                let ping = serde_json::json!({"id": ping_id, "type": "ping"});
                if write.send(tokio_tungstenite::tungstenite::Message::Text(ping.to_string())).await.is_err() {
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

                if !first_data_logged {
                    if v.get("type").and_then(|t| t.as_str()) == Some("message") {
                        first_data_logged = true;
                        println!("KuCoin ws[{}] first data message received", worker_id);
                    }
                }

                if v.get("type").and_then(|t| t.as_str()) != Some("message") {
                    continue;
                }

                let topic = match v.get("topic").and_then(|t| t.as_str()) {
                    Some(t) => t,
                    None => continue,
                };

                let subject = v.get("subject").and_then(|s| s.as_str()).unwrap_or("");
                let data = v.get("data");

                // Handle tickerV2 with incremental publishing
                if topic.starts_with("/contractMarket/tickerV2:") {
                    if let Some(symbol) = data
                        .and_then(|d| d.get("symbol"))
                        .and_then(|s| s.as_str())
                        .or_else(|| topic.split(':').nth(1))
                    {
                        let state = symbol_state.entry(symbol.to_string()).or_insert_with(|| v.clone());
                        
                        // Merge tickerV2 data into existing state
                        if let Some(state_obj) = state.as_object_mut() {
                            if let Some(new_obj) = v.as_object() {
                                for (k, val) in new_obj {
                                    state_obj.insert(k.clone(), val.clone());
                                }
                            }
                        }
                        
                        // Publish merged state
                        let key = format!("kucoin:futures:tickerV2:{}", symbol);
                        let payload = serde_json::to_string(&state)?;
                        if tx.send((key, payload)).await.is_err() {
                            break;
                        }
                    }
                    continue;
                }

                if topic.starts_with("/contract/instrument:") && subject == "funding.rate" {
                    if let Some(symbol) = topic.split(':').nth(1) {
                        // Publish funding rate separately for now
                        let key = format!("kucoin:futures:instrument:{}:{}", symbol, subject);
                        let payload = serde_json::to_string(&v)?;
                        if tx.send((key, payload)).await.is_err() {
                            break;
                        }
                    }
                    continue;
                }

                let (key, payload) = match kucoin_redis_key_and_payload(topic, subject, data, &v) {
                    Some(kp) => kp,
                    None => continue,
                };

                if tx.send((key, payload)).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

fn kucoin_redis_key_and_payload(
    topic: &str,
    subject: &str,
    data: Option<&serde_json::Value>,
    full: &serde_json::Value,
) -> Option<(String, String)> {
    if topic.starts_with("/contractMarket/tickerV2:") {
        let symbol = data
            .and_then(|d| d.get("symbol"))
            .and_then(|s| s.as_str())
            .or_else(|| topic.split(':').nth(1));
        let symbol = symbol?;
        let key = format!("kucoin:futures:tickerV2:{}", symbol);
        let payload = serde_json::to_string(full).ok()?;
        return Some((key, payload));
    }

    if topic.starts_with("/contractMarket/level2:") {
        let symbol = topic.split(':').nth(1)?;
        let key = format!("kucoin:futures:level2:{}", symbol);
        let payload = serde_json::to_string(full).ok()?;
        return Some((key, payload));
    }

    if topic.starts_with("/contract/instrument:") {
        let symbol = topic.split(':').nth(1)?;
        let key = format!("kucoin:futures:instrument:{}:{}", symbol, subject);
        let payload = serde_json::to_string(full).ok()?;
        return Some((key, payload));
    }

    if topic == "/contract/announcement" {
        let symbol = data
            .and_then(|d| d.get("symbol"))
            .and_then(|s| s.as_str())?;
        let key = format!("kucoin:futures:funding_settlement:{}:{}", symbol, subject);
        let payload = serde_json::to_string(full).ok()?;
        return Some((key, payload));
    }

    None
}

async fn run_ws_worker(worker_id: usize, symbols: Arc<Vec<String>>, client: reqwest::Client, tx: mpsc::Sender<(String, String)>) {
    // Pin WebSocket thread to cores 2-7 for optimal cache performance
    // Requirement: 4.2 (Pin WebSocket threads to cores 2-7)
    if let Err(e) = crate::strategy::thread_pinning::pin_websocket_thread(worker_id) {
        eprintln!("[THREAD-PIN] Warning: Failed to pin KuCoin worker {}: {}", worker_id, e);
        eprintln!("[THREAD-PIN] Continuing without thread pinning (performance may be degraded)");
    }
    
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_ws_batch(worker_id, &symbols[..], client.clone(), tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] KuCoin ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] KuCoin ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }

        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}
