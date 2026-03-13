use anyhow::Result;
use tracing::error;

use crate::execution::hyperliquid::HlExecutor;
use crate::execution::polymarket::PmExecutor;

/// Kill switch: emergency close all positions.
pub struct KillSwitch {
    pub triggered: bool,
}

impl KillSwitch {
    pub fn new() -> Self {
        Self { triggered: false }
    }

    /// Trigger kill switch and close all positions on both exchanges.
    pub async fn trigger_with_executors(
        &mut self,
        hl: &HlExecutor,
        pm: &PmExecutor,
    ) -> Result<()> {
        error!("KILL SWITCH ACTIVATED — closing all positions");
        self.triggered = true;

        if let Err(e) = hl.close_all().await {
            error!("Failed to close HL positions: {e:#}");
        }
        if let Err(e) = pm.cancel_all().await {
            error!("Failed to cancel PM orders: {e:#}");
        }

        Ok(())
    }

    pub fn trigger(&mut self) {
        error!("KILL SWITCH ACTIVATED");
        self.triggered = true;
    }
}
