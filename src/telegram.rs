use anyhow::Result;
use serde::Serialize;
use tracing::{info, warn};

use crate::types::TradeRecord;

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

    /// Send a raw text message.
    pub async fn send(&self, text: &str) -> Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.bot_token
        );

        let msg = SendMessage {
            chat_id: self.chat_id.clone(),
            text: text.to_string(),
            parse_mode: "Markdown".to_string(),
        };

        let resp = self.client.post(&url).json(&msg).send().await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(body = %body, "Telegram send failed");
        }
        Ok(())
    }

    /// Send trade close alert with PnL.
    pub async fn send_trade_close(&self, record: &TradeRecord) {
        let pnl = record.pnl_usd.unwrap_or(0.0);
        let emoji = if pnl >= 0.0 { "+" } else { "" };
        let exit = record.exit_reason.as_deref().unwrap_or("?");
        let msg = format!(
            "*Trade Closed*\n{} {} | {} | {}\nPnL: `{emoji}{pnl:.2}` USD\nCapital: `${:.2}`",
            record.direction,
            record.play_type,
            record.signal_level,
            exit,
            record.capital_after.unwrap_or(0.0),
        );
        if let Err(e) = self.send(&msg).await {
            warn!("Telegram trade alert failed: {e:#}");
        }
    }

    /// Send report summary.
    pub async fn send_report(&self, summary: &str) {
        let msg = format!("*20-Trade Report*\n{summary}");
        if let Err(e) = self.send(&msg).await {
            warn!("Telegram report alert failed: {e:#}");
        }
    }

    /// Send kill switch alert.
    pub async fn send_kill_switch(&self) {
        let msg = "🚨 *KILL SWITCH TRIGGERED*\nAll positions closed. Trading halted.";
        if let Err(e) = self.send(msg).await {
            warn!("Telegram kill switch alert failed: {e:#}");
        }
    }
}
