use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Symbols & directions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Symbol {
    BTC,
    ETH,
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Symbol::BTC => write!(f, "BTC"),
            Symbol::ETH => write!(f, "ETH"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Long,
    Short,
}

// ---------------------------------------------------------------------------
// Agent identity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentName {
    WatchDog,
    LongTerm,
    MidTerm,
    ShortTerm,
}

impl std::fmt::Display for AgentName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentName::WatchDog => write!(f, "WatchDog"),
            AgentName::LongTerm => write!(f, "LongTerm"),
            AgentName::MidTerm => write!(f, "MidTerm"),
            AgentName::ShortTerm => write!(f, "ShortTerm"),
        }
    }
}

// ---------------------------------------------------------------------------
// Events — unified schema for the event bus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Price(PriceEvent),
    Funding(FundingEvent),
    Liquidation(LiquidationEvent),
    OpenInterest(OpenInterestEvent),
    OnChain(OnChainEvent),
    News(TextEvent),
    DrawdownAlert(DrawdownEvent),
    MacroCalendar(MacroEvent),
    MemoryReload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceEvent {
    pub symbol: Symbol,
    pub price: f64,
    pub volume_24h: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingEvent {
    pub symbol: Symbol,
    pub rate: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationEvent {
    pub symbol: Symbol,
    pub direction: Direction,
    pub size_usd: f64,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenInterestEvent {
    pub symbol: Symbol,
    pub oi_usd: f64,
    pub change_pct: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnChainEvent {
    pub metric: String, // e.g. "MVRV", "NUPL", "Puell"
    pub value: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEvent {
    pub source: String,
    pub headline: String,
    pub sentiment: f64,  // -1.0 to 1.0
    pub urgency: f64,    // 0.0 to 1.0
    pub dedup_hash: u64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawdownEvent {
    pub current_drawdown_pct: f64,
    pub threshold_pct: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroEvent {
    pub event_name: String, // e.g. "FOMC", "CPI"
    pub scheduled_at: DateTime<Utc>,
    pub impact: MacroImpact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MacroImpact {
    Low,
    Medium,
    High,
}

// ---------------------------------------------------------------------------
// Trade proposal & decision
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeProposal {
    pub id: Uuid,
    pub proposer: AgentName,
    pub symbol: Symbol,
    pub direction: Direction,
    pub size_usd: f64,
    pub leverage: f64,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub rationale: String,
    pub signals: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Accept,
    Reject,
    Adjust,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDecision {
    pub proposal: TradeProposal,
    pub verdict: Verdict,
    pub reason: String,
    pub adjusted_size: Option<f64>,
    pub adjusted_leverage: Option<f64>,
    pub timestamp: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Portfolio state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: Uuid,
    pub proposer: AgentName,
    pub symbol: Symbol,
    pub direction: Direction,
    pub size_usd: f64,
    pub leverage: f64,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub opened_at: DateTime<Utc>,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Portfolio {
    pub total_equity_usd: f64,
    pub free_cash_usd: f64,
    pub positions: Vec<Position>,
    pub realized_pnl: f64,
    pub peak_equity: f64,
}

impl Portfolio {
    pub fn drawdown_pct(&self) -> f64 {
        if self.peak_equity <= 0.0 {
            return 0.0;
        }
        ((self.peak_equity - self.total_equity_usd) / self.peak_equity) * 100.0
    }

    pub fn agent_exposure(&self, agent: AgentName) -> f64 {
        self.positions
            .iter()
            .filter(|p| p.proposer == agent)
            .map(|p| p.size_usd)
            .sum()
    }
}

// ---------------------------------------------------------------------------
// Agent status (for TUI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    Active,
    Watching,
    InTrade,
    Paused,
    KillSwitchTriggered,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Active => write!(f, "ACTIVE"),
            AgentStatus::Watching => write!(f, "WATCHING"),
            AgentStatus::InTrade => write!(f, "IN TRADE"),
            AgentStatus::Paused => write!(f, "PAUSED"),
            AgentStatus::KillSwitchTriggered => write!(f, "KILLED"),
        }
    }
}

// ---------------------------------------------------------------------------
// Risk limits (per agent)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RiskLimits {
    pub max_leverage: f64,
    pub max_per_trade_pct: f64,
    pub max_total_exposure_pct: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_position(agent: AgentName, size: f64) -> Position {
        Position {
            id: Uuid::new_v4(),
            proposer: agent,
            symbol: Symbol::BTC,
            direction: Direction::Long,
            size_usd: size,
            leverage: 1.0,
            entry_price: 50000.0,
            stop_loss: 47500.0,
            take_profit: 55000.0,
            opened_at: Utc::now(),
            unrealized_pnl: 0.0,
        }
    }

    #[test]
    fn drawdown_pct_no_drawdown() {
        let p = Portfolio {
            total_equity_usd: 10000.0,
            peak_equity: 10000.0,
            ..Default::default()
        };
        assert!((p.drawdown_pct() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn drawdown_pct_with_loss() {
        let p = Portfolio {
            total_equity_usd: 8500.0,
            peak_equity: 10000.0,
            ..Default::default()
        };
        assert!((p.drawdown_pct() - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn drawdown_pct_zero_peak() {
        let p = Portfolio {
            total_equity_usd: 0.0,
            peak_equity: 0.0,
            ..Default::default()
        };
        assert!((p.drawdown_pct() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn agent_exposure_empty() {
        let p = Portfolio::default();
        assert!((p.agent_exposure(AgentName::MidTerm) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn agent_exposure_filters_by_agent() {
        let p = Portfolio {
            positions: vec![
                make_position(AgentName::MidTerm, 500.0),
                make_position(AgentName::MidTerm, 300.0),
                make_position(AgentName::ShortTerm, 200.0),
            ],
            ..Default::default()
        };
        assert!((p.agent_exposure(AgentName::MidTerm) - 800.0).abs() < f64::EPSILON);
        assert!((p.agent_exposure(AgentName::ShortTerm) - 200.0).abs() < f64::EPSILON);
        assert!((p.agent_exposure(AgentName::LongTerm) - 0.0).abs() < f64::EPSILON);
    }
}
