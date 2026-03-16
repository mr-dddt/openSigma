# openSigma Trading Memory

This file is loaded by all agents on startup and after each
trade close. Append new entries below. Do not edit past
entries.

---

## Report #10 — 2026-03-15 19:42 UTC

### Summary

Batch of 5 identical LEAN_SHORT setups (score -4) all
expired as losses, suggesting lean threshold too low for
current sideways conditions.

### Parameter Snapshot

strong_threshold=6 lean_threshold=3 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)

- none

### Failed Conditions (examples)

- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12
  bb=0.37 -> expired ($-0.22)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12
  bb=0.37 -> expired ($-0.15)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12
  bb=0.37 -> expired ($-0.18)

### Patterns

- All 5 trades identical setup: RSI=60, CVD=-110, OB=0.94,
  ATR%=0.12, BB_pos=0.37 - same market conditions
- LEAN_SHORT score -4 with 0% win rate (5/5 losses) -
  threshold too permissive for these conditions

### Memory Rules

- LEAN_SHORT score -4 failing consistently in current
  conditions - lean_threshold=4 may filter better
- Identical setups (RSI=60, CVD=-110) producing serial
  losses suggest avoiding repetitive entries

### Param Tuning Suggestions

- lean_threshold = 4 (Score -4 LEAN_SHORT entries show 0%
  win rate (5/5 losses) - raise threshold to filter marginal
  signals)

## Tune — 2026-03-15 19:59 UTC

### Trigger

Inactivity

### Adjustments

- lean_threshold: 4.0000 -> 5.0000
- vwap_weight: 1.0000 -> 2.0000
- vwap_dev_reversion_pct: 0.3000 -> 0.2500

### Why

All 5 recent LEAN_SHORT trades lost money with identical -4
scores, suggesting the lean threshold is too low for current
market conditions. The recent price action shows small
upward momentum (+0.039% over 8m) with relatively tight
range (0.148%), indicating a consolidating market where mean
reversion signals should be weighted more heavily. Raising
lean threshold to 5 will reduce marginal signal frequency,
while increasing VWAP weight and tightening reversion
threshold will better capture mean-reverting moves in this
low-volatility environment.

### Strategy Pattern

consolidation range - favor mean reversion over momentum

### Parameter Snapshot

strong_threshold=6 lean_threshold=4 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70
vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=2
ob_weight=2

## Tune — 2026-03-15 20:25 UTC

### Trigger

Inactivity

### Adjustments

- lean_threshold: 4.0000 -> 5.0000
- rsi_weight: 1.0000 -> 2.0000
- vwap_weight: 2.0000 -> 3.0000

### Why

All 5 recent trades were LEAN_SHORT with identical -4 scores
and all lost money, indicating the current lean threshold of
4 is too permissive. The market shows small but consistent
upward movement (+0.039% over 8m) with low volatility,
suggesting shorts at -4 score lack conviction. Raising lean
threshold to 5 will reduce false signals. Increasing RSI and
VWAP weights will better capture mean reversion
opportunities in this low-volatility, slightly bullish
environment where technical levels matter more.

### Strategy Pattern

Low vol grind up - wait for RSI >70 or strong VWAP deviation
before shorting

### Parameter Snapshot

strong_threshold=6 lean_threshold=4 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70
vwap_dev_reversion_pct=0.25 vwap_weight=3 cvd_weight=2
ob_weight=2

## Tune — 2026-03-15 20:39 UTC

### Trigger

TradeCount(21)

### Adjustments

- lean_threshold: 4.0000 -> 5.0000
- vwap_weight: 3.0000 -> 2.0000

### Why

All 5 recent trades were LEAN_SHORT signals with score=-4
that lost money during a slight upward movement (+0.039%
over 8m). The consistent -4 score suggests signals are
barely meeting the lean_threshold of 4, leading to weak
conviction trades in choppy conditions. Raising
lean_threshold to 5 will filter out these marginal signals.
Additionally, reducing vwap_weight from 3 to 2 will decrease
the influence of VWAP deviation signals that may be
contributing to poor short entries during this consolidation
phase.

### Strategy Pattern

avoid shorts in low-volatility consolidation

### Parameter Snapshot

strong_threshold=6 lean_threshold=4 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70
vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=2
ob_weight=2

## Report #11 — 2026-03-15 20:39 UTC

### Summary

All 5 identical LEAN_SHORT entries (score -4) failed,
confirming lean threshold too low for current sideways
market conditions.

