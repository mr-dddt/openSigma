use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::types::Event;

/// CoinGlass REST API client ($30/mo).
/// Provides: cross-exchange liquidation heatmap, OI history, funding trends.
pub struct CoinglassFeed {
    event_tx: mpsc::Sender<Event>,
    _api_key: String,
}

impl CoinglassFeed {
    pub fn new(event_tx: mpsc::Sender<Event>, api_key: String) -> Self {
        Self {
            event_tx,
            _api_key: api_key,
        }
    }

    /// Poll CoinGlass at regular intervals.
    pub async fn run(&self) -> Result<()> {
        info!("CoinglassFeed started (Phase 0 stub)");
        // TODO Phase 1:
        // - GET /api/pro/v1/futures/liquidation_heatmap
        // - GET /api/pro/v1/futures/open_interest_history
        // - Parse → Event::Liquidation / Event::OpenInterest
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
        }
    }

    #[allow(dead_code)]
    fn _event_tx(&self) -> &mpsc::Sender<Event> {
        &self.event_tx
    }
}
