/// Configuration storage module for repricing configuration
/// 
/// This module provides stub implementations for Redis-based configuration storage.
/// In production, this would connect to a real Redis instance.
use crate::strategy::price_chaser::RepricingConfig;
use serde::{Serialize, Deserialize};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

#[cfg(test)]
use crate::strategy::price_chaser::ExecutionMode;

/// Configuration storage interface
pub trait ConfigStorage: Send + Sync {
    fn save_repricing_config(&self, config: &RepricingConfig) -> Result<(), String>;
    fn load_repricing_config(&self) -> Result<RepricingConfig, String>;
    fn save_confidence_thresholds(&self, ultra_fast: f64, balanced: f64) -> Result<(), String>;
    fn load_confidence_thresholds(&self) -> Result<(f64, f64), String>;
}

/// In-memory configuration storage (stub implementation)
/// 
/// This is a stub implementation that stores configuration in memory.
/// In production, this would be replaced with a Redis-backed implementation.
pub struct InMemoryConfigStorage {
    storage: Arc<Mutex<HashMap<String, String>>>,
}

impl InMemoryConfigStorage {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryConfigStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigStorage for InMemoryConfigStorage {
    fn save_repricing_config(&self, config: &RepricingConfig) -> Result<(), String> {
        let json = serde_json::to_string(config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        
        let mut storage = self.storage.lock()
            .map_err(|e| format!("Failed to acquire lock: {}", e))?;
        
        storage.insert("strategy:config:repricing".to_string(), json);
        
        eprintln!("[CONFIG] Saved repricing config: mode={:?}, threshold={} bps, max_reprices={}", 
            config.execution_mode, config.reprice_threshold_bps, config.max_reprices);
        
        Ok(())
    }
    
    fn load_repricing_config(&self) -> Result<RepricingConfig, String> {
        let storage = self.storage.lock()
            .map_err(|e| format!("Failed to acquire lock: {}", e))?;
        
        match storage.get("strategy:config:repricing") {
            Some(json) => {
                let config: RepricingConfig = serde_json::from_str(json)
                    .map_err(|e| format!("Failed to deserialize config: {}", e))?;
                
                eprintln!("[CONFIG] Loaded repricing config: mode={:?}, threshold={} bps", 
                    config.execution_mode, config.reprice_threshold_bps);
                
                Ok(config)
            }
            None => {
                eprintln!("[CONFIG] No saved config found, using balanced preset");
                Ok(RepricingConfig::balanced())
            }
        }
    }
    
    fn save_confidence_thresholds(&self, ultra_fast: f64, balanced: f64) -> Result<(), String> {
        let thresholds = ConfidenceThresholds {
            ultra_fast_threshold: ultra_fast,
            balanced_threshold: balanced,
        };
        
        let json = serde_json::to_string(&thresholds)
            .map_err(|e| format!("Failed to serialize thresholds: {}", e))?;
        
        let mut storage = self.storage.lock()
            .map_err(|e| format!("Failed to acquire lock: {}", e))?;
        
        storage.insert("strategy:config:confidence_thresholds".to_string(), json);
        
        eprintln!("[CONFIG] Saved confidence thresholds: ultra_fast={:.1}%, balanced={:.1}%", 
            ultra_fast, balanced);
        
        Ok(())
    }
    
    fn load_confidence_thresholds(&self) -> Result<(f64, f64), String> {
        let storage = self.storage.lock()
            .map_err(|e| format!("Failed to acquire lock: {}", e))?;
        
        match storage.get("strategy:config:confidence_thresholds") {
            Some(json) => {
                let thresholds: ConfidenceThresholds = serde_json::from_str(json)
                    .map_err(|e| format!("Failed to deserialize thresholds: {}", e))?;
                
                eprintln!("[CONFIG] Loaded confidence thresholds: ultra_fast={:.1}%, balanced={:.1}%", 
                    thresholds.ultra_fast_threshold, thresholds.balanced_threshold);
                
                Ok((thresholds.ultra_fast_threshold, thresholds.balanced_threshold))
            }
            None => {
                eprintln!("[CONFIG] No saved thresholds found, using defaults: 90%, 75%");
                Ok((90.0, 75.0))
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ConfidenceThresholds {
    ultra_fast_threshold: f64,
    balanced_threshold: f64,
}

/// Configuration validator
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate repricing configuration parameters
    pub fn validate_repricing_config(config: &RepricingConfig) -> Result<(), String> {
        // Validate reprice threshold (1-20 bps)
        if config.reprice_threshold_bps < 1.0 || config.reprice_threshold_bps > 20.0 {
            return Err(format!(
                "Invalid reprice_threshold_bps: {} (must be 1-20)",
                config.reprice_threshold_bps
            ));
        }
        
        // Validate max reprices (1-10)
        if config.max_reprices < 1 || config.max_reprices > 10 {
            return Err(format!(
                "Invalid max_reprices: {} (must be 1-10)",
                config.max_reprices
            ));
        }
        
        // Validate reprice interval (50-500ms)
        if config.reprice_interval_ms < 50 || config.reprice_interval_ms > 500 {
            return Err(format!(
                "Invalid reprice_interval_ms: {} (must be 50-500)",
                config.reprice_interval_ms
            ));
        }
        
        // Validate total timeout (1-10 seconds)
        if config.total_timeout_seconds < 1 || config.total_timeout_seconds > 10 {
            return Err(format!(
                "Invalid total_timeout_seconds: {} (must be 1-10)",
                config.total_timeout_seconds
            ));
        }
        
        // Validate spread collapse threshold (20-100 bps)
        if config.spread_collapse_threshold_bps < 20.0 || config.spread_collapse_threshold_bps > 100.0 {
            return Err(format!(
                "Invalid spread_collapse_threshold_bps: {} (must be 20-100)",
                config.spread_collapse_threshold_bps
            ));
        }
        
        Ok(())
    }
    
    /// Validate confidence thresholds
    pub fn validate_confidence_thresholds(ultra_fast: f64, balanced: f64) -> Result<(), String> {
        // Ultra-fast threshold must be higher than balanced
        if ultra_fast <= balanced {
            return Err(format!(
                "Invalid thresholds: ultra_fast ({:.1}) must be > balanced ({:.1})",
                ultra_fast, balanced
            ));
        }
        
        // Both must be in valid range (0-100)
        if !(0.0..=100.0).contains(&ultra_fast) {
            return Err(format!(
                "Invalid ultra_fast threshold: {:.1} (must be 0-100)",
                ultra_fast
            ));
        }
        
        if !(0.0..=100.0).contains(&balanced) {
            return Err(format!(
                "Invalid balanced threshold: {:.1} (must be 0-100)",
                balanced
            ));
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_save_and_load_repricing_config() {
        let storage = InMemoryConfigStorage::new();
        let config = RepricingConfig::ultra_fast();
        
        storage.save_repricing_config(&config).unwrap();
        let loaded = storage.load_repricing_config().unwrap();
        
        assert_eq!(loaded.execution_mode, ExecutionMode::UltraFast);
        assert_eq!(loaded.max_reprices, 5);
    }
    
    #[test]
    fn test_load_default_config() {
        let storage = InMemoryConfigStorage::new();
        let loaded = storage.load_repricing_config().unwrap();
        
        // Should return balanced preset as default
        assert_eq!(loaded.execution_mode, ExecutionMode::Balanced);
    }
    
    #[test]
    fn test_save_and_load_confidence_thresholds() {
        let storage = InMemoryConfigStorage::new();
        
        storage.save_confidence_thresholds(95.0, 80.0).unwrap();
        let (ultra_fast, balanced) = storage.load_confidence_thresholds().unwrap();
        
        assert_eq!(ultra_fast, 95.0);
        assert_eq!(balanced, 80.0);
    }
    
    #[test]
    fn test_validate_repricing_config_valid() {
        let config = RepricingConfig::balanced();
        assert!(ConfigValidator::validate_repricing_config(&config).is_ok());
    }
    
    #[test]
    fn test_validate_repricing_config_invalid_threshold() {
        let mut config = RepricingConfig::balanced();
        config.reprice_threshold_bps = 25.0; // Too high
        
        assert!(ConfigValidator::validate_repricing_config(&config).is_err());
    }
    
    #[test]
    fn test_validate_repricing_config_invalid_max_reprices() {
        let mut config = RepricingConfig::balanced();
        config.max_reprices = 15; // Too high
        
        assert!(ConfigValidator::validate_repricing_config(&config).is_err());
    }
    
    #[test]
    fn test_validate_confidence_thresholds_valid() {
        assert!(ConfigValidator::validate_confidence_thresholds(90.0, 75.0).is_ok());
    }
    
    #[test]
    fn test_validate_confidence_thresholds_invalid_order() {
        // Ultra-fast must be higher than balanced
        assert!(ConfigValidator::validate_confidence_thresholds(75.0, 90.0).is_err());
    }
    
    #[test]
    fn test_validate_confidence_thresholds_out_of_range() {
        assert!(ConfigValidator::validate_confidence_thresholds(105.0, 75.0).is_err());
        assert!(ConfigValidator::validate_confidence_thresholds(90.0, -5.0).is_err());
    }
}
