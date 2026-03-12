# openSigma — Technical Engineering Document

> **Version:** 0.1.0-draft  
> **Date:** 2026-03-11

---

## 1. Executive Summary

openSigma is a multi-agent autonomous trading system built in **Rust**, targeting **Hyperliquid** as its primary execution venue. The system employs four specialized agents — a core WatchDog supervisor and three time-horizon sub-agents (Long Term, Mid Term, Short Term) — coordinated through a central Event Bus. Each agent operates on distinct trading theses, signal sets, and risk parameters, while a shared `memory.md` file enables cross-agent learning from historical trade outcomes.

The system trades only whitelisted assets (initially BTC and ETH) and is designed for a single EOA wallet funded with USDC.

---

## 2. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       EVENT BUS (Central)                       │
│  Events: price_spike / liquidation / PnL_alert / drawdown /    │
│          regime_change / news                                   │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1 — Rule Engine    │  Layer 2 — LLM Filter              │
│  (structured numeric data)│  (unstructured text → typed events)│
│  Latency < 100ms          │  Async, non-critical path          │
└────────────┬──────────────┴──────────────┬──────────────────────┘
             │      Topic Router           │
             │  (static subscription table)│
             ▼                             ▼
  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
  │  Short Term  │  │   Mid Term   │  │  Long Term   │
  │    Agent     │  │    Agent     │  │    Agent      │
  │  (push)      │  │  (pull)      │  │  (pull)       │
  └──────┬───────┘  └──────┬───────┘  └──────┬────────┘
         │                 │                  │
         └────────┬────────┴──────────────────┘
                  ▼
         ┌───────────────┐
         │   WatchDog    │
         │  (Core Agent) │──── EOA Wallet (Hyperliquid)
         │               │──── memory.md
         └───────────────┘
