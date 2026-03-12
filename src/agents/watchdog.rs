use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::agents::traits::Agent;
use crate::memory::store::MemoryStore;
use crate::risk::manager::RiskManager;
use crate::types::*;

pub struct WatchDogAgent {
    status: AgentStatus,
    portfolio: Portfolio,
    risk_manager: RiskManager,
    proposal_rx: mpsc::Receiver<TradeProposal>,
    decision_log: Vec<TradeDecision>,
    kill_switch_triggered: bool,
}

impl WatchDogAgent {
    pub fn new(
        risk_manager: RiskManager,
        proposal_rx: mpsc::Receiver<TradeProposal>,
        initial_equity: f64,
    ) -> Self {
        Self {
            status: AgentStatus::Active,
            portfolio: Portfolio {
                total_equity_usd: initial_equity,
                free_cash_usd: initial_equity,
                peak_equity: initial_equity,
                ..Default::default()
            },
            risk_manager,
            proposal_rx,
            decision_log: Vec::new(),
            kill_switch_triggered: false,
        }
    }

    /// Main loop: listen for proposals and events concurrently.
    pub async fn run(
        &mut self,
        mut event_rx: mpsc::Receiver<Event>,
        _memory_store: MemoryStore,
    ) -> Result<()> {
        loop {
            tokio::select! {
                Some(proposal) = self.proposal_rx.recv() => {
                    self.handle_proposal(proposal).await?;
                }
                Some(event) = event_rx.recv() => {
                    self.on_event(event).await?;
                }
                else => break,
            }
        }
        Ok(())
    }

    async fn handle_proposal(&mut self, proposal: TradeProposal) -> Result<()> {
        let decision = self.risk_manager.evaluate(&proposal, &self.portfolio);
        info!(
            agent = %proposal.proposer,
            symbol = %proposal.symbol,
            verdict = ?decision.verdict,
            "Trade decision"
        );

        if decision.verdict == Verdict::Accept || decision.verdict == Verdict::Adjust {
            // TODO: execute trade via Hyperliquid
            info!("Would execute trade: {:?}", decision);
        }

        self.decision_log.push(decision);
        Ok(())
    }

    pub fn trigger_kill_switch(&mut self) {
        warn!("KILL SWITCH TRIGGERED — closing all positions");
        self.kill_switch_triggered = true;
        self.status = AgentStatus::KillSwitchTriggered;
        // TODO: close all positions on Hyperliquid
    }

    pub fn portfolio(&self) -> &Portfolio {
        &self.portfolio
    }

    pub fn decision_log(&self) -> &[TradeDecision] {
        &self.decision_log
    }
}

#[async_trait]
impl Agent for WatchDogAgent {
    fn name(&self) -> AgentName {
        AgentName::WatchDog
    }

    fn status(&self) -> AgentStatus {
        self.status
    }

    async fn on_event(&mut self, event: Event) -> Result<()> {
        match &event {
            Event::DrawdownAlert(da) => {
                if da.current_drawdown_pct >= da.threshold_pct {
                    self.trigger_kill_switch();
                }
            }
            Event::News(text) => {
                if text.urgency > 0.8 {
                    warn!(headline = %text.headline, "High-urgency news — reducing exposure");
                    // TODO: reduce position sizes
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn propose(&self) -> Result<Option<TradeProposal>> {
        // WatchDog does not propose trades; it only evaluates.
        Ok(None)
    }

    async fn load_memory(&mut self, _memory: &MemoryStore) -> Result<()> {
        // WatchDog reads memory for risk tuning
        Ok(())
    }
}
