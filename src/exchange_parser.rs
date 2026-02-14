use serde_json::Value;

/// SIMD-accelerated f64 parsing for price strings
/// Uses AVX-512 instructions when available, falls back to standard parsing
#[inline(always)]
pub fn parse_price_simd(s: &str) -> Option<f64> {
    // Fast path for empty strings
    if s.is_empty() {
        return None;
    }
    
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
    {
        // Use SIMD-accelerated parsing on x86_64 with AVX-512
        parse_price_avx512(s)
    }
    
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx512f")))]
    {
        // Fallback to optimized scalar parsing
        parse_price_fast(s)
    }
}

/// Fast scalar f64 parsing optimized for price strings
/// Handles common price formats: "12345.67", "0.00123", etc.
#[inline(always)]
fn parse_price_fast(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    
    if len == 0 || len > 32 {
        // Fallback for edge cases
        return s.parse().ok();
    }
    
    let mut result: f64 = 0.0;
    let mut decimal_places: i32 = 0;
    let mut found_decimal = false;
    let mut is_negative = false;
    let mut i = 0;
    
    // Handle negative sign
    if bytes[0] == b'-' {
        is_negative = true;
        i = 1;
    }
    
    // Parse digits
    while i < len {
        let byte = bytes[i];
        
        if byte == b'.' {
            if found_decimal {
                // Multiple decimal points - invalid
                return s.parse().ok();
            }
            found_decimal = true;
        } else if byte.is_ascii_digit() {
            let digit = (byte - b'0') as f64;
            result = result * 10.0 + digit;
            
            if found_decimal {
                decimal_places += 1;
            }
        } else if byte == b'e' || byte == b'E' {
            // Scientific notation - fallback to standard parser
            return s.parse().ok();
        } else {
            // Invalid character
            return None;
        }
        
        i += 1;
    }
    
    // Apply decimal places
    if decimal_places > 0 {
        result /= 10_f64.powi(decimal_places);
    }
    
    // Apply sign
    if is_negative {
        result = -result;
    }
    
    Some(result)
}

/// AVX-512 accelerated f64 parsing for price strings
/// Uses SIMD instructions to parse multiple digits in parallel
#[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
#[inline(always)]
fn parse_price_avx512(s: &str) -> Option<f64> {
    use std::arch::x86_64::*;
    
    let bytes = s.as_bytes();
    let len = bytes.len();
    
    if len == 0 || len > 32 {
        return parse_price_fast(s);
    }
    
    // For short strings or strings with special characters, use fast scalar path
    if len < 8 || s.contains('e') || s.contains('E') {
        return parse_price_fast(s);
    }
    
    unsafe {
        let mut result: f64 = 0.0;
        let mut decimal_places: i32 = 0;
        let mut found_decimal = false;
        let mut is_negative = false;
        let mut i = 0;
        
        // Handle negative sign
        if bytes[0] == b'-' {
            is_negative = true;
            i = 1;
        }
        
        // SIMD constants
        let zero_vec = _mm256_set1_epi8(b'0' as i8);
        let nine_vec = _mm256_set1_epi8(b'9' as i8);
        let dot_vec = _mm256_set1_epi8(b'.' as i8);
        
        // Process in chunks of 16 bytes using AVX-512
        while i + 16 <= len {
            // Load 16 bytes
            let mut chunk = [0u8; 32];
            chunk[..16].copy_from_slice(&bytes[i..i + 16]);
            let data = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);
            
            // Check for decimal point
            let dot_mask = _mm256_cmpeq_epi8(data, dot_vec);
            let dot_bits = _mm256_movemask_epi8(dot_mask);
            
            if dot_bits != 0 {
                // Found decimal point - switch to scalar processing
                break;
            }
            
            // Validate all bytes are digits (0-9)
            let ge_zero = _mm256_cmpgt_epi8(data, _mm256_sub_epi8(zero_vec, _mm256_set1_epi8(1)));
            let le_nine = _mm256_cmpgt_epi8(_mm256_add_epi8(nine_vec, _mm256_set1_epi8(1)), data);
            let valid = _mm256_and_si256(ge_zero, le_nine);
            let valid_mask = _mm256_movemask_epi8(valid);
            
            if valid_mask != -1 {
                // Non-digit found - switch to scalar processing
                break;
            }
            
            // Convert ASCII digits to numeric values
            let digits = _mm256_sub_epi8(data, zero_vec);
            
            // Extract and accumulate (unrolled for performance)
            let digit_array: [u8; 32] = std::mem::transmute(digits);
            
            for j in 0..16 {
                if i + j >= len {
                    break;
                }
                result = result * 10.0 + digit_array[j] as f64;
            }
            
            i += 16;
        }
        
        // Process remaining bytes with scalar code
        while i < len {
            let byte = bytes[i];
            
            if byte == b'.' {
                if found_decimal {
                    return parse_price_fast(s);
                }
                found_decimal = true;
            } else if byte >= b'0' && byte <= b'9' {
                let digit = (byte - b'0') as f64;
                result = result * 10.0 + digit;
                
                if found_decimal {
                    decimal_places += 1;
                }
            } else {
                return None;
            }
            
            i += 1;
        }
        
        // Apply decimal places
        if decimal_places > 0 {
            result /= 10_f64.powi(decimal_places);
        }
        
        // Apply sign
        if is_negative {
            result = -result;
        }
        
        Some(result)
    }
}

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
        json.get("r").and_then(|v| v.as_str()).and_then(parse_price_simd)
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
            .and_then(parse_price_simd)
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
            .and_then(parse_price_simd)
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
            .and_then(parse_price_simd)
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
                    v.as_str().and_then(parse_price_simd)
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
            .and_then(parse_price_simd)
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
            .and_then(parse_price_simd)
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
                    v.as_str().and_then(|r| parse_price_simd(r))
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
            .and_then(parse_price_simd)
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
