# openSigma v2 — Short-Term Dual-Market Trading Agent

**Status:** Design Phase  
**Authors:** Daniel.T + Team  
**Created:** March 12, 2026  
**Language:** Rust (pure — no TypeScript sidecar needed)  
**Markets:** Hyperliquid Perps + Polymarket BTC Binary Markets  
**Trade Duration:** < 10 minutes  
**Initial Capital:** $5,000 USDC  
**LLM:** Claude Sonnet (via Anthropic API)  
**Environment:** Mainnet from day 1  

---

## 1. Why v2 — Lessons from v1

The original openSigma design was a multi-agent system with Long/Mid/Short agents plus a WatchDog supervisor. We're shelving that for now because:

- **Too large in scope** — 4 agents, complex inter-agent communication, memory broadcasting
- **No on-chain trading experience yet** — we need to ship, learn from real trades, iterate
- **Cost-heavy** — multiple LLM calls per decision cycle across 4 agents

**v2 philosophy: one agent, two markets, fast feedback loops, learn by doing.**

We keep the best ideas from v1 (self-evolving memory, structured trade logging, kill switch) but strip everything down to a single focused short-term trader that we can build fast, deploy on mainnet with small capital, and learn from real results.

**No testnet.** Testnet teaches you nothing about real fills, real slippage, real Polymarket crowd behavior, or real news impact. We start on mainnet with $5k and <3% per trade ($150 max per trade). The risk is capped, the learning is real.

---

## 2. Core Concept

A single AI-powered trading agent that:

1. **Trades BTC on Hyperliquid** (leveraged perpetual futures) for directional bets
2. **Trades BTC on Polymarket** (5-min / 15-min binary markets) for hedging, arbitrage, or standalone binary bets
3. **Holds no position longer than 10 minutes**
4. **Every signal goes through Claude Sonnet** — even strong ones get a fast LLM sanity check
5. **Self-evolves** by journaling every trade and generating improvement reports every 20 trades
6. **SecondLook ability** — can defer entry, schedule a re-check, and auto-invoke itself for better timing

---

## 3. Why Dual-Market: Hyperliquid × Polymarket

### 3.1 What Each Market Offers

| Feature | Hyperliquid Perps | Polymarket BTC Binary |
|---|---|---|
| **Instrument** | Perpetual futures (BTC-USD) | Binary outcome: "BTC Up or Down" in 5m / 15m windows |
| **Leverage** | 1–50x configurable | None (binary — pay $0.01–$0.99 per share, win $1.00 or $0.00) |
| **Settlement** | Continuous (close anytime) | Fixed window (resolves at end of 5m/15m period via Chainlink) |
| **Edge** | Precise entries, tight stops, scalp micro-moves | Mispriced probabilities, crowd-sentiment lag |
| **Risk profile** | Variable (depends on leverage + stop) | Capped (max loss = cost of shares) |
| **Fees** | Maker: 0.01%, Taker: 0.035% | Maker: 0% + rebates (post-Feb 2026), Taker: ~1.5% |
| **API** | REST + WebSocket, official Rust SDK | CLOB REST + WebSocket, official Rust SDK (`polymarket-client-sdk`) |
| **Data source** | Hyperliquid native | Chainlink BTC/USD data stream |

### 3.2 The Strategic Edge: How They Work Together

Polymarket's 5-min and 15-min BTC "Up or Down" markets are essentially ultra-short-term binary options settled by Chainlink oracle price. The key insight is that these markets often misprice because:

- **Crowd momentum lag** — Polymarket odds react slower than Hyperliquid perp price during fast moves. When BTC spikes +0.5% in 2 minutes on Hyperliquid, the Polymarket "Up" share might still be priced at $0.55 when it should be $0.80+
- **Binary simplification** — a complex price move gets compressed into a simple yes/no, creating edge when the agent has higher-resolution information from Hyperliquid's order book and microstructure
- **Sentiment overshoot** — after a sharp move, Polymarket crowds often overprice continuation (recency bias), creating fade opportunities
- **Fixed-window risk profile** — the agent knows exactly when each binary market resolves, enabling precise risk/reward calculations that perps can't offer
- **Asymmetric hedging** — buy "Up" at $0.30 as insurance on a perp short. If the short works, you lose $0.30/share (small). If it fails and BTC rips, the "Up" shares pay $1.00 (3.3x return) to offset the stop-loss