```

### Data Flow

1. **Event Bus** is the sole data ingestion layer. No agent touches raw data sources directly.
2. **Layer 1 (Rule Engine)** processes structured numeric data (price, funding, OI, liquidations) via WebSocket with pure threshold logic — no LLM, latency < 100ms.
3. **Layer 2 (LLM Filter)** processes unstructured text (news, Twitter, whale alerts) through a small/cheap model, normalizing into typed events with sentiment + urgency scores. Deduplicates across sources. Async — not on the critical execution path.
4. **Topic Router** maintains a static subscription table routing event types to relevant agents. Short Term receives push events; Mid/Long Term agents pull periodically.
5. **Sub-Agents** produce trade proposals (with reasoning) and submit them to WatchDog.
6. **WatchDog** evaluates proposals against risk limits, accepts/rejects with adjustment reasons, executes on Hyperliquid, and logs everything to `memory.md`.

---

## 3. Technology Stack

### 3.1 Core Language & Runtime

| Component            | Choice                 | Rationale                                                                                                                         |
| -------------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| **Primary Language** | **Rust (stable)**      | Memory safety without GC, zero-cost abstractions, excellent async ecosystem. Critical for sub-100ms event processing in Layer 1.  |
| **Async Runtime**    | **Tokio**              | Industry-standard async runtime for Rust. Required for concurrent WebSocket connections, HTTP clients, and agent task scheduling. |
| **Serialization**    | **serde + serde_json** | De-facto standard for Rust serialization. All event schemas, config, and memory are JSON-serializable.                            |

### 3.2 Networking & Data Ingestion

| Component            | Choice                | Rationale                                                                                        |
| -------------------- | --------------------- | ------------------------------------------------------------------------------------------------ |
| **WebSocket Client** | **tokio-tungstenite** | Async WebSocket for Hyperliquid real-time feeds (price, OB, trades, funding, liquidations).      |
| **HTTP Client**      | **reqwest**           | REST API calls to Coinglass, Glassnode, CoinGecko, Alternative.me, FRED, SoSoValue, Cryptopanic. |
| **Rate Limiting**    | **governor**          | Token-bucket rate limiter to respect API quotas across all external data sources.                |

### 3.3 Hyperliquid Integration

| Component             | Choice                    | Rationale                                                                                                                                                       |
| --------------------- | ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **SDK**               | **hyperliquid-rust-sdk**  | Official/community Rust SDK for Hyperliquid. Covers order placement, cancellation, position queries, account info. Falls back to raw REST/WS if SDK gaps exist. |
| **Signing**           | **ethers-rs** / **alloy** | EVM-compatible signing for Hyperliquid L1 transactions. Private key loaded from `.env`.                                                                         |
| **Wallet Management** | Single EOA                | One address, one private key in `.env`. WatchDog is the sole executor — no sub-agent has direct wallet access.                                                  |

### 3.4 LLM Integration (Layer 2 — Event Filter)

| Component           | Choice                                                         | Rationale                                                                                                                                                  |
| ------------------- | -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **LLM Provider**    | **Anthropic API (Claude Haiku)** or **local Ollama (Llama 3)** | Haiku for low-cost, fast structured output. Ollama as fallback for zero-cost local inference. Layer 2 is async/non-critical, so latency tolerance is high. |
| **Prompt Strategy** | Structured JSON output with typed schema                       | Each news/text input is normalized to `{ event_type, asset, sentiment, urgency, summary }`.                                                                |
| **Client**          | **reqwest** (REST to Anthropic API) or **ollama-rs**           | Simple HTTP POST; no streaming needed for classification tasks.                                                                                            |

### 3.5 Event Bus & Inter-Agent Communication

| Component          | Choice                                         | Rationale                                                                                                                                                                                  |
| ------------------ | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **In-Process Bus** | **tokio::sync::broadcast** + **mpsc** channels | Lightweight, zero-overhead pub/sub within a single Rust binary. `broadcast` for fan-out events, `mpsc` for agent → WatchDog proposals. No need for external message brokers at this scale. |
| **Topic Routing**  | Static `HashMap<EventType, Vec<AgentId>>`      | Compiled-in subscription table. Short Term subscribes to high-frequency signals; Long Term to macro/on-chain. `drawdown_alert` routes directly to WatchDog.                                |
| **Event Schema**   | Rust enums with `serde`                        | Strongly typed events: `PriceUpdate`, `FundingRate`, `LiquidationEvent`, `NewsEvent`, `OnChainSignal`, etc. All normalized before routing.                                                 |

### 3.6 Storage & Persistence

| Component             | Choice                                  | Rationale                                                                                                                                         |
| --------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Trade History**     | **SQLite** via **rusqlite** or **sqlx** | Embedded, zero-config. Stores all historical trades, ongoing positions, proposer agent, accept/reject reasons, PnL.                               |
| **Memory / Learning** | **memory.md** (Markdown file)           | Human-readable, git-diffable. Updated by WatchDog after each trade close. All agents reload on update. Inspired by self-improving agent patterns. |
| **Configuration**     | **TOML** via **toml** crate             | Agent parameters (leverage limits, fund allocation %, signal thresholds) stored in `config.toml`. Hot-reloadable via file watcher.                |
| **Secrets**           | **.env** via **dotenvy**                | Private key, API keys (Coinglass, Glassnode). Never committed to VCS.                                                                             |

### 3.7 Terminal UI (TUI)

| Component            | Choice                     | Rationale                                                                                                                                  |
| -------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| **TUI Framework**    | **Ratatui** (`ratatui.rs`) | Modern Rust TUI library (successor to tui-rs). Renders portfolio view, agent status, event bus health, decision history — all in-terminal. |
| **Terminal Backend** | **crossterm**              | Cross-platform terminal manipulation. Paired with Ratatui for input handling and rendering.                                                |

**TUI Dashboard Panels (from design mockup):**

- **Portfolio Panel** — Active positions, entry price, PnL, percentage allocation, cash balance.
- **Agent Status Panel** — Per-agent heartbeat, last proposal timestamp, last proposal status.
- **Event Bus Panel** — Data source health per timeframe (Hyperliquid, Coinglass, Glassnode, etc.) with status indicators.
- **Core Decision History** — Paginated table: timestamp, trade, proposer agent, status (accepted/rejected), tx hash.
- **Memory Status** — Last update timestamp for `memory.md`.
- **Navigation** — Drill into individual agent pages for detailed logs, decision reasoning, event history.

### 3.8 Observability & Logging

| Component              | Choice                               | Rationale                                                                                               |
| ---------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| **Structured Logging** | **tracing** + **tracing-subscriber** | Async-aware, span-based structured logging. Each agent runs in its own tracing span for easy filtering. |
| **Log Output**         | File + TUI panel                     | Logs to `logs/opensigma.log` (rolling) and rendered in TUI.                                             |
| **Metrics (future)**   | **prometheus** crate                 | Expose PnL, win rate, drawdown, latency metrics. Optional Grafana dashboard.                            |

### 3.9 Testing & CI

| Component             | Choice                                       | Rationale                                                          |
| --------------------- | -------------------------------------------- | ------------------------------------------------------------------ |
| **Unit Tests**        | Built-in `#[test]`                           | Signal calculation, risk limit checks, event routing logic.        |
| **Integration Tests** | **Hyperliquid testnet**                      | End-to-end order flow against testnet.                             |
| **Backtesting**       | Custom module reading historical SQLite data | Replay historical events through agents, measure PnL and drawdown. |
| **CI**                | **GitHub Actions**                           | `cargo clippy`, `cargo test`, `cargo build --release` on every PR. |

