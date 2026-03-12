use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

/// Secrets loaded from .env
#[derive(Debug, Clone)]
pub struct Secrets {
    pub hl_private_key: String,
    pub pm_private_key: String,
    pub anthropic_api_key: String,
}

impl Secrets {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        Ok(Self {
            hl_private_key: std::env::var("HL_PRIVATE_KEY")
                .context("HL_PRIVATE_KEY must be set in .env")?,
            pm_private_key: std::env::var("PM_PRIVATE_KEY").unwrap_or_default(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
        })
    }
}

/// Full config loaded from config.toml
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub capital: CapitalConfig,
    pub hyperliquid: HlConfig,
    pub polymarket: PmConfig,
    pub execution: ExecutionConfig,
    pub sessions: HashMap<String, SessionConfig>,
    pub llm: LlmConfig,
    pub signals: SignalConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CapitalConfig {
    pub initial_usd: f64,
    pub max_trade_pct: f64,
    pub max_concurrent_positions: u32,
    pub max_daily_loss_pct: f64,
    pub kill_switch_drawdown_pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HlConfig {
    pub max_leverage: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PmConfig {
    pub max_bet_usd: f64,
    pub max_hedge_ratio: f64,
    pub prefer_maker_orders: bool,
    pub min_window_remaining_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionConfig {
    pub max_trade_duration_secs: u64,
    pub max_second_looks: u32,
    pub signal_eval_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    pub start: String,
    pub end: String,
    pub size_mult: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub model: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignalConfig {
    pub strong_threshold: i32,
    pub lean_threshold: i32,
    pub min_atr_pct: f64,
    pub max_funding_same_dir: f64,
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;
        let config: Config =
            toml::from_str(&content).context("Failed to parse config.toml")?;
        Ok(config)
    }

    /// Check if we're in an active trading session. Returns (in_session, size_multiplier).
    pub fn active_session(&self) -> (bool, f64) {
        let now = chrono::Utc::now();
        let hour_min = now.format("%H:%M").to_string();

        for session in self.sessions.values() {
            if hour_min >= session.start && hour_min < session.end {
                return (true, session.size_mult);
            }
        }
        (false, 0.0)
    }
}
