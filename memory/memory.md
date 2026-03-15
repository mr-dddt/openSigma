# openSigma Trading Memory

This file is loaded by all agents on startup and after each trade close.
Append new entries below. Do not edit past entries.

---

## Report #1 — 2026-03-14 00:30 UTC
- STRONG_SHORT PurePerpScalp entries show 25% win rate vs 37.5% for LEAN_SHORT - favor LEAN_SHORT signals

## Report #2 — 2026-03-14 01:03 UTC
- PurePerpScalp score -5 entries significantly outperform score -4 (70% vs 30% win rate)

## Tune — 2026-03-15 19:06 UTC
### Trigger
TradeCount(5)

### Adjustments
- strong_threshold: 6.0000 -> 7.0000
- rsi_oversold: 40.0000 -> 35.0000
- rsi_overbought: 60.0000 -> 65.0000

### Why
Analysis shows consistent short bias with mixed results. The one losing trade was STRONG_SHORT with score -6, suggesting the strong threshold of 6 may be too aggressive for current conditions. Raising to 7 will filter out marginal strong signals. The recent price action shows small upward movement (+0.039%) with low volatility, indicating we're in a consolidation phase where traditional RSI levels may not be as effective. Widening RSI bands (35/65) will reduce false signals in this low-volatility regime and require more extreme conditions for RSI contributions.

### Strategy Pattern
Consolidation break - watch for volume spike above 71750 resistance

### Parameter Snapshot
strong_threshold=6 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=65 vwap_dev_reversion_pct=0.3 vwap_weight=1 cvd_weight=2 ob_weight=2

## Report #9 — 2026-03-15 19:06 UTC
### Summary
Strong short bias batch (RSI=60, CVD=-110) with 80% win rate, but STRONG_SHORT score -6 failed while -7 succeeded.

### Parameter Snapshot
strong_threshold=6 lean_threshold=2 min_atr_pct=0.035 rsi_oversold=35 rsi_overbought=65

### Worked Conditions (examples)
- WEAK score=0 rsi=n/a cvd=n/a ob=n/a atr%=n/a bb=n/a -> expired ($0.01)
- LEAN_SHORT score=-2 rsi=60.0 cvd=-110 ob=1.37 atr%=0.12 bb=0.49 -> expired ($0.02)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.97 atr%=0.12 bb=0.41 -> expired ($0.00)

### Failed Conditions (examples)
- STRONG_SHORT score=-6 rsi=60.0 cvd=-110 ob=0.70 atr%=0.12 bb=0.35 -> stop_loss ($-0.11)

### Patterns
- Short bias batch with RSI=60, CVD=-110 across all valid entries - bearish conditions
- STRONG_SHORT score -6 hit stop loss while score -7 expired for profit - threshold matters
- BB position 0.35-0.50 range with squeeze setups - all shorts in middle band territory

### Memory Rules
- STRONG_SHORT score -7 outperforms score -6 in current conditions - consider strong_threshold=7
- Short entries with CVD=-110 and RSI=60 show 80% win rate when score ≤-4

### Param Tuning Suggestions
- strong_threshold = 7 (Score -6 STRONG_SHORT hit stop loss while score -7 expired for profit)

## Tune — 2026-03-15 19:42 UTC
### Trigger
TradeCount(13)

### Adjustments
- lean_threshold: 2.0000 -> 3.0000
- rsi_overbought: 65.0000 -> 70.0000
- rsi_oversold: 35.0000 -> 30.0000

### Why
All 5 recent trades were LEAN_SHORT with score -4, all losing trades with small losses (~$0.15-0.26). The consistent score of -4 suggests the lean threshold of 2 is too low, allowing marginal signals to trigger. Recent price action shows small upward movement (+0.039%) with low volatility (0.148% range, 0.016% avg body), indicating choppy/sideways conditions where weak signals perform poorly. Raising lean threshold to 3 will filter out these marginal -4 score signals. Also expanding RSI bounds to reduce false signals in low-volatility sideways markets.

### Strategy Pattern
avoid shorts in low-vol sideways chop

### Parameter Snapshot
strong_threshold=6 lean_threshold=3 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70 vwap_dev_reversion_pct=0.3 vwap_weight=1 cvd_weight=2 ob_weight=2

## Report #10 — 2026-03-15 19:42 UTC
### Summary
Batch of 5 identical LEAN_SHORT setups (score -4) all expired as losses, suggesting lean threshold too low for current sideways conditions.

