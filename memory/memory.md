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

## Report #16 — 2026-03-16 01:08 UTC
### Summary
4/4 strong shorts failed at same entry conditions (RSI~43, CVD~-94, BB<-0.5), suggesting model over-triggering in current consolidation.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)
- WEAK score=0 rsi=n/a cvd=n/a ob=n/a atr%=n/a bb=n/a div=n/a -> fast_take ($0.09)

### Failed Conditions (examples)
- STRONG_SHORT score=-7 rsi=43.4 cvd=-94 ob=0.11 atr%=0.21 bb=-0.74 div=n/a -> stale_exit ($-0.27)
- STRONG_SHORT score=-7 rsi=43.4 cvd=-94 ob=0.03 atr%=0.21 bb=-0.65 div=n/a -> stale_exit ($-0.47)
- STRONG_SHORT score=-7 rsi=43.4 cvd=-94 ob=0.52 atr%=0.21 bb=-0.55 div=n/a -> stale_exit ($-0.03)

### Patterns
- All 4 STRONG_SHORT trades at score -7 with RSI~43.4, CVD~-94, BB position around -0.65 failed via stale_exit
- Shorts near BB lower band (-0.55 to -0.74) in current conditions show poor timing despite strong scores
- Only win came from a recovered session trade with no entry conditions - suggests model may be over-fitting recent conditions

### Memory Rules
- STRONG_SHORT at RSI>40 with BB position below -0.5 shows poor timing in current regime - wait for deeper oversold
- Score -7 shorts failing consistently suggests strong_threshold should remain elevated until market regime shifts

### Param Tuning Suggestions
- rsi_oversold = 25.0 (RSI 43.4 shorts failing consistently - require deeper oversold before shorting)

## Tune — 2026-03-16 21:55 UTC
### Trigger
Inactivity

### Adjustments
- strong_threshold: 4.0000 -> 6.0000
- cvd_weight: 3.0000 -> 2.0000
- vwap_weight: 1.0000 -> 2.0000

### Why
Recent performance shows 4/5 trades profitable but one large loss (-$0.46) from a STRONG_LONG signal. The strong upward momentum (+0.233% in 8m) suggests we're in a bullish regime but need better signal quality. Raising strong_threshold from 4 to 6 will reduce false positives. Reducing CVD weight as volume patterns may be less reliable in momentum moves. Increasing VWAP weight to better capture mean reversion opportunities in this volatile range (0.681% range).

### Strategy Pattern
momentum continuation with pullback entries

### Parameter Snapshot
strong_threshold=6 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=40 rsi_overbought=60 vwap_dev_reversion_pct=0.3 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-16 22:19 UTC
### Trigger
Inactivity

### Adjustments
- strong_threshold: 6.0000 -> 7.0000
- lean_threshold: 2.0000 -> 3.0000

### Why
All 5 recent trades are profitable longs with 4/5 being STRONG signals (scores 7-8), showing excellent signal quality in this upward momentum regime. The +0.466% move with strong candle bodies suggests continuation. Raising thresholds slightly will filter for only the highest conviction signals while maintaining the profitable pattern, reducing noise trades during this momentum phase.

### Strategy Pattern
momentum continuation - strong bullish candles with expanding range

### Parameter Snapshot
strong_threshold=6 lean_threshold=3 min_atr_pct=0.035 rsi_oversold=40 rsi_overbought=60 vwap_dev_reversion_pct=0.3 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-16 22:46 UTC
### Trigger
TradeCount(5)

### Adjustments
- strong_threshold: 4.0000 -> 6.0000
- rsi_oversold: 40.0000 -> 35.0000
- rsi_overbought: 60.0000 -> 65.0000
- vwap_dev_reversion_pct: 0.3000 -> 0.4000

### Why
All 5 recent trades are losses, with 4/5 being STRONG_SHORT signals that failed. The market is showing downward movement (-0.229% over 8m) but our strong short signals are not capturing profitable moves. Raising strong_threshold from 4 to 6 will require higher conviction before taking STRONG positions. Widening RSI bands (35/65 vs 40/60) will reduce false momentum signals in this choppy environment. Increasing vwap_dev_reversion_pct to 0.4% will better identify true mean-reversion opportunities rather than catching falling knives.

### Strategy Pattern
Wait for RSI<35 + VWAP dev>0.4% confluence for shorts in this declining regime

### Parameter Snapshot
strong_threshold=6 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=65 vwap_dev_reversion_pct=0.4 vwap_weight=1 cvd_weight=3 ob_weight=2

## Report #10 — 2026-03-16 22:46 UTC
### Summary
5/5 STRONG_SHORT trades failed with identical RSI=56.8/CVD=-250 conditions, suggesting model over-fitting to single failed setup.