### Parameter Snapshot

strong_threshold=6 lean_threshold=4 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)

- none

### Failed Conditions (examples)

- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12
  bb=0.37 -> expired ($-0.13)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12
  bb=0.37 -> expired ($-0.13)
- LEAN_SHORT score=-4 rsi=60.0 cvd=-110 ob=0.94 atr%=0.12
  bb=0.37 -> expired ($-0.21)

### Patterns

- All 5 trades were identical LEAN_SHORT setups (RSI=60,
  CVD=-110, OB=0.94, ATR%=0.12, BB_pos=0.37) with score -4
- 100% failure rate for LEAN_SHORT score -4 in current
  conditions - all expired as losses

### Memory Rules

- LEAN_SHORT score -4 shows 100% loss rate in recent
  batches - lean_threshold=5 needed to filter weak signals
- Identical entry conditions producing serial losses - avoid
  repetitive setups until market structure changes

### Param Tuning Suggestions

- lean_threshold = 5 (Score -4 LEAN_SHORT entries show 0%
  win rate across multiple recent batches - raise threshold
  to filter marginal signals)

## Tune — 2026-03-16 00:00 UTC

### Trigger

TradeCount(5)

### Adjustments

- strong_threshold: 4.0000 -> 5.0000
- lean_threshold: 2.0000 -> 3.0000

### Why

The recent trades show a clear pattern: STRONG_LONG signals
(scores 5-6) are profitable with $0.12-0.30 gains, while
weaker signals are marginal or losing (WEAK score=0 made
$0.04, LEAN_SHORT score=-2 lost $0.02). The market is
showing directional momentum with +0.101% recent movement
and good volatility (0.222% range). By raising thresholds
slightly, we filter out the weak/marginal signals that
aren't performing well while keeping the profitable strong
signals intact.

### Strategy Pattern

Bullish momentum regime - focus on long-biased entries on
pullbacks

### Parameter Snapshot

strong_threshold=5 lean_threshold=3 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70
vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=2
ob_weight=2

## Report #12 — 2026-03-16 00:01 UTC

### Summary

Strong bullish batch: STRONG_LONG entries (RSI=63, CVD=665)
achieved 100% win rate while single LEAN_SHORT failed.

### Parameter Snapshot

strong_threshold=5 lean_threshold=3 min_atr_pct=0.035
rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)

- WEAK score=0 rsi=n/a cvd=n/a ob=n/a atr%=n/a bb=n/a ->
  take_profit ($0.04)
- STRONG_LONG score=5 rsi=62.7 cvd=665 ob=1.25 atr%=0.27
  bb=0.26 -> reversed ($0.30)
- STRONG_LONG score=6 rsi=62.7 cvd=665 ob=3.77 atr%=0.27
  bb=0.32 -> reversed ($0.12)

### Failed Conditions (examples)

- LEAN_SHORT score=-2 rsi=62.7 cvd=665 ob=0.36 atr%=0.27
  bb=0.42 -> stop_loss ($-0.02)

### Patterns

- STRONG_LONG entries with RSI=62.7 and CVD=665 showing 100%
  win rate (3/3 trades)
- Strong entries at score 5-6 outperforming with consistent
  profits ($0.12-0.30 range)
- Single LEAN_SHORT entry failed while all STRONG_LONG
  entries succeeded

### Memory Rules

- STRONG_LONG with RSI~63 and CVD>600 shows excellent win
  rate - favor these bullish setups
- Score 5-6 STRONG_LONG entries consistently profitable in
  current conditions - threshold working well

### Param Tuning Suggestions

- No param changes suggested.

## Tune — 2026-03-16 00:10 UTC
### Trigger
TradeCount(6)

### Adjustments
- strong_threshold: 5.0000 -> 6.0000
- vwap_dev_reversion_pct: 0.2500 -> 0.3500

### Why
All 5 recent trades were losing STRONG_LONG signals with scores 6-7, indicating the strong threshold of 5 is too low for current market conditions. The price is showing upward momentum (+0.33% in 8m) but trades are still failing, suggesting we're entering positions too early in the move. Raising strong_threshold to 6 will require higher conviction before triggering. Also increasing vwap_dev_reversion_pct to 0.35 will reduce anti-chasing behavior and allow riding momentum moves better.

### Strategy Pattern
Wait for pullbacks on momentum - let price extend further before entry

