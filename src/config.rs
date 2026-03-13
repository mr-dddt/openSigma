use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Secrets loaded from .env
#[derive(Debug, Clone)]
pub struct Secrets {
    pub hl_private_key: String,
    pub pm_api_key: String,
    pub pm_api_secret: String,
    pub pm_passphrase: String,
    pub anthropic_api_key: String,
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
}

impl Secrets {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        Ok(Self {
            hl_private_key: std::env::var("HL_PRIVATE_KEY")
                .context("HL_PRIVATE_KEY must be set in .env")?,
            pm_api_key: std::env::var("POLY_API_KEY").unwrap_or_default(),
            pm_api_secret: std::env::var("POLY_API_SECRET").unwrap_or_default(),
            pm_passphrase: std::env::var("POLY_PASSPHRASE").unwrap_or_default(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY")
                .context("ANTHROPIC_API_KEY must be set in .env")?,
            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default(),
            telegram_chat_id: std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default(),
        })
    }
}

/// Full config loaded from config.toml
#[allow(dead_code)] // polymarket config read from TOML but not yet wired
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub capital: CapitalConfig,
    pub hyperliquid: HlConfig,
    pub polymarket: PmConfig,
    pub execution: ExecutionConfig,
    pub sessions: HashMap<String, SessionConfig>,
    pub llm: LlmConfig,
    pub signals: SignalConfig,
    #[serde(default)]
    pub tuning: TuningConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
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

#[allow(dead_code)] // PM config loaded from TOML, will be used when PM is wired
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignalConfig {
    pub strong_threshold: i32,
    pub lean_threshold: i32,
    pub min_atr_pct: f64,
    pub max_funding_same_dir: f64,
    // Tunable indicator weights (LLM can adjust at runtime)
    #[serde(default = "default_2")]
    pub ema_cross_weight: i32,
    #[serde(default = "default_2")]
    pub cvd_weight: i32,
    #[serde(default = "default_1")]
    pub rsi_weight: i32,
    #[serde(default = "default_1")]
    pub ob_weight: i32,
    #[serde(default = "default_1")]
    pub stoch_rsi_weight: i32,
    // Tunable RSI thresholds
    #[serde(default = "default_rsi_oversold")]
    pub rsi_oversold: f64,
    #[serde(default = "default_rsi_overbought")]
    pub rsi_overbought: f64,
}

fn default_2() -> i32 { 2 }
fn default_1() -> i32 { 1 }
fn default_rsi_oversold() -> f64 { 35.0 }
fn default_rsi_overbought() -> f64 { 65.0 }

#[derive(Debug, Clone, Deserialize)]
pub struct TuningConfig {
    #[serde(default = "default_tune_trades")]
    pub tune_every_n_trades: u64,
    #[serde(default = "default_inactivity")]
    pub inactivity_timeout_secs: u64,
}

fn default_tune_trades() -> u64 { 20 }
fn default_inactivity() -> u64 { 600 }

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl Default for TuningConfig {
    fn default() -> Self {
        Self {
            tune_every_n_trades: 20,
            inactivity_timeout_secs: 600,
        }
    }
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
        let now = chrono::Utc::now().time();
        for session in self.sessions.values() {
            let start = chrono::NaiveTime::parse_from_str(&session.start, "%H:%M")
                .unwrap_or(chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            let end = chrono::NaiveTime::parse_from_str(&session.end, "%H:%M")
                .unwrap_or(chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            let in_session = if start <= end {
                now >= start && now < end
            } else {
                now >= start || now < end // spans midnight
            };
            if in_session {
                return (true, session.size_mult);
            }
        }
        (false, 0.0)
    }
}
