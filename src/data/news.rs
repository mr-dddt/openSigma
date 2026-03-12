use tracing::info;

/// News feed for circuit breaker events.
/// Phase 1: stub — will poll Cryptopanic + FRED macro calendar in Phase 2.
pub struct NewsFeed {
    pub circuit_breaker_active: bool,
}

impl NewsFeed {
    pub fn new() -> Self {
        Self {
            circuit_breaker_active: false,
        }
    }

    pub async fn run(&mut self) {
        info!("NewsFeed started (Phase 1 stub — circuit breaker defaults OFF)");
        // Phase 2:
        // - Poll Cryptopanic every 5 min for high-impact BTC/ETH news
        // - Poll FRED for macro calendar (CPI, FOMC, PPI dates)
        // - Set circuit_breaker_active = true when high-impact event imminent
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }
}
