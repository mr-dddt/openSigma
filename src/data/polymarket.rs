use tokio::sync::mpsc;
use tracing::info;

use crate::types::*;

/// Polymarket data feed for BTC 5m/15m binary markets.
/// Phase 1: stub — will connect to Polymarket CLOB WebSocket in Phase 2.
pub struct PolymarketFeed {
    _event_tx: mpsc::Sender<MarketEvent>,
}

impl PolymarketFeed {
    pub fn new(event_tx: mpsc::Sender<MarketEvent>) -> Self {
        Self { _event_tx: event_tx }
    }

    pub async fn run(&self) {
        info!("PolymarketFeed started (Phase 1 stub — no active connection)");
        // Phase 2: connect to Polymarket CLOB WebSocket
        // - Discover active BTC 5m/15m binary markets via REST
        // - Subscribe to orderbook + price updates
        // - Emit MarketEvent::PmOdds with current Up/Down prices
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }
}
