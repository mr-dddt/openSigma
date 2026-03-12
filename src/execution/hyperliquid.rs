use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ethers::signers::{LocalWallet, Signer};
use tracing::{debug, info, warn};

use crate::types::*;

use super::hyperliquid_signer::{sign_l1_action, wallet_from_key};
use super::hyperliquid_types::*;

const TESTNET_URL: &str = "https://api.hyperliquid-testnet.xyz";
const MAINNET_URL: &str = "https://api.hyperliquid.xyz";

/// Hyperliquid order execution client.
/// Handles signing and submitting orders via the Hyperliquid REST API.
pub struct HyperliquidExecutor {
    wallet: LocalWallet,
    address: String,
    http: reqwest::Client,
    base_url: String,
    is_mainnet: bool,
    /// Maps symbol name (e.g. "BTC") to asset index
    asset_map: HashMap<String, u32>,
    /// Maps symbol name to szDecimals
    sz_decimals_map: HashMap<String, u32>,
}

impl HyperliquidExecutor {
    /// Create a new executor, fetching asset metadata from the API.
    pub async fn new(private_key: &str, is_mainnet: bool) -> Result<Self> {
        let wallet = wallet_from_key(private_key)?;
        let address = format!("{:?}", wallet.address());
        let base_url = if is_mainnet { MAINNET_URL } else { TESTNET_URL };
        let http = reqwest::Client::new();

        info!(address = %address, network = if is_mainnet { "mainnet" } else { "testnet" }, "Initializing HyperliquidExecutor");

        let mut executor = Self {
            wallet,
            address,
            http,
            base_url: base_url.to_string(),
            is_mainnet,
            asset_map: HashMap::new(),
            sz_decimals_map: HashMap::new(),
        };

        // Fetch asset metadata to build the asset index map
        executor.refresh_meta().await?;

        Ok(executor)
    }

    /// Fetch /info meta and rebuild asset_map + sz_decimals_map.
    pub async fn refresh_meta(&mut self) -> Result<()> {
        let meta = self.fetch_meta().await?;
        self.asset_map.clear();
        self.sz_decimals_map.clear();
        for (idx, asset) in meta.universe.iter().enumerate() {
            self.asset_map.insert(asset.name.clone(), idx as u32);
            self.sz_decimals_map
                .insert(asset.name.clone(), asset.sz_decimals);
        }
        info!(
            asset_count = meta.universe.len(),
            "Loaded asset metadata"
        );
        Ok(())
    }

    /// Get the asset index for a symbol, returning an error if not found.
    fn asset_index(&self, symbol: &str) -> Result<u32> {
        self.asset_map
            .get(symbol)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Unknown asset: {}", symbol))
    }

    /// Get szDecimals for a symbol.
    fn sz_decimals(&self, symbol: &str) -> u32 {
        self.sz_decimals_map.get(symbol).copied().unwrap_or(3)
    }

    // -----------------------------------------------------------------------
    // Info API (read-only)
    // -----------------------------------------------------------------------