### 3.3 Five Play Types

| # | Play Type | Hyperliquid | Polymarket | When |
|---|---|---|---|---|
| 1 | **Pure Perp Scalp** | Long/Short BTC perp | — | Strong technical signal, clean setup |
| 2 | **Pure Binary Bet** | — | Buy Up/Down shares | PM odds clearly mispriced vs. agent's read |
| 3 | **Hedged Perp** | Long/Short BTC perp | Buy opposite direction on PM | High conviction but elevated vol risk |
| 4 | **Binary Arbitrage** | — | Buy mispriced side | PM Up + Down combined < $1.00 (spread capture) |
| 5 | **Cross-Market Momentum** | Follow perp direction | Buy same direction on PM | HL price moving fast, PM odds haven't caught up |

### 3.4 Hedging Example (with real numbers at $5k capital)

Agent detects a short setup on Hyperliquid. It opens a 5x short with $150 notional (3% of $5k). Simultaneously buys $15 of Polymarket "Up" shares at $0.35 each (42 shares, ~10% of perp notional as hedge).

- **Short works (BTC drops):** Perp profit ~$7.50 (5% on $150). PM "Up" shares expire worthless: -$15. Net: still profitable, hedge cost was acceptable.
- **Short fails (BTC rips up):** Perp stopped out at -$6.00. PM "Up" shares pay out: 42 × $1.00 = $42. Net: +$36. The hedge turned a loss into a profit.

With $5k and max 3% per trade, our max exposure per trade is $150 on Hyperliquid + ~$15 on Polymarket. Polymarket 5-min windows have ~$70K volume — our $15 orders are noise. **Liquidity is a non-issue at our size.**

---

## 4. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        DATA LAYER (WebSocket)                   │
│                                                                 │
│   Hyperliquid WS          Polymarket WS        News Feed       │
│   ├─ BTC price             ├─ 5m/15m odds       ├─ Cryptopanic │
│   ├─ Order book depth      ├─ Book depth         └─ FRED macro │
│   ├─ Trades stream         └─ Recent trades          calendar  │
│   ├─ Funding rate                                               │
│   └─ Liquidations                                               │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                  SIGNAL ENGINE (Rule-Based, No LLM)             │
│                                                                 │
│   Computes a numeric score every tick:                          │
│                                                                 │
│   bull_score / bear_score from weighted indicators:             │
│     EMA(9,21) cross      [weight 2]  — trend direction         │
│     RSI(14) 5m           [weight 1]  — overbought/oversold     │
│     CVD 5m               [weight 2]  — real buy/sell pressure   │
│     OB imbalance          [weight 1]  — immediate order flow    │
│     Stoch RSI 1m         [weight 1]  — entry timing            │
│     PM odds divergence   [weight 1]  — cross-market signal     │
│                                                                 │
│   Hard filters (override to NO_TRADE):                          │
│     BB squeeze active / ATR too low / news circuit breaker /    │
│     outside kill zone / daily loss hit / funding extreme        │
│                                                                 │
│   Output: score + direction + filter_pass                       │
│                                                                 │
│   |net| >= 5  →  STRONG     (high confluence)                   │
│   |net| >= 3  →  LEAN       (moderate confluence)               │
│   |net| < 3   →  WEAK       (conflicting indicators)           │
│   filter_fail →  NO_TRADE   (hard block, skip LLM entirely)    │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                    filter_pass == true?
                     │              │
                    YES             NO → log → done (no LLM cost)
                     │
                     ▼
