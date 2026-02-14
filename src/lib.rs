#![allow(internal_features)]

use std::error::Error;

pub type DynError = Box<dyn Error + Send + Sync>;

pub mod exchange_parser;
pub mod strategy;

// Export exchange connectors for use in binaries
pub mod binance;
pub mod bitget;
pub mod bybit;
pub mod kucoin;
pub mod okx;
pub mod hyperliquid;
pub mod paradex;
pub mod gateio;
pub mod utils;