---

## 4. Agent Specifications

### 4.1 WatchDog (Core Agent)

**Role:** Supervisor, risk manager, sole executor.

| Parameter      | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Wallet         | Single EOA, private key in `.env`                                           |
| Execution      | Only agent that submits transactions to Hyperliquid                         |
| Kill Switch    | Can shut down any sub-agent or halt all trading                             |
| Decision Logic | Evaluates sub-agent proposals against risk limits, then executes or rejects |

**Responsibilities:**

- Overall portfolio management and risk enforcement.
- Accept/reject/adjust trade proposals from sub-agents.
- Log every decision (proposer, reasoning, accept/reject, adjustments) to SQLite.
- After each trade close: update `memory.md` with gain/loss analysis, broadcast reload signal to all agents.
- Trigger kill switch on drawdown alerts.
- Counter bad-performing agents (reduce allocation, pause, or shut down).

### 4.2 Long Term Agent

| Parameter  | Value                                                                         |
| ---------- | ----------------------------------------------------------------------------- |
| Thesis     | Ride macro BTC/ETH cycles (weeks → months) based on halving cycle positioning |
| Duration   | > 1 month                                                                     |
| Leverage   | 1–2x                                                                          |
| Fund Limit | < 6% per trade; unlimited if no leverage; < 50% of free cash if leveraged     |
| Style      | Low frequency, high conviction, large size                                    |

**Primary Signals:** MVRV Z-Score, NUPL, 200-week MA  
**Confirmation:** Puell Multiple, exchange net flow  
**Sentiment:** Fear & Greed Index, cumulative funding  
**Regime:** Halving cycle month, BTC dominance

**Entry/Exit Logic:**

- Buy: MVRV < 1, NUPL < 0, Puell low, Fear & Greed < 20
- Sell: MVRV > 7, NUPL > 0.75, Pi Cycle top triggered, post-halving month 12–18

**Data Sources:** Hyperliquid (1d/1w candles), Glassnode ($29/mo), Alternative.me (free), CoinGecko (free), FRED API (free)

### 4.3 Mid Term Agent

| Parameter  | Value                                                                   |
| ---------- | ----------------------------------------------------------------------- |
| Thesis     | Capture 3–10 day trend moves; institutional flows take days to price in |
| Duration   | < 1 week                                                                |
| Leverage   | 2–5x                                                                    |
| Fund Limit | < 5% per trade; < 20% total                                             |
| Style      | Medium frequency, 4H chart entries, 3–10% targets                       |

**Signals:** EMA ribbon, weekly MACD, SuperTrend, weekly RSI, CVD 7d, OI trend, liquidity heatmap, 7d funding avg, weekly S/R levels, HTF order blocks, whale flows 7d, exchange netflow 7d

**Decision Logic:**

- Long bias: EMA ribbon bullish AND weekly RSI < 65 AND 7d funding < 0.05 → entry on daily dip
- Short bias: Weekly MACD histogram < 0 AND OI rising AND price falling
- **Must align with Long Term bias — never fight it.**

**Data Sources:** Hyperliquid (1h/4h candles, OI, funding history), Coinglass ($30/mo — OI history, funding trends)

### 4.4 Short Term Agent

| Parameter  | Value                                                                                |
| ---------- | ------------------------------------------------------------------------------------ |
| Thesis     | Scalp liquidity events and microstructure inefficiencies intraday                    |
| Duration   | < 30 minutes                                                                         |
| Leverage   | 1–20x                                                                                |
| Fund Limit | < 2% per trade; < 10% total                                                          |
| Style      | High frequency, tight ATR-based stops, small size, many trades, never hold overnight |