┌─────────────────────────────────────────────────────────────────┐
│              CLAUDE SONNET — EVERY TRADE GOES THROUGH LLM       │
│                                                                 │
│  Even STRONG signals get a fast sanity check (~200-500ms).      │
│  Cost: ~$0.003-0.01 per call. Negligible.                       │
│                                                                 │
│  Returns one of:                                                │
│    ✓ EXECUTE  { play_type, direction, size, leverage, stops,    │
│                 pm_hedge, reasoning }                            │
│    ✗ SKIP     { reasoning }                                     │
│    ⏳ SECOND_LOOK { recheck_after_secs, what_to_watch,          │
│                     original_bias, reasoning }                  │
│                                                                 │
│  On SECOND_LOOK:                                                │
│    1. Timer fires → Signal Engine re-scores (FREE)              │
│    2. New score + original context → back to Claude Sonnet      │
│    3. Max 3 SecondLooks per setup, then force SKIP              │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                     EXECUTE?
                     │        │
                    YES       NO → log → done
                     │
                     ▼
┌─────────────────────────────────────────────────────────────────┐
│                    EXECUTION ENGINE                              │
│                                                                 │
│   Risk Pre-Check (hard-coded, LLM cannot override)             │
│   ├─ Size ≤ 3% of capital ($150 at $5k)                        │
│   ├─ Max 2 concurrent positions                                 │
│   ├─ Daily loss ≤ 5%  │  Kill switch at 10% drawdown           │
│   ├─ PM hedge ≤ 10% of perp notional                           │
│   └─ News circuit breaker check                                 │
│                                                                 │
│   Hyperliquid Executor         Polymarket Executor              │
│   (hyperliquid-rust-sdk)       (polymarket-client-sdk v0.4)     │
│   ├─ Perp order + stops        ├─ Maker limit order (0% fee)   │
│   └─ Position monitor (1s)     └─ Track window resolution      │
│                                                                 │
│   Hard limits: 10 min max hold, trail stop, emergency close    │
└───────────────────────────┬─────────────────────────────────────┘
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    TRADE JOURNAL + MEMORY                        │
│                                                                 │
│   journal.jsonl  — every trade, full context                    │
│   Every 20 trades → Claude Sonnet improvement report            │
│   memory.md → lessons loaded into LLM system prompt             │
│   Reports → Telegram → we manually review + update code         │
└─────────────────────────────────────────────────────────────────┘
```

---

## 5. Signal Scoring — How Levels Are Defined

### 5.1 Indicator Weights

| Indicator | Timeframe | Weight | Bull Condition | Bear Condition |
|---|---|---|---|---|
| EMA(9, 21) | 1m candles | **2** | EMA9 > EMA21 (bullish cross) | EMA9 < EMA21 (bearish cross) |
| RSI(14) | 5m | **1** | RSI < 35 (oversold bounce) | RSI > 65 (overbought fade) |
| CVD (cumulative vol delta) | 5m | **2** | Rising (real buying pressure) | Falling (real selling pressure) |
| Order Book Imbalance | Real-time | **1** | Buy orders > 2x sell orders | Sell orders > 2x buy orders |
| Stochastic RSI | 1m | **1** | Stoch RSI < 20 (oversold) | Stoch RSI > 80 (overbought) |
| PM Odds Divergence | Real-time | **1** | PM "Down" cheap while technicals bullish | PM "Up" cheap while technicals bearish |

**Maximum possible score:** 8 in one direction (all 6 indicators firing)

### 5.2 Signal Levels

```
net_score = bull_score - bear_score

 net >= +5   →  STRONG_LONG    (5+ of 8 points aligned bullish)
 net >= +3   →  LEAN_LONG      (moderate bullish confluence)
 net <= -5   →  STRONG_SHORT   (5+ of 8 points aligned bearish)
 net <= -3   →  LEAN_SHORT     (moderate bearish confluence)
 |net| < 3   →  WEAK           (conflicting, low confluence)