### Parameter Snapshot
strong_threshold=6 lean_threshold=3 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70 vwap_dev_reversion_pct=0.35 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #13 — 2026-03-16 00:11 UTC
### Summary
5 identical STRONG_LONG setups (RSI=68.6, CVD=627) all failed via stop loss, suggesting overbought entries despite high conviction scores.

### Parameter Snapshot
strong_threshold=6 lean_threshold=3 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=70

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- STRONG_LONG score=7 rsi=68.6 cvd=627 ob=3.61 atr%=0.28 bb=0.55 div=n/a -> stop_loss ($-0.32)
- STRONG_LONG score=7 rsi=68.6 cvd=627 ob=166.28 atr%=0.28 bb=0.55 div=n/a -> stop_loss ($-0.34)
- STRONG_LONG score=6 rsi=68.6 cvd=627 ob=1.40 atr%=0.28 bb=0.56 div=n/a -> stop_loss ($-0.23)

### Patterns
- All 5 STRONG_LONG entries (RSI=68.6, CVD=627, BB_pos~0.5) with scores 6-7 hit stop loss during upward momentum
- Identical entry conditions (RSI=68.6, CVD=627, ATR%=0.28) producing 100% failure rate suggests overbought timing

### Memory Rules
- STRONG_LONG with RSI>68 and CVD>600 shows poor timing - entries too late in move despite high scores
- Score 6-7 STRONG_LONG failing consistently when RSI>68 - consider higher RSI overbought threshold

### Param Tuning Suggestions
- strong_threshold = 8 (Scores 6-7 STRONG_LONG with 0% win rate (5/5 losses) - need higher conviction filter)
- rsi_overbought = 65 (All entries at RSI=68.6 failed - lower overbought threshold to avoid late entries)

## Tune — 2026-03-16 00:20 UTC
### Trigger
TradeCount(11)

### Adjustments
- strong_threshold: 6.0000 -> 7.0000
- lean_threshold: 3.0000 -> 4.0000
- vwap_dev_reversion_pct: 0.3500 -> 0.2500

### Why
All 5 recent trades are losing longs despite upward price movement (+0.33% in 8m), suggesting signals are firing too early in the move. The STRONG_LONG with score 7 still lost money, indicating the threshold of 6 is too low. Multiple LEAN_LONG signals with score 5 also failed, so raising lean threshold to 4. The consistent long bias during an uptrend suggests VWAP mean reversion is triggering too late - tightening to 0.25% to catch reversions earlier and avoid chasing.

### Strategy Pattern
Wait for pullbacks in uptrend - avoid chasing breakouts

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=65 vwap_dev_reversion_pct=0.25 vwap_weight=2 cvd_weight=2 ob_weight=2

## Report #14 — 2026-03-16 00:20 UTC
### Summary
5 identical long setups at overbought RSI=68.6 all failed via stop loss despite positive CVD, confirming need to avoid late entries.

### Parameter Snapshot
strong_threshold=6 lean_threshold=4 min_atr_pct=0.035 rsi_oversold=30 rsi_overbought=65

### Worked Conditions (examples)
- none

### Failed Conditions (examples)
- STRONG_LONG score=7 rsi=68.6 cvd=627 ob=29.48 atr%=0.28 bb=0.47 div=n/a -> stop_loss ($-0.31)
- LEAN_LONG score=5 rsi=68.6 cvd=627 ob=21.87 atr%=0.28 bb=0.23 div=n/a -> stop_loss ($-0.05)
- LEAN_LONG score=5 rsi=68.6 cvd=627 ob=15.09 atr%=0.28 bb=0.25 div=n/a -> stop_loss ($-0.06)

### Patterns
- All 5 long entries at RSI=68.6 (above overbought threshold of 65) with identical CVD=627 failed via stop loss
- Strong entries (score 7) and lean entries (score 5) both failed with same market conditions - suggests timing issue rather than conviction
- BB position 0.23-0.47 range with identical ATR%=0.28 across all trades - low volatility squeeze conditions

### Memory Rules
- Long entries at RSI>68 consistently fail even with high CVD - avoid overbought longs regardless of score
- Identical entry conditions (RSI=68.6, CVD=627) producing serial losses suggest waiting for different market structure

### Param Tuning Suggestions
- rsi_overbought = 65.0 (All entries at RSI=68.6 failed - current threshold of 70 allows overbought longs)
- strong_threshold = 8 (Score 7 STRONG_LONG still failed at overbought RSI - need higher conviction filter)
