use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::utils;
use crate::DynError;

type BitgetWrite = futures_util::stream::SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>;

const BITGET_BASE_URL: &str = "https://api.bitget.com";
const BITGET_WS_PUBLIC_URL: &str = "wss://ws.bitget.com/v2/ws/public";

const SYMBOLS_PER_CONNECTION: usize = 40;
const SUBSCRIBE_SYMBOLS_PER_MSG: usize = 20;
const SUBSCRIBE_BATCH_DELAY_MS: u64 = 100;

pub struct BitgetUsdtFuturesConnector;

#[derive(Debug, Deserialize)]
struct BitgetResponse<T> {
    code: String,
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct BitgetContract {
    symbol: String,
    #[serde(rename = "quoteCoin")]
    quote_coin: String,
    #[serde(rename = "symbolType")]
    symbol_type: String,
    #[serde(rename = "symbolStatus")]
    symbol_status: String,
}

impl BitgetUsdtFuturesConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let url = format!("{}/api/v2/public/time", BITGET_BASE_URL);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            eprintln!("Bitget Futures connection check failed: {}", response.status());
            std::process::exit(1);
        }
        println!("Bitget Futures connection check OK");
        Ok(())
    }

    pub async fn run(client: &reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let symbols = fetch_valid_usdt_perp_symbols(client).await?;
        println!("Valid BITGET USDT futures symbols (perpetual, normal): {}", symbols.len());

        let batches = utils::chunk_vec(&symbols, SYMBOLS_PER_CONNECTION);
        println!("Starting Bitget websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_bitget_ws_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

async fn fetch_valid_usdt_perp_symbols(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let url = format!("{}/api/v2/mix/market/contracts?productType=usdt-futures", BITGET_BASE_URL);
    let resp = client.get(url).send().await?.json::<BitgetResponse<BitgetContract>>().await?;

    if resp.code != "00000" {
        return Err(format!("Bitget contracts returned code={}", resp.code).into());
    }

    let mut symbols: Vec<String> = resp
        .data
        .into_iter()
        .filter(|c| c.quote_coin == "USDT")
        .filter(|c| c.symbol_type == "perpetual")
        .filter(|c| c.symbol_status == "normal")
        .map(|c| c.symbol)
        .collect();

    symbols.sort();
    symbols.dedup();
    Ok(symbols)
}

async fn subscribe_bitget_channel(
    write: &mut BitgetWrite,
    channel: &str,
    inst_ids: &[String],
) -> Result<(), DynError> {
    let mut args = Vec::with_capacity(inst_ids.len());
    for inst_id in inst_ids {
        args.push(serde_json::json!({
            "instType": "USDT-FUTURES",
            "channel": channel,
            "instId": inst_id,
        }));
    }

    let subscribe = serde_json::json!({
        "op": "subscribe",
        "args": args,
    });

    write
        .send(tokio_tungstenite::tungstenite::Message::Text(subscribe.to_string()))
        .await?;

    Ok(())
}

async fn run_bitget_ws_batch(worker_id: usize, symbols: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let (ws, _) = tokio_tungstenite::connect_async(BITGET_WS_PUBLIC_URL).await?;
    let (mut write, mut read) = ws.split();

    println!("Bitget ws[{}] connected", worker_id);

    let mut first_data_logged = false;

    for chunk in symbols.chunks(SUBSCRIBE_SYMBOLS_PER_MSG) {
        subscribe_bitget_channel(&mut write, "ticker", chunk).await?;
        time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;

        subscribe_bitget_channel(&mut write, "books5", chunk).await?;
        time::sleep(std::time::Duration::from_millis(SUBSCRIBE_BATCH_DELAY_MS)).await;
    }

    let mut ping_tick = time::interval(std::time::Duration::from_secs(30));

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

                if !msg.is_text() {
                    continue;
                }

                let text = msg.into_text()?;
                if text == "pong" {
                    continue;
                }

                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if !first_data_logged {
                    if v.get("data").is_some() {
                        first_data_logged = true;
                        println!("Bitget ws[{}] first data message received", worker_id);
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

                let payload = match serde_json::to_string(&v) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                match channel {
                    "ticker" => {
                        let key = format!("bitget:usdt:tickers:{}", inst_id);
                        if tx.send((key, payload.clone())).await.is_err() {
                            break;
                        }

                        let funding_key = format!("bitget:usdt:funding:{}", inst_id);
                        if tx.send((funding_key, payload)).await.is_err() {
                            break;
                        }
                    }
                    "books5" => {
                        let key = format!("bitget:usdt:book:{}", inst_id);
                        if tx.send((key, payload)).await.is_err() {
                            break;
                        }
                    }
                    _ => continue,
                }
            }
        }
    }

    Ok(())
}

async fn run_bitget_ws_worker(worker_id: usize, symbols: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_bitget_ws_batch(worker_id, &symbols[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Bitget ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Bitget ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }

        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}
