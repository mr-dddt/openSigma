use std::collections::VecDeque;

use crate::types::Candle;

/// Rolling indicator calculator. Maintains candle history and computes
/// EMA, RSI, Stochastic RSI, Bollinger Bands, ATR, and CVD.
pub struct Indicators {
    /// 1-minute candles (used for EMA, StochRSI)
    candles_1m: VecDeque<Candle>,
    /// 5-minute candles (used for RSI, BB, ATR)
    candles_5m: VecDeque<Candle>,
    /// Trade-level CVD accumulator (5-minute rolling)
    cvd_buys: f64,
    cvd_sells: f64,
    cvd_reset_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Max candles to keep
    max_candles: usize,
}

impl Indicators {
    pub fn new() -> Self {
        Self {
            candles_1m: VecDeque::new(),
            candles_5m: VecDeque::new(),
            cvd_buys: 0.0,
            cvd_sells: 0.0,
            cvd_reset_at: None,
            max_candles: 200,
        }
    }

    /// Push a 1-minute candle.
    pub fn push_candle_1m(&mut self, candle: Candle) {
        self.candles_1m.push_back(candle);
        if self.candles_1m.len() > self.max_candles {
            self.candles_1m.pop_front();
        }
    }

    /// Push a 5-minute candle.
    pub fn push_candle_5m(&mut self, candle: Candle) {
        self.candles_5m.push_back(candle);
        if self.candles_5m.len() > self.max_candles {
            self.candles_5m.pop_front();
        }
    }

    /// Accumulate trade for CVD calculation.
    pub fn add_trade(&mut self, size: f64, is_buy: bool) {
        let now = chrono::Utc::now();

        // Reset CVD every 5 minutes
        if let Some(reset_at) = self.cvd_reset_at {
            if now >= reset_at {
                self.cvd_buys = 0.0;
                self.cvd_sells = 0.0;
                self.cvd_reset_at = Some(now + chrono::Duration::minutes(5));
            }
        } else {
            self.cvd_reset_at = Some(now + chrono::Duration::minutes(5));
        }

        if is_buy {
            self.cvd_buys += size;
        } else {
            self.cvd_sells += size;
        }
    }

    // -----------------------------------------------------------------------
    // EMA (Exponential Moving Average)
    // -----------------------------------------------------------------------

    pub fn ema_9(&self) -> Option<f64> {
        Self::ema(&self.candles_1m, 9)
    }

    pub fn ema_21(&self) -> Option<f64> {
        Self::ema(&self.candles_1m, 21)
    }

    fn ema(candles: &VecDeque<Candle>, period: usize) -> Option<f64> {
        if candles.len() < period {
            return None;
        }
        let k = 2.0 / (period as f64 + 1.0);
        let mut ema = candles[candles.len() - period].close;
        for c in candles.iter().skip(candles.len() - period + 1) {
            ema = c.close * k + ema * (1.0 - k);
        }
        Some(ema)
    }

    // -----------------------------------------------------------------------
    // RSI (Relative Strength Index) on 5m candles
    // -----------------------------------------------------------------------

    pub fn rsi_14(&self) -> Option<f64> {
        Self::rsi(&self.candles_5m, 14)
    }

    fn rsi(candles: &VecDeque<Candle>, period: usize) -> Option<f64> {
        if candles.len() < period + 1 {
            return None;
        }

        let start = candles.len() - period - 1;
        let mut avg_gain = 0.0;
        let mut avg_loss = 0.0;

        for i in (start + 1)..=(start + period) {
            let change = candles[i].close - candles[i - 1].close;
            if change > 0.0 {
                avg_gain += change;
            } else {
                avg_loss += change.abs();
            }
        }
        avg_gain /= period as f64;
        avg_loss /= period as f64;

        if avg_loss == 0.0 {
            return Some(100.0);
        }

        let rs = avg_gain / avg_loss;
        Some(100.0 - (100.0 / (1.0 + rs)))
    }

    // -----------------------------------------------------------------------
    // Stochastic RSI on 1m candles
    // -----------------------------------------------------------------------

