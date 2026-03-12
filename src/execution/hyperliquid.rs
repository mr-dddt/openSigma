use tracing::info;

/// Hyperliquid order executor — places perp orders via REST API with EIP-712 signing.
/// Phase 2: will use hyperliquid-rust-sdk for order placement, stops, position monitoring.
pub struct HlExecutor {
    _private_key: String,
}

impl HlExecutor {
    pub fn new(private_key: String) -> Self {
        info!("HlExecutor initialized (Phase 1 stub)");
        Self {
            _private_key: private_key,
        }
    }
}
