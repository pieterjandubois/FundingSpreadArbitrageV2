use std::error::Error;
use std::sync::Arc;
use std::collections::HashMap;
use reqwest::Client;
use serde_json::{json, Value};
use crate::strategy::types::{SimulatedOrder, OrderStatus, OrderSide};
use crate::strategy::testnet_config::ExchangeCredentials;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use hex::encode;
use tokio::sync::Mutex;
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

const BYBIT_DEMO_URL: &str = "https://api-demo.bybit.com";

// Cache TTL for order book data (100ms as per spec)
const ORDER_BOOK_CACHE_TTL: Duration = Duration::from_millis(100);

/// Cached order book data with timestamp
#[derive(Clone, Debug)]
struct CachedOrderBook {
    depth: crate::strategy::types::OrderBookDepth,
    cached_at: Instant,
}

pub struct BybitDemoClient {
    client: Client,
    api_key: String,
    api_secret: String,
    // Cache of symbol precision: symbol -> qtyStep
    qty_step_cache: Arc<Mutex<HashMap<String, f64>>>,
    // Cache of order book data: (symbol, levels) -> CachedOrderBook
    order_book_cache: Arc<Mutex<HashMap<(String, usize), CachedOrderBook>>>,
    // Time offset in milliseconds (server_time - local_time)
    time_offset: Arc<Mutex<i64>>,
    // Last time we synchronized with server
    last_sync: Arc<Mutex<std::time::Instant>>,
}

impl BybitDemoClient {
    pub fn new(credentials: ExchangeCredentials) -> Self {
        Self {
            client: Client::new(),
            api_key: credentials.api_key,
            api_secret: credentials.api_secret,
            qty_step_cache: Arc::new(Mutex::new(HashMap::new())),
            order_book_cache: Arc::new(Mutex::new(HashMap::new())),
            time_offset: Arc::new(Mutex::new(0)),
            last_sync: Arc::new(Mutex::new(std::time::Instant::now())),
        }
    }

