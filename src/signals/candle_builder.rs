use chrono::{DateTime, Timelike, Utc};

use crate::types::Candle;

/// Aggregates price ticks into 1-minute and 5-minute OHLCV candles.
pub struct CandleBuilder {
    current_1m: Option<CandleState>,
    current_5m: Option<CandleState>,
}

struct CandleState {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    /// The minute (0-59) when this candle started.
    start_minute: u32,
    start_ts: DateTime<Utc>,
}

impl CandleState {
    fn new(price: f64, volume: f64, minute: u32, ts: DateTime<Utc>) -> Self {
        Self {
            open: price,
            high: price,
            low: price,
            close: price,
            volume,
            start_minute: minute,
            start_ts: ts,
        }
    }

    fn update(&mut self, price: f64, volume: f64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.volume += volume;
    }

    fn into_candle(self) -> Candle {
        Candle {
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
            timestamp: self.start_ts,
        }
    }
}

impl CandleBuilder {
    pub fn new() -> Self {
        Self {
            current_1m: None,
            current_5m: None,
        }
    }

    /// Push a trade tick. Returns completed (1m_candle, 5m_candle) if a time boundary was crossed.
    pub fn push_tick(
        &mut self,
        price: f64,
        volume: f64,
        ts: DateTime<Utc>,
    ) -> (Option<Candle>, Option<Candle>) {
        let minute = ts.minute();
        let five_min_slot = minute / 5;

        let mut completed_1m = None;
        let mut completed_5m = None;

        // --- 1-minute candle ---
        if let Some(ref state) = self.current_1m {
            if minute != state.start_minute {
                // Minute boundary crossed — close current candle
                completed_1m = Some(self.current_1m.take().unwrap().into_candle());
                self.current_1m = Some(CandleState::new(price, volume, minute, ts));
            } else {
                self.current_1m.as_mut().unwrap().update(price, volume);
            }
        } else {
            self.current_1m = Some(CandleState::new(price, volume, minute, ts));
        }

        // --- 5-minute candle ---
        if let Some(ref state) = self.current_5m {
            let prev_slot = state.start_minute / 5;
            if five_min_slot != prev_slot {
                completed_5m = Some(self.current_5m.take().unwrap().into_candle());
                self.current_5m = Some(CandleState::new(price, volume, minute, ts));
            } else {
                self.current_5m.as_mut().unwrap().update(price, volume);
            }
        } else {
            self.current_5m = Some(CandleState::new(price, volume, minute, ts));
        }

        (completed_1m, completed_5m)
    }
}
