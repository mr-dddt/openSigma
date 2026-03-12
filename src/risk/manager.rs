use chrono::Utc;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::types::*;

/// Central risk manager used by WatchDog to validate trade proposals.
pub struct RiskManager {
    limits: HashMap<AgentName, RiskLimits>,
}

impl RiskManager {
    pub fn new(limits: HashMap<AgentName, RiskLimits>) -> Self {
        Self { limits }
    }

    /// Evaluate a trade proposal against risk limits and portfolio state.
    pub fn evaluate(&self, proposal: &TradeProposal, portfolio: &Portfolio) -> TradeDecision {
        let limits = match self.limits.get(&proposal.proposer) {
            Some(l) => l,
            None => {
                return self.reject(proposal, "Unknown agent — no risk limits defined");
            }
        };

        // Check leverage
        if proposal.leverage > limits.max_leverage {
            return self.reject(
                proposal,
                &format!(
                    "Leverage {:.1}x exceeds max {:.1}x for {}",
                    proposal.leverage, limits.max_leverage, proposal.proposer
                ),
            );
        }

        // Check per-trade size
        let max_size = portfolio.total_equity_usd * (limits.max_per_trade_pct / 100.0);
        if proposal.size_usd > max_size {
            // Try to adjust down instead of rejecting
            info!(
                "Adjusting size from {:.2} to {:.2} for {}",
                proposal.size_usd, max_size, proposal.proposer
            );
            return TradeDecision {
                proposal: proposal.clone(),
                verdict: Verdict::Adjust,
                reason: format!(
                    "Size reduced from {:.2} to {:.2} (max {:.1}% per trade)",
                    proposal.size_usd, max_size, limits.max_per_trade_pct
                ),
                adjusted_size: Some(max_size),
                adjusted_leverage: None,
                timestamp: Utc::now(),
            };
        }

        // Check total exposure for this agent
        let current_exposure = portfolio.agent_exposure(proposal.proposer);
        let max_exposure = portfolio.total_equity_usd * (limits.max_total_exposure_pct / 100.0);
        if current_exposure + proposal.size_usd > max_exposure {
            let remaining = (max_exposure - current_exposure).max(0.0);
            if remaining < 1.0 {
                return self.reject(
                    proposal,
                    &format!(
                        "{} at max total exposure ({:.1}%)",
                        proposal.proposer, limits.max_total_exposure_pct
                    ),
                );
            }
            warn!(
                "Adjusting size to fit within total exposure cap for {}",
                proposal.proposer
            );
            return TradeDecision {
                proposal: proposal.clone(),
                verdict: Verdict::Adjust,
                reason: format!(
                    "Size reduced to {:.2} to stay within {:.1}% total exposure",
                    remaining, limits.max_total_exposure_pct
                ),
                adjusted_size: Some(remaining),
                adjusted_leverage: None,
                timestamp: Utc::now(),
            };
        }

        // All checks passed
        TradeDecision {
            proposal: proposal.clone(),
            verdict: Verdict::Accept,
            reason: "All risk checks passed".into(),
            adjusted_size: None,
            adjusted_leverage: None,
            timestamp: Utc::now(),
        }
    }

    fn reject(&self, proposal: &TradeProposal, reason: &str) -> TradeDecision {
        warn!(
            agent = %proposal.proposer,
            symbol = %proposal.symbol,
            reason = reason,
            "Trade REJECTED"
        );
        TradeDecision {
            proposal: proposal.clone(),
            verdict: Verdict::Reject,
            reason: reason.to_string(),
            adjusted_size: None,
            adjusted_leverage: None,
            timestamp: Utc::now(),
        }
    }
}
