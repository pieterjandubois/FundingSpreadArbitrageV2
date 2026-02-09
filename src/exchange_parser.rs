use serde_json::Value;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OrderbookDepth {
    pub bids: Vec<(String, String)>,
    pub asks: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ExchangeData {
    pub exchange: String,
    pub ticker: String,
    pub funding_rate: Option<f64>,
    pub bid: Option<String>,
    pub ask: Option<String>,
    pub orderbook_depth: Option<OrderbookDepth>,
}



pub trait ExchangeParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64>;
    fn parse_bid(&self, json: &Value) -> Option<String>;
    fn parse_ask(&self, json: &Value) -> Option<String>;
    
    #[allow(dead_code)]
    fn parse_ticker(&self, _json: &Value) -> Option<String> {
        None
    }
    
    #[allow(dead_code)]
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
    
    #[allow(dead_code)]
    fn extract_all(&self, exchange: &str, json: &Value) -> Option<ExchangeData> {
        Some(ExchangeData {
            exchange: exchange.to_string(),
            ticker: self.parse_ticker(json)?,
            funding_rate: self.parse_funding_rate(json),
            bid: self.parse_bid(json),
            ask: self.parse_ask(json),
            orderbook_depth: self.parse_orderbook(json),
        })
    }
}

pub struct BinanceParser;
impl ExchangeParser for BinanceParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("r").and_then(|v| v.as_str()).and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("b").and_then(|v| v.as_str()).map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("a").and_then(|v| v.as_str()).map(|s| s.to_string())
    }
}

