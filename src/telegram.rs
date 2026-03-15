use anyhow::Result;
use serde::Serialize;
use tracing::{info, warn};

use crate::types::{Direction, PlayType, TradeRecord};

/// Telegram Bot API client for trade alerts.
#[derive(Clone)]
pub struct TelegramClient {
    client: reqwest::Client,
    bot_token: String,
    chat_id: String,
}

#[derive(Serialize)]
struct SendMessage {
    chat_id: String,
    text: String,
    parse_mode: String,
}

/// Escape special characters for Telegram MarkdownV2 parse mode.
fn escape_md(text: &str) -> String {
    let special = ['\\', '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'];
    let mut escaped = String::with_capacity(text.len() + text.len() / 4);
    for ch in text.chars() {
        if special.contains(&ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

impl TelegramClient {
    pub fn new(bot_token: String, chat_id: String) -> Option<Self> {
        if bot_token.is_empty() || chat_id.is_empty() {
            info!("Telegram not configured (missing token or chat_id)");
            return None;
        }
        info!("TelegramClient initialized");
        Some(Self {
            client: reqwest::Client::new(),
            bot_token,
            chat_id,
        })
    }

    /// Send a raw text message (MarkdownV2).
    pub async fn send(&self, text: &str) -> Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.bot_token
        );

        let msg = SendMessage {
            chat_id: self.chat_id.clone(),
            text: text.to_string(),
            parse_mode: "MarkdownV2".to_string(),
        };

        let resp = self.client.post(&url).json(&msg).send().await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(body = %body, "Telegram send failed");
        }
        Ok(())
    }

    /// Send trade open alert.
    pub async fn send_trade_open(
        &self,
        play_type: PlayType,
        direction: Direction,
        size_usd: f64,
        entry_price: f64,
        leverage: u8,
        reasoning: &str,
    ) {
        let dir_emoji = if direction == Direction::Long { "🟢" } else { "🔴" };
        let msg = format!(
            "{dir_emoji} *Trade Opened*\n{dir} {pt} \\| ${sz} @{px} lev\\={lev}\n_{reason}_",
            dir = escape_md(&direction.to_string()),
            pt = escape_md(&play_type.to_string()),
            sz = escape_md(&format!("{size_usd:.0}")),
            px = escape_md(&format!("{entry_price:.0}")),
            lev = leverage,
            reason = escape_md(&reasoning.chars().take(100).collect::<String>()),
        );
        if let Err(e) = self.send(&msg).await {
            warn!("Telegram trade open alert failed: {e:#}");
        }
    }

    /// Send trade close alert with PnL.
    pub async fn send_trade_close(&self, record: &TradeRecord) {
        let pnl = record.pnl_usd.unwrap_or(0.0);
        let pnl_emoji = if pnl >= 0.0 { "✅" } else { "❌" };
        let sign = if pnl >= 0.0 { "\\+" } else { "" };
        let exit = escape_md(record.exit_reason.as_deref().unwrap_or("?"));
        let msg = format!(
            "{pnl_emoji} *Trade Closed*\n{dir} {pt} \\| {lvl} \\| {exit}\nPnL: `{sign}{pnl:.2}` USD\nCapital: `${cap:.2}`",
            dir = escape_md(&record.direction.to_string()),
            pt = escape_md(&record.play_type.to_string()),
            lvl = escape_md(&record.signal_level.to_string()),
            cap = record.capital_after.unwrap_or(0.0),
        );
        if let Err(e) = self.send(&msg).await {
            warn!("Telegram trade alert failed: {e:#}");
        }
    }

    /// Send report summary.
    pub async fn send_report(&self, summary: &str) {
        let msg = format!("📊 *20\\-Trade Report*\n{}", escape_md(summary));
        if let Err(e) = self.send(&msg).await {
            warn!("Telegram report alert failed: {e:#}");
        }
    }

    /// Send kill switch alert.
    pub async fn send_kill_switch(&self) {
        let msg = "🚨 *KILL SWITCH TRIGGERED*\nAll positions closed\\. Trading halted\\.";
        if let Err(e) = self.send(msg).await {
            warn!("Telegram kill switch alert failed: {e:#}");
        }
    }
}
