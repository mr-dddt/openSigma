use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::agent::llm_client::LlmClient;
use crate::config::Config;
use crate::journal::memory::MemoryManager;
use crate::types::*;

const SYSTEM_PROMPT: &str = r#"You are openSigma, a short-term BTC trading agent operating on Hyperliquid (perps) and Polymarket (binary markets). Your job is to review a signal snapshot and decide whether to trade.

You MUST respond with ONLY a valid JSON object matching one of these three variants:

1. Execute:
{"Execute":{"play_type":"PurePerpScalp|PureBinaryBet|HedgedPerp|BinaryArbitrage|CrossMarketMomentum","direction":"Long|Short","size_pct":1.0-3.0,"hl_leverage":1-10,"stop_loss_pct":0.1-1.0,"take_profit_pct":0.2-2.0,"pm_hedge":null,"reasoning":"..."}}

2. Skip:
{"Skip":{"reasoning":"..."}}

3. SecondLook:
{"SecondLook":{"recheck_after_secs":15-180,"what_to_watch":"...","original_bias":"Long|Short","reasoning":"..."}}

Rules:
- size_pct MUST NOT exceed the max_trade_pct from config
- hl_leverage MUST NOT exceed max_leverage from config
- Prefer SKIP when indicators conflict or confidence is low
- Use SecondLook when setup looks promising but timing is uncertain
- Consider memory/lessons when making decisions
- Keep reasoning concise (1-2 sentences)"#;

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
