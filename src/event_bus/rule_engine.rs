use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::types::*;

/// Layer 1: Pure Rust rule engine for structured market data.
/// No LLM. Latency target < 100ms.
///
/// Ingests raw WebSocket data from Hyperliquid and emits typed Events.
pub struct RuleEngine {
    event_tx: mpsc::Sender<Event>,
}

impl RuleEngine {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self { event_tx }
    }

    /// Start the rule engine ingestion loop.
    /// In Phase 0 this is a stub — will connect to Hyperliquid WS in Phase 1.
    pub async fn run(&self) -> Result<()> {
        info!("RuleEngine started (Phase 0 stub)");
        // TODO Phase 1: connect to Hyperliquid WebSocket
        // - Parse price ticks → emit Event::Price
        // - Parse funding updates → emit Event::Funding
        // - Parse liquidation feed → emit Event::Liquidation
        // - Parse OI snapshots → emit Event::OpenInterest
        //
        // For now, the engine just idles.
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            debug!("RuleEngine heartbeat");
        }
    }

    /// Manual event injection (for testing / future REST pollers).
    pub async fn inject(&self, event: Event) -> Result<()> {
        self.event_tx.send(event).await?;
        Ok(())
    }
}
