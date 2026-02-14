use std::error::Error;
use reqwest::Client;
use serde_json::{json, Value};
use crate::strategy::types::{SimulatedOrder, OrderStatus, OrderSide};
use crate::strategy::testnet_config::ExchangeCredentials;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::{engine::general_purpose, Engine as _};

type HmacSha256 = Hmac<Sha256>;

const BITGET_DEMO_URL: &str = "https://api.bitget.com";

pub struct BitgetDemoClient {
    client: Client,
    api_key: String,
    api_secret: String,
    passphrase: String,
}

impl BitgetDemoClient {
    pub fn new(credentials: ExchangeCredentials) -> Self {
        Self {
            client: Client::new(),
            api_key: credentials.api_key,
            api_secret: credentials.api_secret,
            passphrase: credentials.passphrase.unwrap_or_default(),
        }
    }

    /// Generate signature for Bitget API
    /// Format: timestamp + method.toUpperCase() + requestPath + [queryString] + body
    fn generate_signature(&self, timestamp: &str, method: &str, path: &str, query_string: &str, body: &str) -> String {
        let message = if query_string.is_empty() {
            format!("{}{}{}{}", timestamp, method.to_uppercase(), path, body)
        } else {
            format!("{}{}{}?{}{}", timestamp, method.to_uppercase(), path, query_string, body)
        };

        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    /// Check if a symbol is tradeable on Bitget futures
    pub async fn is_symbol_tradeable(&self, symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/api/v2/mix/market/contracts?productType=usdt-futures", BITGET_DEMO_URL);
        
        let response = self.client.get(&url).send().await?;
        let response_json: Value = serde_json::from_str(&response.text().await?)?;
        
        eprintln!("[BITGET VALIDATION] Checking if {} is tradeable...", symbol);
        
        if let Some(data) = response_json.get("data").and_then(|d| d.as_array()) {
            for contract in data {
                if let Some(contract_symbol) = contract.get("symbol").and_then(|s| s.as_str()) {
                    if contract_symbol == symbol {
                        // Check if it's a perpetual and normal status
                        let symbol_type = contract.get("symbolType")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let symbol_status = contract.get("symbolStatus")
                            .and_then(|s| s.as_str())
                            .unwrap_or("");
                        
                        let is_perpetual = symbol_type == "perpetual";
                        let is_normal = symbol_status == "normal";
                        
                        eprintln!("[BITGET VALIDATION] {} found: type={}, status={}, tradeable={}", 
                            symbol, symbol_type, symbol_status, is_perpetual && is_normal);
                        
                        return Ok(is_perpetual && is_normal);
                    }
                }
            }
        }
        
        eprintln!("[BITGET VALIDATION] {} NOT FOUND in contracts list", symbol);
        Ok(false)
    }

    /// Place an order on Bitget demo
    pub async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        let timestamp = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis())
            .to_string();

        let side = match order.side {
            OrderSide::Long => "buy",
            OrderSide::Short => "sell",
        };

        let request_body = json!({
            "symbol": order.symbol.clone(),
            "productType": "USDT-FUTURES",
            "marginMode": "crossed",
            "marginCoin": "USDT",
            "size": order.size.to_string(),
            "price": order.price.to_string(),
            "side": side,
            "tradeSide": "open",
            "orderType": "limit",
            "force": "gtc",
        });

        let body_str = request_body.to_string();
        let path = "/api/v2/mix/order/place-order";
        let signature = self.generate_signature(&timestamp, "POST", path, "", &body_str);

        let url = format!("{}{}", BITGET_DEMO_URL, path);

        let response = self
            .client
            .post(&url)
            .header("ACCESS-KEY", &self.api_key)
            .header("ACCESS-SIGN", signature)
            .header("ACCESS-TIMESTAMP", &timestamp)
            .header("ACCESS-PASSPHRASE", &self.passphrase)
            .header("paptrading", "1")
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let response_text = response.text().await?;
        let response_json: Value = serde_json::from_str(&response_text)?;

        // Check for errors
        if let Some(code) = response_json.get("code").and_then(|v| v.as_str()) {
            if code != "00000" {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Bitget API error: {}", msg).into());
            }
        }

