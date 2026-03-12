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
