use std::error::Error;
use std::collections::HashMap;
use reqwest::Client;
use serde_json::Value;
use crate::strategy::types::{SimulatedOrder, OrderStatus, OrderSide};
use crate::strategy::testnet_config::ExchangeCredentials;
use crate::strategy::rate_limiter::RateLimiter;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use hex::encode;
use tokio::sync::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use once_cell::sync::Lazy;

type HmacSha256 = Hmac<Sha256>;

// Type alias for symbol precision: (quantity_precision, price_precision, min_price, price_tick_size)
type SymbolPrecision = (u32, u32, f64, f64);

// Binance DEMO trading environment (futures demo)
const BINANCE_DEMO_URL: &str = "https://testnet.binancefuture.com";

// Cache TTL for order book data (100ms as per spec)
const ORDER_BOOK_CACHE_TTL: Duration = Duration::from_millis(100);

// Global rate limiter for all Binance HTTP requests
// 20 requests/second with burst capacity of 30
// This prevents triggering Binance's IP-level rate limits that cause websocket disconnections
static BINANCE_RATE_LIMITER: Lazy<RateLimiter> = Lazy::new(|| {
    eprintln!("[BINANCE RATE LIMITER] Initialized: 20 req/s, burst capacity: 30");
    RateLimiter::new(20, 30)
});

/// Cached order book data with timestamp
#[derive(Clone, Debug)]
struct CachedOrderBook {
    depth: crate::strategy::types::OrderBookDepth,
    cached_at: Instant,
}

pub struct BinanceDemoClient {
    client: Client,
    api_key: String,
    api_secret: String,
    // Cache of symbol precision: symbol -> (quantity_precision, price_precision, min_price, price_tick_size)
    precision_cache: Arc<Mutex<HashMap<String, SymbolPrecision>>>,
    // Cache of order book data: (symbol, levels) -> CachedOrderBook
    order_book_cache: Arc<Mutex<HashMap<(String, usize), CachedOrderBook>>>,
    // Time offset in milliseconds (server_time - local_time)
    time_offset: Arc<Mutex<i64>>,
    // Last time we synchronized with server
    last_sync: Arc<Mutex<std::time::Instant>>,
}

impl BinanceDemoClient {
    pub fn new(credentials: ExchangeCredentials) -> Self {
        Self {
            client: Client::new(),
            api_key: credentials.api_key,
            api_secret: credentials.api_secret,
            precision_cache: Arc::new(Mutex::new(HashMap::new())),
            order_book_cache: Arc::new(Mutex::new(HashMap::new())),
            time_offset: Arc::new(Mutex::new(0)),
            last_sync: Arc::new(Mutex::new(std::time::Instant::now())),
        }
    }

    /// Helper method to make rate-limited HTTP GET request
    async fn rate_limited_get(&self, url: &str) -> Result<reqwest::Response, Box<dyn Error + Send + Sync>> {
        BINANCE_RATE_LIMITER.acquire().await;
        Ok(self.client.get(url).send().await?)
    }

    /// Helper method to make rate-limited HTTP POST request with API key
    async fn rate_limited_post(&self, url: &str) -> Result<reqwest::Response, Box<dyn Error + Send + Sync>> {
        BINANCE_RATE_LIMITER.acquire().await;
        Ok(self.client.post(url).header("X-MBX-APIKEY", &self.api_key).send().await?)
    }

    /// Helper method to make rate-limited HTTP DELETE request with API key
    async fn rate_limited_delete(&self, url: &str) -> Result<reqwest::Response, Box<dyn Error + Send + Sync>> {
        BINANCE_RATE_LIMITER.acquire().await;
        Ok(self.client.delete(url).header("X-MBX-APIKEY", &self.api_key).send().await?)
    }

    /// Helper method to make rate-limited HTTP GET request with API key
    async fn rate_limited_get_with_key(&self, url: &str) -> Result<reqwest::Response, Box<dyn Error + Send + Sync>> {
        BINANCE_RATE_LIMITER.acquire().await;
        Ok(self.client.get(url).header("X-MBX-APIKEY", &self.api_key).send().await?)
    }

