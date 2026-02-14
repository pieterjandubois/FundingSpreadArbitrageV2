use std::error::Error;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex;
use crate::strategy::execution_backend::ExecutionBackend;
use crate::strategy::types::{SimulatedOrder, OrderStatus, OrderSide, OrderType, QueuePosition};
use crate::strategy::testnet::bybit_testnet::BybitDemoClient;
use crate::strategy::testnet_config::TestnetConfig;

pub struct TestnetBackend {
    bybit: Option<Arc<BybitDemoClient>>,
    // Store order metadata: order_id -> (exchange, symbol)
    order_metadata: Arc<Mutex<HashMap<String, (String, String)>>>,
    // Blacklist of symbols that failed to place orders: (exchange, symbol)
    failed_symbols: Arc<Mutex<HashMap<(String, String), bool>>>,
    // Single-exchange mode: route all orders to primary_exchange
    single_exchange_mode: bool,
    primary_exchange: String,
}

impl TestnetBackend {
    pub fn new(config: TestnetConfig) -> Self {
        let bybit = config.bybit.map(|creds| Arc::new(BybitDemoClient::new(creds)));

        if bybit.is_some() {
            eprintln!("[DEMO] Bybit demo client initialized");
        }

        Self { 
            bybit,
            order_metadata: Arc::new(Mutex::new(HashMap::new())),
            failed_symbols: Arc::new(Mutex::new(HashMap::new())),
            single_exchange_mode: config.single_exchange_mode,
            primary_exchange: config.primary_exchange,
        }
    }