    /// POST to /info endpoint.
    async fn post_info<T: serde::de::DeserializeOwned>(
        &self,
        request: &InfoRequest,
    ) -> Result<T> {
        let url = format!("{}/info", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(request)
            .send()
            .await
            .context("Failed to send info request")?;

        let status = resp.status();
        let body = resp.text().await.context("Failed to read response body")?;

        if !status.is_success() {
            anyhow::bail!("Info API error ({}): {}", status, body);
        }

        serde_json::from_str(&body).context(format!("Failed to parse info response: {}", body))
    }

    /// Fetch asset metadata (universe of available assets).
    pub async fn fetch_meta(&self) -> Result<Meta> {
        self.post_info(&InfoRequest::Meta).await
    }

    /// Fetch clearinghouse state (margin, positions) for our address.
    pub async fn fetch_clearinghouse_state(&self) -> Result<ClearinghouseState> {
        self.post_info(&InfoRequest::ClearinghouseState {
            user: self.address.clone(),
        })
        .await
    }

    /// Get account balance (account value from margin summary).
    pub async fn get_balance(&self) -> Result<f64> {
        let state = self.fetch_clearinghouse_state().await?;
        let value: f64 = state
            .margin_summary
            .account_value
            .parse()
            .context("Failed to parse account_value")?;
        Ok(value)
    }

    /// Get all open positions.
    pub async fn get_positions(&self) -> Result<Vec<AssetPosition>> {
        let state = self.fetch_clearinghouse_state().await?;
        Ok(state
            .asset_positions
            .into_iter()
            .filter(|p| {
                // Filter out zero-size positions
                p.position
                    .szi
                    .parse::<f64>()
                    .map(|s| s.abs() > 1e-12)
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Get open orders for our address.
    pub async fn get_open_orders(&self) -> Result<Vec<OpenOrder>> {
        self.post_info(&InfoRequest::OpenOrders {
            user: self.address.clone(),
        })
        .await
    }

    // -----------------------------------------------------------------------
    // Exchange API (write — requires signing)
    // -----------------------------------------------------------------------

    /// Generate a nonce from current timestamp in milliseconds.
    fn nonce() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64
    }

    /// POST a signed request to /exchange.
    async fn post_exchange(&self, action: serde_json::Value) -> Result<ExchangeResponse> {
        let nonce = Self::nonce();
        let signature =
            sign_l1_action(&self.wallet, &action, nonce, None, self.is_mainnet).await?;

        let request = ExchangeRequest {
            action,
            nonce,
            signature,
            vault_address: None,
        };

        let url = format!("{}/exchange", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send exchange request")?;

        let status = resp.status();
        let body = resp.text().await.context("Failed to read response body")?;

        debug!(status = %status, body = %body, "Exchange API response");

        if !status.is_success() {
            anyhow::bail!("Exchange API error ({}): {}", status, body);
        }

        serde_json::from_str(&body)
            .context(format!("Failed to parse exchange response: {}", body))
    }

    /// Place a limit order on Hyperliquid from a TradeDecision.
    pub async fn place_order(&self, decision: &TradeDecision) -> Result<String> {
        let size_usd = decision.adjusted_size.unwrap_or(decision.proposal.size_usd);
        let leverage = decision
            .adjusted_leverage
            .unwrap_or(decision.proposal.leverage);
        let symbol = decision.proposal.symbol.to_string();
        let price = decision.proposal.entry_price;
        let is_buy = decision.proposal.direction == Direction::Long;

        let asset_idx = self.asset_index(&symbol)?;
        let sz_dec = self.sz_decimals(&symbol);

        // Convert USD size to coin size
        let size_in_coin = size_usd / price;

        // Update leverage first
        self.update_leverage(&symbol, leverage as u32, true).await?;

        let order_wire = OrderWire {
            a: asset_idx,
            b: is_buy,
            p: format_price(price),
            s: format_size(size_in_coin, sz_dec),
            r: false,
            t: OrderType::Limit(LimitOrderType {
                limit: LimitOrder {
                    tif: "Gtc".to_string(),
                },
            }),
        };

        let action = serde_json::to_value(&OrderAction {
            action_type: "order".to_string(),
            orders: vec![order_wire],
            grouping: "na".to_string(),
        })?;

        info!(
            symbol = %symbol,
            direction = ?decision.proposal.direction,
            size_usd = size_usd,
            size_coin = %format_size(size_in_coin, sz_dec),
            price = %format_price(price),
            leverage = leverage,
            "Placing order"
        );

        let resp = self.post_exchange(action).await?;

        if resp.status == "ok" {
            // Try to extract the order ID from the response
            if let Some(data) = &resp.response {
                if let Some(order_data) = &data.data {
                    if let Some(status) = order_data.statuses.first() {
                        if let Some(resting) = status.get("resting") {
                            if let Some(oid) = resting.get("oid") {
                                return Ok(oid.to_string());
                            }
                        }
                        if let Some(filled) = status.get("filled") {
                            if let Some(oid) = filled.get("oid") {
                                return Ok(oid.to_string());
                            }
                        }
                        // Return status JSON if we can't extract oid
                        return Ok(status.to_string());
                    }
                }
            }
            Ok("ok".to_string())
        } else {
            anyhow::bail!("Order rejected: {:?}", resp)
        }
    }

    /// Place a raw limit order (lower-level API).
    pub async fn place_limit_order(
        &self,
        symbol: &str,
        is_buy: bool,
        price: f64,
        size_coin: f64,
        reduce_only: bool,
        tif: &str,
    ) -> Result<ExchangeResponse> {
        let asset_idx = self.asset_index(symbol)?;
        let sz_dec = self.sz_decimals(symbol);

        let order_wire = OrderWire {
            a: asset_idx,
            b: is_buy,
            p: format_price(price),
            s: format_size(size_coin, sz_dec),
            r: reduce_only,
            t: OrderType::Limit(LimitOrderType {
                limit: LimitOrder {
                    tif: tif.to_string(),
                },
            }),
        };

        let action = serde_json::to_value(&OrderAction {
            action_type: "order".to_string(),
            orders: vec![order_wire],
            grouping: "na".to_string(),
        })?;

        self.post_exchange(action).await
    }

    /// Cancel an order by symbol and order ID.
    pub async fn cancel_order(&self, symbol: &str, order_id: u64) -> Result<()> {
        let asset_idx = self.asset_index(symbol)?;

        let action = serde_json::to_value(&CancelAction {
            action_type: "cancel".to_string(),
            cancels: vec![CancelWire {
                a: asset_idx,
                o: order_id,
            }],
        })?;

        info!(symbol = %symbol, order_id = order_id, "Cancelling order");

        let resp = self.post_exchange(action).await?;
        if resp.status == "ok" {
            Ok(())
        } else {
            anyhow::bail!("Cancel rejected: {:?}", resp)
        }
    }

    /// Update leverage for an asset.
    pub async fn update_leverage(
        &self,
        symbol: &str,
        leverage: u32,
        is_cross: bool,
    ) -> Result<()> {
        let asset_idx = self.asset_index(symbol)?;

        let action = serde_json::to_value(&UpdateLeverageAction {
            action_type: "updateLeverage".to_string(),
            asset: asset_idx,
            is_cross,
            leverage,
        })?;

        debug!(symbol = %symbol, leverage = leverage, "Updating leverage");

        let resp = self.post_exchange(action).await?;
        if resp.status == "ok" {
            Ok(())
        } else {
            warn!(symbol = %symbol, "Leverage update may have failed: {:?}", resp);
            Ok(()) // Non-fatal: leverage might already be set
        }
    }

    /// Close all open positions with market orders.
    pub async fn close_all_positions(&self) -> Result<()> {
        let positions = self.get_positions().await?;

        if positions.is_empty() {
            info!("No open positions to close");
            return Ok(());
        }

        info!(
            count = positions.len(),
            "Closing all positions (kill switch)"
        );

        for pos in &positions {
            let szi: f64 = pos.position.szi.parse().unwrap_or(0.0);
            if szi.abs() < 1e-12 {
                continue;
            }

            let symbol = &pos.position.coin;
            let asset_idx = match self.asset_index(symbol) {
                Ok(idx) => idx,
                Err(e) => {
                    warn!(symbol = %symbol, error = %e, "Skipping unknown asset in close_all");
                    continue;
                }
            };
            let sz_dec = self.sz_decimals(symbol);

            // To close: sell if long (szi > 0), buy if short (szi < 0)
            let is_buy = szi < 0.0;
            let size = szi.abs();

            let order_wire = OrderWire {
                a: asset_idx,
                b: is_buy,
                p: format_price(0.0), // Market order: use 0 price with IoC
                s: format_size(size, sz_dec),
                r: true, // reduce only
                t: OrderType::Limit(LimitOrderType {
                    limit: LimitOrder {
                        tif: "Ioc".to_string(), // Immediate-or-cancel = market order
                    },
                }),
            };

            let action = serde_json::to_value(&OrderAction {
                action_type: "order".to_string(),
                orders: vec![order_wire],
                grouping: "na".to_string(),
            })?;

            info!(symbol = %symbol, size = %format_size(size, sz_dec), is_buy = is_buy, "Closing position");

            match self.post_exchange(action).await {
                Ok(resp) => {
                    if resp.status != "ok" {
                        warn!(symbol = %symbol, "Close order may have failed: {:?}", resp);
                    }
                }
                Err(e) => {
                    warn!(symbol = %symbol, error = %e, "Failed to close position");
                }
            }
        }

        Ok(())
    }

    /// Get the wallet address as a hex string.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_recent_millis() {
        let nonce = HyperliquidExecutor::nonce();
        // Should be roughly current time in millis (after 2024)
        assert!(nonce > 1_700_000_000_000);
    }

    #[test]
    fn base_url_testnet() {
        assert_eq!(TESTNET_URL, "https://api.hyperliquid-testnet.xyz");
    }

    #[test]
    fn base_url_mainnet() {
        assert_eq!(MAINNET_URL, "https://api.hyperliquid.xyz");
    }
}