**Signals:** BOS/CHoCH, FVG fill, liquidity grab, Stoch RSI 15m, CVD delta, VWAP, real-time funding, liquidity heatmap, order book depth, ATR-14, Bollinger Band squeeze, realized volatility, session markers (Asia/London/NY)

**Core Pattern:**

1. Price sweeps a liquidity level (stop hunt)
2. CHoCH confirms reversal
3. Enter on first retracement into FVG
4. Target next liquidity pool

**Session Windows (UTC):**

- 00:00–04:00 — Asia (low vol, range)
- 07:00–09:00 — London open (first big move)
- 13:30–15:30 — NY open (highest volume)
- 20:00–00:00 — Late NY/overlap

**Must align with Mid Term bias.**

**Data Sources:** Hyperliquid (real-time price, OB, trades, funding, liquidations), Coinglass ($30/mo — cross-exchange liquidation heatmap)

---

## 5. Event Bus Design

### 5.1 Typed Event Schema

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    // Layer 1 — Rule Engine (structured, < 100ms)
    PriceUpdate { asset: Asset, price: f64, timestamp: u64 },
    FundingRate { asset: Asset, rate: f64, timestamp: u64 },
    OIChange { asset: Asset, oi: f64, delta: f64 },
    Liquidation { asset: Asset, side: Side, size: f64, price: f64 },
    CandleClose { asset: Asset, timeframe: Timeframe, candle: Candle },

    // Layer 1 — On-chain / Macro
    OnChainMetric { metric: OnChainType, value: f64, timestamp: u64 },
    MacroIndicator { indicator: MacroType, value: f64 },
    FearGreedIndex { value: u8 },

    // Layer 2 — LLM-processed (async)
    NewsEvent { source: String, sentiment: f64, urgency: u8, summary: String },
    WhaleAlert { asset: Asset, direction: Side, size_usd: f64 },

    // System events
    DrawdownAlert { current_dd: f64, threshold: f64 },
    PnlAlert { trade_id: String, pnl_pct: f64 },
    RegimeChange { from: MarketRegime, to: MarketRegime },
    AgentHealthCheck { agent: AgentId, status: HealthStatus },
}
```

### 5.2 Subscription Table

```rust
lazy_static! {
    static ref SUBSCRIPTIONS: HashMap<EventCategory, Vec<AgentId>> = {
        let mut m = HashMap::new();
        m.insert(HighFreqPrice,   vec![ShortTerm]);
        m.insert(MidFreqTrend,    vec![MidTerm]);
        m.insert(MacroOnChain,    vec![LongTerm]);
        m.insert(News,            vec![MidTerm, LongTerm]);
        m.insert(DrawdownAlert,   vec![WatchDog]);  // direct route
        m.insert(PnlAlert,        vec![WatchDog]);
        m
    };
}
```

### 5.3 Delivery Model

| Agent      | Delivery                                 | Rationale                                              |
| ---------- | ---------------------------------------- | ------------------------------------------------------ |
| Short Term | **Push** (broadcast channel)             | Latency-critical; must react within milliseconds       |
| Mid Term   | **Pull** (polling every ~1 min)          | 4H chart cadence; no need for real-time push           |
| Long Term  | **Pull** (polling every ~15 min)         | Daily/weekly signals; minimal urgency                  |
| WatchDog   | **Push** for alerts; **Pull** for status | Drawdown/PnL alerts are urgent; routine status polling |

---

## 6. External Data Sources & Costs

| Source              | Cost   | Used By          | Data                                                           |
| ------------------- | ------ | ---------------- | -------------------------------------------------------------- |
| Hyperliquid WS/REST | Free   | All agents       | Price, OB, trades, funding, OI, liquidations, candles          |
| Coinglass API       | $30/mo | Short + Mid Term | Cross-exchange liquidation heatmap, OI history, funding trends |
| Glassnode API       | $29/mo | Long Term        | MVRV, NUPL, Puell Multiple, on-chain metrics                   |
| Alternative.me      | Free   | Long Term        | Fear & Greed Index                                             |
| CoinGecko API       | Free   | Long Term        | BTC dominance, market cap                                      |
| FRED API            | Free   | Long Term + News | DXY, interest rates, M2, macro calendar (CPI/FOMC/PPI)         |
| SoSoValue           | Free   | News/Events      | Daily BTC/ETH ETF flows                                        |
| Cryptopanic         | Free   | News/Events      | High-impact BTC/ETH news                                       |

**Total monthly cost: $59/mo**

---

## 7. Risk Management Rules

### 7.1 Per-Agent Fund Limits

| Agent      | Max Per Trade                                                          | Max Total Exposure |
| ---------- | ---------------------------------------------------------------------- | ------------------ |
| Long Term  | 6% of assets (unlimited if no leverage; 50% of free cash if leveraged) | —                  |
| Mid Term   | 5% of assets                                                           | 20% of total       |
| Short Term | 2% of assets                                                           | 10% of total       |

### 7.2 News / Event Circuit Breaker

When high-impact news is incoming (CPI, FOMC, PPI, major regulatory events):

1. Reduce all position sizes.
2. Pause Short Term agent entirely.
3. WatchDog enters alert mode.

### 7.3 Kill Switch

WatchDog can:

- Shut down any individual agent whose performance degrades below threshold.
- Counter (reverse) a bad agent's position.
- Halt all trading system-wide.

---

## 8. Memory & Self-Learning System

### 8.1 `memory.md` Structure

```markdown
# openSigma Memory

