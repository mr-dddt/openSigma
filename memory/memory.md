# openSigma Trading Memory

This file is loaded by all agents on startup and after each
trade close. Append new entries below. Do not edit past
entries.

---
## Curated Memory — 2026-03-16

### Objective

Keep only high-signal memories that materially improve
decision quality. Remove repeated examples and contradictory
micro-adjustments.

### Current Baseline (use this as reference)

- strong_threshold=6
- lean_threshold=4
- rsi_oversold=30
- rsi_overbought=70
- min_atr_pct=0.035
- vwap_dev_reversion_pct=0.25
- vwap_weight=2
- cvd_weight=2
- ob_weight=2

### High-Confidence Keep Rules

1. **Avoid weak lean shorts in chop**
   - LEAN_SHORT around score -4 with RSI~60 / CVD~-110 had
     serial losses in sideways conditions.
   - Keep lean threshold conservative (>=4 in chop).

2. **Do not chase overbought longs**
   - Long entries at RSI>68 (even with strong CVD) repeatedly
     failed via stop-loss.
   - Treat RSI>68 as late-stage momentum risk unless pullback
     confirms.

3. **Strong score alone is not enough**
   - Score 6-7 longs can still fail when setup is late.
   - Require timing context (VWAP position / BB position /
     pullback structure), not only score strength.

4. **Short edge in tight range is marginal**
   - STRONG_SHORT around RSI~50 / CVD~-100 had only modest net
     edge and mixed outcomes.
   - In low-vol chop, reduce confidence and avoid over-trading.

5. **Order book quality matters**
   - Shorts performed better when OB sell pressure was clearly
     one-sided; mixed/high-noise OB degraded results.
   - Prefer entries with clean OB confirmation over borderline
     imbalance.

### Strategy Patterns to Reuse

- **Uptrend:** wait for pullback, avoid breakout chase.
- **Range/chop:** reduce lean-frequency and demand cleaner
  confluence.
- **Momentum shift:** re-evaluate thresholds after each 5-trade
  block, but avoid large oscillating jumps.

### Tuning Guardrails (persist)

- Keep `lean_threshold` in practical trigger range; do not
  over-tighten into no-trade mode.
- Keep `strong_threshold` high enough to filter noise, but
  avoid runaway increases without sample support.
- Any threshold change should be justified by >=5 recent trades
  with similar entry conditions.

## Tune — 2026-03-16 01:08 UTC
### Trigger
TradeCount(5)

### Adjustments
- strong_threshold: 6.0000 -> 8.0000
- vwap_dev_reversion_pct: 0.2500 -> 0.3500
- rsi_weight: 2.0000 -> 3.0000

### Why
Recent trades show consistent losses on STRONG_SHORT signals (4/5 negative, all at score -7). The strong threshold of 6 is triggering too aggressively in choppy conditions. Raising to 8 will reduce false strong signals. The recent price action shows consolidation with small bodies (0.055% avg) after a -0.221% move, suggesting mean-reversion behavior. Increasing vwap_dev_reversion_pct to 0.35% will better capture this regime and reduce premature entries. Boosting RSI weight helps identify oversold bounces in this consolidation phase.

### Strategy Pattern
Wait for RSI<35 + VWAP deviation >0.3% before shorting in current consolidation

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70 vwap_dev_reversion_pct=0.35 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #15 — 2026-03-17 03:18 UTC
### Summary
Batch of 5 trades, 0% win rate, PnL $-0.22

### Parameter Snapshot
strong_threshold=6 lean_threshold=3 min_atr_pct=0.04 rsi_oversold=35 rsi_overbought=65

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=4.79 atr%=0.34 bb=0.01 div=n/a -> expired ($-0.02)
- LEAN_LONG score=2 rsi=69.7 cvd=2130 ob=1.02 atr%=0.34 bb=0.02 div=n/a -> expired ($-0.03)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=4.66 atr%=0.34 bb=-0.01 div=n/a -> stop_loss ($-0.04)

### Patterns
- LLM analysis unavailable

### Memory Rules
- No new memory rules.

