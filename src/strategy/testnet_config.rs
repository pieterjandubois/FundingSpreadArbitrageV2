use std::error::Error;

#[derive(Debug, Clone)]
pub struct TestnetConfig {
    pub bybit: Option<ExchangeCredentials>,
    pub okx: Option<ExchangeCredentials>,
    pub kucoin: Option<ExchangeCredentials>,
    pub bitget: Option<ExchangeCredentials>,
    pub single_exchange_mode: bool,
    pub primary_exchange: String,
}

#[derive(Debug, Clone)]
pub struct ExchangeCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
}

impl TestnetConfig {
    pub fn from_env() -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Load single-exchange mode configuration
        let single_exchange_mode = std::env::var("SINGLE_EXCHANGE_MODE")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
        
        let primary_exchange = std::env::var("PRIMARY_EXCHANGE")
            .unwrap_or_else(|_| "bybit".to_string());

        // Validation: if single_exchange_mode is enabled, primary_exchange must be set
        if single_exchange_mode && primary_exchange.is_empty() {
            return Err("PRIMARY_EXCHANGE must be set when SINGLE_EXCHANGE_MODE=true".into());
        }

        Ok(Self {
            bybit: Self::load_exchange_creds("BYBIT_DEMO")?,
            okx: Self::load_exchange_creds_with_passphrase("OKX_DEMO")?,
            kucoin: Self::load_exchange_creds_with_passphrase("KUCOIN_DEMO")?,
            bitget: Self::load_exchange_creds_with_passphrase("BITGET_DEMO")?,
            single_exchange_mode,
            primary_exchange,
        })
    }

    fn load_exchange_creds(prefix: &str) -> Result<Option<ExchangeCredentials>, Box<dyn Error + Send + Sync>> {
        let api_key = std::env::var(format!("{}_API_KEY", prefix)).ok();
        let api_secret = std::env::var(format!("{}_API_SECRET", prefix)).ok();

        match (api_key, api_secret) {
            (Some(key), Some(secret)) => {
                Ok(Some(ExchangeCredentials {
                    api_key: key,
                    api_secret: secret,
                    passphrase: None,
                }))
            }
            (None, None) => Ok(None),
            _ => Err(format!("Incomplete credentials for {}: both API_KEY and API_SECRET required", prefix).into()),
        }
    }

    fn load_exchange_creds_with_passphrase(prefix: &str) -> Result<Option<ExchangeCredentials>, Box<dyn Error + Send + Sync>> {
        let api_key = std::env::var(format!("{}_API_KEY", prefix)).ok();
        let api_secret = std::env::var(format!("{}_API_SECRET", prefix)).ok();
        let passphrase = std::env::var(format!("{}_PASSPHRASE", prefix)).ok();

        match (api_key, api_secret) {
            (Some(key), Some(secret)) => {
                Ok(Some(ExchangeCredentials {
                    api_key: key,
                    api_secret: secret,
                    passphrase,
                }))
            }
            (None, None) => Ok(None),
            _ => Err(format!("Incomplete credentials for {}: both API_KEY and API_SECRET required", prefix).into()),
        }
    }

    pub fn has_any_configured(&self) -> bool {
        self.bybit.is_some()
            || self.okx.is_some()
            || self.kucoin.is_some()
            || self.bitget.is_some()
    }
}
