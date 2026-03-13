use anyhow::Result;
use tracing::error;

use crate::execution::hyperliquid::HlExecutor;

/// Kill switch: emergency close all positions.
pub struct KillSwitch {
    pub triggered: bool,
}

impl KillSwitch {
    pub fn new() -> Self {
        Self { triggered: false }
    }

    /// Trigger kill switch and close all positions.
    #[allow(dead_code)]
    pub async fn trigger_with_executor(
        &mut self,
        hl: &HlExecutor,
    ) -> Result<()> {
        error!("KILL SWITCH ACTIVATED — closing all positions");
        self.triggered = true;

        if let Err(e) = hl.close_all().await {
            error!("Failed to close HL positions: {e:#}");
        }

        Ok(())
    }

    pub fn trigger(&mut self) {
        error!("KILL SWITCH ACTIVATED");
        self.triggered = true;
    }
}
