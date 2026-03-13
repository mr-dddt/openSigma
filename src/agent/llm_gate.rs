use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::agent::llm_client::LlmClient;
use crate::config::Config;
use crate::journal::memory::MemoryManager;
use crate::types::*;

const SYSTEM_PROMPT: &str = r#"You are openSigma, an aggressive short-term BTC scalping agent on Hyperliquid perps. You trade fast, use high leverage, and aim for quick profits. Trades last under 10 minutes.

Respond with ONLY a valid JSON object — one of three variants:

1. Execute (example):
{"Execute":{"play_type":"PurePerpScalp","direction":"Long","size_pct":8.0,"hl_leverage":30,"stop_loss_pct":0.08,"take_profit_pct":0.15,"pm_hedge":null,"reasoning":"Strong EMA cross with CVD confirmation"}}

2. Skip:
{"Skip":{"reasoning":"Indicators conflicting"}}

3. SecondLook (recheck_after_secs: 10–30):
{"SecondLook":{"recheck_after_secs":15,"what_to_watch":"VWAP retest","original_bias":"Long","reasoning":"Entry timing uncertain"}}

AGGRESSIVE SCALPING RULES:
- You are biased toward EXECUTE. Only SKIP when signals clearly conflict.
- Default leverage: 20-30x. Use 30-50x on STRONG signals.
- stop_loss_pct: 0.05-0.15% of PRICE. Keep stops TIGHT — you'd rather get stopped and re-enter than hold a loser.
- take_profit_pct: 0.1-0.25% of PRICE. Take profits quickly — don't be greedy.
- size_pct: use 5-10% of capital per trade. You want meaningful position sizes.
- At 30x leverage with 0.1% SL = 3% account risk per trade. Acceptable.
- At 30x leverage with 0.15% TP = 4.5% account gain per trade. That's the edge.
- hl_leverage MUST NOT exceed max_leverage from config
- size_pct MUST NOT exceed max_trade_pct from config
- LEAN signals (net 3-4): Execute with 15-25x leverage
- STRONG signals (net 5+): Execute with 25-50x leverage, full size
- During BB squeeze near bands: HIGH CONVICTION mean-reversion, go aggressive
- On BB breakout: FULL SEND in breakout direction
- SecondLook only if timing is genuinely bad (not for hesitation)
- Keep reasoning concise (1 sentence)"#;

/// LLM Gate: builds context from signal snapshot + memory, sends to Claude,
/// parses the LlmDecision response.
pub struct LlmGate {
    client: LlmClient,
    memory: Arc<MemoryManager>,
}

impl LlmGate {
    pub fn new(client: LlmClient, memory: Arc<MemoryManager>) -> Self {
        Self { client, memory }
    }

    /// Build context string from signal snapshot and config.
    fn build_context(&self, snapshot: &SignalSnapshot, config: &Config) -> String {
        let ind = &snapshot.indicators;
        let (in_session, size_mult) = config.active_session();

        let bb_state = if ind.bb_squeeze {
            format!(
                "SQUEEZE (bw={:.3}%, pos={:+.2}) — price {} of bands",
                ind.bb_bandwidth.unwrap_or(0.0) * 100.0,
                ind.bb_position.unwrap_or(0.0),
                match ind.bb_position {
                    Some(p) if p <= -0.7 => "near LOWER (mean-reversion long candidate)",
                    Some(p) if p >= 0.7 => "near UPPER (mean-reversion short candidate)",
                    _ => "mid-range",
                },
            )
        } else {
            format!(
                "normal (bw={:.3}%, pos={:+.2}){}",
                ind.bb_bandwidth.unwrap_or(0.0) * 100.0,
                ind.bb_position.unwrap_or(0.0),
                match ind.bb_position {
                    Some(p) if p > 1.0 => " — BREAKOUT ABOVE upper band",
                    Some(p) if p < -1.0 => " — BREAKOUT BELOW lower band",
                    _ => "",
                },
            )
        };

        format!(
            "Signal: {} (net_score={}, bull={}, bear={})\n\
             EMA9={:.1} EMA21={:.1} RSI={:.1} StochRSI={:.1}\n\
             CVD={:.2} OB_Imbalance={:.2} ATR%={:.3}\n\
             BB: {} PM_div={}\n\
             Session: {} (size_mult={:.1})\n\
             Config: max_trade_pct={:.1}, max_leverage={}, max_duration={}s\n\
             \nMemory:\n{}",
            snapshot.level,
            snapshot.net_score,
            snapshot.bull_score,
            snapshot.bear_score,
            ind.ema_9.unwrap_or(0.0),
            ind.ema_21.unwrap_or(0.0),
            ind.rsi_14.unwrap_or(50.0),
            ind.stoch_rsi.unwrap_or(50.0),
            ind.cvd.unwrap_or(0.0),
            ind.ob_imbalance.unwrap_or(1.0),
            ind.atr_pct.unwrap_or(0.0),
            bb_state,
            ind.pm_divergence.map_or("none".to_string(), |v| format!("{:.2}", v)),
            if in_session { "active" } else { "inactive" },
            size_mult,
            config.capital.max_trade_pct,
            config.hyperliquid.max_leverage,
            config.execution.max_trade_duration_secs,
            self.memory.recent_summary(),
        )
    }

    /// Main entry: signal → LLM → decision.
    pub async fn evaluate(
        &self,
        snapshot: &SignalSnapshot,
        config: &Config,
    ) -> Result<LlmDecision> {
        let context = self.build_context(snapshot, config);
        info!(level = %snapshot.level, "Sending signal to LLM gate");
        self.client.decide(SYSTEM_PROMPT, &context).await
    }
}
