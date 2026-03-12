use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::types::Event;

/// Hyperliquid WebSocket data feed.
/// Provides real-time price, order book, trades, funding, liquidations.
pub struct HyperliquidFeed {
    event_tx: mpsc::Sender<Event>,
}

impl HyperliquidFeed {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self { event_tx }
    }

    /// Connect to Hyperliquid WS and start streaming events.
    pub async fn run(&self) -> Result<()> {
        info!("HyperliquidFeed started (Phase 0 stub)");
        // TODO Phase 1:
        // 1. Connect to wss://api.hyperliquid.xyz/ws
        // 2. Subscribe to channels: allMids, trades, liquidations, funding
        // 3. Parse messages → emit typed Events via event_tx
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }

    #[allow(dead_code)]
    fn _event_tx(&self) -> &mpsc::Sender<Event> {
        &self.event_tx
    }
}
