use crate::types::{AgentName, RiskLimits};
use anyhow::{Context, Result};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Config {
    pub private_key: String,
    pub coinglass_api_key: String,
    pub glassnode_api_key: String,
    pub anthropic_api_key: String,
    pub is_mainnet: bool,
    pub kill_switch_drawdown_pct: f64,
    pub whitelisted_symbols: Vec<String>,
    pub risk_limits: HashMap<AgentName, RiskLimits>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let mut risk_limits = HashMap::new();

        risk_limits.insert(
            AgentName::LongTerm,
            RiskLimits {
                max_leverage: 2.0,
                max_per_trade_pct: 6.0,
                max_total_exposure_pct: 100.0, // unlimited when unleveraged
            },
        );

        risk_limits.insert(
            AgentName::MidTerm,
            RiskLimits {
                max_leverage: 5.0,
                max_per_trade_pct: 5.0,
                max_total_exposure_pct: 20.0,
            },
        );

        risk_limits.insert(
            AgentName::ShortTerm,
            RiskLimits {
                max_leverage: 20.0,
                max_per_trade_pct: 2.0,
                max_total_exposure_pct: 10.0,
            },
        );

        Ok(Config {
            private_key: std::env::var("PRIVATE_KEY")
                .context("PRIVATE_KEY must be set in .env")?,
            coinglass_api_key: std::env::var("COINGLASS_API_KEY").unwrap_or_default(),
            glassnode_api_key: std::env::var("GLASSNODE_API_KEY").unwrap_or_default(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            is_mainnet: std::env::var("HYPERLIQUID_MAINNET")
                .unwrap_or_else(|_| "false".into())
                .parse()
                .context("Invalid HYPERLIQUID_MAINNET (expected true/false)")?,
            kill_switch_drawdown_pct: std::env::var("KILL_SWITCH_DRAWDOWN_PCT")
                .unwrap_or_else(|_| "15.0".into())
                .parse()
                .context("Invalid KILL_SWITCH_DRAWDOWN_PCT")?,
            whitelisted_symbols: vec!["BTC".into(), "ETH".into()],
            risk_limits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_risk_limits_correct() {
        // Set required env var for Config::from_env
        std::env::set_var("PRIVATE_KEY", "0x0000000000000000000000000000000000000001");
        let config = Config::from_env().unwrap();

        let lt = config.risk_limits.get(&AgentName::LongTerm).unwrap();
        assert!((lt.max_leverage - 2.0).abs() < f64::EPSILON);
        assert!((lt.max_per_trade_pct - 6.0).abs() < f64::EPSILON);
        assert!((lt.max_total_exposure_pct - 100.0).abs() < f64::EPSILON);

        let mt = config.risk_limits.get(&AgentName::MidTerm).unwrap();
        assert!((mt.max_leverage - 5.0).abs() < f64::EPSILON);
        assert!((mt.max_per_trade_pct - 5.0).abs() < f64::EPSILON);
        assert!((mt.max_total_exposure_pct - 20.0).abs() < f64::EPSILON);

        let st = config.risk_limits.get(&AgentName::ShortTerm).unwrap();
        assert!((st.max_leverage - 20.0).abs() < f64::EPSILON);
        assert!((st.max_per_trade_pct - 2.0).abs() < f64::EPSILON);
        assert!((st.max_total_exposure_pct - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn whitelisted_symbols() {
        std::env::set_var("PRIVATE_KEY", "0x0000000000000000000000000000000000000001");
        let config = Config::from_env().unwrap();
        assert_eq!(config.whitelisted_symbols, vec!["BTC", "ETH"]);
    }

    #[test]
    fn default_kill_switch_pct() {
        std::env::set_var("PRIVATE_KEY", "0x0000000000000000000000000000000000000001");
        std::env::remove_var("KILL_SWITCH_DRAWDOWN_PCT");
        let config = Config::from_env().unwrap();
        assert!((config.kill_switch_drawdown_pct - 15.0).abs() < f64::EPSILON);
    }
}