    /// Synchronize with Binance server time
    /// Fetches server time and calculates offset to apply to all future requests
    pub async fn sync_server_time(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/fapi/v1/time", BINANCE_DEMO_URL);
        
        let local_time_before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        
        let response = self.rate_limited_get(&url).await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;
        
        let local_time_after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        
        let server_time = response_json
            .get("serverTime")
            .and_then(|v| v.as_i64())
            .ok_or("Failed to extract server time")?;
        
        // Use average of before/after for more accurate local time
        let local_time_avg = (local_time_before + local_time_after) / 2;
        let offset = server_time - local_time_avg;
        
        let mut time_offset = self.time_offset.lock().await;
        *time_offset = offset;
        
        let mut last_sync = self.last_sync.lock().await;
        *last_sync = std::time::Instant::now();
        
        eprintln!("[BINANCE DEMO] ⏰ Time synchronized | Offset: {}ms | Server: {} | Local: {}", 
            offset, server_time, local_time_avg);
        
        Ok(())
    }

    /// Get current timestamp adjusted for server time offset
    /// Auto-resyncs if more than 5 seconds have passed since last sync
    async fn get_timestamp(&self) -> Result<String, Box<dyn Error + Send + Sync>> {
        // Check if we need to resync (every 5 seconds to handle fast clock drift)
        {
            let last_sync = self.last_sync.lock().await;
            if last_sync.elapsed() > Duration::from_secs(5) {
                drop(last_sync);
                // Resync in background, don't wait for it
                if let Err(e) = self.sync_server_time().await {
                    eprintln!("[BINANCE DEMO] ⚠️  Auto-resync failed: {}", e);
                }
            }
        }
        
        let local_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        
        let offset = *self.time_offset.lock().await;
        let adjusted_time = local_time + offset;
        
        Ok(adjusted_time.to_string())
    }

    /// Get precision and tick size for a symbol
    pub async fn get_symbol_precision(&self, symbol: &str) -> Result<(u32, u32, f64, f64), Box<dyn Error + Send + Sync>> {
        // Check cache first
        let cache = self.precision_cache.lock().await;
        if let Some(&(qty_prec, price_prec, min_price, tick_size)) = cache.get(symbol) {
            return Ok((qty_prec, price_prec, min_price, tick_size));
        }
        drop(cache);

        // Fetch from API
        let url = format!("{}/fapi/v1/exchangeInfo", BINANCE_DEMO_URL);
        let response = self.rate_limited_get(&url).await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;

        if let Some(symbols) = response_json.get("symbols").and_then(|s| s.as_array()) {
            for sym in symbols {
                if let Some(sym_name) = sym.get("symbol").and_then(|s| s.as_str()) {
                    if sym_name == symbol {
                        // Get quantity and price precision
                        let qty_prec = sym.get("quantityPrecision")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(8) as u32;
                        let price_prec = sym.get("pricePrecision")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(8) as u32;

                        // Get price tick size from filters
                        let mut tick_size = 0.00000001;
                        let mut min_price = 0.00000001;
                        
                        if let Some(filters) = sym.get("filters").and_then(|f| f.as_array()) {
                            for filter in filters {
                                if let Some(filter_type) = filter.get("filterType").and_then(|t| t.as_str()) {
                                    if filter_type == "PRICE_FILTER" {
                                        if let Some(tick_str) = filter.get("tickSize").and_then(|t| t.as_str()) {
                                            tick_size = tick_str.parse::<f64>().unwrap_or(0.00000001);
                                        }
                                        if let Some(min_str) = filter.get("minPrice").and_then(|m| m.as_str()) {
                                            min_price = min_str.parse::<f64>().unwrap_or(0.00000001);
                                        }
                                    }
                                }
                            }
                        }

                        // Cache it
                        let mut cache = self.precision_cache.lock().await;
                        cache.insert(symbol.to_string(), (qty_prec, price_prec, min_price, tick_size));

                        return Ok((qty_prec, price_prec, min_price, tick_size));
                    }
                }
            }
        }

        Err(format!("Symbol {} not found in exchange info", symbol).into())
    }

    /// Round quantity to the correct precision
    fn round_quantity(quantity: f64, precision: u32) -> f64 {
        let multiplier = 10_f64.powi(precision as i32);
        (quantity * multiplier).floor() / multiplier
    }