    pub fn stoch_rsi(&self) -> Option<f64> {
        let period = 14;
        let candles = &self.candles_1m;
        if candles.len() < period + period {
            return None;
        }

        // Compute RSI values for the last `period` candles
        let mut rsi_values = Vec::new();
        for end in (candles.len() - period)..candles.len() {
            // Build a sub-slice ending at `end+1`
            let sub_len = end + 1;
            if sub_len < period + 1 {
                continue;
            }
            let mut ag = 0.0;
            let mut al = 0.0;
            for i in (sub_len - period)..sub_len {
                let change = candles[i].close - candles[i - 1].close;
                if change > 0.0 {
                    ag += change;
                } else {
                    al += change.abs();
                }
            }
            ag /= period as f64;
            al /= period as f64;
            let rsi = if al == 0.0 {
                100.0
            } else {
                100.0 - (100.0 / (1.0 + ag / al))
            };
            rsi_values.push(rsi);
        }

        if rsi_values.is_empty() {
            return None;
        }

        let min_rsi = rsi_values.iter().cloned().fold(f64::MAX, f64::min);
        let max_rsi = rsi_values.iter().cloned().fold(f64::MIN, f64::max);
        let current_rsi = *rsi_values.last()?;

        if (max_rsi - min_rsi).abs() < f64::EPSILON {
            return Some(50.0);
        }

        Some(((current_rsi - min_rsi) / (max_rsi - min_rsi)) * 100.0)
    }

    // -----------------------------------------------------------------------
    // Bollinger Bands (20-period SMA ± 2 std dev) on 5m candles
    // -----------------------------------------------------------------------

    pub fn bollinger_bands(&self) -> Option<(f64, f64, f64)> {
        let period = 20;
        let candles = &self.candles_5m;
        if candles.len() < period {
            return None;
        }

        let closes: Vec<f64> = candles
            .iter()
            .rev()
            .take(period)
            .map(|c| c.close)
            .collect();
        let sma = closes.iter().sum::<f64>() / period as f64;
        let variance = closes.iter().map(|c| (c - sma).powi(2)).sum::<f64>() / period as f64;
        let std_dev = variance.sqrt();

        let upper = sma + 2.0 * std_dev;
        let lower = sma - 2.0 * std_dev;

        Some((upper, sma, lower))
    }

    /// BB squeeze: bandwidth < 4% of SMA (tight range, breakout imminent).
    pub fn bb_squeeze(&self) -> bool {
        if let Some((upper, sma, lower)) = self.bollinger_bands() {
            if sma > 0.0 {
                let bandwidth = (upper - lower) / sma;
                return bandwidth < 0.04;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // ATR (Average True Range) on 5m candles
    // -----------------------------------------------------------------------

    pub fn atr_14(&self) -> Option<f64> {
        let period = 14;
        let candles = &self.candles_5m;
        if candles.len() < period + 1 {
            return None;
        }

        let start = candles.len() - period;
        let mut atr_sum = 0.0;

        for i in start..candles.len() {
            let high_low = candles[i].high - candles[i].low;
            let high_close = (candles[i].high - candles[i - 1].close).abs();
            let low_close = (candles[i].low - candles[i - 1].close).abs();
            let tr = high_low.max(high_close).max(low_close);
            atr_sum += tr;
        }

        Some(atr_sum / period as f64)
    }

    /// ATR as percentage of current price.
    pub fn atr_pct(&self, current_price: f64) -> Option<f64> {
        let atr = self.atr_14()?;
        if current_price > 0.0 {
            Some((atr / current_price) * 100.0)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // CVD (Cumulative Volume Delta) — 5-minute rolling
    // -----------------------------------------------------------------------

    pub fn cvd(&self) -> f64 {
        self.cvd_buys - self.cvd_sells
    }

    /// CVD direction: positive = net buying, negative = net selling.
    #[allow(dead_code)]
    pub fn cvd_rising(&self) -> bool {
        self.cvd() > 0.0
    }

    // -----------------------------------------------------------------------
    // Order Book Imbalance
    // -----------------------------------------------------------------------

    /// Compute bid/ask volume ratio from top N levels.
    /// > 1.0 = buy pressure, < 1.0 = sell pressure.
    pub fn ob_imbalance(bids: &[(f64, f64)], asks: &[(f64, f64)], levels: usize) -> f64 {
        let bid_vol: f64 = bids.iter().take(levels).map(|(_, sz)| sz).sum();
        let ask_vol: f64 = asks.iter().take(levels).map(|(_, sz)| sz).sum();
        if ask_vol > 0.0 {
            bid_vol / ask_vol
        } else {
            1.0
        }
    }
}
