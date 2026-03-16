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
