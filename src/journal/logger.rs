use tracing::info;

/// JSONL trade journal — appends every trade as structured JSON.
/// Phase 2: full implementation with journal.jsonl output.
pub struct TradeLogger {
    _path: String,
}

impl TradeLogger {
    pub fn new(path: &str) -> Self {
        info!(path = path, "TradeLogger initialized (Phase 1 stub)");
        Self {
            _path: path.to_string(),
        }
    }
}
