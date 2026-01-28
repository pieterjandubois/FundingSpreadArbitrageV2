use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::DynError;
use crate::utils;

const BINANCE_USDM_BASE_URL: &str = "https://fapi.binance.com";
const BINANCE_USDM_WS_COMBINED_STREAM_BASE_URL: &str = "wss://fstream.binance.com/stream?streams=";

const STREAMS_PER_CONNECTION: usize = 50;

pub struct BinanceUsdmConnector;

impl BinanceUsdmConnector {
    async fn connection_check(client: &reqwest::Client) -> Result<(), DynError> {
        let url = format!("{}/fapi/v1/time", BINANCE_USDM_BASE_URL);
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            eprintln!("Binance Futures connection check failed: {}", response.status());
            std::process::exit(1);
        }
        println!("Binance Futures connection check OK");
        Ok(())
    }

    pub async fn run(client: &reqwest::Client, tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
        Self::connection_check(client).await?;

        let symbols = fetch_valid_usdt_perp_symbols(client).await?;
        println!("Valid USDT PERPETUAL symbols (TRADING): {}", symbols.len());

        let streams = build_streams(&symbols);
        let batches = utils::chunk_vec(&streams, STREAMS_PER_CONNECTION);
        println!("Starting Binance websocket workers: {}", batches.len());

        for (worker_id, batch) in batches.into_iter().enumerate() {
            tokio::spawn(run_ws_worker(worker_id, Arc::new(batch), tx.clone()));
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ExchangeInfo {
    symbols: Vec<ExchangeSymbol>,
}

#[derive(Debug, Deserialize)]
struct ExchangeSymbol {
    symbol: String,
    status: String,
    #[serde(rename = "quoteAsset")]
    quote_asset: String,
    #[serde(rename = "contractType")]
    contract_type: String,
}

async fn fetch_valid_usdt_perp_symbols(client: &reqwest::Client) -> Result<Vec<String>, DynError> {
    let url = format!("{}/fapi/v1/exchangeInfo", BINANCE_USDM_BASE_URL);
    let exchange_info = client.get(url).send().await?.json::<ExchangeInfo>().await?;

    let mut symbols: Vec<String> = exchange_info
        .symbols
        .into_iter()
        .filter(|s| s.status == "TRADING")
        .filter(|s| s.quote_asset == "USDT")
        .filter(|s| s.contract_type == "PERPETUAL")
        .filter(|s| is_plain_usdt_symbol(&s.symbol))
        .map(|s| s.symbol)
        .collect();

    symbols.sort();
    Ok(symbols)
}

fn build_streams(symbols: &[String]) -> Vec<String> {
    let mut streams = Vec::with_capacity(symbols.len() * 2);
    for s in symbols {
        let sym = s.to_lowercase();
        streams.push(format!("{}@markPrice@1s", sym));
        streams.push(format!("{}@bookTicker", sym));
    }
    streams
}

fn redis_key_for(stream_name: &str, symbol: &str) -> Option<String> {
    if stream_name.ends_with("@bookTicker") {
        return Some(format!("binance:usdm:book:{}", symbol));
    }
    if stream_name.contains("@markPrice") {
        return Some(format!("binance:usdm:mark:{}", symbol));
    }
    None
}

async fn run_ws_batch(worker_id: usize, streams: &[String], tx: mpsc::Sender<(String, String)>) -> Result<(), DynError> {
    let url = format!("{}{}", BINANCE_USDM_WS_COMBINED_STREAM_BASE_URL, streams.join("/"));
    let (ws, _) = tokio_tungstenite::connect_async(url).await?;
    let (mut write, mut read) = ws.split();

    println!("Binance ws[{}] connected", worker_id);

    let mut first_data_logged = false;

    let mut ping_tick = utils::interval_secs(20);

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                if write.send(tokio_tungstenite::tungstenite::Message::Ping(Vec::new())).await.is_err() {
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

                if !first_data_logged {
                    first_data_logged = true;
                    println!("Binance ws[{}] first data message received", worker_id);
                }

                let stream_name = match v.get("stream").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => continue,
                };
                let data = match v.get("data") {
                    Some(d) => d,
                    None => continue,
                };
                let symbol = match data.get("s").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => continue,
                };

                let key = match redis_key_for(stream_name, symbol) {
                    Some(k) => k,
                    None => continue,
                };

                let payload = serde_json::to_string(data)?;
                if tx.send((key, payload)).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn run_ws_worker(worker_id: usize, streams: Arc<Vec<String>>, tx: mpsc::Sender<(String, String)>) {
    let mut backoff_ms: u64 = 0;
    loop {
        let res = run_ws_batch(worker_id, &streams[..], tx.clone()).await;
        match &res {
            Ok(()) => println!("[{}] Binance ws[{}] disconnected -> reconnecting", utils::ts_hm(), worker_id),
            Err(e) => println!("[{}] Binance ws[{}] error: {} -> reconnecting", utils::ts_hm(), worker_id, e),
        }
        match res {
            Ok(()) => utils::reset_backoff(&mut backoff_ms),
            Err(_) => utils::apply_backoff(&mut backoff_ms).await,
        }
    }
}

fn is_plain_usdt_symbol(symbol: &str) -> bool {
    if !symbol.is_ascii() {
        return false;
    }
    if !symbol.ends_with("USDT") {
        return false;
    }

    symbol
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'Z'))
}
