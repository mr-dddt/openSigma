use tracing::error;

/// Kill switch: emergency close all positions.
/// Phase 2: will call HlExecutor::close_all + PmExecutor::cancel_all.
pub struct KillSwitch {
    pub triggered: bool,
}

impl KillSwitch {
    pub fn new() -> Self {
        Self { triggered: false }
    }

    pub fn trigger(&mut self) {
        error!("KILL SWITCH ACTIVATED — closing all positions");
        self.triggered = true;
        // Phase 2: close all HL positions, cancel all PM orders
    }
}
