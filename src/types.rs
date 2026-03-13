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

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::Long => write!(f, "Long"),
            Direction::Short => write!(f, "Short"),
        }
    }
}

// ---------------------------------------------------------------------------
// Market data types (from data feeds)
// ---------------------------------------------------------------------------

#[allow(dead_code)] // fields populated by data feed
#[derive(Debug, Clone)]
pub struct PriceTick {
    pub symbol: Symbol,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

#[allow(dead_code)] // fields populated by data feed
#[derive(Debug, Clone)]
pub struct TradeTick {
    pub symbol: Symbol,
    pub price: f64,
    pub size: f64,
    pub side: Direction,
    pub timestamp: DateTime<Utc>,
}

#[allow(dead_code)] // fields populated by data feed
#[derive(Debug, Clone)]
pub struct OrderBookSnapshot {
    pub symbol: Symbol,
    pub bids: Vec<(f64, f64)>, // (price, size)
    pub asks: Vec<(f64, f64)>,
    pub timestamp: DateTime<Utc>,
}

#[allow(dead_code)] // fields used by data feed structs
#[derive(Debug, Clone)]
pub struct FundingTick {
    pub symbol: Symbol,
    pub rate: f64,
    pub timestamp: DateTime<Utc>,
}

#[allow(dead_code)] // Phase 2 stub — will be wired when liquidation feed is added
#[derive(Debug, Clone)]
pub struct LiquidationTick {
    pub symbol: Symbol,
    pub direction: Direction,
    pub size_usd: f64,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

/// Candle (OHLCV) for indicator computation.
#[allow(dead_code)] // some fields used internally by indicators
#[derive(Debug, Clone)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub timestamp: DateTime<Utc>,
}

/// Polymarket binary market odds.
#[allow(dead_code)] // fields used by PM feed
#[derive(Debug, Clone)]
pub struct PmOdds {
    pub market_id: String,
    pub window: PmWindow,
    pub up_price: f64,
    pub down_price: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PmWindow {
    FiveMin,
    FifteenMin,
}

impl std::fmt::Display for PmWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PmWindow::FiveMin => write!(f, "5m"),
            PmWindow::FifteenMin => write!(f, "15m"),
        }
    }
}

// ---------------------------------------------------------------------------
// Unified market data event (sent through channels)
// ---------------------------------------------------------------------------

#[allow(dead_code)] // Liquidation and PmOdds variants are Phase 2 stubs
#[derive(Debug, Clone)]
pub enum MarketEvent {
    Price(PriceTick),
    Trade(TradeTick),
    OrderBook(OrderBookSnapshot),
    Funding(FundingTick),
    Liquidation(LiquidationTick),
    PmOdds(PmOdds),
}

// ---------------------------------------------------------------------------
// Signal engine types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalLevel {
    StrongLong,
    LeanLong,
    Weak,
    LeanShort,
    StrongShort,
    NoTrade,
}

