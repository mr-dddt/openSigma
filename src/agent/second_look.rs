use chrono::{DateTime, Utc};

use crate::types::{Direction, LlmDecision};

/// SecondLook scheduler: manages deferred re-checks for better trade timing.
pub struct SecondLookScheduler {
    pending: Vec<SecondLookEntry>,
    max_second_looks: u32,
}

#[allow(dead_code)]
pub struct SecondLookEntry {
    pub recheck_at: DateTime<Utc>,
    pub original_bias: Direction,
    pub what_to_watch: String,
    pub attempt: u32,
}

impl SecondLookScheduler {
    pub fn new(max_second_looks: u32) -> Self {
        Self {
            pending: Vec::new(),
            max_second_looks,
        }
    }

    /// Schedule a SecondLook from an LLM decision. Returns false if max attempts reached.
    pub fn schedule(&mut self, decision: &LlmDecision) -> bool {
        if let LlmDecision::SecondLook {
            recheck_after_secs,
            what_to_watch,
            original_bias,
            ..
        } = decision
        {
            let existing = self
                .pending
                .iter()
                .filter(|e| e.original_bias == *original_bias)
                .count() as u32;

            if existing >= self.max_second_looks {
                return false;
            }

            self.pending.push(SecondLookEntry {
                recheck_at: Utc::now()
                    + chrono::Duration::seconds(*recheck_after_secs as i64),
                original_bias: *original_bias,
                what_to_watch: what_to_watch.clone(),
                attempt: existing + 1,
            });
            true
        } else {
            false
        }
    }

    /// Return all entries whose recheck time has passed, removing them from pending.
    pub fn poll_due(&mut self) -> Vec<SecondLookEntry> {
        let now = Utc::now();
        let (due, remaining): (Vec<_>, Vec<_>) =
            self.pending.drain(..).partition(|e| e.recheck_at <= now);
        self.pending = remaining;
        due
    }

    #[allow(dead_code)]
    pub fn clear_all(&mut self) {
        self.pending.clear();
    }

    #[allow(dead_code)]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}