```

**Why these thresholds?**
- **5+** means at least the two heavy-weight indicators (EMA + CVD = 4 points) plus at least one more are aligned. That's real confluence.
- **3–4** means some agreement but not full alignment — exactly the zone where LLM judgment matters most.
- **< 3** means indicators are fighting. The LLM will almost always SKIP or SECOND_LOOK here, but we still let it see the context in case memory.md has a relevant pattern.

### 5.3 Hard Filters (Override Everything → NO_TRADE, Skip LLM)

```
NO_TRADE if ANY of these are true:
  - Bollinger Band squeeze active (breakout imminent, direction unknown)
  - ATR(14) 5m < 0.05% (dead market, no edge)
  - News circuit breaker ON
  - Session = Asia (00:00–07:00 UTC)
  - Daily loss ≥ 5% of capital
  - Funding rate > 0.03% in same direction as signal
  - Kill switch triggered (10% drawdown from peak)
```

These are the ONLY cases where the LLM is NOT called. Filter hits are logged but cost nothing.

### 5.4 Why ALL Passing Signals Go Through Claude Sonnet

Even STRONG signals pass through the LLM. Here's why:

1. **Cost is negligible** — Sonnet call with our context (~800 tokens in, ~200 out) costs ~$0.003–0.01. Even 80 calls/day is under $1/day.
2. **Rules can't read memory** — a STRONG_LONG doesn't know the last 3 longs were all stop-hunts that reversed. The LLM, with memory.md, does.
3. **Play type selection** — rules know direction and strength. Choosing between the 5 play types requires contextual reasoning.
4. **Position sizing nuance** — 3% is the ceiling, but the LLM might size down to 1.5% for elevated uncertainty even within a strong signal.
5. **SecondLook opportunity** — even strong setups sometimes have better entries 30 seconds later.

The LLM call adds ~200–500ms. For sub-10-minute trades, this is fine. We're an intelligent scalper, not HFT.

---

## 6. SecondLook Mechanism

The agent can defer entry and schedule an automatic re-check for better timing.

### 6.1 Flow

```
Signal Engine → score passes filter → Claude Sonnet
                                          │
                         ┌────────────────┼────────────────┐
                     EXECUTE            SKIP          SECOND_LOOK
                                                    recheck: 30s
                                                    watch: "VWAP retest"
                                                    bias: Long
                                                          │
                                                    ┌─ 30s timer ─┐
                                                    │              │
                                               Signal Engine      │
                                               re-scores (FREE)   │
                                                    │              │
                                           Now STRONG?     Still LEAN?
                                                │              │
                                           Quick LLM      Full LLM
                                           confirm        re-eval
                                                │              │
                                         EXECUTE/SKIP    EXECUTE/SKIP
                                                        /SECOND_LOOK(#2)
                                                           max 3 total
```

### 6.2 Data Structure

```rust
enum LlmDecision {
    Execute {
        play_type: PlayType,
        direction: Direction,
        size_pct: f64,          // 1.0–3.0
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
        recheck_after_secs: u64,  // 15–180
        what_to_watch: String,
        original_bias: Direction,
        reasoning: String,
    },
}

struct PmHedge {
    side: BinarySide,     // Up or Down
    budget_usd: f64,      // max spend on shares
    max_price: f64,       // don't buy if share price above this
}
```

### 6.3 Why This Saves Money

- Rule engine runs every tick: **free**
- LLM only called when filters pass: ~30–80 calls/day
- SecondLook re-checks use free rule engine first, only call LLM again if ambiguous
- **Estimated: $5–18/month on Claude Sonnet**

---

## 7. Self-Evolving Memory System

### 7.1 Trade Journal (journal.jsonl)

Every trade logged as structured JSON:

```json
{
  "id": 47,
  "ts_open": "2026-03-12T14:32:11Z",
  "ts_close": "2026-03-12T14:38:44Z",
  "duration_secs": 393,
  "play_type": "hedged_perp",
  "direction": "short",
  "signal_level": "LEAN_SHORT",
  "signal_score": -4,
  "decision_path": "llm_execute",
  "second_looks": 1,
  "hl": {
    "entry": 83145.0, "exit": 82890.0,
    "leverage": 5, "size_usd": 150.0,
    "pnl_usd": 7.65, "exit_reason": "take_profit"
  },
  "pm": {
    "window": "5m", "side": "up", "shares": 42,
    "cost_usd": 14.70, "payout_usd": 0.00,
    "pnl_usd": -14.70, "resolution": "down"
  },
  "net_pnl_usd": -7.05,
  "llm_reasoning": "Short CHoCH setup, hedged with PM Up at $0.35",
  "session": "NY_OPEN",
  "capital_after": 4842.95
}
```

### 7.2 Every-20-Trade Improvement Report

Claude Sonnet analyzes the batch and generates a structured report:

```markdown
## openSigma Report — Trades #41–60

### Performance
- Win Rate: 12/20 (60%) | Net PnL: +$34.20
- Best: #48 Cross-Market Momentum +$22.10
- Worst: #55 Pure Perp Scalp -$14.30

### Patterns Detected
1. HEDGE OVERSIZING in #43,47,52 → SUGGESTION: reduce MAX_HEDGE_RATIO
2. NY SESSION DOMINANCE (70% vs 40% London) → reduce London sizing
3. SECOND_LOOK 80% WIN RATE → increase trust in deferred entries
4. RSI without CVD confirmation → 2 losses → hard-code CVD check

### Suggested Code Changes
1. config.toml: max_hedge_ratio = 0.08
2. config.toml: london size_mult = 0.3
3. signals/aggregator.rs: require CVD for RSI divergence entries
```

We review, discuss, decide what ships. Agent proposes, humans decide.

### 7.3 memory.md

```markdown
# openSigma Memory — Last Updated: 2026-03-14

## Validated Rules (implemented in code)
- RSI divergence requires CVD confirmation (report #3)
- Max PM hedge: 10% of perp notional (report #3)

## Soft Patterns (LLM judgment, not coded)
- After 3 consecutive losses, SecondLook setups win at 80%
- PM 5m odds > 0.75 within first 60s tend to mean-revert
- BB squeeze break + CVD confirm = strongest signal
```

---

## 8. Risk Parameters

```toml
[capital]
initial_usd = 5000
max_trade_pct = 3.0              # $150 max per trade
max_concurrent_positions = 2
max_daily_loss_pct = 5.0         # $250 daily stop
kill_switch_drawdown_pct = 10.0  # $500 total drawdown halt

[hyperliquid]
max_leverage = 10

[polymarket]
max_bet_usd = 50.0
max_hedge_ratio = 0.10           # PM hedge ≤ 10% of perp notional
prefer_maker_orders = true       # 0% fee + rebates
min_window_remaining_secs = 60   # don't enter < 60s before resolution

[execution]
max_trade_duration_secs = 600    # 10 min hard limit
max_second_looks = 3
signal_eval_interval_secs = 5

[sessions]  # UTC
ny_kill_zone = { start = "13:30", end = "16:00", size_mult = 1.0 }
london_open  = { start = "07:00", end = "09:30", size_mult = 0.4 }
late_ny      = { start = "20:00", end = "22:00", size_mult = 0.6 }

[llm]
model = "claude-sonnet-4-20250514"
timeout_ms = 3000
```

---

## 9. Tech Stack — Pure Rust

| Crate | Purpose |
|---|---|
| `hyperliquid-rust-sdk` | HL trading + WebSocket (official) |
| `polymarket-client-sdk` v0.4+ | PM CLOB + WebSocket (official Rust, crates.io, 380★) |
| `tokio` | Async runtime |
| `ratatui` + `crossterm` | Terminal dashboard |
| `reqwest` | HTTP (Claude API, news) |
| `serde` + `serde_json` | Serialization |
| `rust_decimal` | Precise price math |
| `chrono` | Timestamps |
| `tracing` | Logging |

**Polymarket Rust SDK confirmed viable:** `polymarket-client-sdk` is Polymarket's official Rust client. Published on crates.io v0.4.1, 380 stars, 15 contributors, MIT licensed. Has typed CLOB requests, WebSocket streaming (orderbook, prices, trades), signed order builders, maker/limit/market orders, `alloy` signer support. Features needed: `clob` + `ws` + `gamma`. **No TypeScript sidecar required.**

---

## 10. Project Structure

```
openSigma/
├── Cargo.toml
├── .env                     # HL private key, PM private key, Claude API key
├── config.toml              # All risk params, sessions, thresholds
├── memory.md                # Lessons (loaded into LLM system prompt)
├── src/
│   ├── main.rs              # Entry, tokio runtime, event loop
│   ├── config.rs
│   ├── data/
│   │   ├── hyperliquid.rs   # HL WebSocket + REST
│   │   ├── polymarket.rs    # PM CLOB + WebSocket + market discovery
│   │   └── news.rs          # Cryptopanic + FRED
│   ├── signals/
│   │   ├── indicators.rs    # EMA, RSI, Stoch RSI, BB, ATR, CVD, OB
│   │   ├── aggregator.rs    # Weighted scoring + levels
│   │   └── pm_analyzer.rs   # PM odds divergence + spread detection
│   ├── agent/
│   │   ├── llm_client.rs    # Claude Sonnet API
│   │   ├── llm_gate.rs      # Context builder + response parser
│   │   ├── second_look.rs   # SecondLook scheduler
│   │   └── play_selector.rs # 5 play types
│   ├── execution/
│   │   ├── risk.rs          # Hard limits (LLM cannot override)
│   │   ├── hyperliquid.rs   # HL orders + position monitor
│   │   ├── polymarket.rs    # PM orders + settlement
│   │   └── kill_switch.rs
│   ├── journal/
│   │   ├── logger.rs        # JSONL trade log
│   │   ├── reporter.rs      # 20-trade reports
│   │   └── memory.rs        # memory.md R/W
│   └── tui/
│       └── dashboard.rs     # Ratatui
├── data/
│   ├── journal.jsonl
│   └── reports/
└── tests/
```

---

## 11. Development Phases (Compressed — ~10 days)

### Phase 1 — Data + Signals (3–4 days)
- [ ] Project setup with both Rust SDKs
- [ ] HL WebSocket: BTC price, OB, trades, funding
- [ ] PM WebSocket: active 5m/15m BTC market odds
- [ ] Indicators: EMA, RSI, Stoch RSI, BB, ATR, CVD, OB imbalance
- [ ] Signal aggregator with weighted scoring + levels
- [ ] Config system (.env + config.toml)

### Phase 2 — LLM + Execution (3–4 days)
- [ ] Claude Sonnet client + structured JSON parsing
- [ ] LLM Gate: context builder, SecondLook scheduler
- [ ] HL execution: orders, stops, position monitor
- [ ] PM execution: maker limits, settlement tracking
- [ ] Risk checks + kill switch
- [ ] Trade journal (journal.jsonl)

### Phase 3 — Ship (2–3 days)
- [ ] 20-trade report generator + memory.md integration
- [ ] Ratatui dashboard
- [ ] Telegram alerts (optional)
- [ ] **Deploy on mainnet with $5k**

---

## 12. Monthly Costs

| Item | Cost |
|---|---|
| Claude Sonnet API | ~$5–18/mo |
| Cryptopanic + FRED | Free |
| Hyperliquid + Polymarket APIs | Free |
| VPS | ~$10–20/mo |
| **Total** | **~$15–38/mo** |

---

## 13. Summary

openSigma v2: one agent, two markets, pure Rust, ~10 days to first live trade.

- **All signals through Claude Sonnet** — cost is negligible, value of second opinion is high
- **SecondLook** — defers entry for better timing without wasting LLM calls
- **Pure Rust** — both exchanges have official Rust SDKs, no polyglot mess
- **Mainnet day 1** — $5k, <3% per trade, real learning
- **Human-in-the-loop evolution** — agent proposes changes every 20 trades, we decide

Ship fast. Trade small. Learn from real data. Evolve.