    /// Round price to the nearest valid tick size and format cleanly
    fn round_price_to_tick(price: f64, tick_size: f64) -> String {
        if tick_size <= 0.0 {
            return format!("{}", price);
        }
        let rounded = (price / tick_size).round() * tick_size;
        
        // Determine decimal places from tick size
        let tick_str = format!("{:.10}", tick_size).trim_end_matches('0').to_string();
        let decimals = if let Some(dot_pos) = tick_str.find('.') {
            tick_str.len() - dot_pos - 1
        } else {
            0
        };
        
        format!("{:.prec$}", rounded, prec = decimals)
    }

    /// Generate signature for Binance API
    /// HMAC-SHA256 of the query string
    fn generate_signature(&self, query_string: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(query_string.as_bytes());
        encode(mac.finalize().into_bytes()).to_lowercase()
    }

    /// Set leverage for a symbol to 1x
    /// Must be called BEFORE placing any orders for the symbol
    pub async fn set_leverage(&self, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        // Build query string for signature
        let query_string = format!(
            "symbol={}&leverage=1&recvWindow=5000&timestamp={}",
            symbol, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/leverage?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        eprintln!("[BINANCE DEMO] Setting leverage to 1x for {}", symbol);

        let response = self.rate_limited_post(&url).await?;

        let response_text = response.text().await?;
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("[BINANCE DEMO] ⚠️  Failed to set leverage: {}", msg);
                // Don't fail - leverage might already be set
                return Ok(());
            }
        }

