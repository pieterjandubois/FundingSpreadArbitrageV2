/* use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{self, Duration};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::utils;
use crate::DynError;

type GateWrite = futures_util::stream::SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>;

const GATE_FUTURES_REST_BASE_URL: &str = "https://fx-api.gateio.ws";
const GATE_FUTURES_WS_USDT_URL: &str = "wss://fx-ws.gateio.ws/v4/ws/usdt";

const CONTRACTS_PER_CONNECTION: usize = 100;
const SUBSCRIBE_CONTRACTS_PER_MSG: usize = 50;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 100;

pub struct GateioUsdtPerpConnector;

#[derive(Debug, Deserialize)]
struct GateContract {
    name: String,
    #[serde(default)]
    in_delisting: bool,
    #[serde(default)]
    trade_status: Option<String>,
}

impl GateioUsdtPerpConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let url = format!("{}/api/v4/futures/usdt/contracts", GATE_FUTURES_REST_BASE_URL);
        match tokio::time::timeout(Duration::from_secs(5), client.get(url).send()).await {
            Ok(Ok(response)) => {
                if !response.status().is_success() {
                    eprintln!("Gate.io Futures connection check failed: {}", response.status());
                    std::process::exit(1);
                }
                println!("Gate.io Futures connection check OK");
                Ok(())
            }
            Ok(Err(e)) => {
                eprintln!("Gate.io Futures connection check error: {}", e);
                std::process::exit(1);
            }
            Err(_) => {
                eprintln!("Gate.io Futures connection check timeout");
                std::process::exit(1);
            }
        }
    }

    pub async fn run(
        client: &reqwest::Client, 
        tx: mpsc::Sender<(String, String)>,
        _market_producer: Option<crate::strategy::pipeline::MarketProducer>,
    ) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let contracts = fetch_valid_usdt_contracts(client).await?;
        println!("Valid GATE USDT futures contracts: {}", contracts.len());

        let batches = utils::chunk_vec(&contracts, CONTRACTS_PER_CONNECTION);
        println!("Starting Gate.io websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_gate_ws_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_valid_usdt_contracts(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let url = format!("{}/api/v4/futures/usdt/contracts", GATE_FUTURES_REST_BASE_URL);
    let resp = match tokio::time::timeout(Duration::from_secs(5), client.get(url).send()).await {
        Ok(Ok(r)) => r.json::<Vec<GateContract>>().await?,
        Ok(Err(e)) => return Err(format!("Gateio fetch error: {}", e).into()),
        Err(_) => return Err("Gateio fetch timeout".into()),
    };

    let mut contracts: Vec<String> = resp
        .into_iter()
        .filter(|c| !c.in_delisting)
        .filter(|c| c.trade_status.as_deref().map(|s| s == "tradable").unwrap_or(true))
        .map(|c| c.name)
        .collect();

    contracts.sort();
    contracts.dedup();
    Ok(contracts)
}

async fn gate_send(
    write: &mut GateWrite,
    channel: &str,
    event: &str,
    payload: serde_json::Value,
) -> Result<(), DynError> {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let msg = serde_json::json!({
        "time": time,
        "channel": channel,
        "event": event,
        "payload": payload,
    });

    write
        .send(tokio_tungstenite::tungstenite::Message::Text(msg.to_string()))
        .await?;

    Ok(())
}

async fn run_gate_ws_batch(worker_id: usize, contracts: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(GATE_FUTURES_WS_USDT_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("Gate ws[{}] connected", worker_id);

    let mut first_data_logged = false;

    for chunk in contracts.chunks(SUBSCRIBE_CONTRACTS_PER_MSG) {
        let list = serde_json::Value::Array(chunk.iter().map(|c| serde_json::Value::String(c.clone())).collect());

        // Try different channel names for tickers/funding data
        gate_send(&mut write, "futures.ticker", "subscribe", list.clone()).await?;
        time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;

        gate_send(&mut write, "futures.tickers", "subscribe", list.clone()).await?;
        time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;

        gate_send(&mut write, "futures.book_ticker", "subscribe", list).await?;
        time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;
    }

    let mut ping_tick = time::interval(std::time::Duration::from_secs(20));

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                let _ = gate_send(&mut write, "futures.ping", "", serde_json::Value::Null).await;
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

                if !first_data_logged {
                    if channel == "futures.tickers" || channel == "futures.ticker" || channel == "futures.book_ticker" {
                        if v.get("result").is_some() {
                            first_data_logged = true;
                            println!("Gate ws[{}] first data message received", worker_id);
                        }
                    }
                }

                // Debug: Log all messages to see what we're receiving
                if !channel.is_empty() {
                    println!("Gate ws[{}] channel: '{}', event: '{}', msg: {}", worker_id, channel, v.get("event").and_then(|e| e.as_str()).unwrap_or(""), serde_json::to_string(&v).unwrap_or_default().chars().take(200).collect::<String>());
                }

                if channel == "futures.tickers" || channel == "futures.ticker" {
                    let result = match v.get("result") {
                        Some(r) => r,
                        None => continue,
                    };

                    let contract = result.get("contract").and_then(|c| c.as_str());
                    let contract = match contract {
                        Some(c) => c,
                        None => continue,
                    };

                    let payload = serde_json::to_string(&v)?;

                    let key = format!("gateio:usdt:tickers:{}", contract);
                    if tx.send((key, payload.clone())).await.is_err() {
                        break;
                    }

                    let funding_key = format!("gateio:usdt:funding:{}", contract);
                    if tx.send((funding_key, payload)).await.is_err() {
                        break;
                    }
                }

                if channel == "futures.book_ticker" {
                    let result = match v.get("result") {
                        Some(r) => r,
                        None => continue,
                    };

                    let contract = result.get("s").and_then(|c| c.as_str());
                    let contract = match contract {
                        Some(c) => c,
                        None => continue,
                    };

                    let payload = serde_json::to_string(&v)?;
                    let key = format!("gateio:usdt:book:{}", contract);
                    if tx.send((key, payload)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_gate_ws_worker(worker_id: usize, contracts: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_gate_ws_batch(worker_id, &contracts[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Gate ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Gate ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }

        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}
 */