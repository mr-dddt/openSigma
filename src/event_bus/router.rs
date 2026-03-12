use anyhow::Result;
use tokio::sync::mpsc;
use tracing::debug;

use crate::types::*;

/// Topic Router: dispatches events from the event bus to per-agent queues
/// based on a static subscription table.
pub struct TopicRouter {
    event_rx: mpsc::Receiver<Event>,
    watchdog_tx: mpsc::Sender<Event>,
    longterm_tx: mpsc::Sender<Event>,
    midterm_tx: mpsc::Sender<Event>,
    shortterm_tx: mpsc::Sender<Event>,
}

impl TopicRouter {
    pub fn new(
        event_rx: mpsc::Receiver<Event>,
        watchdog_tx: mpsc::Sender<Event>,
        longterm_tx: mpsc::Sender<Event>,
        midterm_tx: mpsc::Sender<Event>,
        shortterm_tx: mpsc::Sender<Event>,
    ) -> Self {
        Self {
            event_rx,
            watchdog_tx,
            longterm_tx,
            midterm_tx,
            shortterm_tx,
        }
    }

    /// Run the router dispatch loop.
    pub async fn run(&mut self) -> Result<()> {
        while let Some(event) = self.event_rx.recv().await {
            self.dispatch(event).await?;
        }
        Ok(())
    }

