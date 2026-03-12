use tracing::info;

/// Polymarket order executor — places binary market orders.
/// Phase 2: will use polymarket-client-sdk for maker limit orders + settlement tracking.
pub struct PmExecutor {
    _private_key: String,
}

impl PmExecutor {
    pub fn new(private_key: String) -> Self {
        info!("PmExecutor initialized (Phase 1 stub)");
        Self {
            _private_key: private_key,
        }
    }
}