    /// Synchronize server time with Bybit exchange
    /// Should be called once after initialization
    pub async fn sync_server_time(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        eprintln!("[DEMO] 🕐 Synchronizing server time with Bybit...");
        
        if let Some(client) = &self.bybit {
            match client.sync_server_time().await {
                Ok(_) => eprintln!("[DEMO] ✅ Bybit time synchronized"),
                Err(e) => eprintln!("[DEMO] ⚠️  Failed to sync Bybit time: {}", e),
            }
        }
        
        eprintln!("[DEMO] 🕐 Server time synchronization complete");
        Ok(())
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for TestnetBackend {
    async fn set_leverage(&self, exchange: &str, symbol: &str, _leverage: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Always set to 1x leverage (ignore the leverage parameter)
        match exchange {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.set_leverage(symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn set_margin_type_isolated(&self, exchange: &str, symbol: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        match exchange {
            "bybit" => {
                // Bybit uses isolated margin by default in demo, no action needed
                Ok(())
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn place_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        // In single-exchange mode, only execute orders on primary exchange
        // Simulate orders for other exchanges
        if self.single_exchange_mode {
            let is_primary_exchange = order.exchange == self.primary_exchange;
            
            if is_primary_exchange {
                // REAL execution on primary exchange (Bybit)
                eprintln!("[REAL EXECUTION] Placing {} order on {} @ ${:.2}", 
                    if order.side == OrderSide::Long { "LONG" } else { "SHORT" },
                    order.exchange, order.price);
                
                let result = match order.exchange.as_str() {
                    "bybit" => {
                        if let Some(client) = &self.bybit {
                            client.place_order(order.clone()).await
                        } else {
                            Err("Bybit demo not configured".into())
                        }
                    }
                    _ => Err(format!("Exchange {} not supported in demo", order.exchange).into()),
                };
                
                // Store order metadata if successful
                if let Ok(ref placed_order) = result {
                    let mut metadata = self.order_metadata.lock().await;
                    metadata.insert(
                        placed_order.id.clone(),
                        (order.exchange.clone(), placed_order.symbol.clone())
                    );
                }
                
                result
            } else {
                // SIMULATED execution for non-primary exchange
                eprintln!("[SIMULATED EXECUTION] Simulating {} order on {} @ ${:.2}", 
                    if order.side == OrderSide::Long { "LONG" } else { "SHORT" },
                    order.exchange, order.price);
                
                // Create a simulated filled order
                let simulated_order = SimulatedOrder {
                    id: format!("sim_{}", uuid::Uuid::new_v4()),
                    exchange: order.exchange.clone(),
                    symbol: order.symbol.clone(),
                    side: order.side,
                    order_type: order.order_type,
                    price: order.price,
                    size: order.size,
                    queue_position: Some(QueuePosition {
                        price: order.price,
                        cumulative_volume_at_price: 0.0,
                        resting_depth_at_entry: 0.0,
                        fill_threshold_pct: 100.0,
                        is_filled: true,
                    }),
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    filled_at: Some(std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()),
                    fill_price: Some(order.price),
                    status: OrderStatus::Filled,
                };
                
                // Store simulated order metadata
                let mut metadata = self.order_metadata.lock().await;
                metadata.insert(
                    simulated_order.id.clone(),
                    (order.exchange.clone(), simulated_order.symbol.clone())
                );
                
                Ok(simulated_order)
            }
        } else {
            // Normal mode: execute on specified exchange
            let result = match order.exchange.as_str() {
                "bybit" => {
                    if let Some(client) = &self.bybit {
                        client.place_order(order.clone()).await
                    } else {
                        Err("Bybit demo not configured".into())
                    }
                }
                _ => Err(format!("Exchange {} not supported in demo", order.exchange).into()),
            };

            // Store order metadata if successful
            if let Ok(ref placed_order) = result {
                let mut metadata = self.order_metadata.lock().await;
                metadata.insert(
                    placed_order.id.clone(),
                    (order.exchange.clone(), placed_order.symbol.clone())
                );
            }

            result
        }
    }

    async fn place_market_order(&self, order: SimulatedOrder) -> Result<SimulatedOrder, Box<dyn Error + Send + Sync>> {
        // In single-exchange mode, only execute orders on primary exchange
        // Simulate orders for other exchanges
        if self.single_exchange_mode {
            let is_primary_exchange = order.exchange == self.primary_exchange;
            
            if is_primary_exchange {
                // REAL execution on primary exchange (Bybit)
                eprintln!("[REAL EXECUTION] Placing {} market order on {} @ ${:.2}", 
                    if order.side == OrderSide::Long { "LONG" } else { "SHORT" },
                    order.exchange, order.price);
                
                let result = match order.exchange.as_str() {
                    "bybit" => {
                        if let Some(client) = &self.bybit {
                            client.place_market_order(order.clone()).await
                        } else {
                            Err("Bybit demo not configured".into())
                        }
                    }
                    _ => Err(format!("Exchange {} not supported in demo", order.exchange).into()),
                };
                
                // Store order metadata if successful
                if let Ok(ref placed_order) = result {
                    let mut metadata = self.order_metadata.lock().await;
                    metadata.insert(
                        placed_order.id.clone(),
                        (order.exchange.clone(), placed_order.symbol.clone())
                    );
                }
                
                result
            } else {
                // SIMULATED execution for non-primary exchange
                eprintln!("[SIMULATED EXECUTION] Simulating {} market order on {} @ ${:.2}", 
                    if order.side == OrderSide::Long { "LONG" } else { "SHORT" },
                    order.exchange, order.price);
                
                // Create a simulated filled order
                let simulated_order = SimulatedOrder {
                    id: format!("sim_{}", uuid::Uuid::new_v4()),
                    exchange: order.exchange.clone(),
                    symbol: order.symbol.clone(),
                    side: order.side,
                    order_type: OrderType::Market,
                    price: order.price,
                    size: order.size,
                    queue_position: Some(QueuePosition {
                        price: order.price,
                        cumulative_volume_at_price: 0.0,
                        resting_depth_at_entry: 0.0,
                        fill_threshold_pct: 100.0,
                        is_filled: true,
                    }),
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    filled_at: Some(std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()),
                    fill_price: Some(order.price),
                    status: OrderStatus::Filled,
                };
                
                // Store simulated order metadata
                let mut metadata = self.order_metadata.lock().await;
                metadata.insert(
                    simulated_order.id.clone(),
                    (order.exchange.clone(), simulated_order.symbol.clone())
                );
                
                Ok(simulated_order)
            }
        } else {
            // Normal mode: execute on specified exchange
            let result = match order.exchange.as_str() {
                "bybit" => {
                    if let Some(client) = &self.bybit {
                        client.place_market_order(order.clone()).await
                    } else {
                        Err("Bybit demo not configured".into())
                    }
                }
                _ => Err(format!("Exchange {} not supported in demo", order.exchange).into()),
            };

            // Store order metadata if successful
            if let Ok(ref placed_order) = result {
                let mut metadata = self.order_metadata.lock().await;
                metadata.insert(
                    placed_order.id.clone(),
                    (order.exchange.clone(), placed_order.symbol.clone())
                );
            }

            result
        }
    }

    async fn cancel_order(&self, _exchange: &str, order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Check if this is a simulated order
        if order_id.starts_with("sim_") {
            eprintln!("[SIMULATED] Cancelling simulated order: {}", order_id);
            // Simulated orders are already "filled", so cancellation is a no-op
            return Ok(());
        }
        
        // Retrieve order metadata for real orders
        let metadata = self.order_metadata.lock().await;
        let (exchange, symbol) = metadata.get(order_id)
            .ok_or_else(|| format!("Order {} not found in metadata", order_id))?;

        match exchange.as_str() {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.cancel_order(order_id, symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn get_order_status(&self, _exchange: &str, order_id: &str) -> Result<OrderStatus, Box<dyn Error + Send + Sync>> {
        // Check if this is a simulated order
        if order_id.starts_with("sim_") {
            // Simulated orders are always filled
            return Ok(OrderStatus::Filled);
        }
        
        // Retrieve order metadata for real orders
        let metadata = self.order_metadata.lock().await;
        let (exchange, symbol) = metadata.get(order_id)
            .ok_or_else(|| format!("Order {} not found in metadata", order_id))?;

        match exchange.as_str() {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_order_status(order_id, symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn get_order_status_detailed(&self, _exchange: &str, order_id: &str, symbol: &str) -> Result<crate::strategy::types::OrderStatusInfo, Box<dyn Error + Send + Sync>> {
        // Retrieve order metadata to get the actual exchange
        let metadata = self.order_metadata.lock().await;
        let (exchange, _) = metadata.get(order_id)
            .ok_or_else(|| format!("Order {} not found in metadata", order_id))?;

        match exchange.as_str() {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_order_status_detailed(order_id, symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn get_available_balance(&self, exchange: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        // In single-exchange mode, only check balance on primary exchange
        // Return a large balance for other exchanges since we're not actually using them
        if self.single_exchange_mode && exchange != self.primary_exchange {
            eprintln!("[SINGLE-EXCHANGE] Skipping balance check for {} (not primary exchange)", exchange);
            return Ok(1_000_000.0); // Return large balance to pass validation
        }
        
        match exchange {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_balance().await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn get_all_balances(&self) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
        let mut balances = HashMap::new();
        
        if let Some(client) = &self.bybit {
            match client.get_balance().await {
                Ok(balance) => {
                    balances.insert("bybit".to_string(), balance);
                    eprintln!("[BALANCES] Bybit: ${:.2}", balance);
                }
                Err(e) => {
                    eprintln!("[BALANCES] Failed to fetch Bybit balance: {}", e);
                }
            }
        }
Ok(balances)
    }

    async fn is_symbol_tradeable(&self, exchange: &str, symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        // Check blacklist first
        let failed = self.failed_symbols.lock().await;
        if failed.contains_key(&(exchange.to_string(), symbol.to_string())) {
            eprintln!("[VALIDATION] {} on {} is blacklisted (previously failed)", symbol, exchange);
            return Ok(false);
        }
        drop(failed);
        
        // Temporarily trust all symbols - let order placement fail if symbol doesn't exist
        // This will help us identify which symbols actually work
        eprintln!("[VALIDATION] Allowing {} on {} (trusting symbol exists)", symbol, exchange);
        Ok(true)
    }

    async fn get_order_book_depth(
        &self,
        exchange: &str,
        symbol: &str,
        levels: usize,
    ) -> Result<crate::strategy::types::OrderBookDepth, Box<dyn Error + Send + Sync>> {
        match exchange {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_order_book_depth(symbol, levels).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            "bitget" => {
                // TODO: Implement for Bitget in task 4.1
                Err("get_order_book_depth not yet implemented for Bitget".into())
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn get_best_bid(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        match exchange {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_best_bid(symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            "bitget" => {
                // TODO: Implement for Bitget in task 4.2
                Err("get_best_bid not yet implemented for Bitget".into())
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    async fn get_best_ask(
        &self,
        exchange: &str,
        symbol: &str,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        match exchange {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_best_ask(symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            "bitget" => {
                // TODO: Implement for Bitget in task 4.3
                Err("get_best_ask not yet implemented for Bitget".into())
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }

    fn backend_name(&self) -> &str {
        "Demo"
    }
    
    async fn get_quantity_step(&self, exchange: &str, symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {
        match exchange {
            "bybit" => {
                if let Some(client) = &self.bybit {
                    client.get_qty_step(symbol).await
                } else {
                    Err("Bybit demo not configured".into())
                }
            }
            _ => Err(format!("Exchange {} not supported in demo", exchange).into()),
        }
    }
}
