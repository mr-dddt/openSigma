use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{info, warn};

use crate::types::BinarySide;

const PM_CLOB_URL: &str = "https://clob.polymarket.com";

#[allow(dead_code)] // PM trading not yet wired in main loop
/// Polymarket order executor — places binary market maker limit orders.
/// Uses the 3-part credential system (API key, secret, passphrase).
pub struct PmExecutor {
    client: reqwest::Client,
    api_key: String,
    api_secret: String,
    passphrase: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PmOrderResult {
    pub success: bool,
    pub order_id: Option<String>,
    pub message: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct PmPosition {
    pub token_id: String,
    pub size: f64,
    pub avg_price: f64,
}

#[allow(dead_code)]
impl PmExecutor {
    pub fn new(api_key: &str, api_secret: &str, passphrase: &str) -> Self {
        let has_creds = !api_key.is_empty() && !api_secret.is_empty() && !passphrase.is_empty();
        if has_creds {
            info!("PmExecutor initialized with API credentials");
        } else {
            warn!("PmExecutor initialized without credentials — PM trading disabled");
        }
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            passphrase: passphrase.to_string(),
        }
    }

    /// Query USDC balance on Polymarket (Polygon).
    pub async fn account_balance(&self) -> Result<f64> {
        if self.api_key.is_empty() {
            return Ok(0.0);
        }
        let resp = self
            .client
            .get(format!("{}/balance", PM_CLOB_URL))
            .header("POLY_API_KEY", &self.api_key)
            .header("POLY_API_SECRET", &self.api_secret)
            .header("POLY_PASSPHRASE", &self.passphrase)
            .send()
            .await
            .context("Failed to query PM balance")?;

        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            Ok(body.get("balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0))
        } else {
            Ok(0.0)
        }
    }

    /// Place a maker limit order on a binary market.
    pub async fn place_limit_order(
        &self,
        token_id: &str,
        side: BinarySide,
        price: f64,
        size: f64,
    ) -> Result<PmOrderResult> {
        let order_side = match side {
            BinarySide::Up => "BUY",
            BinarySide::Down => "SELL",
        };

        let body = serde_json::json!({
            "tokenID": token_id,
            "price": format!("{:.2}", price),
            "size": format!("{:.2}", size),
            "side": order_side,
            "type": "GTC"  // Good-til-cancelled for maker orders
        });

        let resp = self
            .client
            .post(format!("{}/order", PM_CLOB_URL))
            .header("POLY_API_KEY", &self.api_key)
            .header("POLY_API_SECRET", &self.api_secret)
            .header("POLY_PASSPHRASE", &self.passphrase)
            .json(&body)
            .send()
            .await
            .context("Failed to send PM order")?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp.json().await.unwrap_or_default();

        if status.is_success() {
            Ok(PmOrderResult {
                success: true,
                order_id: resp_body
                    .get("orderID")
                    .and_then(|o| o.as_str())
                    .map(|s| s.to_string()),
                message: resp_body.to_string(),
            })
        } else {
            warn!(status = %status, body = %resp_body, "PM order failed");
            Ok(PmOrderResult {
                success: false,
                order_id: None,
                message: resp_body.to_string(),
            })
        }
    }

    /// Cancel a specific order.
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let body = serde_json::json!({ "orderID": order_id });

        self.client
            .delete(format!("{}/order", PM_CLOB_URL))
            .header("POLY_API_KEY", &self.api_key)
            .header("POLY_API_SECRET", &self.api_secret)
            .header("POLY_PASSPHRASE", &self.passphrase)
            .json(&body)
            .send()
            .await
            .context("Failed to cancel PM order")?;

        Ok(())
    }

    /// Cancel all open orders.
    pub async fn cancel_all(&self) -> Result<()> {
        info!("Cancelling all PM orders");
        self.client
            .delete(format!("{}/cancel-all", PM_CLOB_URL))
            .header("POLY_API_KEY", &self.api_key)
            .header("POLY_API_SECRET", &self.api_secret)
            .header("POLY_PASSPHRASE", &self.passphrase)
            .send()
            .await
            .context("Failed to cancel all PM orders")?;

        Ok(())
    }

    /// Get current positions.
    pub async fn get_positions(&self) -> Result<Vec<PmPosition>> {
        let resp = self
            .client
            .get(format!("{}/positions", PM_CLOB_URL))
            .header("POLY_API_KEY", &self.api_key)
            .header("POLY_API_SECRET", &self.api_secret)
            .header("POLY_PASSPHRASE", &self.passphrase)
            .send()
            .await
            .context("Failed to query PM positions")?;

        if resp.status().is_success() {
            let positions: Vec<PmPosition> = resp.json().await.unwrap_or_default();
            Ok(positions)
        } else {
            Ok(vec![])
        }
    }
}
