use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::config::Config;
use crate::signals::candle_builder::CandleBuilder;
use crate::signals::indicators::Indicators;
use crate::types::*;

/// Signal aggregator: computes weighted bull/bear scores from indicators,
/// applies hard filters, and outputs a SignalSnapshot with level classification.
pub struct SignalAggregator {
    pub indicators: Indicators,
    candle_builder: CandleBuilder,
    // Latest state
    latest_price: f64,
    latest_funding: f64,
    latest_ob_imbalance: f64,
    latest_pm_up: Option<f64>,
    daily_pnl_pct: f64,
    kill_switch_triggered: bool,
    news_circuit_breaker: bool,
}

impl SignalAggregator {
    pub fn new() -> Self {
        Self {
            indicators: Indicators::new(),
            candle_builder: CandleBuilder::new(),
            latest_price: 0.0,
            latest_funding: 0.0,
            latest_ob_imbalance: 1.0,
            latest_pm_up: None,
            daily_pnl_pct: 0.0,
            kill_switch_triggered: false,
            news_circuit_breaker: false,
        }
    }

    pub fn update_price(&mut self, price: f64) {
        self.latest_price = price;
    }

    pub fn latest_price(&self) -> f64 {
        self.latest_price
    }

    pub fn update_funding(&mut self, rate: f64) {
        self.latest_funding = rate;
    }

    pub fn update_ob_imbalance(&mut self, imbalance: f64) {
        self.latest_ob_imbalance = imbalance;
    }

    pub fn update_pm_odds(&mut self, up_price: f64) {
        self.latest_pm_up = Some(up_price);
    }

    pub fn update_daily_pnl(&mut self, pnl_pct: f64) {
        self.daily_pnl_pct = pnl_pct;
    }

    pub fn set_kill_switch(&mut self, triggered: bool) {
        self.kill_switch_triggered = triggered;
    }

    #[allow(dead_code)] // will be wired when NewsFeed sends events
    pub fn set_news_circuit_breaker(&mut self, active: bool) {
        self.news_circuit_breaker = active;
    }

    /// Push a trade tick for candle building. Completed candles are automatically
    /// forwarded to the indicator calculator.
    pub fn push_trade_for_candles(&mut self, price: f64, size: f64, ts: DateTime<Utc>) {
        let (candle_1m, candle_5m) = self.candle_builder.push_tick(price, size, ts);
        if let Some(c) = candle_1m {
            info!(close = c.close, "1m candle closed");
            self.indicators.push_candle_1m(c);
        }
        if let Some(c) = candle_5m {
            info!(close = c.close, "5m candle closed");
            self.indicators.push_candle_5m(c);
        }
    }

