use chrono::{Datelike, Utc};
use tracing::error;

/// Kill switch: emergency close all positions.
pub struct KillSwitch {
    pub triggered: bool,
    triggered_day: Option<u32>,
}

impl KillSwitch {
    pub fn new() -> Self {
        Self {
            triggered: false,
            triggered_day: None,
        }
    }

    pub fn trigger(&mut self) {
        error!("KILL SWITCH ACTIVATED");
        self.triggered = true;
        self.triggered_day = Some(Utc::now().ordinal());
    }

    /// Reset kill switch (e.g. at UTC midnight for auto recovery).
    pub fn reset(&mut self) {
        self.triggered = false;
        self.triggered_day = None;
    }

    /// Day when kill switch was triggered (for auto reset at midnight).
    pub fn triggered_day(&self) -> Option<u32> {
        self.triggered_day
    }
}