        eprintln!("[BINANCE DEMO] ✅ Leverage set to 1x for {}", symbol);
        Ok(())
    }

    /// Set margin type to ISOLATED for a symbol
    /// Must be called BEFORE placing any orders for the symbol
    pub async fn set_margin_type_isolated(&self, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        // Build query string for signature
        let query_string = format!(
            "symbol={}&marginType=ISOLATED&recvWindow=5000&timestamp={}",
            symbol, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/marginType?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        eprintln!("[BINANCE DEMO] Setting margin type to ISOLATED for {}", symbol);

        let response = self.rate_limited_post(&url).await?;

        let response_text = response.text().await?;
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                // Code -4046 means "No need to change margin type" - this is OK
                if code == -4046 {
                    eprintln!("[BINANCE DEMO] ℹ️  Margin type already ISOLATED for {}", symbol);
                    return Ok(());
                }
                eprintln!("[BINANCE DEMO] ⚠️  Failed to set margin type: {}", msg);
                // Don't fail - margin type might already be set
                return Ok(());
            }
        }

        eprintln!("[BINANCE DEMO] ✅ Margin type set to ISOLATED for {}", symbol);
        Ok(())
    }

    /// Check if a symbol is tradeable on Binance futures
    pub async fn is_symbol_tradeable(&self, symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/fapi/v1/exchangeInfo", BINANCE_DEMO_URL);
        
        let response = self.rate_limited_get(&url).await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;
        
        if let Some(symbols) = response_json.get("symbols").and_then(|s| s.as_array()) {
            for sym in symbols {
                if let Some(sym_name) = sym.get("symbol").and_then(|s| s.as_str()) {
                    if sym_name == symbol {
                        // Check if it's tradeable
                        let status = sym.get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("");
                        return Ok(status == "TRADING");
                    }
                }
            }
        }
        
        Ok(false)
    }

    /// Place a market order on Binance testnet (for immediate hedging)
    pub async fn place_market_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let side = match order.side {
            OrderSide::Long => "BUY",
            OrderSide::Short => "SELL",
        };

        // Get precision for this symbol
        let (qty_precision, _price_precision, _min_price, _tick_size) = self.get_symbol_precision(&order.symbol).await?;
        
        // Round quantity to correct precision
        let rounded_qty = Self::round_quantity(order.size, qty_precision);

        eprintln!("[BINANCE DEMO] Placing MARKET order: {} | Side: {} | Size: {}", 
            order.symbol, side, rounded_qty);

        // Build query string for signature (no price for market orders)
        let query_string = format!(
            "symbol={}&side={}&type=MARKET&quantity={}&recvWindow=5000&timestamp={}",
            order.symbol, side, rounded_qty, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/order?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        let response = self.rate_limited_post(&url).await?;

        let response_text = response.text().await?;
        eprintln!("[BINANCE DEMO] MARKET order response: {}", response_text);
        
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Binance API error: {}", msg).into());
            }
        }

        // Extract order ID and fill price from response
        let order_id = response_json
            .get("orderId")
            .and_then(|v| v.as_i64())
            .ok_or("Failed to extract order ID from response")?
            .to_string();

        // For market orders, avgPrice might be "0.00" initially
        // We need to query the order status to get the actual fill price
        let fill_price = response_json
            .get("avgPrice")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|&p| p > 0.0)
            .unwrap_or(order.price); // Use order price as fallback

        // Wait a moment and check order status to get actual fill price
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        
        let actual_fill_price = match self.get_order_status(&order_id, &order.symbol).await {
            Ok(OrderStatus::Filled) => {
                // Query the order details to get avgPrice
                let status_timestamp = self.get_timestamp().await?;

                let status_query = format!(
                    "symbol={}&orderId={}&recvWindow=5000&timestamp={}",
                    order.symbol, order_id, status_timestamp
                );

                let status_signature = self.generate_signature(&status_query);

                let status_url = format!(
                    "{}/fapi/v1/order?{}&signature={}",
                    BINANCE_DEMO_URL, status_query, status_signature
                );

                if let Ok(status_response) = self.rate_limited_get_with_key(&status_url).await {
                    if let Ok(status_text) = status_response.text().await {
                        if let Ok(status_json) = serde_json::from_str::<Value>(&status_text) {
                            status_json
                                .get("avgPrice")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .filter(|&p| p > 0.0)
                                .unwrap_or(fill_price)
                        } else {
                            fill_price
                        }
                    } else {
                        fill_price
                    }
                } else {
                    fill_price
                }
            }
            _ => fill_price,
        };

        let mut filled_order = order;
        filled_order.id = order_id;
        filled_order.status = OrderStatus::Filled; // Market orders fill immediately
        filled_order.created_at = timestamp.parse().unwrap_or(0);
        filled_order.filled_at = Some(timestamp.parse().unwrap_or(0));
        filled_order.size = rounded_qty;
        filled_order.price = actual_fill_price;
        filled_order.fill_price = Some(actual_fill_price);

        eprintln!("[BINANCE DEMO] ✅ MARKET order filled: {} | Symbol: {} | Side: {} | Size: {} | Fill Price: {}", 
            filled_order.id, filled_order.symbol, side, filled_order.size, actual_fill_price);

        Ok(filled_order)
    }

    /// Place an order on Binance testnet
    pub async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let side = match order.side {
            OrderSide::Long => "BUY",
            OrderSide::Short => "SELL",
        };

        // Get precision and tick size for this symbol
        let (qty_precision, _price_precision, _min_price, tick_size) = self.get_symbol_precision(&order.symbol).await?;
        
        // Round quantity to correct precision
        let rounded_qty = Self::round_quantity(order.size, qty_precision);
        
        // Round price to nearest tick size (returns formatted string)
        let rounded_price_str = Self::round_price_to_tick(order.price, tick_size);
        let rounded_price_f64 = rounded_price_str.parse::<f64>().unwrap_or(order.price);

        eprintln!("[BINANCE DEMO] Rounding {} qty: {} -> {} (precision: {})", 
            order.symbol, order.size, rounded_qty, qty_precision);
        eprintln!("[BINANCE DEMO] Rounding {} price: {} -> {} (tick: {})", 
            order.symbol, order.price, rounded_price_str, tick_size);

        // Build query string for signature
        let query_string = format!(
            "symbol={}&side={}&type=LIMIT&timeInForce=GTC&quantity={}&price={}&leverage=1&recvWindow=5000&timestamp={}",
            order.symbol, side, rounded_qty, rounded_price_str, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/order?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        let response = self.rate_limited_post(&url).await?;

        let response_text = response.text().await?;
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Binance API error: {}", msg).into());
            }
        }

        // Extract order ID from response
        let order_id = response_json
            .get("orderId")
            .and_then(|v| v.as_i64())
            .ok_or("Failed to extract order ID from response")?
            .to_string();

        let mut filled_order = order;
        filled_order.id = order_id;
        filled_order.status = OrderStatus::Pending;
        filled_order.created_at = timestamp.parse().unwrap_or(0);
        filled_order.size = rounded_qty;
        filled_order.price = rounded_price_f64;

        eprintln!("[BINANCE DEMO] Order placed: {} | Symbol: {} | Side: {} | Size: {} | Price: {}", 
            filled_order.id, filled_order.symbol, side, filled_order.size, filled_order.price);

        Ok(filled_order)
    }

    /// Get order status from Binance testnet
    pub async fn get_order_status(&self, order_id: &str, symbol: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let query_string = format!(
            "symbol={}&orderId={}&recvWindow=5000&timestamp={}",
            symbol, order_id, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/order?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        eprintln!("[BINANCE DEMO] Checking order status: {} | Symbol: {} | URL: {}", order_id, symbol, url);

        let response = self.rate_limited_get_with_key(&url).await?;

        let response_text = response.text().await?;
        eprintln!("[BINANCE DEMO] Status response: {}", response_text);
        
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for API errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("[BINANCE DEMO] ⚠️  API Error checking status: {}", msg);
                return Ok(OrderStatus::Pending); // Assume pending on error
            }
        }

        let status_str = response_json.get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");

        eprintln!("[BINANCE DEMO] Order {} status: {}", order_id, status_str);

        let status = match status_str {
            "FILLED" => OrderStatus::Filled,
            "CANCELED" | "CANCELLED" => OrderStatus::Cancelled,
            "EXPIRED" => OrderStatus::Cancelled,
            _ => OrderStatus::Pending,
        };

        Ok(status)
    }

    /// Get detailed order status including filled quantity from Binance testnet
    pub async fn get_order_status_detailed(&self, order_id: &str, symbol: &str) -> Result<crate::strategy::types::OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        use crate::strategy::types::OrderStatusInfo;
        
        let timestamp = self.get_timestamp().await?;

        let query_string = format!(
            "symbol={}&orderId={}&recvWindow=5000&timestamp={}",
            symbol, order_id, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/order?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        let response = self.rate_limited_get_with_key(&url).await?;

        let response_text = response.text().await?;
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for API errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Binance API error: {}", msg).into());
            }
        }

        let status_str = response_json.get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");

        let status = match status_str {
            "FILLED" => OrderStatus::Filled,
            "CANCELED" | "CANCELLED" => OrderStatus::Cancelled,
            "EXPIRED" => OrderStatus::Cancelled,
            _ => OrderStatus::Pending,
        };

        // Extract filled and total quantities
        let executed_qty = response_json.get("executedQty")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let orig_qty = response_json.get("origQty")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        eprintln!("[BINANCE DEMO] Order {} | Status: {} | Filled: {}/{}", 
            order_id, status_str, executed_qty, orig_qty);

        Ok(OrderStatusInfo::new(status, executed_qty, orig_qty))
    }

    /// Cancel an order on Binance testnet
    pub async fn cancel_order(&self, order_id: &str, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let query_string = format!(
            "symbol={}&orderId={}&recvWindow=5000&timestamp={}",
            symbol, order_id, timestamp
        );

        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v1/order?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        let response = self.rate_limited_delete(&url).await?;

        let response_json: Value = serde_json::from_str(&response.text().await?)?;

        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Failed to cancel order: {}", msg).into());
            }
        }

        eprintln!("[BINANCE DEMO] Order cancelled: {}", order_id);
        Ok(())
    }

    /// Get account balance from Binance testnet
    pub async fn get_balance(&self) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let query_string = format!("recvWindow=5000&timestamp={}", timestamp);
        let signature = self.generate_signature(&query_string);

        let url = format!(
            "{}/fapi/v2/account?{}&signature={}",
            BINANCE_DEMO_URL, query_string, signature
        );

        let response = self.rate_limited_get_with_key(&url).await?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status, response_text).into());
        }

        if response_text.is_empty() {
            return Err("Empty response from API".into());
        }

        let response_json: Value = serde_json::from_str(&response_text)?;

        // Extract USDT available balance (not wallet balance - that includes locked funds)
        let balance = response_json
            .get("assets")
            .and_then(|a| a.as_array())
            .and_then(|arr| {
                arr.iter().find(|asset| {
                    asset.get("asset")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "USDT")
                        .unwrap_or(false)
                })
            })
            .and_then(|usdt| usdt.get("availableBalance"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok());

        match balance {
            Some(bal) => Ok(bal),
            None => {
                // Debug: Print what we got if parsing failed
                eprintln!("[BINANCE DEMO] Failed to parse balance. Response structure:");
                eprintln!("{}", serde_json::to_string_pretty(&response_json).unwrap_or_else(|_| response_text.clone()));
                Err("Failed to extract USDT available balance".into())
            }
        }
    }

    /// Get order book depth from Binance testnet
    /// Returns the top N levels of bids and asks
    /// Uses 100ms cache to reduce API calls
    pub async fn get_order_book_depth(&self, symbol: &str, levels: usize) -> Result<crate::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        use crate::strategy::types::{OrderBookDepth, PriceLevel};

        // Check cache first
        let cache_key = (symbol.to_string(), levels);
        {
            let cache = self.order_book_cache.lock().await;
            if let Some(cached) = cache.get(&cache_key) {
                if cached.cached_at.elapsed() < ORDER_BOOK_CACHE_TTL {
                    eprintln!("[BINANCE DEMO] 📦 Cache HIT for {} order book (age: {:?})", 
                        symbol, cached.cached_at.elapsed());
                    return Ok(cached.depth.clone());
                } else {
                    eprintln!("[BINANCE DEMO] 📦 Cache EXPIRED for {} order book (age: {:?})", 
                        symbol, cached.cached_at.elapsed());
                }
            }
        }

        eprintln!("[BINANCE DEMO] 📦 Cache MISS for {} order book, fetching from API", symbol);

        // Binance API supports limit parameter: 5, 10, 20, 50, 100, 500, 1000
        // Choose the smallest valid limit that satisfies the requested levels
        let limit = if levels <= 5 {
            5
        } else if levels <= 10 {
            10
        } else if levels <= 20 {
            20
        } else if levels <= 50 {
            50
        } else if levels <= 100 {
            100
        } else if levels <= 500 {
            500
        } else {
            1000
        };

        let url = format!(
            "{}/fapi/v1/depth?symbol={}&limit={}",
            BINANCE_DEMO_URL, symbol, limit
        );

        eprintln!("[BINANCE DEMO] Fetching order book depth for {} (limit: {})", symbol, limit);

        let response = self.client.get(&url).send().await?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status, response_text).into());
        }

        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for API errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Binance API error: {}", msg).into());
            }
        }

        // Parse bids and asks
        let bids = response_json
            .get("bids")
            .and_then(|v| v.as_array())
            .ok_or("Missing bids in response")?
            .iter()
            .take(levels)
            .filter_map(|level| {
                let arr = level.as_array()?;
                let price = arr.first()?.as_str()?.parse::<f64>().ok()?;
                let quantity = arr.get(1)?.as_str()?.parse::<f64>().ok()?;
                Some(PriceLevel { price, quantity })
            })
            .collect::<Vec<_>>();

        let asks = response_json
            .get("asks")
            .and_then(|v| v.as_array())
            .ok_or("Missing asks in response")?
            .iter()
            .take(levels)
            .filter_map(|level| {
                let arr = level.as_array()?;
                let price = arr.first()?.as_str()?.parse::<f64>().ok()?;
                let quantity = arr.get(1)?.as_str()?.parse::<f64>().ok()?;
                Some(PriceLevel { price, quantity })
            })
            .collect::<Vec<_>>();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        let depth = OrderBookDepth {
            bids,
            asks,
            timestamp,
        };

        eprintln!("[BINANCE DEMO] ✅ Order book depth fetched: {} bids, {} asks", depth.bids.len(), depth.asks.len());

        // Update cache
        {
            let mut cache = self.order_book_cache.lock().await;
            cache.insert(cache_key, CachedOrderBook {
                depth: depth.clone(),
                cached_at: Instant::now(),
            });
        }

        Ok(depth)
    }

    /// Get the best bid price from Binance testnet
    /// Returns the highest bid price from the order book
    /// Uses cached order book data if available (100ms TTL)
    pub async fn get_best_bid(&self, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // Try to use cached order book data first
        let depth = self.get_order_book_depth(symbol, 5).await?;
        
        let best_bid = depth.bids
            .first()
            .map(|level| level.price)
            .ok_or("No bids available in order book")?;

        Ok(best_bid)
    }

    /// Get the best ask price from Binance testnet
    /// Returns the lowest ask price from the order book
    /// Uses cached order book data if available (100ms TTL)
    pub async fn get_best_ask(&self, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // Try to use cached order book data first
        let depth = self.get_order_book_depth(symbol, 5).await?;
        
        let best_ask = depth.asks
            .first()
            .map(|level| level.price)
            .ok_or("No asks available in order book")?;

        Ok(best_ask)
    }
}
