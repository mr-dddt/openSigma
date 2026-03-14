use chrono::{DateTime, Datelike, Utc};

use crate::config::Config;
use crate::types::{LlmDecision, TradeRecord};
use tracing::{info, warn};

const TRADE_COOLDOWN_SECS: i64 = 30;

/// Hard risk checks that the LLM cannot override.
/// Uses real exchange balance (synced from balance poller) for all calculations.
pub struct RiskChecker {
    daily_loss_usd: f64,
    start_of_day_equity: f64,
    peak_equity: f64,
    current_equity: f64,
    open_positions: u32,
    total_wins: u64,
    total_closed: u64,
    streak: i32,
    at_max_positions: bool,
    last_trade_at: Option<DateTime<Utc>>,
    last_reset_day: u32, // day-of-year of last daily reset
}

impl RiskChecker {
    pub fn new(initial_equity: f64) -> Self {
        Self {
            daily_loss_usd: 0.0,
            start_of_day_equity: initial_equity,
            peak_equity: if initial_equity > 0.0 { initial_equity } else { 0.0 },
            current_equity: initial_equity,
            open_positions: 0,
            total_wins: 0,
            total_closed: 0,
            streak: 0,
            at_max_positions: false,
            last_trade_at: None,
            last_reset_day: Utc::now().ordinal(),
        }
    }

    /// Sync equity from real exchange balance (called by balance poller).
    pub fn sync_balance(&mut self, exchange_equity: f64) {
        if exchange_equity > 0.0 {
            self.current_equity = exchange_equity;
            if exchange_equity > self.peak_equity {
                self.peak_equity = exchange_equity;
            }
            // Initialize peak and start-of-day on first sync
            if self.peak_equity <= 0.0 {
                self.peak_equity = exchange_equity;
            }
            if self.start_of_day_equity <= 0.0 {
                self.start_of_day_equity = exchange_equity;
            }
        }
    }

    /// Reset daily counters if a new UTC day has started.
    pub fn maybe_reset_day(&mut self) {
        let today = Utc::now().ordinal();
        if today != self.last_reset_day {
            self.daily_loss_usd = 0.0;
            self.start_of_day_equity = self.current_equity;
            self.last_reset_day = today;
            tracing::info!(equity = self.current_equity, "Daily risk counters reset (new UTC day)");
        }
    }

    /// Check if a new trade is allowed.
    pub fn can_trade(&mut self, config: &Config) -> Result<(), String> {
        if self.open_positions >= config.capital.max_concurrent_positions {
            self.at_max_positions = true;
            return Err(format!(
                "Max {} concurrent positions reached",
                config.capital.max_concurrent_positions
            ));
        }
        self.at_max_positions = false;

        // Cooldown: wait at least TRADE_COOLDOWN_SECS between entries
        if let Some(last) = self.last_trade_at {
            let elapsed = (Utc::now() - last).num_seconds();
            if elapsed < TRADE_COOLDOWN_SECS {
                return Err(format!(
                    "Cooldown: {}s remaining",
                    TRADE_COOLDOWN_SECS - elapsed
                ));
            }
        }

        if self.current_equity <= 0.0 {
            return Err("No balance available".to_string());
        }

        let base = if self.start_of_day_equity > 0.0 { self.start_of_day_equity } else { self.current_equity };
        let daily_loss_pct = (self.daily_loss_usd / base) * 100.0;
        if daily_loss_pct >= config.capital.max_daily_loss_pct {
            return Err(format!(
                "Daily loss {:.1}% >= {:.1}% limit",
                daily_loss_pct, config.capital.max_daily_loss_pct
            ));
        }

        let drawdown_pct = if self.peak_equity > 0.0 {
            ((self.peak_equity - self.current_equity) / self.peak_equity) * 100.0
        } else {
            0.0
        };
        if drawdown_pct >= config.capital.kill_switch_drawdown_pct {
            warn!(drawdown = drawdown_pct, "Kill switch triggered");
            return Err(format!("Kill switch: {:.1}% drawdown", drawdown_pct));
        }

        Ok(())
    }

    pub fn is_at_max_positions(&self) -> bool {
        self.at_max_positions
    }