    /// Synchronize with Bybit server time
    /// Fetches server time and calculates offset to apply to all future requests
    pub async fn sync_server_time(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v5/market/time", BYBIT_DEMO_URL);
        
        let local_time_before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        
        let response = self.client.get(&url).send().await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;
        
        let local_time_after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        
        let server_time = response_json
            .get("result")
            .and_then(|r| r.get("timeNano"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .map(|nano| nano / 1_000_000) // Convert nanoseconds to milliseconds
            .or_else(|| {
                // Fallback to timeSecond if timeNano not available
                response_json
                    .get("result")
                    .and_then(|r| r.get("timeSecond"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|sec| sec * 1000)
            })
            .ok_or("Failed to extract server time")?;
        
        // Use average of before/after for more accurate local time
        let local_time_avg = (local_time_before + local_time_after) / 2;
        let offset = server_time - local_time_avg;
        
        let mut time_offset = self.time_offset.lock().await;
        *time_offset = offset;
        
        let mut last_sync = self.last_sync.lock().await;
        *last_sync = std::time::Instant::now();
        
        eprintln!("[BYBIT DEMO] ⏰ Time synchronized | Offset: {}ms | Server: {} | Local: {}", 
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
                    eprintln!("[BYBIT DEMO] ⚠️  Auto-resync failed: {}", e);
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

    /// Get the quantity step (precision) for a symbol, with caching
    /// Returns the qtyStep value that quantities must be rounded to
    pub async fn get_qty_step(&self, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // Check cache first
        {
            let cache = self.qty_step_cache.lock().await;
            if let Some(&qty_step) = cache.get(symbol) {
                return Ok(qty_step);
            }
        }

        // Fetch from API if not cached
        let url = format!("{}/v5/market/instruments-info?category=linear&symbol={}", BYBIT_DEMO_URL, symbol);
        
        let response = self.client.get(&url).send().await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;
        
        // Extract qtyStep from response
        let qty_step = response_json
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("lotSizeFilter"))
            .and_then(|lsf| lsf.get("qtyStep"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or("Failed to extract qtyStep from response")?;

        // Cache the result
        {
            let mut cache = self.qty_step_cache.lock().await;
            cache.insert(symbol.to_string(), qty_step);
        }

        eprintln!("[BYBIT DEMO] Cached qtyStep for {}: {}", symbol, qty_step);
        Ok(qty_step)
    }

    /// Generate signature for Bybit API v5 POST requests
    /// For POST: timestamp + API key + recv_window + jsonBodyString
    fn generate_post_signature(&self, timestamp: &str, recv_window: &str, body: &str) -> String {
        let message = format!("{}{}{}{}", timestamp, self.api_key, recv_window, body);
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        encode(mac.finalize().into_bytes()).to_lowercase()
    }

    /// Generate signature for Bybit API v5 GET requests
    /// For GET: timestamp + API key + recv_window + queryString
    fn generate_get_signature(&self, timestamp: &str, recv_window: &str, query_string: &str) -> String {
        let message = format!("{}{}{}{}", timestamp, self.api_key, recv_window, query_string);
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        encode(mac.finalize().into_bytes()).to_lowercase()
    }

    /// Set leverage for a symbol to 1x
    /// Must be called BEFORE placing any orders for the symbol
    pub async fn set_leverage(&self, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let request_body = json!({
            "category": "linear",
            "symbol": symbol,
            "buyLeverage": "1",
            "sellLeverage": "1",
        });

        let body_str = request_body.to_string();
        let signature = self.generate_post_signature(&timestamp, recv_window, &body_str);

        let url = format!("{}/v5/position/set-leverage", BYBIT_DEMO_URL);

        eprintln!("[BYBIT DEMO] Setting leverage to 1x for {}", symbol);

        let response = self
            .client
            .post(&url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let _status_code = response.status();
        let response_text = response.text().await?;
        
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(ret_code) = response_json.get("retCode").and_then(|v| v.as_i64()) {
            if ret_code != 0 {
                let ret_msg = response_json
                    .get("retMsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("[BYBIT DEMO] ⚠️  Failed to set leverage (code {}): {}", ret_code, ret_msg);
                // Don't fail - leverage might already be set or not changeable
                return Ok(());
            }
        }

        eprintln!("[BYBIT DEMO] ✅ Leverage set to 1x for {}", symbol);
        Ok(())
    }

    /// Check if a symbol is tradeable on Bybit linear futures
    pub async fn is_symbol_tradeable(&self, symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v5/market/instruments-info?category=linear", BYBIT_DEMO_URL);
        
        let response = self.client.get(&url).send().await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;
        
        if let Some(result) = response_json.get("result") {
            if let Some(list) = result.get("list").and_then(|l| l.as_array()) {
                for instrument in list {
                    if let Some(instrument_symbol) = instrument.get("symbol").and_then(|s| s.as_str()) {
                        if instrument_symbol == symbol {
                            // Check if it's tradeable
                            let status = instrument.get("status")
                                .and_then(|s| s.as_str())
                                .unwrap_or("");
                            return Ok(status == "Trading");
                        }
                    }
                }
            }
        }
        
        Ok(false)
    }

    /// Place a market order on Bybit demo (for immediate hedging)
    pub async fn place_market_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let side = match order.side {
            OrderSide::Long => "Buy",
            OrderSide::Short => "Sell",
        };

        // Get the correct quantity step for this symbol
        let qty_step = self.get_qty_step(&order.symbol).await?;
        
        // Round quantity to the correct precision using qtyStep
        let rounded_qty = (order.size / qty_step).round() * qty_step;
        let qty_str = format!("{:.8}", rounded_qty)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();

        let request_body = json!({
            "category": "linear",
            "symbol": order.symbol.clone(),
            "side": side,
            "orderType": "Market",
            "qty": qty_str,
            "timeInForce": "IOC",  // Immediate-Or-Cancel for market orders
        });

        let body_str = request_body.to_string();
        let signature = self.generate_post_signature(&timestamp, recv_window, &body_str);

        let url = format!("{}/v5/order/create", BYBIT_DEMO_URL);

        eprintln!("[BYBIT DEMO] Placing MARKET order: {} | Side: {} | Size: {}", 
            order.symbol, side, qty_str);

        let response = self
            .client
            .post(&url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status_code = response.status();
        let response_text = response.text().await?;
        
        eprintln!("[BYBIT DEMO] MARKET order response status: {} | Body: {}", status_code, response_text);
        
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(ret_code) = response_json.get("retCode").and_then(|v| v.as_i64()) {
            if ret_code != 0 {
                let ret_msg = response_json
                    .get("retMsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("[BYBIT DEMO] ❌ MARKET order API Error (code {}): {}", ret_code, ret_msg);
                return Err(format!("Bybit API error ({}): {}", ret_code, ret_msg).into());
            }
        }

        // Extract order ID from response
        let order_id = response_json
            .get("result")
            .and_then(|r| r.get("orderId"))
            .and_then(|v| v.as_str())
            .ok_or("Failed to extract order ID from response")?
            .to_string();

        let mut filled_order = order;
        filled_order.id = order_id.clone();
        filled_order.status = OrderStatus::Filled; // Market orders fill immediately
        filled_order.created_at = timestamp.parse().unwrap_or(0);
        filled_order.filled_at = Some(timestamp.parse().unwrap_or(0));
        filled_order.fill_price = Some(filled_order.price); // Use order price as estimate

        eprintln!("[BYBIT DEMO] ✅ MARKET order filled: {} | Symbol: {} | Side: {} | Size: {}", 
            order_id, filled_order.symbol, side, qty_str);

        Ok(filled_order)
    }

    /// Place an order on Bybit demo
    pub async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let side = match order.side {
            OrderSide::Long => "Buy",
            OrderSide::Short => "Sell",
        };

        // Get the correct quantity step for this symbol
        let qty_step = self.get_qty_step(&order.symbol).await?;
        
        // Round quantity to the correct precision using qtyStep
        let rounded_qty = (order.size / qty_step).round() * qty_step;
        let qty_str = format!("{:.8}", rounded_qty)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
        
        // Round price to 8 decimal places and trim trailing zeros
        let rounded_price = (order.price * 100000000.0).round() / 100000000.0;
        let price_str = format!("{:.8}", rounded_price)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();

        let request_body = json!({
            "category": "linear",
            "symbol": order.symbol.clone(),
            "side": side,
            "orderType": "Limit",
            "qty": qty_str,
            "price": price_str,
            "timeInForce": "GTC",
            "leverage": "1",
        });

        let body_str = request_body.to_string();
        let signature = self.generate_post_signature(&timestamp, recv_window, &body_str);

        let url = format!("{}/v5/order/create", BYBIT_DEMO_URL);

        eprintln!("[BYBIT DEMO] Placing order: {} | Side: {} | Size: {} (rounded from {} with qtyStep {}) | Price: {} (rounded from {}) | URL: {}", 
            order.symbol, side, qty_str, order.size, qty_step, price_str, order.price, url);

        let response = self
            .client
            .post(&url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status_code = response.status();
        let response_text = response.text().await?;
        
        eprintln!("[BYBIT DEMO] Response status: {} | Body: {}", status_code, response_text);
        
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(ret_code) = response_json.get("retCode").and_then(|v| v.as_i64()) {
            if ret_code != 0 {
                let ret_msg = response_json
                    .get("retMsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("[BYBIT DEMO] ❌ API Error (code {}): {}", ret_code, ret_msg);
                return Err(format!("Bybit API error ({}): {}", ret_code, ret_msg).into());
            }
        }

        // Extract order ID from response
        let order_id = response_json
            .get("result")
            .and_then(|r| r.get("orderId"))
            .and_then(|v| v.as_str())
            .ok_or("Failed to extract order ID from response")?
            .to_string();

        let mut filled_order = order;
        filled_order.id = order_id.clone();
        filled_order.status = OrderStatus::Pending;
        filled_order.created_at = timestamp.parse().unwrap_or(0);

        eprintln!("[BYBIT DEMO] ✅ Order placed successfully: {} | Symbol: {} | Side: {} | Size: {} | Price: {}", 
            order_id, filled_order.symbol, side, qty_str, price_str);

        Ok(filled_order)
    }

    /// Get order status from Bybit demo
    pub async fn get_order_status(&self, order_id: &str, symbol: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let query_string = format!("category=linear&symbol={}&orderId={}", symbol, order_id);
        let signature = self.generate_get_signature(&timestamp, recv_window, &query_string);

        let url = format!("{}/v5/order/realtime?{}", BYBIT_DEMO_URL, query_string);

        eprintln!("[BYBIT DEMO] Checking order status: {} | Symbol: {} | URL: {}", order_id, symbol, url);

        let response = self
            .client
            .get(&url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .send()
            .await?;

        let _status_code = response.status();
        let response_text = response.text().await?;
        
        eprintln!("[BYBIT DEMO] Status response code: {} | Body: {}", _status_code, response_text);
        
        let response_json: Value = serde_json::from_str(&response_text)?;

        let status_str = response_json
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("orderStatus"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        eprintln!("[BYBIT DEMO] Order {} status: {}", order_id, status_str);

        let status = match status_str {
            "Filled" => OrderStatus::Filled,
            "Cancelled" => OrderStatus::Cancelled,
            _ => OrderStatus::Pending,
        };

        Ok(status)
    }

    /// Get detailed order status including filled quantity from Bybit demo
    pub async fn get_order_status_detailed(&self, order_id: &str, symbol: &str) -> Result<crate::strategy::types::OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        use crate::strategy::types::OrderStatusInfo;
        
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let query_string = format!("category=linear&symbol={}&orderId={}", symbol, order_id);
        let signature = self.generate_get_signature(&timestamp, recv_window, &query_string);

        let url = format!("{}/v5/order/realtime?{}", BYBIT_DEMO_URL, query_string);

        let response = self
            .client
            .get(&url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .send()
            .await?;

        let response_text = response.text().await?;
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for API errors
        if let Some(ret_code) = response_json.get("retCode").and_then(|v| v.as_i64()) {
            if ret_code != 0 {
                let ret_msg = response_json
                    .get("retMsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Bybit API error: {}", ret_msg).into());
            }
        }

        let order_data = response_json
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .ok_or("Order not found in response")?;

        let status_str = order_data.get("orderStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let status = match status_str {
            "Filled" => OrderStatus::Filled,
            "Cancelled" => OrderStatus::Cancelled,
            _ => OrderStatus::Pending,
        };

        // Extract filled and total quantities
        // Bybit uses "cumExecQty" for filled quantity and "qty" for original quantity
        let cum_exec_qty = order_data.get("cumExecQty")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let qty = order_data.get("qty")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        eprintln!("[BYBIT DEMO] Order {} | Status: {} | Filled: {}/{}", 
            order_id, status_str, cum_exec_qty, qty);

        Ok(OrderStatusInfo::new(status, cum_exec_qty, qty))
    }

    /// Cancel an order on Bybit demo
    pub async fn cancel_order(&self, order_id: &str, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let request_body = json!({
            "category": "linear",
            "symbol": symbol.to_string(),
            "orderId": order_id,
        });

        let body_str = request_body.to_string();
        let signature = self.generate_post_signature(&timestamp, recv_window, &body_str);

        let url = format!("{}/v5/order/cancel", BYBIT_DEMO_URL);

        let response = self
            .client
            .post(&url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let response_json: Value = serde_json::from_str(&response.text().await?)?;

        if let Some(ret_code) = response_json.get("retCode").and_then(|v| v.as_i64()) {
            if ret_code != 0 {
                let ret_msg = response_json
                    .get("retMsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Failed to cancel order: {}", ret_msg).into());
            }
        }

        eprintln!("[BYBIT DEMO] Order cancelled: {}", order_id);
        Ok(())
    }

    /// Get account balance from Bybit demo
    pub async fn get_balance(&self) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let timestamp = self.get_timestamp().await?;

        let recv_window = "5000";
        let query_string = "accountType=UNIFIED";
        let signature = self.generate_get_signature(&timestamp, recv_window, query_string);

        let url = format!("{}/v5/account/wallet-balance?{}", BYBIT_DEMO_URL, query_string);

        let response = self
            .client
            .get(&url)
            .header("X-BAPI-SIGN", &signature)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status, response_text).into());
        }

        if response_text.is_empty() {
            return Err("Empty response from API".into());
        }

        let response_json: Value = serde_json::from_str(&response_text)?;

        // Extract USDT balance
        // Use equity (total available including unrealized PnL) or walletBalance as fallback
        let usdt_coin = response_json
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("coin"))
            .and_then(|c| c.as_array())
            .and_then(|a| {
                a.iter().find(|coin| {
                    coin.get("coin")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "USDT")
                        .unwrap_or(false)
                })
            });

        let balance = if let Some(usdt) = usdt_coin {
            // Use equity (available balance including unrealized PnL)
            usdt.get("equity")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| {
                    // Fallback: walletBalance
                    usdt.get("walletBalance")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                })
        } else {
            None
        };

        match balance {
            Some(bal) => {
                eprintln!("[BYBIT DEMO] Available balance: ${:.2}", bal);
                Ok(bal)
            }
            None => {
                // Debug: Print what we got if parsing failed
                eprintln!("[BYBIT DEMO] Failed to parse balance. Response structure:");
                eprintln!("{}", serde_json::to_string_pretty(&response_json).unwrap_or_else(|_| response_text.clone()));
                Err("Failed to extract USDT available balance".into())
            }
        }
    }

    /// Get order book depth from Bybit testnet
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
                    eprintln!("[BYBIT DEMO] 📦 Cache HIT for {} order book (age: {:?})", 
                        symbol, cached.cached_at.elapsed());
                    return Ok(cached.depth.clone());
                } else {
                    eprintln!("[BYBIT DEMO] 📦 Cache EXPIRED for {} order book (age: {:?})", 
                        symbol, cached.cached_at.elapsed());
                }
            }
        }

        eprintln!("[BYBIT DEMO] 📦 Cache MISS for {} order book, fetching from API", symbol);

        // Bybit API supports limit parameter: 1, 25, 50, 100, 200
        // Choose the smallest valid limit that satisfies the requested levels
        let limit = if levels <= 1 {
            1
        } else if levels <= 25 {
            25
        } else if levels <= 50 {
            50
        } else if levels <= 100 {
            100
        } else {
            200
        };

        let url = format!(
            "{}/v5/market/orderbook?category=linear&symbol={}&limit={}",
            BYBIT_DEMO_URL, symbol, limit
        );

        eprintln!("[BYBIT DEMO] Fetching order book depth for {} (limit: {})", symbol, limit);

        let response = self.client.get(&url).send().await?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status, response_text).into());
        }

        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for API errors
        if let Some(ret_code) = response_json.get("retCode").and_then(|v| v.as_i64()) {
            if ret_code != 0 {
                let ret_msg = response_json
                    .get("retMsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Bybit API error: {}", ret_msg).into());
            }
        }

        // Parse bids and asks from result
        let result = response_json
            .get("result")
            .ok_or("Missing result in response")?;

        let bids = result
            .get("b")
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

        let asks = result
            .get("a")
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

        eprintln!("[BYBIT DEMO] ✅ Order book depth fetched: {} bids, {} asks", depth.bids.len(), depth.asks.len());

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

    /// Get the best bid price from Bybit testnet
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

    /// Get the best ask price from Bybit testnet
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
