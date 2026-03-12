use crate::config::Config;
use tracing::warn;

/// Hard risk checks that the LLM cannot override.
/// Phase 2: full implementation with position tracking.
pub struct RiskChecker {
    daily_loss_usd: f64,
    peak_equity: f64,
    current_equity: f64,
    open_positions: u32,
}

impl RiskChecker {
    pub fn new(initial_equity: f64) -> Self {
        Self {
            daily_loss_usd: 0.0,
            peak_equity: initial_equity,
            current_equity: initial_equity,
            open_positions: 0,
        }
    }

    /// Check if a new trade is allowed.
    pub fn can_trade(&self, config: &Config) -> Result<(), String> {
        // Max concurrent positions
        if self.open_positions >= config.capital.max_concurrent_positions {
            return Err(format!(
                "Max {} concurrent positions reached",
                config.capital.max_concurrent_positions
            ));
        }

        // Daily loss limit
        let daily_loss_pct = (self.daily_loss_usd / config.capital.initial_usd) * 100.0;
        if daily_loss_pct >= config.capital.max_daily_loss_pct {
            return Err(format!("Daily loss {:.1}% >= {:.1}% limit", daily_loss_pct, config.capital.max_daily_loss_pct));
        }

        // Kill switch (drawdown from peak)
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

    /// Max trade size in USD based on config.
    pub fn max_trade_usd(&self, config: &Config) -> f64 {
        self.current_equity * (config.capital.max_trade_pct / 100.0)
    }
}
