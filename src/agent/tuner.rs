use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::config::Config;
use crate::types::{TuneAdjustment, TuneDecision};

/// Signal engine tuner: triggers LLM-driven parameter optimization.
pub struct SignalTuner {
    trade_count: u64,
    last_tune_at_count: u64,
    last_signal_pass_at: DateTime<Utc>,
    tune_every_n_trades: u64,
    inactivity_timeout_secs: u64,
    /// Prevent duplicate inactivity triggers within one period
    inactivity_triggered: bool,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum TuneTrigger {
    TradeCount(u64),
    Inactivity,
}

impl SignalTuner {
    pub fn new(config: &Config) -> Self {
        Self {
            trade_count: 0,
            last_tune_at_count: 0,
            last_signal_pass_at: Utc::now(),
            tune_every_n_trades: config.tuning.tune_every_n_trades,
            inactivity_timeout_secs: config.tuning.inactivity_timeout_secs,
            inactivity_triggered: false,
        }
    }

    /// Called after every executed trade.
    pub fn record_trade(&mut self) {
        self.trade_count += 1;
    }

    /// Called when a signal passes filters (even if LLM skips it).
    pub fn record_signal_pass(&mut self) {
        self.last_signal_pass_at = Utc::now();
        self.inactivity_triggered = false;
    }

    /// Check if tuning is needed.
    pub fn should_tune(&mut self) -> Option<TuneTrigger> {
        if self.trade_count > 0
            && self.trade_count >= self.last_tune_at_count + self.tune_every_n_trades
        {
            return Some(TuneTrigger::TradeCount(self.trade_count));
        }

        if !self.inactivity_triggered {
            let elapsed = (Utc::now() - self.last_signal_pass_at).num_seconds();
            if elapsed >= self.inactivity_timeout_secs as i64 {
                self.inactivity_triggered = true;
                return Some(TuneTrigger::Inactivity);
            }
        }

        None
    }

    /// Mark that tuning was performed (reset counters).
    pub fn mark_tuned(&mut self) {
        self.last_tune_at_count = self.trade_count;
        self.last_signal_pass_at = Utc::now();
        self.inactivity_triggered = true;
    }

    /// Apply tuning adjustments to config with hard limits, then mark tuned.
    pub fn apply_tune(&mut self, config: &mut Config, decision: &TuneDecision) {
        for adj in &decision.adjustments {
            if apply_single_adjustment(config, adj) {
                info!(
                    param = adj.param,
                    old = adj.old_value,
                    requested = adj.new_value,
                    "Tune adjustment applied"
                );
            }
        }
        enforce_signal_invariants(&mut config.signals);
        self.mark_tuned();
    }

}

/// Apply report-driven param adjustments (from memory/learning). Uses same clamp as tune.
pub fn apply_report_adjustments(config: &mut Config, adjustments: &[TuneAdjustment]) {
    for adj in adjustments {
        if apply_single_adjustment(config, adj) {
            info!(param = adj.param, "Report param applied");
        }
    }
    enforce_signal_invariants(&mut config.signals);
}

/// Apply a single clamped adjustment to config. Returns true if recognized.
fn apply_single_adjustment(config: &mut Config, adj: &TuneAdjustment) -> bool {
    let clamped = clamp_adjustment(adj, &config.signals);
    match adj.param.as_str() {
        "ema_cross_weight" => config.signals.ema_cross_weight = clamped as i32,
        "cvd_weight" => config.signals.cvd_weight = clamped as i32,
        "rsi_weight" => config.signals.rsi_weight = clamped as i32,
        "ob_weight" => config.signals.ob_weight = clamped as i32,
        "stoch_rsi_weight" => config.signals.stoch_rsi_weight = clamped as i32,
        "strong_threshold" => config.signals.strong_threshold = clamped as i32,
        "lean_threshold" => config.signals.lean_threshold = clamped as i32,
        "rsi_oversold" => config.signals.rsi_oversold = clamped,
        "rsi_overbought" => config.signals.rsi_overbought = clamped,
        "min_atr_pct" => config.signals.min_atr_pct = clamped,
        other => {
            warn!(param = other, "Unknown tune parameter, ignoring");
            return false;
        }
    }
    true
}

/// Enforce invariants after any tuning to prevent contradictory configs.
fn enforce_signal_invariants(signals: &mut crate::config::SignalConfig) {
    signals.strong_threshold = signals.strong_threshold.max(2);
    signals.lean_threshold = signals.lean_threshold.max(1);
    if signals.lean_threshold >= signals.strong_threshold {
        signals.lean_threshold = signals.strong_threshold - 1;
    }
    signals.rsi_oversold = signals.rsi_oversold.clamp(10.0, 45.0);
    signals.rsi_overbought = signals.rsi_overbought.clamp(55.0, 90.0);
    if signals.rsi_oversold >= signals.rsi_overbought {
        signals.rsi_oversold = signals.rsi_overbought - 10.0;
    }
    signals.min_atr_pct = signals.min_atr_pct.max(0.0);
}

/// Clamp a tuning adjustment to hard limits.
fn clamp_adjustment(adj: &TuneAdjustment, signals: &crate::config::SignalConfig) -> f64 {
    let (current, max_delta) = match adj.param.as_str() {
        "ema_cross_weight" => (signals.ema_cross_weight as f64, 1.0),
        "cvd_weight" => (signals.cvd_weight as f64, 1.0),
        "rsi_weight" => (signals.rsi_weight as f64, 1.0),
        "ob_weight" => (signals.ob_weight as f64, 1.0),
        "stoch_rsi_weight" => (signals.stoch_rsi_weight as f64, 1.0),
        "strong_threshold" => (signals.strong_threshold as f64, 2.0),
        "lean_threshold" => (signals.lean_threshold as f64, 2.0),
        "rsi_oversold" => (signals.rsi_oversold, 10.0),
        "rsi_overbought" => (signals.rsi_overbought, 10.0),
        "min_atr_pct" => (signals.min_atr_pct, 0.02),
        _ => return adj.new_value,
    };

    let delta = adj.new_value - current;
    let clamped_delta = delta.clamp(-max_delta, max_delta);
    let result = current + clamped_delta;

    if adj.param.ends_with("_weight") {
        result.max(0.0)
    } else {
        result
    }
}