    /// Evaluate all indicators and produce a signal snapshot.
    pub fn evaluate(&self, config: &Config) -> SignalSnapshot {
        let mut bull = 0i32;
        let mut bear = 0i32;
        let mut indicators = IndicatorValues::default();

        let sig = &config.signals;

        // --- EMA(9, 21) cross [configurable weight] ---
        let ema_9 = self.indicators.ema_9();
        let ema_21 = self.indicators.ema_21();
        indicators.ema_9 = ema_9;
        indicators.ema_21 = ema_21;
        if let (Some(e9), Some(e21)) = (ema_9, ema_21) {
            if e9 > e21 {
                bull += sig.ema_cross_weight;
            } else if e9 < e21 {
                bear += sig.ema_cross_weight;
            }
        }

        // --- RSI(14) 5m [configurable weight + thresholds] ---
        let rsi = self.indicators.rsi_14();
        indicators.rsi_14 = rsi;
        if let Some(r) = rsi {
            if r < sig.rsi_oversold {
                bull += sig.rsi_weight;
            } else if r > sig.rsi_overbought {
                bear += sig.rsi_weight;
            }
        }

        // --- CVD 5m [configurable weight] ---
        let cvd = self.indicators.cvd();
        indicators.cvd = Some(cvd);
        if cvd > 0.0 {
            bull += sig.cvd_weight;
        } else if cvd < 0.0 {
            bear += sig.cvd_weight;
        }

        // --- Order Book Imbalance [configurable weight] ---
        indicators.ob_imbalance = Some(self.latest_ob_imbalance);
        if self.latest_ob_imbalance > 2.0 {
            bull += sig.ob_weight;
        } else if self.latest_ob_imbalance < 0.5 {
            bear += sig.ob_weight;
        }

        // --- Stochastic RSI 1m [configurable weight] ---
        let stoch = self.indicators.stoch_rsi();
        indicators.stoch_rsi = stoch;
        if let Some(s) = stoch {
            if s < 20.0 {
                bull += sig.stoch_rsi_weight;
            } else if s > 80.0 {
                bear += sig.stoch_rsi_weight;
            }
        }

        // --- PM Odds Divergence [weight 1] ---
        if let Some(pm_up) = self.latest_pm_up {
            // If technicals are bullish but PM "Down" is cheap → extra bull
            // If technicals are bearish but PM "Up" is cheap → extra bear
            let net_so_far = bull - bear;
            if net_so_far > 0 && pm_up < 0.4 {
                // PM thinks bearish, but technicals disagree → divergence bull signal
                bull += 1;
                indicators.pm_divergence = Some(pm_up);
            } else if net_so_far < 0 && pm_up > 0.6 {
                // PM thinks bullish, but technicals disagree → divergence bear signal
                bear += 1;
                indicators.pm_divergence = Some(pm_up);
            }
        }

        // --- Bollinger Bands (smart scoring, not a hard filter) ---
        if let Some((upper, _sma, lower)) = self.indicators.bollinger_bands() {
            indicators.bb_upper = Some(upper);
            indicators.bb_lower = Some(lower);
        }
        indicators.bb_squeeze = self.indicators.bb_squeeze();
        indicators.bb_bandwidth = self.indicators.bb_bandwidth();
        let bb_pos = self.indicators.bb_position(self.latest_price);
        indicators.bb_position = bb_pos;

        if let Some(pos) = bb_pos {
            if indicators.bb_squeeze {
                // During squeeze: mean-reversion scoring.
                // Price near lower band → expect bounce (bull).
                // Price near upper band → expect rejection (bear).
                if pos <= -0.8 {
                    bull += 1;
                } else if pos >= 0.8 {
                    bear += 1;
                }
            } else {
                // Bands expanding (post-squeeze or trending): breakout scoring.
                // Price broke above upper band → momentum long.
                // Price broke below lower band → momentum short.
                if pos > 1.0 {
                    bull += 1;
                } else if pos < -1.0 {
                    bear += 1;
                }
            }
        }

        // --- ATR ---
        indicators.atr_14 = self.indicators.atr_14();
        indicators.atr_pct = self.indicators.atr_pct(self.latest_price);

        // --- Compute net score and level ---
        let net = bull - bear;

        // --- Hard filters → NO_TRADE ---
        let filter_reason = self.check_hard_filters(config, net);
        let level = if filter_reason.is_some() {
            SignalLevel::NoTrade
        } else {
            classify_level(net, &config.signals)
        };

        debug!(
            bull = bull,
            bear = bear,
            net = net,
            level = %level,
            "Signal evaluated"
        );

        SignalSnapshot {
            bull_score: bull,
            bear_score: bear,
            net_score: net,
            level,
            filter_reason,
            indicators,
            timestamp: Utc::now(),
        }
    }

    fn check_hard_filters(&self, config: &Config, net_score: i32) -> Option<String> {
        // BB squeeze is no longer a hard filter — it's handled as a scoring
        // signal above (mean-reversion near bands, breakout on band break).

        // ATR too low → dead market
        if let Some(atr_pct) = self.indicators.atr_pct(self.latest_price) {
            if atr_pct < config.signals.min_atr_pct {
                return Some(format!("ATR {:.3}% < min {:.3}%", atr_pct, config.signals.min_atr_pct));
            }
        }

        // News circuit breaker
        if self.news_circuit_breaker {
            return Some("News circuit breaker ON".into());
        }

        // No time-of-day hard block — BTC is liquid 24/7.
        // Off-peak hours are handled via session size_mult in config.toml
        // (e.g., London 0.4x, Late NY 0.6x). The LLM sees session context
        // and the ATR filter catches genuinely dead markets.

        // Daily loss limit
        if self.daily_pnl_pct <= -config.capital.max_daily_loss_pct {
            return Some(format!(
                "Daily loss {:.1}% >= limit {:.1}%",
                self.daily_pnl_pct, config.capital.max_daily_loss_pct
            ));
        }

        // Kill switch
        if self.kill_switch_triggered {
            return Some("Kill switch triggered".into());
        }

        // Funding rate in same direction as signal
        if net_score > 0 && self.latest_funding > config.signals.max_funding_same_dir {
            return Some(format!(
                "Funding {:.4}% in long direction",
                self.latest_funding
            ));
        }
        if net_score < 0 && self.latest_funding < -config.signals.max_funding_same_dir {
            return Some(format!(
                "Funding {:.4}% in short direction",
                self.latest_funding
            ));
        }

        None
    }
}

use crate::config::SignalConfig;

fn classify_level(net: i32, cfg: &SignalConfig) -> SignalLevel {
    if net >= cfg.strong_threshold {
        SignalLevel::StrongLong
    } else if net >= cfg.lean_threshold {
        SignalLevel::LeanLong
    } else if net <= -cfg.strong_threshold {
        SignalLevel::StrongShort
    } else if net <= -cfg.lean_threshold {
        SignalLevel::LeanShort
    } else {
        SignalLevel::Weak
    }
}
