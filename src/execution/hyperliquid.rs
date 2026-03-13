use anyhow::{Context, Result};
use ethers::signers::{LocalWallet, Signer};
use serde::Deserialize;
use tracing::{info, warn};

const HL_API_URL: &str = "https://api.hyperliquid.xyz";

/// Hyperliquid order executor — places perp orders via REST API with EIP-712 signing.
pub struct HlExecutor {
    client: reqwest::Client,
    wallet: LocalWallet,
}

#[derive(Debug, Clone)]
pub struct OrderResult {
    pub success: bool,
    pub order_id: Option<String>,
    pub filled_price: Option<f64>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HlPosition {
    pub coin: String,
    pub size: f64,
    pub entry_px: f64,
    pub unrealized_pnl: f64,
    pub leverage: u8,
}

impl HlExecutor {
    pub fn new(private_key: &str) -> Result<Self> {
        let wallet: LocalWallet = private_key
            .parse()
            .context("Failed to parse HL private key")?;
        info!(address = %wallet.address(), "HlExecutor initialized");
        Ok(Self {
            client: reqwest::Client::new(),
            wallet,
        })
    }

    /// Place a market order (IOC).
    pub async fn market_order(
        &self,
        coin: &str,
        is_buy: bool,
        sz: f64,
        leverage: u8,
    ) -> Result<OrderResult> {
        self.set_leverage(coin, leverage).await?;

        let action = serde_json::json!({
            "type": "order",
            "orders": [{
                "a": self.coin_index(coin),
                "b": is_buy,
                "p": "0",
                "s": format!("{:.4}", sz),
                "r": false,
                "t": {"limit": {"tif": "Ioc"}}
            }],
            "grouping": "na"
        });

        self.send_exchange_action(action).await
    }

    /// Place a stop-loss trigger order.
    pub async fn stop_loss(
        &self,
        coin: &str,
        trigger_px: f64,
        sz: f64,
        is_buy: bool,
    ) -> Result<OrderResult> {
        let action = serde_json::json!({
            "type": "order",
            "orders": [{
                "a": self.coin_index(coin),
                "b": is_buy,
                "p": "0",
                "s": format!("{:.4}", sz),
                "r": true,
                "t": {"trigger": {
                    "triggerPx": format!("{:.2}", trigger_px),
                    "isMarket": true,
                    "tpsl": "sl"
                }}
            }],
            "grouping": "na"
        });

        self.send_exchange_action(action).await
    }

    /// Place a take-profit trigger order.
    pub async fn take_profit(
        &self,
        coin: &str,
        trigger_px: f64,
        sz: f64,
        is_buy: bool,
    ) -> Result<OrderResult> {
        let action = serde_json::json!({
            "type": "order",
            "orders": [{
                "a": self.coin_index(coin),
                "b": is_buy,
                "p": "0",
                "s": format!("{:.4}", sz),
                "r": true,
                "t": {"trigger": {
                    "triggerPx": format!("{:.2}", trigger_px),
                    "isMarket": true,
                    "tpsl": "tp"
                }}
            }],
            "grouping": "na"
        });

        self.send_exchange_action(action).await
    }

    /// Query current positions.
    pub async fn positions(&self) -> Result<Vec<HlPosition>> {
        let body = serde_json::json!({
            "type": "clearinghouseState",
            "user": format!("{:?}", self.wallet.address())
        });

        let resp = self
            .client
            .post(format!("{}/info", HL_API_URL))
            .json(&body)
            .send()
            .await
            .context("Failed to query HL positions")?;

        let data: serde_json::Value = resp.json().await?;

        let positions = data
            .get("assetPositions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let pos = p.get("position")?;
                        Some(HlPosition {
                            coin: pos.get("coin")?.as_str()?.to_string(),
                            size: pos.get("szi")?.as_str()?.parse().ok()?,
                            entry_px: pos.get("entryPx")?.as_str()?.parse().ok()?,
                            unrealized_pnl: pos.get("unrealizedPnl")?.as_str()?.parse().ok()?,
                            leverage: pos
                                .get("leverage")
                                .and_then(|l| l.get("value"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(1) as u8,
                        })
                    })
                    .filter(|p| p.size.abs() > 0.0)
                    .collect()
            })
            .unwrap_or_default();

        Ok(positions)
    }

    /// Close all positions (market sell/buy to flatten).
    pub async fn close_all(&self) -> Result<()> {
        let positions = self.positions().await?;
        for pos in positions {
            let is_buy = pos.size < 0.0;
            let sz = pos.size.abs();
            info!(coin = %pos.coin, size = sz, "Closing HL position");
            let _ = self.market_order(&pos.coin, is_buy, sz, pos.leverage).await;
        }
        Ok(())
    }

    async fn set_leverage(&self, coin: &str, leverage: u8) -> Result<()> {
        let action = serde_json::json!({
            "type": "updateLeverage",
            "asset": self.coin_index(coin),
            "isCross": true,
            "leverage": leverage
        });
        self.send_exchange_action(action).await?;
        Ok(())
    }

    async fn send_exchange_action(&self, action: serde_json::Value) -> Result<OrderResult> {
        let nonce = chrono::Utc::now().timestamp_millis() as u64;

        // Hyperliquid EIP-712 signing: hash the action+nonce payload
        let payload = serde_json::json!({
            "action": action,
            "nonce": nonce,
            "vaultAddress": null
        });
        let payload_bytes = serde_json::to_vec(&payload)?;
        let hash = ethers::utils::keccak256(&payload_bytes);
        let signature = self
            .wallet
            .sign_hash(hash.into())
            .context("Failed to sign HL action")?;

        let body = serde_json::json!({
            "action": action,
            "nonce": nonce,
            "signature": {
                "r": format!("0x{:064x}", signature.r),
                "s": format!("0x{:064x}", signature.s),
                "v": signature.v as u8
            }
        });

        let resp = self
            .client
            .post(format!("{}/exchange", HL_API_URL))
            .json(&body)
            .send()
            .await
            .context("Failed to send HL exchange action")?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp.json().await.unwrap_or_default();

        if status.is_success() {
            let status_str = resp_body
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");

            Ok(OrderResult {
                success: status_str == "ok",
                order_id: resp_body
                    .pointer("/response/data/statuses/0/resting/oid")
                    .or_else(|| resp_body.pointer("/response/data/statuses/0/filled/oid"))
                    .and_then(|o| o.as_u64())
                    .map(|o| o.to_string()),
                filled_price: resp_body
                    .pointer("/response/data/statuses/0/filled/avgPx")
                    .and_then(|p| p.as_str())
                    .and_then(|p| p.parse().ok()),
                message: resp_body.to_string(),
            })
        } else {
            warn!(status = %status, body = %resp_body, "HL order failed");
            Ok(OrderResult {
                success: false,
                order_id: None,
                filled_price: None,
                message: resp_body.to_string(),
            })
        }
    }

    fn coin_index(&self, coin: &str) -> u32 {
        match coin {
            "BTC" => 0,
            "ETH" => 1,
            _ => 0,
        }
    }
}
