use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use crate::types::ActiveTrade;

/// Tracks active trades and enforces max-hold timeouts.
pub struct PositionMonitor {
    pub active_trades: Vec<ActiveTrade>,
}

#[allow(dead_code)]
pub enum PositionEvent {
    Expired(Uuid),
    StopHit(Uuid),
    TakeProfitHit(Uuid),
}

impl PositionMonitor {
    pub fn new() -> Self {
        Self {
            active_trades: Vec::new(),
        }
    }

    pub fn add_trade(&mut self, trade: ActiveTrade) {
        info!(id = %trade.id, direction = %trade.direction, size = trade.size_usd, "Position opened");
        self.active_trades.push(trade);
    }

    pub fn remove_trade(&mut self, id: &Uuid) -> Option<ActiveTrade> {
        if let Some(pos) = self.active_trades.iter().position(|t| t.id == *id) {
            Some(self.active_trades.remove(pos))
        } else {
            None
        }
    }

    /// Check all positions for max-hold timeout. Returns IDs of trades to close.
    pub fn check_expirations(&self) -> Vec<Uuid> {
        let now = Utc::now();
        self.active_trades
            .iter()
            .filter(|t| {
                let elapsed = (now - t.opened_at).num_seconds();
                elapsed >= t.max_hold_secs as i64
            })
            .map(|t| t.id)
            .collect()
    }

    /// Check stop-loss and take-profit levels against current price.
    pub fn check_price_levels(&self, current_price: f64) -> Vec<PositionEvent> {
        let mut events = Vec::new();

        for trade in &self.active_trades {

            let pnl_pct = match trade.direction {
                crate::types::Direction::Long => {
                    (current_price - trade.entry_price) / trade.entry_price * 100.0
                }
                crate::types::Direction::Short => {
                    (trade.entry_price - current_price) / trade.entry_price * 100.0
                }
            };

            if pnl_pct <= -trade.stop_loss_pct {
                events.push(PositionEvent::StopHit(trade.id));
            } else if pnl_pct >= trade.take_profit_pct {
                events.push(PositionEvent::TakeProfitHit(trade.id));
            }
        }

        events
    }

    pub fn open_count(&self) -> u32 {
        self.active_trades.len() as u32
    }

    #[allow(dead_code)]
    pub fn active_trades(&self) -> &[ActiveTrade] {
        &self.active_trades
    }

    /// Find all positions in the opposite direction. Used to close
    /// counter-trend positions before opening new ones.
    pub fn opposite_direction_ids(&self, direction: crate::types::Direction) -> Vec<Uuid> {
        self.active_trades
            .iter()
            .filter(|t| t.direction != direction)
            .map(|t| t.id)
            .collect()
    }

    /// Check if any positions exist in the opposite direction.
    #[allow(dead_code)]
    pub fn has_opposite(&self, direction: crate::types::Direction) -> bool {
        self.active_trades.iter().any(|t| t.direction != direction)
    }

    /// Calculate total unrealized PnL.
    #[allow(dead_code)]
    pub fn unrealized_pnl(&self, current_price: f64) -> f64 {
        self.active_trades
            .iter()
            .map(|t| {
                match t.direction {
                    crate::types::Direction::Long => {
                        (current_price - t.entry_price) / t.entry_price * t.size_usd
                    }
                    crate::types::Direction::Short => {
                        (t.entry_price - current_price) / t.entry_price * t.size_usd
                    }
                }
            })
            .sum()
    }
}