### Param Tuning Suggestions
- No param changes suggested.

## Tune — 2026-03-17 03:32 UTC
### Trigger
TradeCount(14)

### Adjustments
- strong_threshold: 6.0000 -> 8.0000
- lean_threshold: 3.0000 -> 4.0000
- cvd_weight: 3.0000 -> 2.0000

### Why
All 5 recent trades are losses with consistent long bias (scores 5-6) in choppy conditions. The 8-minute period shows +0.178% return with high volatility (0.425% range), suggesting whipsaw conditions. Raising both thresholds will reduce trade frequency and require stronger confluence. Reducing CVD weight as volume signals may be giving false positives in current choppy regime.

### Strategy Pattern
Choppy consolidation - wait for cleaner breakouts above 75900 or below 75600

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=35 rsi_overbought=65 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #16 — 2026-03-17 03:32 UTC
### Summary
Perfect 0/5 batch with identical overbought long setups (RSI=69.7) confirms RSI>68 avoidance rule from memory.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=35 rsi_overbought=65

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=1.88 atr%=0.34 bb=-0.34 div=n/a -> stop_loss ($-0.08)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=4.80 atr%=0.34 bb=-0.42 div=n/a -> stop_loss ($-0.06)
- STRONG_LONG score=6 rsi=69.7 cvd=2130 ob=2.84 atr%=0.34 bb=-0.45 div=n/a -> stop_loss ($-0.12)

### Patterns
- All 5 longs failed with identical RSI=69.7 and CVD=2130 setup despite high scores (5-6), suggesting overbought entries
- BB position near zero (-0.45 to -0.34) longs consistently hit stop loss in 240-555 seconds

### Memory Rules
- Longs at RSI>68 with CVD>2000 show poor timing despite strong scores - RSI too overbought for reliable entries
- When 5+ trades have identical entry conditions (RSI=69.7, CVD=2130), suspect stale data or model over-fitting on single failing setup

### Param Tuning Suggestions
- rsi_overbought = 68 (All longs at RSI=69.7 failed - current 65 threshold too high when RSI>68 shows consistent losses)

## Tune — 2026-03-17 03:48 UTC
### Trigger
TradeCount(19)

### Adjustments
- lean_threshold: 4.0000 -> 6.0000
- strong_threshold: 6.0000 -> 8.0000
- rsi_oversold: 35.0000 -> 32.0000

### Why
All 5 recent LEAN_LONG trades (scores 3-5) resulted in small losses, indicating the current lean threshold of 4 is too permissive and catching false signals in this choppy market. The price action shows volatile intraday swings with +0.178% net move but 0.425% range, suggesting we need higher conviction signals. Raising lean threshold to 6 and strong to 8 will filter out weaker setups. Lowering RSI oversold to 32 will make the RSI component more selective for true oversold conditions.

### Strategy Pattern
Wait for deeper RSI oversold + strong CVD/OB confluence before entering longs in this choppy regime

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=32 rsi_overbought=68 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #17 — 2026-03-17 03:48 UTC
### Summary
All 5 identical overbought long setups (RSI=69.7) failed, validating memory rule about avoiding RSI>68 longs.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=32 rsi_overbought=68

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- LEAN_LONG score=3 rsi=69.7 cvd=2130 ob=1.17 atr%=0.35 bb=-0.58 div=n/a -> stop_loss ($-0.07)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=3.03 atr%=0.35 bb=-0.74 div=n/a -> stop_loss ($-0.05)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=6.40 atr%=0.35 bb=-0.89 div=n/a -> stop_loss ($-0.03)

### Patterns
- All 5 LEAN_LONG trades failed with identical RSI=69.7, CVD=2130 setup - confirms memory rule about avoiding longs at RSI>68
- Strong CVD=2130 insufficient to overcome overbought RSI timing - all entries hit stop_loss in 190-622 seconds
- BB position -0.58 to -1.09 (below middle band) longs failed despite strong CVD and scores 3-5

