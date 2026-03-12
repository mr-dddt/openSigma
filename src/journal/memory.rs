use tracing::info;

/// Memory system — reads/writes memory.md for LLM system prompt context.
/// Phase 3: full implementation with 20-trade report generation.
pub struct MemoryManager {
    _path: String,
}

impl MemoryManager {
    pub fn new(path: &str) -> Self {
        info!(path = path, "MemoryManager initialized (Phase 1 stub)");
        Self {
            _path: path.to_string(),
        }
    }
}
