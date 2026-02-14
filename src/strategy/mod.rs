pub mod types;
pub mod market_data;
pub mod buffer_pool;
pub mod pipeline;
pub mod symbol_map;
pub mod opportunity_queue;
pub mod opportunity_detector;
pub mod thread_pinning;
pub mod branchless;
pub mod exchange_fees;
pub mod latency;
pub mod latency_tracker;
pub mod confluence;
pub mod scanner;
pub mod entry;
pub mod positions;
pub mod portfolio;
pub mod runner;
pub mod atomic_execution;
pub mod execution_backend;
pub mod paper_trading_backend;
pub mod testnet_config;
pub mod testnet;
pub mod testnet_backend;
pub mod synthetic_config;
// Deleted modules (replaced by main system components):
// - bybit_websocket (replaced by production Bybit connector)
// - synthetic_generator (replaced by OpportunityScanner)
// - single_exchange_executor (replaced by TestnetBackend with single-exchange mode)
pub mod test_metrics;
pub mod depth_checker;
pub mod price_chaser;
pub mod config_storage;
pub mod fill_probability;
pub mod rate_limiter;
