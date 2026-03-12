use anyhow::Result;
use async_trait::async_trait;

use crate::memory::store::MemoryStore;
use crate::types::{AgentName, AgentStatus, Event, TradeProposal};

#[async_trait]
pub trait Agent: Send + Sync {
    /// Unique name of this agent.
    fn name(&self) -> AgentName;

    /// Current status for TUI display.
    fn status(&self) -> AgentStatus;

    /// Called when a routed event arrives.
    async fn on_event(&mut self, event: Event) -> Result<()>;

    /// Generate a trade proposal (or None if no signal).
    async fn propose(&self) -> Result<Option<TradeProposal>>;

    /// Reload lessons from the shared memory store.
    async fn load_memory(&mut self, memory: &MemoryStore) -> Result<()>;
}