    /// Record that a trade was just opened (starts cooldown timer).
    pub fn record_trade_open(&mut self) {
        self.last_trade_at = Some(Utc::now());
    }

    /// Validate an LLM Execute decision against hard limits.
    pub fn validate_decision(&self, decision: &LlmDecision, config: &Config) -> Result<(), String> {
        if let LlmDecision::Execute {
            size_pct,
            hl_leverage,
            ..
        } = decision
        {
            if *size_pct <= 0.0 {
                return Err("LLM size must be positive".to_string());
            }
            if *size_pct > config.capital.max_trade_pct {
                return Err(format!(
                    "LLM size {:.1}% exceeds max {:.1}%",
                    size_pct, config.capital.max_trade_pct
                ));
            }
            if let Some(lev) = hl_leverage {
                if *lev == 0 {
                    return Err("LLM leverage cannot be 0".to_string());
                }
                if *lev > config.hyperliquid.max_leverage {
                    return Err(format!(
                        "LLM leverage {} exceeds max {}",
                        lev, config.hyperliquid.max_leverage
                    ));
                }
            }
        }
        Ok(())
    }

    /// Max trade size in USD based on real equity and config percentage.
    pub fn max_trade_usd(&self, config: &Config) -> f64 {
        self.current_equity * (config.capital.max_trade_pct / 100.0)
    }

    /// Record trade PnL for daily loss tracking + win/loss stats.
    pub fn record_trade_pnl(&mut self, pnl_usd: f64) {
        if pnl_usd < 0.0 {
            self.daily_loss_usd += pnl_usd.abs();
        }
        self.current_equity += pnl_usd;
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }

        // Track win/loss stats
        self.total_closed += 1;
        if pnl_usd >= 0.0 {
            self.total_wins += 1;
            self.streak = if self.streak >= 0 { self.streak + 1 } else { 1 };
        } else {
            self.streak = if self.streak <= 0 { self.streak - 1 } else { -1 };
        }
    }

    pub fn set_open_positions(&mut self, count: u32, config: &Config) {
        self.open_positions = count;
        if count < config.capital.max_concurrent_positions {
            self.at_max_positions = false;
        }
    }

    /// Check if kill switch should trigger based on drawdown.
    pub fn should_kill(&self, config: &Config) -> bool {
        if self.peak_equity <= 0.0 {
            return false;
        }
        let drawdown_pct =
            ((self.peak_equity - self.current_equity) / self.peak_equity) * 100.0;
        drawdown_pct >= config.capital.kill_switch_drawdown_pct
    }

    pub fn current_equity(&self) -> f64 {
        self.current_equity
    }

    #[allow(dead_code)]
    pub fn start_of_day_equity(&self) -> f64 {
        self.start_of_day_equity
    }

    #[allow(dead_code)]
    pub fn daily_pnl_pct(&self) -> f64 {
        if self.peak_equity > 0.0 {
            ((self.current_equity - self.peak_equity) / self.peak_equity) * 100.0
        } else {
            0.0
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.total_closed == 0 {
            0.0
        } else {
            self.total_wins as f64 / self.total_closed as f64
        }
    }

    pub fn streak(&self) -> i32 {
        self.streak
    }

    pub fn total_closed(&self) -> u64 {
        self.total_closed
    }

    pub fn daily_loss_usd(&self) -> f64 {
        self.daily_loss_usd
    }

    /// Restore win/loss stats from historical journal trades on startup.
    /// Does NOT modify equity (that comes from exchange balance sync).
    pub fn restore_from_trades(&mut self, trades: &[TradeRecord]) {
        for t in trades {
            if let Some(pnl) = t.pnl_usd {
                self.total_closed += 1;
                if pnl >= 0.0 {
                    self.total_wins += 1;
                    self.streak = if self.streak >= 0 { self.streak + 1 } else { 1 };
                } else {
                    self.streak = if self.streak <= 0 { self.streak - 1 } else { -1 };
                }
            }
        }
        if self.total_closed > 0 {
            info!(
                total = self.total_closed,
                wins = self.total_wins,
                streak = self.streak,
                "Restored stats from journal"
            );
        }
    }
}
