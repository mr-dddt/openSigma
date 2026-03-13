use anyhow::Result;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::types::*;

const HL_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";

/// Hyperliquid WebSocket data feed.
/// Streams real-time price (allMids), trades, l2Book, and funding for BTC.
pub struct HyperliquidFeed {
    event_tx: mpsc::Sender<MarketEvent>,
}

impl HyperliquidFeed {
    pub fn new(event_tx: mpsc::Sender<MarketEvent>) -> Self {
        Self { event_tx }
    }

    pub async fn run(&self) {
        loop {
            match self.connect_and_stream().await {
                Ok(()) => {
                    warn!("HL WebSocket closed cleanly, reconnecting in 5s...");
                }
                Err(e) => {
                    error!("HL WebSocket error: {e:#}, reconnecting in 5s...");
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    async fn connect_and_stream(&self) -> Result<()> {
        let (ws_stream, _) = connect_async(HL_WS_URL).await?;
        let (mut write, mut read) = ws_stream.split();
        info!("Connected to Hyperliquid WebSocket");

        // Subscribe to allMids (price ticks for all assets)
        let sub_all_mids = serde_json::json!({
            "method": "subscribe",
            "subscription": { "type": "allMids" }
        });
        write.send(Message::Text(sub_all_mids.to_string())).await?;

        // Subscribe to trades for BTC
        let sub_trades = serde_json::json!({
            "method": "subscribe",
            "subscription": { "type": "trades", "coin": "BTC" }
        });
        write.send(Message::Text(sub_trades.to_string())).await?;

        // Subscribe to l2Book for BTC (order book)
        let sub_book = serde_json::json!({
            "method": "subscribe",
            "subscription": { "type": "l2Book", "coin": "BTC" }
        });
        write.send(Message::Text(sub_book.to_string())).await?;

        // Subscribe to activeAssetCtx for BTC (funding rate)
        let sub_ctx = serde_json::json!({
            "method": "subscribe",
            "subscription": { "type": "activeAssetCtx", "coin": "BTC" }
        });
        write.send(Message::Text(sub_ctx.to_string())).await?;

        info!("Subscribed to HL channels: allMids, trades(BTC), l2Book(BTC), activeAssetCtx(BTC)");

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Silently ignore parse errors — HL sends subscription confirmations,
                    // heartbeats, and other messages that don't match our known types.
                    let _ = self.handle_message(&text).await;
                }
                Ok(Message::Ping(data)) => {
                    let _ = write.send(Message::Pong(data)).await;
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    error!("HL WebSocket read error: {e}");
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_message(&self, text: &str) -> Result<()> {
        let msg: HlWsMessage = serde_json::from_str(text)?;

        match msg {
            HlWsMessage::AllMids(envelope) => {
                self.handle_all_mids(&envelope.data).await;
            }
            HlWsMessage::Trades(envelope) => {
                self.handle_trades(&envelope.data).await;
            }
            HlWsMessage::L2Book(envelope) => {
                self.handle_l2_book(&envelope.data).await;
            }
            HlWsMessage::ActiveAssetCtx(envelope) => {
                self.handle_asset_ctx(&envelope.data).await;
            }
            HlWsMessage::Other(_) => {}
        }

        Ok(())
    }

    async fn handle_all_mids(&self, mids: &AllMidsData) {
        // allMids returns a map of coin -> mid price string
        for symbol in [Symbol::BTC, Symbol::ETH] {
            let coin = symbol.to_string();
            if let Some(price_str) = mids.mids.get(&coin) {
                if let Ok(price) = price_str.parse::<f64>() {
                    let _ = self
                        .event_tx
                        .send(MarketEvent::Price(PriceTick {
                            symbol,
                            price,
                            timestamp: Utc::now(),
                        }))
                        .await;
                }
            }
        }
    }

    async fn handle_trades(&self, trades: &TradesData) {
        for t in &trades.trades {
            let symbol = match t.coin.as_str() {
                "BTC" => Symbol::BTC,
                "ETH" => Symbol::ETH,
                _ => continue,
            };
            let side = if t.side == "B" {
                Direction::Long
            } else {
                Direction::Short
            };
            let price = t.px.parse::<f64>().unwrap_or(0.0);
            let size = t.sz.parse::<f64>().unwrap_or(0.0);

            let _ = self
                .event_tx
                .send(MarketEvent::Trade(TradeTick {
                    symbol,
                    price,
                    size,
                    side,
                    timestamp: Utc::now(),
                }))
                .await;
        }
    }

    async fn handle_asset_ctx(&self, ctx: &ActiveAssetCtxData) {
        let symbol = match ctx.coin.as_str() {
            "BTC" => Symbol::BTC,
            "ETH" => Symbol::ETH,
            _ => return,
        };
        if let Some(ref funding) = ctx.ctx.funding {
            if let Ok(rate) = funding.parse::<f64>() {
                info!(symbol = %symbol, rate = rate, "Funding rate update");
                let _ = self
                    .event_tx
                    .send(MarketEvent::Funding(FundingTick {
                        symbol,
                        rate,
                        timestamp: Utc::now(),
                    }))
                    .await;
            }
        }
    }

    async fn handle_l2_book(&self, book: &L2BookData) {
        let symbol = match book.coin.as_str() {
            "BTC" => Symbol::BTC,
            "ETH" => Symbol::ETH,
            _ => return,
        };

        let parse_levels = |levels: &[L2Level]| -> Vec<(f64, f64)> {
            levels
                .iter()
                .filter_map(|l| {
                    let px = l.px.parse::<f64>().ok()?;
                    let sz = l.sz.parse::<f64>().ok()?;
                    Some((px, sz))
                })
                .collect()
        };

        let bids = parse_levels(&book.levels[0]);
        let asks = if book.levels.len() > 1 {
            parse_levels(&book.levels[1])
        } else {
            vec![]
        };

        let _ = self
            .event_tx
            .send(MarketEvent::OrderBook(OrderBookSnapshot {
                symbol,
                bids,
                asks,
                timestamp: Utc::now(),
            }))
            .await;
    }
}

// ---------------------------------------------------------------------------
// Hyperliquid WebSocket JSON structures
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum HlWsMessage {
    AllMids(AllMidsEnvelope),
    Trades(TradesEnvelope),
    L2Book(L2BookEnvelope),
    ActiveAssetCtx(ActiveAssetCtxEnvelope),
    Other(serde_json::Value),
}

// We need to handle the nested envelope: {"channel": "allMids", "data": {...}}

#[derive(Debug, Deserialize)]
struct AllMidsEnvelope {
    #[serde(rename = "channel")]
    _channel: AllMidsChannel,
    data: AllMidsData,
}

#[derive(Debug, Deserialize)]
enum AllMidsChannel {
    #[serde(rename = "allMids")]
    AllMids,
}

// Flatten the envelope pattern — Hyperliquid sends:
// {"channel": "allMids", "data": {"mids": {"BTC": "83000.5", ...}}}
// We use untagged enum + channel field matching.
// Since untagged tries each variant in order, we rely on field presence.

// Actually, let's use a simpler approach with a raw Value first:

use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct AllMidsData {
    mids: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct TradesEnvelope {
    #[serde(rename = "channel")]
    _channel: TradesChannel,
    data: TradesData,
}

#[derive(Debug, Deserialize)]
enum TradesChannel {
    #[serde(rename = "trades")]
    Trades,
}

#[derive(Debug, Deserialize)]
struct TradesData {
    trades: Vec<HlTrade>,
}

#[derive(Debug, Deserialize)]
struct HlTrade {
    coin: String,
    side: String, // "B" or "A"
    px: String,
    sz: String,
}

#[derive(Debug, Deserialize)]
struct L2BookEnvelope {
    #[serde(rename = "channel")]
    _channel: L2BookChannel,
    data: L2BookData,
}

#[derive(Debug, Deserialize)]
enum L2BookChannel {
    #[serde(rename = "l2Book")]
    L2Book,
}

#[derive(Debug, Deserialize)]
struct L2BookData {
    coin: String,
    levels: Vec<Vec<L2Level>>,
}

#[derive(Debug, Deserialize)]
struct L2Level {
    px: String,
    sz: String,
}

// activeAssetCtx envelope: {"channel": "activeAssetCtx", "data": {"coin": "BTC", "ctx": {"funding": "0.00012", ...}}}

#[derive(Debug, Deserialize)]
struct ActiveAssetCtxEnvelope {
    #[serde(rename = "channel")]
    _channel: ActiveAssetCtxChannel,
    data: ActiveAssetCtxData,
}

#[derive(Debug, Deserialize)]
enum ActiveAssetCtxChannel {
    #[serde(rename = "activeAssetCtx")]
    ActiveAssetCtx,
}

#[derive(Debug, Deserialize)]
struct ActiveAssetCtxData {
    coin: String,
    ctx: AssetCtx,
}

#[derive(Debug, Deserialize)]
struct AssetCtx {
    funding: Option<String>,
}
