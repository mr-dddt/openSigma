use anyhow::Result;
use std::collections::HashSet;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::types::*;

/// Layer 2: LLM-based filter for unstructured text data.
/// Normalizes news/tweets/whale alerts into typed TextEvents.
/// Runs async, off the critical execution path.
pub struct LlmFilter {
    event_tx: mpsc::Sender<Event>,
    seen_hashes: HashSet<u64>,
    _api_key: String,
}

impl LlmFilter {
    pub fn new(event_tx: mpsc::Sender<Event>, api_key: String) -> Self {
        Self {
            event_tx,
            seen_hashes: HashSet::new(),
            _api_key: api_key,
        }
    }

    /// Start the LLM filter loop.
    /// Phase 0 stub — will poll Cryptopanic / SoSoValue in Phase 1.
    pub async fn run(&mut self) -> Result<()> {
        info!("LlmFilter started (Phase 0 stub)");
        // TODO Phase 1:
        // 1. Poll news sources periodically
        // 2. Hash content for dedup
        // 3. Send to Claude Haiku for sentiment + urgency scoring
        // 4. Emit Event::News with structured TextEvent
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
            debug!("LlmFilter heartbeat");
        }
    }

    /// Dedup check: returns true if this content hash has been seen.
    pub fn is_duplicate(&mut self, hash: u64) -> bool {
        !self.seen_hashes.insert(hash)
    }
}