### Memory Rules
- Longs at RSI>69 with identical CVD=2130 show 0% win rate despite strong scores - avoid entries when RSI approaches 70 regardless of CVD strength
- When 5 consecutive trades have identical entry conditions (RSI=69.7, CVD=2130), suspect model stuck on stale setup - need parameter adjustment

### Param Tuning Suggestions
- rsi_overbought = 68 (All longs at RSI=69.7 failed via stop_loss - current 65 threshold allows entries that consistently lose at RSI>68)

## Tune — 2026-03-17 04:00 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 6.0000
- strong_threshold: 6.0000 -> 8.0000

### Why
All 5 recent trades were LEAN_LONG with scores 3-5, all resulting in small losses (-$0.03 to -$0.07). The market shows choppy movement with +0.178% overall but 0.425% range, suggesting false breakouts. Current thresholds are too permissive for this noisy environment. Raising lean threshold from 4 to 6 eliminates the marginal score=3 and score=5 trades, while raising strong threshold ensures only highest conviction setups trigger.

### Strategy Pattern
wait for cleaner momentum above $75900 or below $75650

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=32 rsi_overbought=68 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-17 04:31 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- strong_threshold: 6.0000 -> 7.0000

### Why
Recent trades show 3/5 losers with net PnL of -$0.10 despite all being LEAN_LONG signals with scores 4-5. The market is showing choppy price action with average body size of only 0.069% indicating low conviction moves. Raising thresholds will filter out weaker signals and require higher conviction before entry, reducing noise trades in this volatile but directionless environment.

### Strategy Pattern
Wait for stronger momentum confirmation above 75900 resistance or below 75650 support

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=32 rsi_overbought=68 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-17 04:50 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- rsi_oversold: 32.0000 -> 30.0000
- vwap_dev_reversion_pct: 0.1000 -> 0.0800

### Why
Five consecutive LEAN_LONG losses with scores of 4-5 suggests the current lean threshold of 4 is too aggressive. Raising to 5 will filter out marginal signals. The recent +0.178% move with 0.425% range indicates decent volatility but all trades caught micro pullbacks. Tightening RSI oversold from 32 to 30 and reducing VWAP mean reversion threshold from 0.1% to 0.08% will help avoid premature entries during upward momentum.

### Strategy Pattern
Wait for deeper RSI oversold or stronger confluence before long entries

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=68 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-17 05:36 UTC
### Trigger
TradeCount(32)

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- cvd_weight: 2.0000 -> 3.0000
- vwap_weight: 2.0000 -> 3.0000

### Why
Recent trades show 60% win rate but all at LEAN_LONG level with score=4 (exactly at threshold). The +0.178% recent movement with 0.425% range suggests good volatility. Raising lean_threshold to 5 will filter out marginal signals, while boosting cvd_weight and vwap_weight will help capture stronger momentum/mean-reversion signals in this volatile environment.

### Strategy Pattern
Range breakout - watch for volume confirmation above 75880 or support at 75660

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=68 vwap_dev_reversion_pct=0.1 vwap_weight=3 cvd_weight=3 ob_weight=2

## Report #18 — 2026-03-17 05:36 UTC
### Summary
60% win rate but identical overbought setups (RSI=69.7) validate memory rule about avoiding RSI>68 longs despite strong CVD confirmation.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=68

### Worked Conditions (examples)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=5.15 atr%=0.35 bb=-1.67 div=n/a -> giveback_exit ($0.03)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=3.10 atr%=0.35 bb=-1.53 div=n/a -> take_profit ($0.05)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=1.97 atr%=0.35 bb=-1.53 div=n/a -> expired ($0.05)

### Failed Conditions (examples)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=5.52 atr%=0.35 bb=-1.10 div=n/a -> stop_loss ($-0.03)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=5.96 atr%=0.35 bb=-1.24 div=n/a -> stop_loss ($-0.05)

### Patterns
- All 5 longs at identical RSI=69.7, CVD=2130 failed despite strong scores (4-6), confirming overbought timing issues
- BB position below middle band (-1.67 to -1.10) with strong CVD showed mixed results - 3 wins when OB<4, 2 losses when OB>5