### Parameter Snapshot
strong_threshold=6 lean_threshold=3 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12 bb=0.37 -> expired ($-0.22)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12 bb=0.37 -> expired ($-0.15)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12 bb=0.37 -> expired ($-0.18)

### Patterns
- All 5 trades identical setup: RSI=60, CVD=-110, OB=0.94, ATR%=0.12, BB_pos=0.37 - same market conditions
- LEAN_SHORT score -4 with 0% win rate (5/5 losses) - threshold too permissive for these conditions

### Memory Rules
- LEAN_SHORT score -4 failing consistently in current conditions - lean_threshold=4 may filter better
- Identical setups (RSI=60, CVD=-110) producing serial losses suggest avoiding repetitive entries

### Param Tuning Suggestions
- lean_threshold = 4 (Score -4 LEAN_SHORT entries show 0% win rate (5/5 losses) - raise threshold to filter marginal signals)

## Tune — 2026-03-15 19:59 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- vwap_weight: 1.0000 -> 2.0000
- vwap_dev_reversion_pct: 0.3000 -> 0.2500

### Why
All 5 recent LEAN_SHORT trades lost money with identical -4 scores, suggesting the lean threshold is too low for current market conditions. The recent price action shows small upward momentum (+0.039% over 8m) with relatively tight range (0.148%), indicating a consolidating market where mean reversion signals should be weighted more heavily. Raising lean threshold to 5 will reduce marginal signal frequency, while increasing VWAP weight and tightening reversion threshold will better capture mean-reverting moves in this low-volatility environment.

### Strategy Pattern
consolidation range - favor mean reversion over momentum

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70 vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=2 ob_weight=2

## Tune — 2026-03-15 20:25 UTC
### Trigger
Inactivity

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- rsi_weight: 1.0000 -> 2.0000
- vwap_weight: 2.0000 -> 3.0000

### Why
All 5 recent trades were LEAN_SHORT with identical -4 scores and all lost money, indicating the current lean threshold of 4 is too permissive. The market shows small but consistent upward movement (+0.039% over 8m) with low volatility, suggesting shorts at -4 score lack conviction. Raising lean threshold to 5 will reduce false signals. Increasing RSI and VWAP weights will better capture mean reversion opportunities in this low-volatility, slightly bullish environment where technical levels matter more.

### Strategy Pattern
Low vol grind up - wait for RSI >70 or strong VWAP deviation before shorting

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70 vwap_dev_reversion_pct=0.25 vwap_weight=3 cvd_weight=2 ob_weight=2

## Tune — 2026-03-15 20:39 UTC
### Trigger
TradeCount(21)

### Adjustments
- lean_threshold: 4.0000 -> 5.0000
- vwap_weight: 3.0000 -> 2.0000

### Why
All 5 recent trades were LEAN_SHORT signals with score=-4 that lost money during a slight upward movement (+0.039% over 8m). The consistent -4 score suggests signals are barely meeting the lean_threshold of 4, leading to weak conviction trades in choppy conditions. Raising lean_threshold to 5 will filter out these marginal signals. Additionally, reducing vwap_weight from 3 to 2 will decrease the influence of VWAP deviation signals that may be contributing to poor short entries during this consolidation phase.

### Strategy Pattern
avoid shorts in low-volatility consolidation

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70 vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #11 — 2026-03-15 20:39 UTC
### Summary
All 5 identical LEAN_SHORT entries (score -4) failed, confirming lean threshold too low for current sideways market conditions.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12 bb=0.37 -> expired ($-0.13)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12 bb=0.37 -> expired ($-0.13)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12 bb=0.37 -> expired ($-0.21)

### Patterns
- All 5 trades were identical LEAN_SHORT setups (RSI=60, CVD=-110, OB=0.94, ATR%=0.12, BB_pos=0.37) with score -4
- 100% failure rate for LEAN_SHORT score -4 in current conditions - all expired as losses

### Memory Rules
- LEAN_SHORT score -4 shows 100% loss rate in recent batches - lean_threshold=5 needed to filter weak signals
- Identical entry conditions producing serial losses - avoid repetitive setups until market structure changes

### Param Tuning Suggestions
- lean_threshold = 5 (Score -4 LEAN_SHORT entries show 0% win rate across multiple recent batches - raise threshold to filter marginal signals)
