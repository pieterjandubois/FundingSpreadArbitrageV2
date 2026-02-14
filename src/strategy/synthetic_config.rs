use std::error::Error;

/// Configuration for synthetic arbitrage test mode
#[derive(Debug, Clone)]
pub struct SyntheticConfig {
    /// Synthetic spread in basis points (default: 15.0)
    pub synthetic_spread_bps: f64,
    /// Synthetic funding delta per 8 hours (default: 0.0001)
    pub synthetic_funding_delta: f64,
    /// Estimated position size in USD (default: 1000.0)
    pub estimated_position_size: f64,
    /// Maximum concurrent trades (default: 3)
    pub max_concurrent_trades: usize,
    /// Symbols to trade (e.g., ["BTCUSDT", "ETHUSDT"])
    pub symbols_to_trade: Vec<String>,
}

impl SyntheticConfig {
    /// Load configuration from environment variables with defaults
    pub fn from_env() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let synthetic_spread_bps = std::env::var("SYNTHETIC_SPREAD_BPS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(30.0);  // 30 bps to ensure profitability

        let synthetic_funding_delta = std::env::var("SYNTHETIC_FUNDING_DELTA")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.01);  // 1% per 8 hours

        let estimated_position_size = std::env::var("ESTIMATED_POSITION_SIZE")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1000.0);

        let max_concurrent_trades = std::env::var("MAX_CONCURRENT_TRADES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(3);

        let symbols_to_trade = std::env::var("SYMBOLS_TO_TRADE")
            .ok()
            .map(|s| s.split(',').map(|sym| sym.trim().to_string()).collect())
            .unwrap_or_else(|| vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()]);

        let config = Self {
            synthetic_spread_bps,
            synthetic_funding_delta,
            estimated_position_size,
            max_concurrent_trades,
            symbols_to_trade,
        };

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values
    fn validate(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.synthetic_spread_bps <= 0.0 {
            return Err("synthetic_spread_bps must be greater than 0".into());
        }

        if self.synthetic_funding_delta <= 0.0 {
            return Err("synthetic_funding_delta must be greater than 0".into());
        }

        if self.estimated_position_size <= 0.0 {
            return Err("estimated_position_size must be greater than 0".into());
        }

        if self.max_concurrent_trades == 0 {
            return Err("max_concurrent_trades must be greater than 0".into());
        }

        if self.symbols_to_trade.is_empty() {
            return Err("symbols_to_trade cannot be empty".into());
        }

        Ok(())
    }

    /// Create a new config with explicit values (for testing)
    pub fn new(
        synthetic_spread_bps: f64,
        synthetic_funding_delta: f64,
        estimated_position_size: f64,
        max_concurrent_trades: usize,
        symbols_to_trade: Vec<String>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let config = Self {
            synthetic_spread_bps,
            synthetic_funding_delta,
            estimated_position_size,
            max_concurrent_trades,
            symbols_to_trade,
        };
        config.validate()?;
        Ok(config)
    }
}

impl Default for SyntheticConfig {
    fn default() -> Self {
        Self {
            synthetic_spread_bps: 30.0,  // 30 bps to ensure profitability after fees
            synthetic_funding_delta: 0.01,  // 1% per 8 hours (0.01 = 1%)
            estimated_position_size: 1000.0,
            max_concurrent_trades: 3,
            symbols_to_trade: vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()],
        }
    }
}