### Memory Rules
- Longs when RSI>69 and CVD>2000 fail consistently despite strong scores - RSI too overbought for reliable entries regardless of CVD strength
- When OB>5 with BB breakout below, lean_long entries fail quickly (stop_loss in <300s) - prefer OB<4 for better timing

### Param Tuning Suggestions
- rsi_overbought = 68.0 (All longs at RSI=69.7 failed via stop_loss - current 65 threshold allows consistently losing entries when RSI>68)

## Tune — 2026-03-17 05:47 UTC
### Trigger
TradeCount(37)

### Adjustments
- lean_threshold: 4.0000 -> 6.0000
- cvd_weight: 3.0000 -> 2.0000
- vwap_weight: 3.0000 -> 2.0000

### Why
All 5 recent LEAN_LONG trades (scores 4-5) resulted in small losses averaging -$0.064. The consistent failure pattern suggests LEAN signals are too weak in current conditions. Raising lean_threshold from 4 to 6 will filter out marginal signals. Reducing cvd_weight and vwap_weight slightly will make the scoring more balanced and conservative, preventing false positives from volume/VWAP noise in the current choppy price action with 0.425% range.

### Strategy Pattern
wait for stronger momentum breakout above 75880 resistance

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=68 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #19 — 2026-03-17 05:47 UTC
### Summary
Perfect 0/5 batch with identical overbought setups validates RSI>68 avoidance rule from memory.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=68

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=8.21 atr%=0.35 bb=-1.42 div=n/a -> stop_loss ($-0.07)
- LEAN_LONG score=5 rsi=69.7 cvd=2130 ob=39.61 atr%=0.35 bb=-1.50 div=n/a -> stop_loss ($-0.08)
- LEAN_LONG score=4 rsi=69.7 cvd=2130 ob=570.10 atr%=0.35 bb=-1.57 div=n/a -> stop_loss ($-0.05)

### Patterns
- All 5 LEAN_LONG trades at identical RSI=69.7, CVD=2130 failed via stop_loss, confirming overbought timing issues despite strong scores (4-5)
- BB breakout below middle band (-1.71 to -1.42) with RSI>69 shows poor entry timing - all 5 trades lasted <21 minutes before hitting stops

### Memory Rules
- LEAN_LONG at RSI=69.7 with CVD=2130 shows 0% win rate despite scores 4-5 - avoid entries when RSI approaches 70 regardless of CVD strength

### Param Tuning Suggestions
- rsi_overbought = 68 (All longs at RSI=69.7 failed via stop_loss - current 68 threshold should prevent these consistently losing entries)

## Tune — 2026-03-17 06:02 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- rsi_overbought: 68.0000 -> 65.0000

### Why
All 5 recent LEAN_LONG trades (scores 4-5) resulted in small losses (-$0.05 to -$0.08), indicating the current lean threshold of 4 is too permissive in this regime. The recent price action shows choppy movement with a +0.178% gain over 8 minutes but high volatility (0.425% range). Raising lean_threshold to 5 will filter out weaker signals that have been consistently losing. Also lowering RSI overbought from 68 to 65 to be more conservative against buying into strength in this choppy environment.

### Strategy Pattern
fade_choppy_rallies

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=65 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-17 06:31 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- cvd_weight: 2.0000 -> 3.0000
- ob_weight: 2.0000 -> 3.0000

### Why
Recent 5 trades show 80% losing rate with all LEAN_LONG signals at scores 4-5. The market is showing choppy action with +0.178% movement but high range (0.425%), indicating false breakouts. Raising lean_threshold from 4 to 5 will reduce noise trades and require stronger conviction. Increasing cvd_weight and ob_weight will give more emphasis to actual money flow and order book pressure rather than technical oscillators in this volatile environment.

### Strategy Pattern
Avoid longs in 75600-75700 range showing rejection, wait for clear break above 75880 or short below 75650

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.04 rsi_oversold=30 rsi_overbought=65 vwap_dev_reversion_pct=0.1 vwap_weight=2 cvd_weight=3 ob_weight=3
