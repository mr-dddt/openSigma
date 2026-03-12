use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use crate::agents::traits::Agent;
use crate::memory::store::MemoryStore;
use crate::types::*;

/// Long-term agent: rides macro BTC/ETH cycles (weeks to months).
///
/// Entry: MVRV < 1, NUPL < 0, Puell low, Fear & Greed < 20
/// Exit:  MVRV > 7, NUPL > 0.75, Pi Cycle top, post-halving month 12–18
pub struct LongTermAgent {
    status: AgentStatus,
    // On-chain metrics (updated via events)
    mvrv_zscore: Option<f64>,
    nupl: Option<f64>,
    puell_multiple: Option<f64>,
    fear_greed: Option<f64>,
    btc_dominance: Option<f64>,
    price_btc: Option<f64>,
    price_eth: Option<f64>,
}

impl LongTermAgent {
    pub fn new() -> Self {
        Self {
            status: AgentStatus::Watching,
            mvrv_zscore: None,
            nupl: None,
            puell_multiple: None,
            fear_greed: None,
            btc_dominance: None,
            price_btc: None,
            price_eth: None,
        }
    }

    fn should_long(&self) -> bool {
        let mvrv = self.mvrv_zscore.unwrap_or(3.0);
        let nupl = self.nupl.unwrap_or(0.5);
        let fg = self.fear_greed.unwrap_or(50.0);
        mvrv < 1.0 && nupl < 0.0 && fg < 20.0
    }

    fn should_exit(&self) -> bool {
        let mvrv = self.mvrv_zscore.unwrap_or(3.0);
        let nupl = self.nupl.unwrap_or(0.5);
        mvrv > 7.0 || nupl > 0.75
    }
}

#[async_trait]
impl Agent for LongTermAgent {
    fn name(&self) -> AgentName {
        AgentName::LongTerm
    }

    fn status(&self) -> AgentStatus {
        self.status
    }

    async fn on_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::OnChain(oc) => match oc.metric.as_str() {
                "MVRV_zscore" => self.mvrv_zscore = Some(oc.value),
                "NUPL" => self.nupl = Some(oc.value),
                "Puell_multiple" => self.puell_multiple = Some(oc.value),
                _ => {}
            },
            Event::Price(pe) => match pe.symbol {
                Symbol::BTC => self.price_btc = Some(pe.price),
                Symbol::ETH => self.price_eth = Some(pe.price),
            },
            _ => {}
        }
        Ok(())
    }

    async fn propose(&self) -> Result<Option<TradeProposal>> {
        if self.should_long() {
            let price = self.price_btc.unwrap_or(0.0);
            if price <= 0.0 {
                return Ok(None);
            }
            info!("LongTerm: BTC accumulation signal detected");
            return Ok(Some(TradeProposal {
                id: Uuid::new_v4(),
                proposer: AgentName::LongTerm,
                symbol: Symbol::BTC,
                direction: Direction::Long,
                size_usd: 0.0, // WatchDog will size based on risk limits
                leverage: 1.0,
                entry_price: price,
                stop_loss: price * 0.85,    // 15% stop
                take_profit: price * 1.50,  // 50% target
                rationale: "Macro accumulation zone: MVRV < 1, NUPL < 0, extreme fear".into(),
                signals: vec![
                    format!("MVRV={:.2}", self.mvrv_zscore.unwrap_or(0.0)),
                    format!("NUPL={:.2}", self.nupl.unwrap_or(0.0)),
                    format!("FearGreed={:.0}", self.fear_greed.unwrap_or(0.0)),
                ],
                timestamp: Utc::now(),
            }));
        }
        Ok(None)
    }

    async fn load_memory(&mut self, _memory: &MemoryStore) -> Result<()> {
        // TODO: read lessons from memory.md to adjust thresholds
        Ok(())
    }
}