### Parameter Snapshot
strong_threshold=6 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=65

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- WEAK score=0 rsi=n/a cvd=n/a ob=n/a atr%=n/a bb=n/a div=n/a -> reversed ($-0.20)
- STRONG_SHORT score=-5 rsi=56.8 cvd=-250 ob=0.83 atr%=0.24 bb=0.30 div=n/a -> stop_loss ($-0.26)
- STRONG_SHORT score=-8 rsi=56.8 cvd=-250 ob=0.02 atr%=0.24 bb=0.43 div=n/a -> stop_loss ($-0.28)

### Patterns
- All 5 STRONG_SHORT trades with RSI=56.8, CVD=-250, score -5 to -8 failed via stop_loss
- Identical entry conditions (RSI=56.8, CVD=-250, ATR%=0.24) across 4/5 trades suggests model over-clustering on single setup

### Memory Rules
- STRONG_SHORT at RSI>55 with CVD around -250 shows poor timing - shorts need RSI<45 for better edge
- Identical CVD values (-250) across multiple trades indicate stale/cached data - verify CVD freshness before strong entries

### Param Tuning Suggestions
- strong_threshold = 8 (Score -5 to -8 shorts all failed; need higher bar when RSI>55)
- rsi_overbought = 55 (RSI=56.8 shorts consistently failed - treat RSI>55 as too late for shorts)

## Tune — 2026-03-16 22:59 UTC
### Trigger
Inactivity

### Adjustments
- strong_threshold: 6.0000 -> 8.0000
- vwap_dev_reversion_pct: 0.4000 -> 0.3000
- vwap_weight: 1.0000 -> 2.0000

### Why
All 5 recent trades were shorts with losses, indicating the system is entering shorts too aggressively in what appears to be a choppy/ranging market. The price movement shows small body candles with relatively tight range (0.391% over 8m), suggesting consolidation rather than strong directional momentum. Raising strong_threshold from 6 to 8 will reduce false STRONG signals. Lowering vwap_dev_reversion_pct and increasing vwap_weight will help the system better respect mean reversion in this ranging environment and avoid chasing moves away from VWAP.

### Strategy Pattern
Range-bound consolidation - wait for cleaner breakouts

### Parameter Snapshot
strong_threshold=6 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=55 vwap_dev_reversion_pct=0.30000000000000004 vwap_weight=2 cvd_weight=3 ob_weight=2

## Tune — 2026-03-16 23:07 UTC
### Trigger
TradeCount(13)

### Adjustments
- strong_threshold: 6.0000 -> 5.0000
- cvd_weight: 3.0000 -> 4.0000
- vwap_dev_reversion_pct: 0.3000 -> 0.2500

### Why
All 5 recent shorts are profitable with strong negative scores (-2 to -9), indicating excellent signal quality but potentially missing opportunities. The consistent downward movement (-0.229% over 8m) and tight range (0.391%) suggest a controlled selloff where signals are working well. Lowering strong_threshold from 6 to 5 will capture more STRONG signals while maintaining quality. Increasing CVD weight from 3 to 4 will amplify volume-based selling pressure detection. Tightening VWAP reversion from 0.30 to 0.25 will be more selective about mean reversion trades in this trending environment.

### Strategy Pattern
Controlled selloff with tight ranges - favor short bias on bounces to VWAP

### Parameter Snapshot
strong_threshold=5 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=55 vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=4 ob_weight=2

## Report #11 — 2026-03-16 23:07 UTC
### Summary
Perfect 5/5 short batch contradicts previous RSI>55 short failures, suggesting market regime shift toward short-friendly conditions.

### Parameter Snapshot
strong_threshold=5 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=55

### Worked Conditions (examples)
- LEAN_SHORT score=-2 rsi=56.8 cvd=-250 ob=1.78 atr%=0.24 bb=0.44 div=n/a -> take_profit ($0.04)
- STRONG_SHORT score=-9 rsi=56.8 cvd=-250 ob=0.49 atr%=0.24 bb=0.83 div=n/a -> take_profit ($0.52)
- STRONG_SHORT score=-5 rsi=56.8 cvd=-250 ob=0.91 atr%=0.24 bb=0.79 div=n/a -> take_profit ($0.56)

### Failed Conditions (examples)
- none

### Patterns
- All 5 shorts won with identical RSI=56.8/CVD=-250 setup, contradicting memory rules about RSI>55 shorts failing
- STRONG_SHORT scores -5 to -9 all profitable despite being above rsi_overbought=55 threshold
- BB position 0.44-0.83 (upper half) shorts worked well, contrasting past BB<-0.5 failures

### Memory Rules
- RSI=56.8 with CVD=-250 and BB>0.4 shorts can be profitable when market structure supports - previous RSI>55 short avoidance may be outdated
- Strong shorts with scores -5 to -9 show consistent edge when CVD confirmation is strong (-250) - consider lowering strong_threshold when setup aligns

### Param Tuning Suggestions
- strong_threshold = 4 (All STRONG_SHORT scores -5 to -9 won; current threshold=5 may be missing profitable -4 setups)
- rsi_overbought = 60 (RSI=56.8 shorts all won despite being above current 55 threshold - market allows higher RSI shorts)
