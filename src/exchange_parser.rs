use serde_json::Value;

#[derive(Debug, Clone)]
pub struct OrderbookDepth {
    pub bids: Vec<(String, String)>,
    pub asks: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct ExchangeData {
    pub exchange: String,
    pub ticker: String,
    pub funding_rate: Option<f64>,
    pub bid: Option<String>,
    pub ask: Option<String>,
    pub orderbook_depth: Option<OrderbookDepth>,
}

pub trait ExchangeParser {
    fn parse_ticker(&self, json: &Value) -> Option<String>;
    fn parse_funding_rate(&self, json: &Value) -> Option<f64>;
    fn parse_bid(&self, json: &Value) -> Option<String>;
    fn parse_ask(&self, json: &Value) -> Option<String>;
    fn parse_orderbook(&self, json: &Value) -> Option<OrderbookDepth>;
    
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
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        json.get("s").and_then(|v| v.as_str()).map(|s| s.to_string())
    }
    
    fn parse_funding_rate(&self, json: &Value) -> Option<f64> {
        json.get("r").and_then(|v| v.as_str()).and_then(|r| r.parse().ok())
    }
    
    fn parse_bid(&self, json: &Value) -> Option<String> {
        json.get("b").and_then(|v| v.as_str()).map(|s| s.to_string())
    }
    
    fn parse_ask(&self, json: &Value) -> Option<String> {
        json.get("a").and_then(|v| v.as_str()).map(|s| s.to_string())
    }
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
}

pub struct BybitParser;
impl ExchangeParser for BybitParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.get("symbol"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
}

pub struct OKXParser;
impl ExchangeParser for OKXParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("instId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
}

pub struct HyperliquidParser;
impl ExchangeParser for HyperliquidParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.get("coin"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
}

pub struct KucoinParser;
impl ExchangeParser for KucoinParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        // Try data.symbol first
        if let Some(symbol) = json.get("data")
            .and_then(|d| d.get("symbol"))
            .and_then(|v| v.as_str())
        {
            return Some(symbol.to_string());
        }
        
        // Fallback to old structure
        json.get("data")
            .and_then(|d| d.get("symbol"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
}

pub struct BitgetParser;
impl ExchangeParser for BitgetParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        json.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("instId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
    }
}

pub struct GateioParser;
impl ExchangeParser for GateioParser {
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        // Try tickers format first (contract field)
        if let Some(ticker) = json.get("result")
            .and_then(|r| r.get("contract"))
            .and_then(|v| v.as_str())
        {
            return Some(ticker.to_string());
        }
        
        // Fallback to book_ticker format (s field)
        json.get("result")
            .and_then(|r| r.get("s"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
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
    fn parse_ticker(&self, json: &Value) -> Option<String> {
        json.get("params")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.get("market"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    
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
    
    fn parse_orderbook(&self, _json: &Value) -> Option<OrderbookDepth> {
        None
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