## Trade Log

### Trade #042 — 2026-03-10

- **Asset:** BTC
- **Side:** Short 2x
- **Agent:** Short Term
- **Entry:** $84,200 | **Exit:** $83,400
- **PnL:** +$1,600 (+1.9%)
- **Reasoning:** Liquidity grab above $84,500, CHoCH on 15m, entered FVG
- **Lesson:** NY session liquidity grabs on BTC remain high-probability
  when funding > 0.03%

## Patterns Identified

- Short Term liq-grab accuracy: 67% (last 30 trades)
- Mid Term EMA ribbon + low funding: 72% win rate
- Long Term MVRV < 1 entries: 3/3 profitable historically

## Agent Performance

- Short Term: Sharpe 1.8, Max DD -3.2%
- Mid Term: Sharpe 2.1, Max DD -5.1%
- Long Term: No closed trades yet
```

### 8.2 Learning Loop

1. WatchDog closes a trade.
2. WatchDog appends outcome + reasoning to `memory.md`.
3. WatchDog signals all agents to reload `memory.md`.
4. Agents incorporate latest lessons into their next decision cycle.

---

## 9. Project Structure

```
opensigma/
├── Cargo.toml
├── config.toml                  # Agent params, thresholds, limits
├── .env                         # PRIVATE_KEY, API keys
├── memory.md                    # Self-learning memory file
│
├── src/
│   ├── main.rs                  # Entry point, Tokio runtime, TUI launch
│   ├── config.rs                # Config loading (TOML + .env)
│   │
│   ├── event_bus/
│   │   ├── mod.rs
│   │   ├── types.rs             # Event enum, schema definitions
│   │   ├── bus.rs               # Broadcast + mpsc channels
│   │   ├── router.rs            # Topic subscription table
│   │   ├── rule_engine.rs       # Layer 1: numeric threshold logic
│   │   └── llm_filter.rs        # Layer 2: news → structured events
│   │
│   ├── agents/
│   │   ├── mod.rs
│   │   ├── traits.rs            # Agent trait (propose, load_memory, health)
│   │   ├── watchdog.rs          # Core agent: risk, execution, memory
│   │   ├── long_term.rs         # Macro cycle agent
│   │   ├── mid_term.rs          # Trend-following agent
│   │   └── short_term.rs        # Scalping agent
│   │
│   ├── exchange/
│   │   ├── mod.rs
│   │   ├── hyperliquid.rs       # HL SDK wrapper: orders, positions, WS
│   │   └── types.rs             # Order, Position, Fill types
│   │
│   ├── data/
│   │   ├── mod.rs
│   │   ├── coinglass.rs         # Coinglass API client
│   │   ├── glassnode.rs         # Glassnode API client
│   │   ├── coingecko.rs         # CoinGecko API client
│   │   ├── alternative.rs       # Fear & Greed API client
│   │   ├── fred.rs              # FRED macro data client
│   │   ├── sosovalue.rs         # ETF flow data
│   │   └── cryptopanic.rs       # News aggregator
│   │
│   ├── signals/
│   │   ├── mod.rs
│   │   ├── long_term.rs         # MVRV, NUPL, Puell, 200w MA, etc.
│   │   ├── mid_term.rs          # EMA ribbon, MACD, RSI, CVD, etc.
│   │   └── short_term.rs        # BOS/CHoCH, FVG, Stoch RSI, etc.
│   │
│   ├── risk/
│   │   ├── mod.rs
│   │   ├── limits.rs            # Per-agent fund limits, leverage caps
│   │   ├── kill_switch.rs       # Emergency shutdown logic
│   │   └── circuit_breaker.rs   # News-driven position reduction
│   │
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── db.rs                # SQLite: trade history, decisions
│   │   └── memory.rs            # memory.md read/write/reload
│   │
│   └── tui/
│       ├── mod.rs
│       ├── app.rs               # Ratatui app state + event loop
│       ├── portfolio.rs         # Portfolio panel
│       ├── agents.rs            # Agent status panel
│       ├── event_bus.rs         # Data source health panel
│       ├── decisions.rs         # Decision history table
│       └── detail.rs            # Agent detail drill-down view
│
├── tests/
│   ├── test_signals.rs
│   ├── test_risk.rs
│   ├── test_event_routing.rs
│   └── test_e2e_testnet.rs
│
└── scripts/
    ├── backtest.rs              # Historical replay
    └── seed_memory.rs           # Initial memory.md population
