use crate::strategy::types::ArbitrageOpportunity;
use redis::aio::MultiplexedConnection;
use std::error::Error;

pub struct OpportunityScanner;

impl OpportunityScanner {
    pub fn calculate_spread_bps(long_price: f64, short_price: f64) -> f64 {
        if long_price == 0.0 {
            0.0
        } else {
            // Spread = (short_bid - long_ask) / long_ask * 10000
            // long_price = long_ask (what we pay to buy)
            // short_price = short_bid (what we receive to sell)
            ((short_price - long_price) / long_price) * 10000.0
        }
    }

    pub fn rank_by_confidence(mut opportunities: Vec<ArbitrageOpportunity>) -> Vec<ArbitrageOpportunity> {
        opportunities.sort_by(|a, b| b.confidence_score.cmp(&a.confidence_score));
        opportunities
    }

    pub fn get_top_n(opportunities: Vec<ArbitrageOpportunity>, n: usize) -> Vec<ArbitrageOpportunity> {
        opportunities.into_iter().take(n).collect()
    }

    pub async fn log_opportunities_to_redis(
        redis_conn: &MultiplexedConnection,
        opportunities: &[ArbitrageOpportunity],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let key = format!("strategy:opportunities:{}", timestamp);
        let json = serde_json::to_string(opportunities)?;

        redis::cmd("SET")
            .arg(&key)
            .arg(json)
            .arg("EX")
            .arg(60)
            .query_async::<_, ()>(&mut redis_conn.clone())
            .await?;

        Ok(())
    }
}
