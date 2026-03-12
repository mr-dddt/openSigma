use tracing::info;

/// Claude Sonnet API client for trade decision gating.
/// Phase 2: will send signal context to Claude and parse structured JSON response.
pub struct LlmClient {
    _api_key: String,
    _model: String,
}

impl LlmClient {
    pub fn new(api_key: String, model: String) -> Self {
        info!(model = %model, "LlmClient initialized (Phase 1 stub)");
        Self {
            _api_key: api_key,
            _model: model,
        }
    }
}
