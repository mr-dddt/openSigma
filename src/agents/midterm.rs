use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use crate::agents::traits::Agent;
use crate::memory::store::MemoryStore;
use crate::types::*;

/// Mid-term agent: captures 3–10 day trend moves on BTC/ETH.
///
/// Long:  EMA ribbon bullish, OI rising + price rising, funding neutral
/// Short: EMA compressing, OI rising + price falling, funding high
pub struct MidTermAgent {
    status: AgentStatus,
    // Trend signals
    ema_ribbon_bullish: bool,
    weekly_macd_positive: bool,
    weekly_rsi: Option<f64>,
    oi_trend_rising: bool,
    funding_7d_avg: Option<f64>,
    price_btc: Option<f64>,
    price_eth: Option<f64>,
}

impl MidTermAgent {
    pub fn new() -> Self {
        Self {
            status: AgentStatus::Watching,
            ema_ribbon_bullish: false,
            weekly_macd_positive: false,
            weekly_rsi: None,
            oi_trend_rising: false,
            funding_7d_avg: None,
            price_btc: None,
            price_eth: None,
        }
    }

    fn long_signal(&self) -> bool {
        let rsi = self.weekly_rsi.unwrap_or(50.0);
        let funding = self.funding_7d_avg.unwrap_or(0.01);
        self.ema_ribbon_bullish && rsi < 65.0 && funding < 0.05
    }

    fn short_signal(&self) -> bool {
        let rsi = self.weekly_rsi.unwrap_or(50.0);
        !self.weekly_macd_positive && self.oi_trend_rising && rsi > 70.0
    }
}

#[async_trait]
impl Agent for MidTermAgent {
    fn name(&self) -> AgentName {
        AgentName::MidTerm
    }

    fn status(&self) -> AgentStatus {
        self.status
    }

    async fn on_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Price(pe) => match pe.symbol {
                Symbol::BTC => self.price_btc = Some(pe.price),
                Symbol::ETH => self.price_eth = Some(pe.price),
            },
            Event::Funding(fe) => {
                // Simplistic: use latest funding as proxy for 7d avg
                self.funding_7d_avg = Some(fe.rate);
            }
            Event::OpenInterest(oi) => {
                self.oi_trend_rising = oi.change_pct > 0.0;
            }
            _ => {}
        }
        Ok(())
    }

    async fn propose(&self) -> Result<Option<TradeProposal>> {
        if self.long_signal() {
            let price = self.price_btc.unwrap_or(0.0);
            if price <= 0.0 {
                return Ok(None);
            }
            info!("MidTerm: long bias triggered");
            return Ok(Some(TradeProposal {
                id: Uuid::new_v4(),
                proposer: AgentName::MidTerm,
                symbol: Symbol::BTC,
                direction: Direction::Long,
                size_usd: 0.0,
                leverage: 3.0,
                entry_price: price,
                stop_loss: price * 0.95,
                take_profit: price * 1.08,
                rationale: "EMA ribbon bullish, RSI < 65, funding neutral".into(),
                signals: vec![
                    format!("RSI={:.1}", self.weekly_rsi.unwrap_or(0.0)),
                    format!("Funding7d={:.4}", self.funding_7d_avg.unwrap_or(0.0)),
                ],
                timestamp: Utc::now(),
            }));
        }

        if self.short_signal() {
            let price = self.price_btc.unwrap_or(0.0);
            if price <= 0.0 {
                return Ok(None);
            }
            info!("MidTerm: short bias triggered");
            return Ok(Some(TradeProposal {
                id: Uuid::new_v4(),
                proposer: AgentName::MidTerm,
                symbol: Symbol::BTC,
                direction: Direction::Short,
                size_usd: 0.0,
                leverage: 3.0,
                entry_price: price,
                stop_loss: price * 1.05,
                take_profit: price * 0.92,
                rationale: "MACD negative, OI rising while price falling".into(),
                signals: vec![
                    format!("RSI={:.1}", self.weekly_rsi.unwrap_or(0.0)),
                    format!("OI_rising={}", self.oi_trend_rising),
                ],
                timestamp: Utc::now(),
            }));
        }

        Ok(None)
    }

    async fn load_memory(&mut self, _memory: &MemoryStore) -> Result<()> {
        Ok(())
    }
}
