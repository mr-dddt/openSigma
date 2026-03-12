use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::types::Event;

/// Free macro data sources:
/// - Alternative.me (Fear & Greed Index)
/// - CoinGecko (BTC dominance, market cap)
/// - FRED API (DXY, rates, M2, macro calendar)
/// - SoSoValue (BTC/ETH ETF flows)
/// - Cryptopanic (high-impact news)
pub struct MacroFeed {
    event_tx: mpsc::Sender<Event>,
}

impl MacroFeed {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self { event_tx }
    }

    /// Poll all free macro sources periodically.
    pub async fn run(&self) -> Result<()> {
        info!("MacroFeed started (Phase 0 stub)");
        // TODO Phase 1:
        // - Fear & Greed: GET https://api.alternative.me/fng/
        // - CoinGecko: GET /api/v3/global
        // - FRED: GET /series/observations (DXY, FEDFUNDS)
        // - SoSoValue: scrape/API for ETF flows
        // - Cryptopanic: GET /api/v1/posts/
        // → emit Event::OnChain, Event::News, Event::MacroCalendar
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1800)).await;
        }
    }

    #[allow(dead_code)]
    fn _event_tx(&self) -> &mpsc::Sender<Event> {
        &self.event_tx
    }
}
