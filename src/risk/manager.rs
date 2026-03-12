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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_limits() -> HashMap<AgentName, RiskLimits> {
        let mut m = HashMap::new();
        m.insert(
            AgentName::MidTerm,
            RiskLimits {
                max_leverage: 5.0,
                max_per_trade_pct: 5.0,
                max_total_exposure_pct: 20.0,
            },
        );
        m.insert(
            AgentName::ShortTerm,
            RiskLimits {
                max_leverage: 20.0,
                max_per_trade_pct: 2.0,
                max_total_exposure_pct: 10.0,
            },
        );
        m.insert(
            AgentName::LongTerm,
            RiskLimits {
                max_leverage: 2.0,
                max_per_trade_pct: 6.0,
                max_total_exposure_pct: 100.0,
            },
        );
        m
    }

    fn make_proposal(agent: AgentName, size: f64, leverage: f64) -> TradeProposal {
        TradeProposal {
            id: Uuid::new_v4(),
            proposer: agent,
            symbol: Symbol::BTC,
            direction: Direction::Long,
            size_usd: size,
            leverage,
            entry_price: 50000.0,
            stop_loss: 47500.0,
            take_profit: 55000.0,
            rationale: "test".into(),
            signals: vec![],
            timestamp: Utc::now(),
        }
    }

    fn empty_portfolio(equity: f64) -> Portfolio {
        Portfolio {
            total_equity_usd: equity,
            free_cash_usd: equity,
            positions: vec![],
            realized_pnl: 0.0,
            peak_equity: equity,
        }
    }

    #[test]
    fn accept_valid_proposal() {
        let rm = RiskManager::new(make_limits());
        let proposal = make_proposal(AgentName::MidTerm, 400.0, 3.0);
        let portfolio = empty_portfolio(10000.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Accept);
    }

    #[test]
    fn reject_excess_leverage() {
        let rm = RiskManager::new(make_limits());
        let proposal = make_proposal(AgentName::MidTerm, 400.0, 6.0); // max is 5x
        let portfolio = empty_portfolio(10000.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Reject);
        assert!(decision.reason.contains("Leverage"));
    }

    #[test]
    fn adjust_oversized_trade() {
        let rm = RiskManager::new(make_limits());
        // MidTerm max_per_trade = 5% of 10000 = 500
        let proposal = make_proposal(AgentName::MidTerm, 800.0, 3.0);
        let portfolio = empty_portfolio(10000.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Adjust);
        assert_eq!(decision.adjusted_size, Some(500.0));
    }

    #[test]
    fn adjust_when_exposure_exceeded() {
        let rm = RiskManager::new(make_limits());
        // MidTerm max_total_exposure = 20% of 10000 = 2000
        // Already have 1800 exposure
        let mut portfolio = empty_portfolio(10000.0);
        portfolio.positions.push(Position {
            id: Uuid::new_v4(),
            proposer: AgentName::MidTerm,
            symbol: Symbol::BTC,
            direction: Direction::Long,
            size_usd: 1800.0,
            leverage: 3.0,
            entry_price: 50000.0,
            stop_loss: 47500.0,
            take_profit: 55000.0,
            opened_at: Utc::now(),
            unrealized_pnl: 0.0,
        });
        let proposal = make_proposal(AgentName::MidTerm, 400.0, 3.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Adjust);
        // remaining = 2000 - 1800 = 200
        assert_eq!(decision.adjusted_size, Some(200.0));
    }

    #[test]
    fn reject_at_max_exposure() {
        let rm = RiskManager::new(make_limits());
        let mut portfolio = empty_portfolio(10000.0);
        portfolio.positions.push(Position {
            id: Uuid::new_v4(),
            proposer: AgentName::MidTerm,
            symbol: Symbol::BTC,
            direction: Direction::Long,
            size_usd: 2000.0, // exactly at 20% cap
            leverage: 3.0,
            entry_price: 50000.0,
            stop_loss: 47500.0,
            take_profit: 55000.0,
            opened_at: Utc::now(),
            unrealized_pnl: 0.0,
        });
        let proposal = make_proposal(AgentName::MidTerm, 100.0, 3.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Reject);
        assert!(decision.reason.contains("max total exposure"));
    }

    #[test]
    fn reject_unknown_agent() {
        let rm = RiskManager::new(HashMap::new()); // no limits defined
        let proposal = make_proposal(AgentName::MidTerm, 100.0, 1.0);
        let portfolio = empty_portfolio(10000.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Reject);
        assert!(decision.reason.contains("Unknown agent"));
    }

    #[test]
    fn accept_at_exact_limit() {
        let rm = RiskManager::new(make_limits());
        // MidTerm: exactly 5% of 10000 = 500, exactly 5x leverage
        let proposal = make_proposal(AgentName::MidTerm, 500.0, 5.0);
        let portfolio = empty_portfolio(10000.0);
        let decision = rm.evaluate(&proposal, &portfolio);
        assert_eq!(decision.verdict, Verdict::Accept);
    }
}
