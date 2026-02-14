use std::error::Error;
use std::sync::Arc;
use arbitrage2::strategy::runner::StrategyRunner;
use arbitrage2::strategy::testnet_backend::TestnetBackend;
use arbitrage2::strategy::testnet_config::TestnetConfig;
use arbitrage2::strategy::pipeline::MarketPipeline;
use arbitrage2::strategy::symbol_map::SymbolMap;

const REDIS_URL: &str = "redis://127.0.0.1:6379";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenv::dotenv().ok();
    
    // Print thread pinning information
    arbitrage2::strategy::thread_pinning::print_core_assignment_info();

    eprintln!("[DEMO] Starting demo trading runner...");
    eprintln!("[DEMO] This will execute REAL trades on demo accounts!");
    eprintln!("[DEMO] Bybit + Binance only");

    // Load demo credentials from environment
    let config = TestnetConfig::from_env()?;
    
    if !config.has_any_configured() {
        eprintln!("[ERROR] No demo credentials configured in .env");
        eprintln!("[ERROR] Please set BYBIT_DEMO_* and BINANCE_DEMO_* environment variables");
        return Err("Missing demo credentials".into());
    }

    // Initialize demo backend
    let demo_backend = Arc::new(TestnetBackend::new(config));

    // Synchronize server time with exchanges
    demo_backend.sync_server_time().await?;

    // Connect to Redis
    let redis_client = redis::Client::open(REDIS_URL)?;
    let redis_conn = redis_client.get_multiplexed_tokio_connection().await?;

    // Create streaming pipeline for low-latency market data processing
    // Requirement: 1.1 (Direct memory architecture), 14.1 (Streaming architecture)
    let pipeline = MarketPipeline::new();
    let consumer = pipeline.consumer();
    
    // Create dynamic symbol map for all incoming data
    let symbol_map = Arc::new(SymbolMap::new());
    
    eprintln!("[DEMO] ✅ Streaming pipeline initialized (low-latency mode)");

    // Initialize strategy runner with demo backend and "demo" Redis prefix
    // Only trade on Bybit and Binance demo accounts
    // Note: starting_capital will be auto-fetched from exchanges by StrategyRunner::new()
    let mut runner = StrategyRunner::new(
        redis_conn,
        0.0, // Placeholder - will be replaced with actual fetched balance
        demo_backend,
        Some("demo".to_string()),
        Some(vec!["bybit".to_string(), "binance".to_string()]),
        symbol_map,  // Pass the dynamic symbol map
    ).await?;
    
    // Enable streaming mode for low-latency processing
    // Requirement: 14.2 (Process data immediately without batching delays)
    runner.set_market_consumer(consumer);

    eprintln!("[DEMO] Strategy runner initialized");
    eprintln!("[DEMO] Redis prefix: demo");
    eprintln!("[DEMO] Monitoring Bybit + Binance arbitrage opportunities...");

    // Run the strategy
    runner.run_scanning_loop().await?;

    Ok(())
}
