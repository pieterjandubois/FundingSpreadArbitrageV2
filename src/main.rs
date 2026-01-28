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

use tokio::sync::mpsc;
use tokio::time;

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
                    for (k, v) in buffer.drain(..) {
                        pipe.cmd("SET").arg(k).arg(v).ignore();
                    }
                    let _: () = pipe.query_async(&mut conn).await?;
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(item) => {
                        buffer.push(item);
                        if buffer.len() >= REDIS_FLUSH_MAX_ITEMS {
                            let mut pipe = redis::pipe();
                            for (k, v) in buffer.drain(..) {
                                pipe.cmd("SET").arg(k).arg(v).ignore();
                            }
                            let _: () = pipe.query_async(&mut conn).await?;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let (tx, rx) = mpsc::channel::<(String, String)>(32_768);

    let _redis_handle = tokio::spawn(async move { redis_writer(rx).await });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = binance::BinanceUsdmConnector::run(&client_clone, tx_clone).await;
    });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = bybit::BybitLinearConnector::run(&client_clone, tx_clone).await;
    });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = bitget::BitgetUsdtFuturesConnector::run(&client_clone, tx_clone).await;
    });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = kucoin::KucoinFuturesConnector::run(&client_clone, tx_clone).await;
    });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = okx::OkxUsdtSwapConnector::run(&client_clone, tx_clone).await;
    });

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = hyperliquid::HyperliquidPerpsConnector::run(&client_clone, tx_clone).await;
    });

    /* let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = lighter::LighterPerpsConnector::run(&client_clone, tx_clone).await;
    }); */

    let client_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let _ = paradex::ParadexPerpsConnector::run(&client_clone, tx_clone).await;
    });

    drop(tx);

    // Keep main thread alive indefinitely
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
