use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use crate::agents::traits::Agent;
use crate::memory::store::MemoryStore;
use crate::types::*;

/// Short-term agent: scalps liquidity events intraday (minutes to hours).
///
/// Pattern: liq grab → CHoCH → FVG retracement → next liq pool
/// Only trades during kill zones (London / NY open).
pub struct ShortTermAgent {
    status: AgentStatus,
    paused_for_news: bool,
    // Micro-structure signals
    liq_grab_detected: bool,
    choch_confirmed: bool,
    stoch_rsi: Option<f64>,
    cvd_diverging: bool,
    funding_realtime: Option<f64>,
    atr_14: Option<f64>,
    price_btc: Option<f64>,
    price_eth: Option<f64>,
}

impl ShortTermAgent {
    pub fn new() -> Self {
        Self {
            status: AgentStatus::Watching,
            paused_for_news: false,
            liq_grab_detected: false,
            choch_confirmed: false,
            stoch_rsi: None,
            cvd_diverging: false,
            funding_realtime: None,
            atr_14: None,
            price_btc: None,
            price_eth: None,
        }
    }

    fn in_kill_zone(&self) -> bool {
        let hour = Utc::now().time().hour();
        // London open: 07-09 UTC, NY open: 13-15 UTC
        (7..=9).contains(&hour) || (13..=15).contains(&hour)
    }

    fn long_scalp_signal(&self) -> bool {
        let stoch = self.stoch_rsi.unwrap_or(50.0);
        let funding = self.funding_realtime.unwrap_or(0.01);
        self.liq_grab_detected
            && self.choch_confirmed
            && stoch < 20.0
            && funding.abs() < 0.05
            && self.in_kill_zone()
    }

    fn short_scalp_signal(&self) -> bool {
        let stoch = self.stoch_rsi.unwrap_or(50.0);
        let funding = self.funding_realtime.unwrap_or(0.01);
        funding > 0.05 && stoch > 80.0 && self.cvd_diverging && self.in_kill_zone()
    }

    pub fn pause_for_news(&mut self) {
        self.paused_for_news = true;
        self.status = AgentStatus::Paused;
    }

    pub fn resume(&mut self) {
        self.paused_for_news = false;
        self.status = AgentStatus::Watching;
    }
}

use chrono::Timelike;

#[async_trait]
impl Agent for ShortTermAgent {
    fn name(&self) -> AgentName {
        AgentName::ShortTerm
    }

    fn status(&self) -> AgentStatus {
        self.status
    }

    async fn on_event(&mut self, event: Event) -> Result<()> {
        if self.paused_for_news {
            return Ok(());
        }

        match event {
            Event::Price(pe) => match pe.symbol {
                Symbol::BTC => self.price_btc = Some(pe.price),
                Symbol::ETH => self.price_eth = Some(pe.price),
            },
            Event::Funding(fe) => {
                self.funding_realtime = Some(fe.rate);
            }
            Event::Liquidation(_) => {
                // TODO: detect liq grab pattern from cluster of liquidations
                self.liq_grab_detected = true;
            }
            Event::News(text) => {
                if text.urgency > 0.7 {
                    self.pause_for_news();
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn propose(&self) -> Result<Option<TradeProposal>> {
        if self.paused_for_news {
            return Ok(None);
        }

        if self.long_scalp_signal() {
            let price = self.price_btc.unwrap_or(0.0);
            let atr = self.atr_14.unwrap_or(price * 0.005);
            if price <= 0.0 {
                return Ok(None);
            }
            info!("ShortTerm: long scalp signal");
            return Ok(Some(TradeProposal {
                id: Uuid::new_v4(),
                proposer: AgentName::ShortTerm,
                symbol: Symbol::BTC,
                direction: Direction::Long,
                size_usd: 0.0,
                leverage: 10.0,
                entry_price: price,
                stop_loss: price - atr * 1.5,
                take_profit: price + atr * 2.0,
                rationale: "Liq grab + CHoCH + oversold stoch RSI in kill zone".into(),
                signals: vec![
                    format!("StochRSI={:.1}", self.stoch_rsi.unwrap_or(0.0)),
                    format!("Funding={:.4}", self.funding_realtime.unwrap_or(0.0)),
                    "LiqGrab=true".into(),
                ],
                timestamp: Utc::now(),
            }));
        }

        if self.short_scalp_signal() {
            let price = self.price_btc.unwrap_or(0.0);
            let atr = self.atr_14.unwrap_or(price * 0.005);
            if price <= 0.0 {
                return Ok(None);
            }
            info!("ShortTerm: short scalp signal");
            return Ok(Some(TradeProposal {
                id: Uuid::new_v4(),
                proposer: AgentName::ShortTerm,
                symbol: Symbol::BTC,
                direction: Direction::Short,
                size_usd: 0.0,
                leverage: 10.0,
                entry_price: price,
                stop_loss: price + atr * 1.5,
                take_profit: price - atr * 2.0,
                rationale: "Funding spike + overbought + CVD divergence".into(),
                signals: vec![
                    format!("Funding={:.4}", self.funding_realtime.unwrap_or(0.0)),
                    format!("StochRSI={:.1}", self.stoch_rsi.unwrap_or(0.0)),
                    "CVD_diverge=true".into(),
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