```

---

## 10. Dependency Summary (`Cargo.toml`)

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
reqwest = { version = "0.12", features = ["json", "native-tls"] }
governor = "0.7"
rusqlite = { version = "0.32", features = ["bundled"] }
ratatui = "0.29"
crossterm = "0.28"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json"] }
dotenvy = "0.15"
toml = "0.8"
chrono = { version = "0.4", features = ["serde"] }
ethers = { version = "2", features = ["signing"] }
lazy_static = "1.5"
anyhow = "1"
thiserror = "2"

# Optional: LLM integration
# ollama-rs = "0.2"
```

---

## 11. Development Phases

### Phase 0 — Foundation (Weeks 1–2)

- Project scaffold, Cargo workspace setup.
- Config loading (TOML + .env).
- Hyperliquid SDK integration: connect, read positions, place/cancel orders on testnet.
- SQLite schema for trade history.
- Basic TUI shell with Ratatui (portfolio panel + placeholder views).

### Phase 1 — Event Bus + Data Ingestion (Weeks 3–4)

- Implement Event Bus with typed events and topic router.
- Layer 1 Rule Engine: Hyperliquid WebSocket → normalized events.
- Data clients: Coinglass, Glassnode, CoinGecko, Alternative.me, FRED.
- Health check monitoring for all data sources.

### Phase 2 — Agents + Signals (Weeks 5–7)

- Agent trait definition.
- Signal calculation modules for each timeframe.
- Long Term agent: on-chain cycle indicators.
- Mid Term agent: trend + momentum signals.
- Short Term agent: microstructure + session logic.
- WatchDog: proposal evaluation, risk enforcement, execution.

### Phase 3 — Memory + Learning (Week 8)

- `memory.md` read/write/reload pipeline.
- Post-trade analysis and lesson extraction.
- Agent memory integration (reload on update).

### Phase 4 — Risk + Circuit Breakers (Week 9)

- Per-agent fund limit enforcement.
- Kill switch implementation.
- News circuit breaker (pause Short Term, reduce sizes).
- Drawdown alert → WatchDog routing.

### Phase 5 — TUI Polish + Testnet E2E (Weeks 10–11)

- Full TUI dashboard: all panels, agent detail drill-down, pagination.
- End-to-end testing on Hyperliquid testnet.
- Backtesting module with historical data replay.

### Phase 6 — Mainnet Deployment (Week 12+)

- Mainnet configuration with real funds (small initial allocation).
- Monitoring, log review, parameter tuning.
- Gradual fund scaling based on performance.

---

## 12. Open Questions & Future Considerations

- **Multi-asset expansion:** When to move beyond BTC/ETH? What screening criteria?
- **Agent model upgrades:** Should sub-agents eventually use LLM reasoning for proposal generation, or stay pure rule-based?
- **Distributed deployment:** Single binary for now; if latency requirements tighten, consider splitting Short Term agent into a co-located process near Hyperliquid infra.
- **memory.md scaling:** At what trade volume does markdown become unwieldy? Migration path to structured storage (SQLite or vector DB) for pattern retrieval.
- **Backtesting fidelity:** How to model Hyperliquid-specific slippage, funding rates, and liquidation engine behavior in backtests.