    async fn dispatch(&self, event: Event) -> Result<()> {
        match &event {
            // Price ticks → ShortTerm (real-time), others get periodic snapshots
            Event::Price(_) => {
                let _ = self.shortterm_tx.send(event.clone()).await;
                let _ = self.midterm_tx.send(event.clone()).await;
                let _ = self.longterm_tx.send(event).await;
            }

            // Funding → ShortTerm (real-time) + MidTerm
            Event::Funding(_) => {
                let _ = self.shortterm_tx.send(event.clone()).await;
                let _ = self.midterm_tx.send(event).await;
            }

            // Liquidations → ShortTerm only
            Event::Liquidation(_) => {
                let _ = self.shortterm_tx.send(event).await;
            }

            // Open interest → MidTerm
            Event::OpenInterest(_) => {
                let _ = self.midterm_tx.send(event).await;
            }

            // On-chain metrics → LongTerm
            Event::OnChain(_) => {
                let _ = self.longterm_tx.send(event).await;
            }

            // News → MidTerm + LongTerm + WatchDog
            Event::News(_) => {
                let _ = self.watchdog_tx.send(event.clone()).await;
                let _ = self.midterm_tx.send(event.clone()).await;
                let _ = self.longterm_tx.send(event.clone()).await;
                let _ = self.shortterm_tx.send(event).await;
            }

            // Drawdown → WatchDog directly (kill switch path)
            Event::DrawdownAlert(_) => {
                let _ = self.watchdog_tx.send(event).await;
            }

            // Macro calendar → LongTerm + WatchDog
            Event::MacroCalendar(_) => {
                let _ = self.watchdog_tx.send(event.clone()).await;
                let _ = self.longterm_tx.send(event).await;
            }

            // Memory reload → all agents
            Event::MemoryReload => {
                let _ = self.watchdog_tx.send(event.clone()).await;
                let _ = self.longterm_tx.send(event.clone()).await;
                let _ = self.midterm_tx.send(event.clone()).await;
                let _ = self.shortterm_tx.send(event).await;
            }
        }

        debug!("Event dispatched");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    struct RouterTestHarness {
        event_tx: mpsc::Sender<Event>,
        watchdog_rx: mpsc::Receiver<Event>,
        longterm_rx: mpsc::Receiver<Event>,
        midterm_rx: mpsc::Receiver<Event>,
        shortterm_rx: mpsc::Receiver<Event>,
    }

    impl RouterTestHarness {
        fn new() -> (Self, TopicRouter) {
            let (event_tx, event_rx) = mpsc::channel(16);
            let (watchdog_tx, watchdog_rx) = mpsc::channel(16);
            let (longterm_tx, longterm_rx) = mpsc::channel(16);
            let (midterm_tx, midterm_rx) = mpsc::channel(16);
            let (shortterm_tx, shortterm_rx) = mpsc::channel(16);

            let router =
                TopicRouter::new(event_rx, watchdog_tx, longterm_tx, midterm_tx, shortterm_tx);

            (
                Self {
                    event_tx,
                    watchdog_rx,
                    longterm_rx,
                    midterm_rx,
                    shortterm_rx,
                },
                router,
            )
        }
    }

    #[tokio::test]
    async fn price_routes_to_three_agents() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::Price(PriceEvent {
            symbol: Symbol::BTC,
            price: 50000.0,
            volume_24h: 1e9,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx); // close channel so router.run() ends
        router.run().await.unwrap();
        assert!(harness.shortterm_rx.try_recv().is_ok());
        assert!(harness.midterm_rx.try_recv().is_ok());
        assert!(harness.longterm_rx.try_recv().is_ok());
        assert!(harness.watchdog_rx.try_recv().is_err()); // not routed
    }

    #[tokio::test]
    async fn liquidation_routes_to_shortterm_only() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::Liquidation(LiquidationEvent {
            symbol: Symbol::BTC,
            direction: Direction::Long,
            size_usd: 100000.0,
            price: 49000.0,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.shortterm_rx.try_recv().is_ok());
        assert!(harness.midterm_rx.try_recv().is_err());
        assert!(harness.longterm_rx.try_recv().is_err());
        assert!(harness.watchdog_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn onchain_routes_to_longterm_only() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::OnChain(OnChainEvent {
            metric: "MVRV_zscore".into(),
            value: 0.8,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.longterm_rx.try_recv().is_ok());
        assert!(harness.shortterm_rx.try_recv().is_err());
        assert!(harness.midterm_rx.try_recv().is_err());
        assert!(harness.watchdog_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn drawdown_routes_to_watchdog_only() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::DrawdownAlert(DrawdownEvent {
            current_drawdown_pct: 16.0,
            threshold_pct: 15.0,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.watchdog_rx.try_recv().is_ok());
        assert!(harness.shortterm_rx.try_recv().is_err());
        assert!(harness.midterm_rx.try_recv().is_err());
        assert!(harness.longterm_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn news_routes_to_all_agents() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::News(TextEvent {
            source: "test".into(),
            headline: "BTC pump".into(),
            sentiment: 0.9,
            urgency: 0.5,
            dedup_hash: 12345,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.watchdog_rx.try_recv().is_ok());
        assert!(harness.midterm_rx.try_recv().is_ok());
        assert!(harness.longterm_rx.try_recv().is_ok());
        assert!(harness.shortterm_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn macro_calendar_routes_to_watchdog_and_longterm() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::MacroCalendar(MacroEvent {
            event_name: "FOMC".into(),
            scheduled_at: Utc::now(),
            impact: MacroImpact::High,
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.watchdog_rx.try_recv().is_ok());
        assert!(harness.longterm_rx.try_recv().is_ok());
        assert!(harness.midterm_rx.try_recv().is_err());
        assert!(harness.shortterm_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn funding_routes_to_shortterm_and_midterm() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::Funding(FundingEvent {
            symbol: Symbol::BTC,
            rate: 0.01,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.shortterm_rx.try_recv().is_ok());
        assert!(harness.midterm_rx.try_recv().is_ok());
        assert!(harness.longterm_rx.try_recv().is_err());
        assert!(harness.watchdog_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn open_interest_routes_to_midterm_only() {
        let (mut harness, mut router) = RouterTestHarness::new();
        let event = Event::OpenInterest(OpenInterestEvent {
            symbol: Symbol::BTC,
            oi_usd: 5e9,
            change_pct: 2.5,
            timestamp: Utc::now(),
        });
        harness.event_tx.send(event).await.unwrap();
        drop(harness.event_tx);
        router.run().await.unwrap();
        assert!(harness.midterm_rx.try_recv().is_ok());
        assert!(harness.shortterm_rx.try_recv().is_err());
        assert!(harness.longterm_rx.try_recv().is_err());
        assert!(harness.watchdog_rx.try_recv().is_err());
    }
}
