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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::traits::Agent;

    #[test]
    fn defaults_no_signal() {
        let agent = ShortTermAgent::new();
        assert!(!agent.long_scalp_signal());
        assert!(!agent.short_scalp_signal());
        assert!(!agent.paused_for_news);
        assert_eq!(agent.status(), AgentStatus::Watching);
    }

    #[test]
    fn pause_and_resume() {
        let mut agent = ShortTermAgent::new();
        agent.pause_for_news();
        assert!(agent.paused_for_news);
        assert_eq!(agent.status(), AgentStatus::Paused);
        agent.resume();
        assert!(!agent.paused_for_news);
        assert_eq!(agent.status(), AgentStatus::Watching);
    }

    #[tokio::test]
    async fn on_event_ignores_when_paused() {
        let mut agent = ShortTermAgent::new();
        agent.pause_for_news();
        // Price event should be ignored
        agent
            .on_event(Event::Price(PriceEvent {
                symbol: Symbol::BTC,
                price: 50000.0,
                volume_24h: 1e9,
                timestamp: Utc::now(),
            }))
            .await
            .unwrap();
        assert!(agent.price_btc.is_none());
    }

    #[tokio::test]
    async fn on_event_updates_funding() {
        let mut agent = ShortTermAgent::new();
        agent
            .on_event(Event::Funding(FundingEvent {
                symbol: Symbol::BTC,
                rate: 0.02,
                timestamp: Utc::now(),
            }))
            .await
            .unwrap();
        assert_eq!(agent.funding_realtime, Some(0.02));
    }

    #[tokio::test]
    async fn on_event_sets_liq_grab() {
        let mut agent = ShortTermAgent::new();
        assert!(!agent.liq_grab_detected);
        agent
            .on_event(Event::Liquidation(LiquidationEvent {
                symbol: Symbol::BTC,
                direction: Direction::Long,
                size_usd: 100000.0,
                price: 49000.0,
                timestamp: Utc::now(),
            }))
            .await
            .unwrap();
        assert!(agent.liq_grab_detected);
    }

    #[tokio::test]
    async fn high_urgency_news_pauses_agent() {
        let mut agent = ShortTermAgent::new();
        agent
            .on_event(Event::News(TextEvent {
                source: "test".into(),
                headline: "Flash crash".into(),
                sentiment: -0.9,
                urgency: 0.8, // > 0.7 threshold
                dedup_hash: 1,
                timestamp: Utc::now(),
            }))
            .await
            .unwrap();
        assert!(agent.paused_for_news);
        assert_eq!(agent.status(), AgentStatus::Paused);
    }

    #[tokio::test]
    async fn low_urgency_news_does_not_pause() {
        let mut agent = ShortTermAgent::new();
        agent
            .on_event(Event::News(TextEvent {
                source: "test".into(),
                headline: "Minor update".into(),
                sentiment: 0.1,
                urgency: 0.3, // < 0.7
                dedup_hash: 2,
                timestamp: Utc::now(),
            }))
            .await
            .unwrap();
        assert!(!agent.paused_for_news);
    }

    #[tokio::test]
    async fn propose_none_when_paused() {
        let mut agent = ShortTermAgent::new();
        agent.pause_for_news();
        let proposal = agent.propose().await.unwrap();
        assert!(proposal.is_none());
    }

    #[test]
    fn kill_zone_detection() {
        // This test is time-dependent, so we just verify the method runs
        let agent = ShortTermAgent::new();
        let _result = agent.in_kill_zone(); // doesn't panic
    }
}