pub struct BybitParser;
impl ExchangeParser for BybitParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("data")
            .and_then(|d| {
                if d.is_array() {
                    d.as_array().and_then(|a| a.first())
                } else {
                    Some(d)
                }
            })
            .and_then(|f| f.get("fundingRate"))
            .and_then(|v| v.as_str())
            .and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.get("bid1Price"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.get("ask1Price"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub struct OKXParser;
impl ExchangeParser for OKXParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("fundingRate"))
            .and_then(|v| v.as_str())
            .and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("bidPx"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("askPx"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub struct HyperliquidParser;
impl ExchangeParser for HyperliquidParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("data")
            .and_then(|d| d.get("ctx"))
            .and_then(|c| c.get("funding"))
            .and_then(|v| v.as_str())
            .and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        // Try impactPxs from activeAssetCtx (array [bid, ask])
        if let Some(bid) = json.get("data")
            .and_then(|d| d.get("ctx"))
            .and_then(|c| c.get("impactPxs"))
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
        {
            return Some(bid.to_string());
        }
        
        // Try bbo structure (array of objects with px field)
        json.get("data")
            .and_then(|d| d.get("bbo"))
            .and_then(|b| b.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("px"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        // Try impactPxs from activeAssetCtx (array [bid, ask])
        if let Some(ask) = json.get("data")
            .and_then(|d| d.get("ctx"))
            .and_then(|c| c.get("impactPxs"))
            .and_then(|p| p.as_array())
            .and_then(|a| a.get(1))
            .and_then(|v| v.as_str())
        {
            return Some(ask.to_string());
        }
        
        // Try bbo structure (array of objects with px field)
        json.get("data")
            .and_then(|d| d.get("bbo"))
            .and_then(|b| b.as_array())
            .and_then(|a| a.get(1))
            .and_then(|f| f.get("px"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub struct KucoinParser;
impl ExchangeParser for KucoinParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("data")
            .and_then(|d| d.get("fundingRate"))
            .and_then(|v| {
                if v.is_f64() {
                    v.as_f64()
                } else {
                    v.as_str().and_then(|r| r.parse().ok())
                }
            })
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.get("bestBidPrice"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.get("bestAskPrice"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub struct BitgetParser;
impl ExchangeParser for BitgetParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("fundingRate"))
            .and_then(|v| v.as_str())
            .and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("bidPr"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("askPr"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub struct GateioParser;
impl ExchangeParser for GateioParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("result")
            .and_then(|r| r.get("funding_rate"))
            .and_then(|v| v.as_str())
            .and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        // Try tickers format first (highest_bid field)
        if let Some(bid) = json.get("result")
            .and_then(|r| r.get("highest_bid"))
            .and_then(|v| v.as_str())
        {
            return Some(bid.to_string());
        }
        
        // Fallback to book_ticker format (b field)
        json.get("result")
            .and_then(|r| r.get("b"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        // Try tickers format first (lowest_ask field)
        if let Some(ask) = json.get("result")
            .and_then(|r| r.get("lowest_ask"))
            .and_then(|v| v.as_str())
        {
            return Some(ask.to_string());
        }
        
        // Fallback to book_ticker format (a field)
        json.get("result")
            .and_then(|r| r.get("a"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

/* pub struct LighterParser;
impl ExchangeParser for LighterParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        // Extract market ID from channel field (e.g., "market_stats/0" -> "0")
        json.get("channel")
            .and_then(|c| c.as_str())
            .and_then(|s| s.split('/').nth(1))
            .map(|s| s.to_string())
    } */
    
    /* fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        // Lighter provides funding rate in market_stats data
        json.get("data")
            .and_then(|d| d.get("funding_rate"))
            .and_then(|v| {
                if v.is_f64() {
                    v.as_f64()
                } else {
                    v.as_str().and_then(|r| r.parse().ok())
                }
            })
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        // Lighter provides bid in order_book data
        json.get("data")
            .and_then(|d| d.get("bids"))
            .and_then(|b| b.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.as_array())
            .and_then(|p| p.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        // Lighter provides ask in order_book data
        json.get("data")
            .and_then(|d| d.get("asks"))
            .and_then(|a| a.as_array())
            .and_then(|arr| arr.first())
            .and_then(|f| f.as_array())
            .and_then(|p| p.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
} */

pub struct ParadexParser;
impl ExchangeParser for ParadexParser {
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("params")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.get("funding_rate"))
            .and_then(|v| v.as_str())
            .and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("params")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.get("bid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("params")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.get("ask"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub fn get_parser(exchange: &str) -> Box<dyn ExchangeParser> {
    match exchange {
        "binance" => Box::new(BinanceParser),
        "bybit" => Box::new(BybitParser),
        "okx" => Box::new(OKXParser),
        "hyperliquid" => Box::new(HyperliquidParser),
        "kucoin" => Box::new(KucoinParser),
        "bitget" => Box::new(BitgetParser),
        "gateio" => Box::new(GateioParser),
        /* "lighter" => Box::new(LighterParser), */
        "paradex" => Box::new(ParadexParser),
        _ => Box::new(BinanceParser),
    }
}

/// Get the Redis key patterns for a given exchange and symbol
/// Returns a vector of possible key patterns to try (in priority order)
pub fn get_redis_key_patterns(exchange: &str, symbol: &str) -> Vec<String> {
    match exchange.to_lowercase().as_str() {
        "bybit" => vec![format!("{}:linear:tickers:{}", exchange, symbol)],
        "bitget" => vec![format!("{}:usdt:tickers:{}", exchange, symbol)],
        "binance" => vec![
            format!("{}:linear:tickers:{}", exchange, symbol),
            format!("{}:usdm:book:{}", exchange, symbol),
        ],
        "okx" => {
            // OKX uses format: BASE-USDT-SWAP
            // Convert BTCUSDT -> BTC-USDT-SWAP
            let base = symbol.trim_end_matches("USDT");
            vec![
                format!("{}:usdt:tickers:{}-USDT-SWAP", exchange, base),
                format!("{}:usdt:tickers:{}", exchange, symbol),  // Fallback
            ]
        },
        "kucoin" => vec![
            format!("{}:futures:tickerV2:{}M", exchange, symbol),  // KuCoin adds M suffix
            format!("{}:futures:tickerV2:{}", exchange, symbol),   // Fallback without M
        ],
        "hyperliquid" => {
            // Hyperliquid uses format: hyperliquid:usdc:ctx:BASE (without USDT)
            // Convert BTCUSDT -> BTC
            let base = symbol.trim_end_matches("USDT");
            vec![
                format!("{}:usdc:ctx:{}", exchange, base),
                format!("{}:usdc:bbo:{}", exchange, base),  // Fallback to bbo
            ]
        },
        "gateio" => vec![format!("{}:usdt:tickers:{}", exchange, symbol)],
        "paradex" => {
            // Paradex uses format: BASE-USD-PERP
            // Convert BTCUSDT -> BTC-USD-PERP
            let base = symbol.trim_end_matches("USDT");
            vec![
                format!("{}:usdt:bbo:{}-USD-PERP", exchange, base),
                format!("{}:usdt:tickers:{}", exchange, symbol),  // Fallback
            ]
        },
        "lighter" => vec![format!("{}:usdt:data:{}", exchange, symbol)],
        _ => vec![format!("{}:linear:tickers:{}", exchange, symbol)],
    }
}

#[allow(dead_code)]
pub fn normalize_symbol(symbol: &str) -> String {
    let mut normalized = symbol.to_uppercase();
    
    // Remove dashes (OKX: LDO-USDT-SWAP, Paradex: CAKE-USD-PERP)
    normalized = normalized.replace("-", "");
    
    // Remove SWAP suffix (OKX)
    if normalized.ends_with("SWAP") {
        normalized.truncate(normalized.len() - 4);
    }
    
    // Remove PERP suffix (Paradex)
    if normalized.ends_with("PERP") {
        normalized.truncate(normalized.len() - 4);
    }
    
    // Remove M suffix for KuCoin (MASKUSDTM -> MASKUSDT, LDOUSDTM -> LDOUSDT)
    if normalized.ends_with("USDTM") {
        normalized.truncate(normalized.len() - 1);
    }
    
    // Normalize USD/USDT endings
    if normalized.contains("USD") && !normalized.contains("USDT") {
        normalized = normalized.replace("USD", "USDT");
    }
    
    // If it doesn't have USDT at all (like Hyperliquid's "LDO"), add it
    if !normalized.contains("USDT") {
        normalized.push_str("USDT");
    }
    
    normalized
}
