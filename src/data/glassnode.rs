use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::types::Event;

/// Glassnode REST API client ($29/mo).
/// Provides: MVRV z-score, NUPL, Puell multiple, exchange netflow.
pub struct GlassnodeFeed {
    event_tx: mpsc::Sender<Event>,
    _api_key: String,
}

impl GlassnodeFeed {
    pub fn new(event_tx: mpsc::Sender<Event>, api_key: String) -> Self {
        Self {
            event_tx,
            _api_key: api_key,
        }
    }

    /// Poll Glassnode periodically (long-term metrics, hourly/daily is fine).
    pub async fn run(&self) -> Result<()> {
        info!("GlassnodeFeed started (Phase 0 stub)");
        // TODO Phase 1:
        // - GET /v1/metrics/market/mvrv_z_score
        // - GET /v1/metrics/indicators/net_unrealized_profit_loss
        // - GET /v1/metrics/mining/puell_multiple
        // - GET /v1/metrics/transactions/transfers_volume_exchanges_net
        // - Parse → Event::OnChain
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
        }
    }

    #[allow(dead_code)]
    fn _event_tx(&self) -> &mpsc::Sender<Event> {
        &self.event_tx
    }
}
