# openSigma Project

Created time: March 11, 2026 3:46 PM
Status: Not started
Created by: Daniel.T

### Architecture

![image.png](openSigma%20Project/image.png)

<[https://excalidraw.com/#room=de5c8782e0906fe45f7b,9f2iag_273yQRsT_QMgV5w](https://excalidraw.com/#room=de5c8782e0906fe45f7b,9f2iag_273yQRsT_QMgV5w)>

UI Library → we like this stype, using TUI tool

Rust TUI library [https://ratatui.rs/](https://ratatui.rs/)

our Drawing:

![image.png](openSigma%20Project/image%201.png)

A trading system on Hyperliquid using rust, only trading whitelisted assets (early stage, ETH / BTC), where there are 4 agents:

- core WatchDog Agent:
    - controls an EOA address with intial deposited USDC fund
        - privatekey in .env
    - the smartest killer, can manage the overall portfolio
        - Risk manager, make sure no violation of fund limitation
        - Decision maker: take the advice from each sub agent → and ececute on herself
        - kill switch
        - store each trade and each trade proposer, reason why reject or accept and adjustment reasons,
        - [memory.md](http://memory.md) file (can refering the structure and design of the self-improving agent)
            - After close of each trade
                - can self-learning from gain and loss
                - update the gain and loss reasons to the [memeory.md](http://memeory.md) file
                - ask all other agents to load the [memory.md](http://memory.md) files have the latest experience
- it will store all trading history:
    - historical trades
    - ongoing trade
    - most importantly, mark which trade from which agent → and if it
- Long Term Agent:
    - Long Term Thesis:
        
        ```jsx
        WHAT:  Ride macro BTC/ETH cycles (weeks to months)
        WHY:   Crypto moves in 4-year halving cycles with
               predictable accumulation/distribution phases
        HOW:   On-chain data reveals where "smart money" is
               — whales accumulate quietly, retail buys tops
        
        EDGE:
          Buy when:  MVRV < 1  (market undervalued vs cost basis)
                     NUPL < 0  (holders in loss = capitulation)
                     Puell low (miners stressed = cycle bottom)
                     Fear & Greed < 20 (extreme fear)
                     
          Sell when: MVRV > 7  (euphoria, everyone profitable)
                     NUPL > 0.75 (max greed)
                     Pi Cycle top triggered
                     Post-halving month 12-18
        
        STYLE:  Low frequency, high conviction, large size
                Hold weeks to months, ignore daily noise
                Never trade against the macro cycle
        
        ```
        
    - Long Term Indicator:
        
        ```jsx
        LONG_TERM_SIGNALS = {
        "primary":   ["MVRV_zscore", "NUPL", "200w_MA"],
        "confirm":   ["Puell_multiple", "exchange_netflow"],
        "sentiment": ["fear_greed", "funding_cumulative"],
        "regime":    ["halving_cycle_month", "BTC_dominance"]
        }
        ```
        
    - Investment Duration: > 1 month
    - Investment leverage: limit within 1-2x
    - Fund limitation: less than 6% of overall assets each trade, totally no limitation if no leverage, if with leverage (less than 50% of free cash)
- Mid Term Agent:
    - Mid Term These:
        
        ```jsx
        WHAT:  Capture 3–10 day trend moves on BTC/ETH
        WHY:   Institutional flows and sentiment shifts take
               days to fully price in — trend is your friend
        HOW:   Follow momentum when confirmed by multiple
               timeframes + smart money positioning
        
        EDGE:
          Long when:  EMA ribbon fanning up (1H/4H)
                      OI rising + price rising (real demand)
                      Funding neutral-to-negative (not crowded)
                      Whale accumulation on-chain
                      Weekly MACD histogram positive
        
          Short when: EMA ribbon compressing below price
                      OI rising + price falling (shorts loading)
                      Funding persistently high (overleveraged)
                      Exchange inflows spiking (sell pressure)
        
        STYLE:  Medium frequency, 4H chart entries
                Target 3–10% moves, wider stops
                Cut quickly if thesis breaks
                Align with LongTerm bias — never fight it
        
        ```
        
    - Mid Term Indicator:
        
        ```jsx
        MID_TERM_SIGNALS = {
            "trend":     ["ema_ribbon", "weekly_macd", "supertrend"],
            "momentum":  ["weekly_rsi", "cvd_7d", "oi_trend"],
            "liquidity": ["liq_heatmap", "funding_7d_avg"],
            "structure": ["weekly_sr_levels", "htf_order_blocks"],
            "onchain":   ["whale_flows_7d", "exchange_netflow_7d"]
        }
        
        # Decision logic
        if ema_ribbon == "bullish" and weekly_rsi < 65 and funding_7d < 0.05:
            → LONG bias, look for entry on daily dip
            
        if weekly_macd_hist < 0 and oi_rising and price_falling:
            → SHORT bias or sit out
        ```
        
    - Investment Duration: < 1 week
    - Investment leverage: limit within 2-5x
    - Fund limitation: less than 5% of overall assets each trade, totally less than 20%
- Short Term Agent
    - Short term thesis
        
        ```jsx
        WHAT:  Scalp liquidity events and microstructure
               inefficiencies intraday (minutes to hours)
        WHY:   Market makers hunt stop clusters before
               real moves — these are predictable and precise
        HOW:   Identify where stops are clustered (Coinglass
               heatmap), wait for the grab, enter reversal
        
        EDGE:
          Core pattern:
            1. Price sweeps a liquidity level (stop hunt)
            2. CHoCH (Change of Character) confirms reversal
            3. Enter on first retracement into FVG
            4. Target next liquidity pool above/below
            
          Conditions needed:
            ✓ Aligns with MidTerm bias
            ✓ Session kill zone active (NY/London)
            ✓ Stoch RSI oversold/overbought at key level
            ✓ CVD diverging (fakeout confirmed)
            ✓ Funding not extreme (no squeeze risk)
        
        STYLE:  High frequency, tight stops (ATR-based)
                Small size per trade, many trades
                Quick in/out — never hold overnight
                Ruthless with stops — losers cut fast
        
        ```
        
    - Short Term Indicator
        
        ```jsx
        
        SHORT_TERM_SIGNALS = {
            "entry_trigger": ["bos_choch", "fvg_fill", "liq_grab"],
            "momentum":      ["stoch_rsi_15m", "cvd_delta", "vwap"],
            "microstructure":["funding_realtime", "liq_heatmap", "ob_depth"],
            "volatility":    ["atr_14", "bb_squeeze", "realized_vol"],
            "sessions":      ["asia_hl", "london_open", "ny_killzone"]
        }
        
        # ShortTerm decision flow
        if liq_grab_below and choch_1h and stoch_rsi_oversold:
            → LONG scalp, tight stop, 1-2% target
            
        if funding_spike > 0.05 and price_at_resistance and cvd_diverge:
            → SHORT fade, quick exit
        
        if bb_squeeze and atr_low:
            → WAIT, breakout incoming, don't enter yet
        
        Session timing (critical for short-term):
        
        00:00-04:00 UTC  Asia session    → low vol, range
        07:00-09:00 UTC  London open     → first big move
        13:30-15:30 UTC  NY open         → highest volume
        20:00-00:00 UTC  Late NY/overlap → second wind or fade
        ```
        
    - Investment Duration: < 30 mins
    - Investment Leverage: 1-20x
    - Fund limitation: less than 2% of overall assets each trade, totally less than 10%

Event Bus

```jsx

SHORT TERM
├── Hyperliquid (free)    → price, OB, trades, funding, liq
└── Coinglass ($30/mo)    → cross-exchange liq heatmap

MID TERM
├── Hyperliquid (free)    → 1h/4h candles, OI, funding history
└── Coinglass ($30/mo)    → OI history, funding trends (same sub)

LONG TERM
├── Hyperliquid (free)    → 1d/1w candles
├── Glassnode ($29/mo)    → MVRV, NUPL, Puell, on-chain
├── Alternative.me (free) → Fear & Greed Index
├── CoinGecko (free)      → BTC dominance, market cap
└── FRED API (free)       → DXY, rates, M2 macro

NEWS / EVENTS (circuit breaker only)
├── SoSoValue (free)      → daily BTC/ETH ETF flows
├── Cryptopanic (free)    → high-impact BTC/ETH news only
└── FRED API (free)       → macro calendar (CPI, FOMC, PPI)

Total cost: $59/mo

Coinglass  $30/mo
Glassnode  $29/mo
Everything else = free

News usage rule:

if high_impact_news_incoming:
    → reduce position sizes
    → pause ShortTerm agent
    → WatchDog on alert
```

The event bus is the sole data ingestion layer. All external signals pass through it before reaching any agents. No agent ever touches a raw data source directly.

**Layer 1 - Rule Engine** handles structured numerical data (price, funding rates, OI, liquidations, etc) via Hyperliquid WebSocket (or alternative trading data source like Coinglass). Pure threshold logic, no LLM, latency under 100ms

**Layer 2 - LLM Filter** handles unstructured text (news, Twitter, on-chain whale alerts, etc). A small cheap model normalizes raw text into structured events with sentiment and urgency scores. Deduplicates across sources. Not on the critical execution path.

All events are normalized into the same typed schema before routing.

**Topic Router** maintains a static subscription table. Each agent has an isolated queue and only receives event types relevant to its timeframe. e.g., Short term → high-frequency price signals. Long term receives low-urgency macro news. `drawdown_alert` is forwarded directly to Watch dog to trigger the skill switch, etc.

Short term agent receives events via Push from Event bus. Mid/Long term agents can pull from Event bus periodically.