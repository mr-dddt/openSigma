use crate::config::Config;
use crate::types::LlmDecision;
use tracing::warn;

/// Hard risk checks that the LLM cannot override.
pub struct RiskChecker {
    daily_loss_usd: f64,
    peak_equity: f64,
    current_equity: f64,
    open_positions: u32,
    total_wins: u64,
    total_closed: u64,
    streak: i32, // positive = consecutive wins, negative = consecutive losses
}

impl RiskChecker {
    pub fn new(initial_equity: f64) -> Self {
        Self {
            daily_loss_usd: 0.0,
            peak_equity: initial_equity,
            current_equity: initial_equity,
            open_positions: 0,
            total_wins: 0,
            total_closed: 0,
            streak: 0,
        }
    }

    /// Check if a new trade is allowed.
    pub fn can_trade(&self, config: &Config) -> Result<(), String> {
        if self.open_positions >= config.capital.max_concurrent_positions {
            return Err(format!(
                "Max {} concurrent positions reached",
                config.capital.max_concurrent_positions
            ));
        }

        let daily_loss_pct = (self.daily_loss_usd / config.capital.initial_usd) * 100.0;
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

    /// Validate an LLM Execute decision against hard limits.
    pub fn validate_decision(&self, decision: &LlmDecision, config: &Config) -> Result<(), String> {
        if let LlmDecision::Execute {
            size_pct,
            hl_leverage,
            ..
        } = decision
        {
            if *size_pct > config.capital.max_trade_pct {
                return Err(format!(
                    "LLM size {:.1}% exceeds max {:.1}%",
                    size_pct, config.capital.max_trade_pct
                ));
            }
            if let Some(lev) = hl_leverage {
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

    /// Max trade size in USD based on config.
    pub fn max_trade_usd(&self, config: &Config) -> f64 {
        self.current_equity * (config.capital.max_trade_pct / 100.0)
    }

    /// Update equity after a trade closes.
    #[allow(dead_code)]
    pub fn update_equity(&mut self, new_equity: f64) {
        self.current_equity = new_equity;
        if new_equity > self.peak_equity {
            self.peak_equity = new_equity;
        }
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

    pub fn set_open_positions(&mut self, count: u32) {
        self.open_positions = count;
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
    pub fn daily_pnl_pct(&self, config: &Config) -> f64 {
        let pnl = self.current_equity - config.capital.initial_usd;
        (pnl / config.capital.initial_usd) * 100.0
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
}
