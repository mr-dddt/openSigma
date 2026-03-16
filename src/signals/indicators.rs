use std::collections::VecDeque;

use crate::types::Candle;

#[derive(Debug, Clone, Copy)]
struct PriceCvdPoint {
    price: f64,
    cvd: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaDivergence {
    Bullish,
    Bearish,
}

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
    /// Recent CVD snapshots for momentum (slope) estimation.
    cvd_samples: VecDeque<f64>,
    /// 1m snapshots for price-vs-CVD divergence detection.
    price_cvd_samples: VecDeque<PriceCvdPoint>,
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
            cvd_samples: VecDeque::new(),
            price_cvd_samples: VecDeque::new(),
            max_candles: 200,
        }
    }

    /// Push a 1-minute candle.
    pub fn push_candle_1m(&mut self, candle: Candle) {
        self.candles_1m.push_back(candle);
        if self.candles_1m.len() > self.max_candles {
            self.candles_1m.pop_front();
        }
        self.price_cvd_samples.push_back(PriceCvdPoint {
            price: self.candles_1m.back().map(|c| c.close).unwrap_or(0.0),
            cvd: self.cvd(),
        });
        if self.price_cvd_samples.len() > self.max_candles {
            self.price_cvd_samples.pop_front();
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
        self.cvd_samples.push_back(self.cvd());
        if self.cvd_samples.len() > 60 {
            self.cvd_samples.pop_front();
        }
    }

    /// Seed CVD from historical candles using close-vs-open as buy/sell proxy.
    /// Not as accurate as real trade-level data, but gives a directional
    /// starting point instead of 0.0 on startup.
    pub fn seed_cvd_from_candles(&mut self, candles: &[Candle]) {
        let recent: Vec<&Candle> = candles.iter().rev().take(5).collect();
        let mut buys = 0.0f64;
        let mut sells = 0.0f64;
        for c in recent {
            if c.close >= c.open {
                buys += c.volume;
            } else {
                sells += c.volume;
            }
        }
        self.cvd_buys = buys;
        self.cvd_sells = sells;
        self.cvd_reset_at = Some(chrono::Utc::now() + chrono::Duration::minutes(5));
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
        // Seed with SMA of the first `period` candles (matches TradingView)
        let sma: f64 = candles.iter().take(period).map(|c| c.close).sum::<f64>() / period as f64;
        let mut ema = sma;
        // Apply exponential smoothing through all remaining candles
        for c in candles.iter().skip(period) {
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

        // Seed: SMA of first `period` gains/losses (Wilder method, matches TradingView)
        let mut avg_gain = 0.0;
        let mut avg_loss = 0.0;
        for i in 1..=period {
            let change = candles[i].close - candles[i - 1].close;
            if change > 0.0 {
                avg_gain += change;
            } else {
                avg_loss += change.abs();
            }
        }
        avg_gain /= period as f64;
        avg_loss /= period as f64;

        // Wilder smoothing through all remaining candles
        for i in (period + 1)..candles.len() {
            let change = candles[i].close - candles[i - 1].close;
            let gain = if change > 0.0 { change } else { 0.0 };
            let loss = if change < 0.0 { change.abs() } else { 0.0 };
            avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
            avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
        }

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
        let rsi_period: usize = 14;
        let stoch_period: usize = 14;
        let candles = &self.candles_1m;
        let n = candles.len();
        if n < rsi_period + stoch_period + 1 {
            return None;
        }

        // Compute RSI at each candle endpoint using Wilder smoothing.
        // Seed the first RSI with SMA of gains/losses over the first rsi_period changes.
        let mut avg_gain;
        let mut avg_loss;
        {
            let mut g = 0.0;
            let mut l = 0.0;
            for i in 1..=rsi_period {
                let change = candles[i].close - candles[i - 1].close;
                if change > 0.0 { g += change; } else { l += change.abs(); }
            }
            avg_gain = g / rsi_period as f64;
            avg_loss = l / rsi_period as f64;
        }

        // Continue with exponential smoothing for the remaining candles,
        // collecting the last stoch_period RSI values.
        let collect_from = n - stoch_period;
        let mut rsi_values = Vec::with_capacity(stoch_period);

        // First RSI value is at index rsi_period
        if rsi_period >= collect_from {
            let rsi = if avg_loss == 0.0 { 100.0 } else { 100.0 - (100.0 / (1.0 + avg_gain / avg_loss)) };
            rsi_values.push(rsi);
        }

        for i in (rsi_period + 1)..n {
            let change = candles[i].close - candles[i - 1].close;
            let gain = if change > 0.0 { change } else { 0.0 };
            let loss = if change < 0.0 { change.abs() } else { 0.0 };
            avg_gain = (avg_gain * (rsi_period - 1) as f64 + gain) / rsi_period as f64;
            avg_loss = (avg_loss * (rsi_period - 1) as f64 + loss) / rsi_period as f64;

            if i >= collect_from {
                let rsi = if avg_loss == 0.0 { 100.0 } else { 100.0 - (100.0 / (1.0 + avg_gain / avg_loss)) };
                rsi_values.push(rsi);
            }
        }

        if rsi_values.len() < stoch_period {
            return None;
        }

        let min_rsi = rsi_values.iter().cloned().fold(f64::MAX, f64::min);
        let max_rsi = rsi_values.iter().cloned().fold(f64::MIN, f64::max);
        let current_rsi = *rsi_values.last()?;

        // When RSI barely moved, return neutral instead of extreme 0/100
        if (max_rsi - min_rsi).abs() < 0.5 {
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

    /// BB squeeze: bandwidth < 1% of SMA (genuinely tight consolidation).
    /// 4% was too aggressive — blocked trades during normal BTC ranges.
    pub fn bb_squeeze(&self) -> bool {
        self.bb_bandwidth().map_or(false, |bw| bw < 0.01)
    }

    /// BB bandwidth as fraction of SMA (0.0 = zero width, typical 0.01–0.06).
    pub fn bb_bandwidth(&self) -> Option<f64> {
        let (upper, sma, lower) = self.bollinger_bands()?;
        if sma > 0.0 {
            Some((upper - lower) / sma)
        } else {
            None
        }
    }

    /// Where is price relative to the bands? Returns -1.0 (at lower) to +1.0 (at upper).
    /// 0.0 = at SMA. Values outside [-1, 1] mean price broke through a band.
    pub fn bb_position(&self, current_price: f64) -> Option<f64> {
        let (upper, sma, lower) = self.bollinger_bands()?;
        let half_width = (upper - lower) / 2.0;
        if half_width > 0.0 {
            Some((current_price - sma) / half_width)
        } else {
            None
        }
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

        // Seed: SMA of first `period` true ranges (Wilder method, matches TradingView)
        let mut atr = 0.0;
        for i in 1..=period {
            let high_low = candles[i].high - candles[i].low;
            let high_close = (candles[i].high - candles[i - 1].close).abs();
            let low_close = (candles[i].low - candles[i - 1].close).abs();
            atr += high_low.max(high_close).max(low_close);
        }
        atr /= period as f64;

        // Wilder smoothing through all remaining candles
        for i in (period + 1)..candles.len() {
            let high_low = candles[i].high - candles[i].low;
            let high_close = (candles[i].high - candles[i - 1].close).abs();
            let low_close = (candles[i].low - candles[i - 1].close).abs();
            let tr = high_low.max(high_close).max(low_close);
            atr = (atr * (period - 1) as f64 + tr) / period as f64;
        }

        Some(atr)
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

    /// Rolling VWAP from recent 1m candles.
    pub fn vwap_1m(&self, period: usize) -> Option<f64> {
        if self.candles_1m.len() < period {
            return None;
        }
        let mut pv = 0.0;
        let mut vol = 0.0;
        for c in self.candles_1m.iter().rev().take(period) {
            pv += c.close * c.volume;
            vol += c.volume;
        }
        if vol > 0.0 {
            Some(pv / vol)
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

    /// CVD momentum over recent samples.
    pub fn cvd_slope(&self) -> Option<f64> {
        if self.cvd_samples.len() < 6 {
            return None;
        }
        let n = self.cvd_samples.len();
        Some(self.cvd_samples[n - 1] - self.cvd_samples[n - 6])
    }

    /// Detects CVD/price delta divergence:
    /// - Bearish: price makes a new local high but CVD does not confirm.
    /// - Bullish: price makes a new local low but CVD does not confirm.
    pub fn delta_divergence(&self, lookback_bars: usize) -> Option<DeltaDivergence> {
        if lookback_bars < 3 || self.price_cvd_samples.len() < lookback_bars + 1 {
            return None;
        }

        let n = self.price_cvd_samples.len();
        let samples: Vec<PriceCvdPoint> = self.price_cvd_samples.iter().copied().collect();
        let current = samples[n - 1];
        let prev_window = &samples[n - 1 - lookback_bars..n - 1];

        let prev_high = prev_window.iter().max_by(|a, b| a.price.total_cmp(&b.price))?;
        let prev_low = prev_window.iter().min_by(|a, b| a.price.total_cmp(&b.price))?;

        if current.price > prev_high.price && current.cvd < prev_high.cvd {
            return Some(DeltaDivergence::Bearish);
        }
        if current.price < prev_low.price && current.cvd > prev_low.cvd {
            return Some(DeltaDivergence::Bullish);
        }
        None
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

    /// Summarize recent 1m price movement for regime-aware tuning context.
    pub fn recent_price_movement_summary(&self, minutes: usize) -> String {
        if minutes < 2 || self.candles_1m.len() < minutes {
            return "Price movement: n/a (insufficient 1m candles)".to_string();
        }
        let recent: Vec<&Candle> = self.candles_1m.iter().rev().take(minutes).collect();
        let first = *recent.last().unwrap_or(&recent[0]);
        let last = recent[0];

        if first.close <= 0.0 || last.close <= 0.0 {
            return "Price movement: n/a (invalid candle data)".to_string();
        }

        let ret_pct = ((last.close - first.close) / first.close) * 100.0;
        let hi = recent.iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let lo = recent.iter().map(|c| c.low).fold(f64::MAX, f64::min);
        let range_pct = if first.close > 0.0 {
            ((hi - lo) / first.close) * 100.0
        } else {
            0.0
        };
        let avg_body_pct = recent
            .iter()
            .map(|c| ((c.close - c.open).abs() / c.open.max(1.0)) * 100.0)
            .sum::<f64>()
            / minutes as f64;

        let bars = recent
            .iter()
            .rev()
            .take(6)
            .map(|c| {
                format!(
                    "{} o={:.0} h={:.0} l={:.0} c={:.0}",
                    c.timestamp.format("%H:%M"),
                    c.open,
                    c.high,
                    c.low,
                    c.close
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");

        format!(
            "Recent 1m movement ({}m): ret={:+.3}% range={:.3}% avg_body={:.3}%\nLast candles: {}",
            minutes, ret_pct, range_pct, avg_body_pct, bars
        )
    }
}