impl std::fmt::Display for SignalLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalLevel::StrongLong => write!(f, "STRONG_LONG"),
            SignalLevel::LeanLong => write!(f, "LEAN_LONG"),
            SignalLevel::Weak => write!(f, "WEAK"),
            SignalLevel::LeanShort => write!(f, "LEAN_SHORT"),
            SignalLevel::StrongShort => write!(f, "STRONG_SHORT"),
            SignalLevel::NoTrade => write!(f, "NO_TRADE"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalSnapshot {
    pub bull_score: i32,
    pub bear_score: i32,
    pub net_score: i32,
    pub level: SignalLevel,
    pub filter_reason: Option<String>,
    pub indicators: IndicatorValues,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IndicatorValues {
    pub ema_9: Option<f64>,
    pub ema_21: Option<f64>,
    pub rsi_14: Option<f64>,
    pub stoch_rsi: Option<f64>,
    pub bb_upper: Option<f64>,
    pub bb_lower: Option<f64>,
    pub bb_squeeze: bool,
    pub atr_14: Option<f64>,
    pub atr_pct: Option<f64>,
    pub cvd: Option<f64>,
    pub ob_imbalance: Option<f64>,
    pub pm_divergence: Option<f64>,
}

// ---------------------------------------------------------------------------
// Play types & LLM decisions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayType {
    PurePerpScalp,
    PureBinaryBet,
    HedgedPerp,
    BinaryArbitrage,
    CrossMarketMomentum,
}

impl std::fmt::Display for PlayType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayType::PurePerpScalp => write!(f, "PurePerpScalp"),
            PlayType::PureBinaryBet => write!(f, "PureBinaryBet"),
            PlayType::HedgedPerp => write!(f, "HedgedPerp"),
            PlayType::BinaryArbitrage => write!(f, "BinaryArbitrage"),
            PlayType::CrossMarketMomentum => write!(f, "CrossMarketMomentum"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmDecision {
    Execute {
        play_type: PlayType,
        direction: Direction,
        size_pct: f64,
        hl_leverage: Option<u8>,
        stop_loss_pct: f64,
        take_profit_pct: f64,
        pm_hedge: Option<PmHedge>,
        reasoning: String,
    },
    Skip {
        reasoning: String,
    },
    SecondLook {
        recheck_after_secs: u64,
        what_to_watch: String,
        original_bias: Direction,
        reasoning: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PmHedge {
    pub side: BinarySide,
    pub budget_usd: f64,
    pub max_price: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinarySide {
    Up,
    Down,
}

// ---------------------------------------------------------------------------
// Tuning types (signal engine auto-optimization)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneDecision {
    pub adjustments: Vec<TuneAdjustment>,
    pub reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneAdjustment {
    pub param: String,
    pub old_value: f64,
    pub new_value: f64,
}

// ---------------------------------------------------------------------------
// Trade record (for journal)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub id: Uuid,
    pub ts_open: DateTime<Utc>,
    pub ts_close: Option<DateTime<Utc>>,
    pub duration_secs: Option<u64>,
    pub play_type: PlayType,
    pub direction: Direction,
    pub signal_level: SignalLevel,
    pub signal_score: i32,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub size_usd: f64,
    pub leverage: Option<u8>,
    pub pnl_usd: Option<f64>,
    pub exit_reason: Option<String>,
    pub llm_reasoning: String,
    pub capital_after: Option<f64>,
}

// ---------------------------------------------------------------------------
// Execution types
// ---------------------------------------------------------------------------

#[allow(dead_code)] // symbol and pm_hedge will be used when multi-asset + PM hedge is wired
#[derive(Debug, Clone)]
pub struct ActiveTrade {
    pub id: Uuid,
    pub symbol: Symbol,
    pub direction: Direction,
    pub play_type: PlayType,
    pub entry_price: f64,
    pub size_usd: f64,
    pub leverage: Option<u8>,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub opened_at: DateTime<Utc>,
    pub max_hold_secs: u64,
    pub pm_hedge: Option<PmHedge>,
    pub llm_reasoning: String,
    pub signal_level: SignalLevel,
    pub signal_score: i32,
}

// ---------------------------------------------------------------------------
// Agent status (for TUI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    Scanning,
    SignalDetected,
    WaitingLlm,
    Executing,
    InPosition,
    SecondLook,
    Paused,
    KillSwitch,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Scanning => write!(f, "SCANNING"),
            AgentStatus::SignalDetected => write!(f, "SIGNAL"),
            AgentStatus::WaitingLlm => write!(f, "LLM..."),
            AgentStatus::Executing => write!(f, "EXECUTING"),
            AgentStatus::InPosition => write!(f, "IN POSITION"),
            AgentStatus::SecondLook => write!(f, "2ND LOOK"),
            AgentStatus::Paused => write!(f, "PAUSED"),
            AgentStatus::KillSwitch => write!(f, "KILLED"),
        }
    }
}