        // Extract order ID from response
        let order_id = response_json
            .get("data")
            .and_then(|d| d.get("orderId"))
            .and_then(|v| v.as_str())
            .ok_or("Failed to extract order ID from response")?
            .to_string();

        let mut filled_order = order;
        filled_order.id = order_id;
        filled_order.status = OrderStatus::Pending;
        filled_order.created_at = timestamp.parse().unwrap_or(0);

        eprintln!("[BITGET DEMO] Order placed: {} | Symbol: {} | Side: {} | Size: {} | Price: {}", 
            filled_order.id, filled_order.symbol, side, filled_order.size, filled_order.price);

        Ok(filled_order)
    }

    /// Get order status from Bitget demo
    pub async fn get_order_status(&self, order_id: &str, symbol: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        let timestamp = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis())
            .to_string();

        let query_string = format!("symbol={}&productType=USDT-FUTURES&orderId={}", symbol, order_id);
        let path = "/api/v2/mix/order/orders-pending";
        let signature = self.generate_signature(&timestamp, "GET", path, &query_string, "");

        let url = format!("{}{}?{}", BITGET_DEMO_URL, path, query_string);

        let response = self
            .client
            .get(&url)
            .header("ACCESS-KEY", &self.api_key)
            .header("ACCESS-SIGN", signature)
            .header("ACCESS-TIMESTAMP", &timestamp)
            .header("ACCESS-PASSPHRASE", &self.passphrase)
            .header("paptrading", "1")
            .send()
            .await?;

        let response_json: Value = serde_json::from_str(&response.text().await?)?;

        let status_str = response_json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let status = match status_str {
            "filled" => OrderStatus::Filled,
            "canceled" => OrderStatus::Cancelled,
            _ => OrderStatus::Pending,
        };

        Ok(status)
    }

    /// Cancel an order on Bitget demo
    pub async fn cancel_order(&self, order_id: &str, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis())
            .to_string();

        let request_body = json!({
            "symbol": symbol.to_string(),
            "productType": "USDT-FUTURES",
            "marginCoin": "USDT",
            "orderId": order_id,
        });

        let body_str = request_body.to_string();
        let path = "/api/v2/mix/order/cancel-order";
        let signature = self.generate_signature(&timestamp, "POST", path, "", &body_str);

        let url = format!("{}{}", BITGET_DEMO_URL, path);

        let response = self
            .client
            .post(&url)
            .header("ACCESS-KEY", &self.api_key)
            .header("ACCESS-SIGN", signature)
            .header("ACCESS-TIMESTAMP", &timestamp)
            .header("ACCESS-PASSPHRASE", &self.passphrase)
            .header("paptrading", "1")
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let response_json: Value = serde_json::from_str(&response.text().await?)?;

        if let Some(code) = response_json.get("code").and_then(|v| v.as_str()) {
            if code != "00000" {
                let msg = response_json
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Failed to cancel order: {}", msg).into());
            }
        }

        eprintln!("[BITGET DEMO] Order cancelled: {}", order_id);
        Ok(())
    }

    /// Get account balance from Bitget demo
    pub async fn get_balance(&self) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let timestamp = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis())
            .to_string();

        let path = "/api/v2/mix/account/accounts";
        let query_params = vec![
            ("productType", "usdt-futures"),
        ];
        
        // Build query string for signature
        let query_string = query_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let signature = self.generate_signature(&timestamp, "GET", path, &query_string, "");

        let url = format!("{}{}", BITGET_DEMO_URL, path);

        let response = self
            .client
            .get(&url)
            .header("ACCESS-KEY", &self.api_key)
            .header("ACCESS-SIGN", signature)
            .header("ACCESS-TIMESTAMP", &timestamp)
            .header("ACCESS-PASSPHRASE", &self.passphrase)
            .header("paptrading", "1")
            .header("locale", "en-US")
            .header("Content-Type", "application/json")
            .query(&query_params)
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

        // Extract USDT balance from response
        // Response structure might be an array of accounts
        let balance = response_json
            .get("data")
            .and_then(|d| {
                if d.is_array() {
                    d.as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|first| first.get("available"))
                } else {
                    d.get("available")
                }
            })
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or("Failed to extract USDT balance from response")?;

        Ok(balance)
    }
}
