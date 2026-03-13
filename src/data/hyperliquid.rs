use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::types::*;

const HL_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";

/// Map HL coin string to our Symbol enum.
fn coin_to_symbol(coin: &str) -> Option<Symbol> {
    match coin {
        "BTC" => Some(Symbol::BTC),
        "ETH" => Some(Symbol::ETH),
        "SOL" => Some(Symbol::SOL),
        "HYPE" => Some(Symbol::HYPE),
        "CL/USDC" => Some(Symbol::ClUsdc),
        _ => None,
    }
}

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

        // Subscribe to trades, l2Book, and activeAssetCtx for each tradeable symbol
        for symbol in Symbol::all() {
            let coin = symbol.hl_coin();

            let sub_trades = serde_json::json!({
                "method": "subscribe",
                "subscription": { "type": "trades", "coin": coin }
            });
            write.send(Message::Text(sub_trades.to_string())).await?;

            let sub_book = serde_json::json!({
                "method": "subscribe",
                "subscription": { "type": "l2Book", "coin": coin }
            });
            write.send(Message::Text(sub_book.to_string())).await?;

            let sub_ctx = serde_json::json!({
                "method": "subscribe",
                "subscription": { "type": "activeAssetCtx", "coin": coin }
            });
            write.send(Message::Text(sub_ctx.to_string())).await?;
        }

        let coins: Vec<&str> = Symbol::all().iter().map(|s| s.hl_coin()).collect();
        info!(coins = ?coins, "Subscribed to HL channels: allMids + trades/l2Book/activeAssetCtx per coin");

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
        for symbol in Symbol::all() {
            let coin = symbol.hl_coin().to_string();
            if let Some(price_str) = mids.mids.get(&coin) {
                if let Ok(price) = price_str.parse::<f64>() {
                    let _ = self
                        .event_tx
                        .send(MarketEvent::Price(PriceTick {
                            symbol: *symbol,
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
            let symbol = match coin_to_symbol(&t.coin) {
                Some(s) => s,
                None => continue,
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
        let symbol = match coin_to_symbol(&ctx.coin) {
            Some(s) => s,
            None => return,
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
        let symbol = match coin_to_symbol(&book.coin) {
            Some(s) => s,
            None => return,
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

use chrono::TimeZone;
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

// ---------------------------------------------------------------------------
// Historical candle snapshot (REST API) — eliminates warm-up delay
// ---------------------------------------------------------------------------

const HL_INFO_URL: &str = "https://api.hyperliquid.xyz/info";

#[derive(Debug, Deserialize)]
struct HlCandleRaw {
    #[serde(rename = "t")]
    timestamp_ms: u64,
    #[serde(rename = "o")]
    open: String,
    #[serde(rename = "h")]
    high: String,
    #[serde(rename = "l")]
    low: String,
    #[serde(rename = "c")]
    close: String,
    #[serde(rename = "v")]
    volume: String,
}

/// Fetch historical candles from Hyperliquid's REST API.
/// Returns parsed Candle structs sorted oldest-first, ready to push
/// into Indicators.
pub async fn fetch_historical_candles(
    coin: &str,
    interval: &str,
    count: usize,
) -> Result<Vec<Candle>> {
    let client = reqwest::Client::new();

    let now_ms = Utc::now().timestamp_millis() as u64;
    let interval_ms: u64 = match interval {
        "1m" => 60_000,
        "5m" => 300_000,
        _ => 60_000,
    };
    let start_ms = now_ms - (count as u64 * interval_ms);

    let body = serde_json::json!({
        "type": "candleSnapshot",
        "req": {
            "coin": coin,
            "interval": interval,
            "startTime": start_ms,
            "endTime": now_ms
        }
    });

    let resp = client
        .post(HL_INFO_URL)
        .json(&body)
        .send()
        .await
        .context("Failed to fetch HL historical candles")?;

    let status = resp.status();
    if !status.is_success() {
        let text: String = resp.text().await.unwrap_or_default();
        anyhow::bail!("HL candle API returned {status}: {text}");
    }

    let raw: Vec<HlCandleRaw> = resp
        .json::<Vec<HlCandleRaw>>()
        .await
        .context("Failed to parse HL candle response")?;

    let candles: Vec<Candle> = raw
        .into_iter()
        .filter_map(|c| {
            Some(Candle {
                open: c.open.parse().ok()?,
                high: c.high.parse().ok()?,
                low: c.low.parse().ok()?,
                close: c.close.parse().ok()?,
                volume: c.volume.parse().ok()?,
                timestamp: Utc.timestamp_millis_opt(c.timestamp_ms as i64).single()?,
            })
        })
        .collect();

    info!(
        coin = coin,
        interval = interval,
        fetched = candles.len(),
        "Historical candles loaded"
    );

    Ok(candles)
}
