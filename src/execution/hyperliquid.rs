use anyhow::{Context, Result};
use ethers::signers::{LocalWallet, Signer};
use tracing::{info, warn};

use hyperliquid_rust_sdk::{
    BaseUrl, ClientOrder, ClientOrderRequest, ClientTrigger,
    ExchangeClient, ExchangeResponseStatus, ExchangeDataStatus,
    InfoClient, MarketOrderParams,
};

/// Lightweight account state query — no signing needed, just reads.
/// Used by the balance poller task which doesn't need order execution.
pub async fn query_account_state(wallet_address: ethers::types::Address) -> Result<(f64, f64, Vec<HlPosition>)> {
    let info = InfoClient::new(None, Some(BaseUrl::Mainnet))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create InfoClient: {e:?}"))?;

    let state = info
        .user_state(wallet_address)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to query user state: {e:?}"))?;

    let positions: Vec<HlPosition> = state
        .asset_positions
        .iter()
        .filter_map(|ap| {
            let szi: f64 = ap.position.szi.parse().ok()?;
            if szi.abs() < 1e-12 {
                return None;
            }
            Some(HlPosition {
                coin: ap.position.coin.clone(),
                size: szi,
                entry_px: ap.position.entry_px.as_ref()?.parse().ok()?,
                unrealized_pnl: ap.position.unrealized_pnl.parse().unwrap_or(0.0),
                leverage: ap.position.leverage.value as u8,
            })
        })
        .collect();

    // Use HL's own account_value from margin_summary — already includes
    // unrealized PnL. Do NOT manually add spot + unrealized_pnl, because
    // for isolated positions margin already includes unrealized PnL and
    // spot_total reflects that, causing double-counting.
    let total_equity = state.margin_summary.account_value
        .parse::<f64>()
        .unwrap_or(0.0);

    let withdrawable = state.withdrawable
        .parse::<f64>()
        .unwrap_or(0.0);

    Ok((total_equity, withdrawable, positions))
}

/// Hyperliquid order executor — uses the official SDK for proper EIP-712 signing.
pub struct HlExecutor {
    exchange: ExchangeClient,
    wallet: LocalWallet,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OrderResult {
    pub success: bool,
    pub order_id: Option<String>,
    pub filled_price: Option<f64>,
    pub message: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct HlPosition {
    pub coin: String,
    pub size: f64,
    pub entry_px: f64,
    pub unrealized_pnl: f64,
    pub leverage: u8,
}

impl HlExecutor {
    pub async fn new(private_key: &str) -> Result<Self> {
        let wallet: LocalWallet = private_key
            .parse()
            .context("Failed to parse HL private key")?;
        info!(address = %wallet.address(), "HlExecutor initializing with SDK...");

        let exchange = ExchangeClient::new(None, wallet.clone(), Some(BaseUrl::Mainnet), None, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize ExchangeClient: {e:?}"))?;

        info!("HlExecutor initialized with proper EIP-712 signing");
        Ok(Self { exchange, wallet })
    }

    /// Place a market order (IOC) via SDK.
    pub async fn market_order(
        &self,
        coin: &str,
        is_buy: bool,
        sz: f64,
        leverage: u8,
    ) -> Result<OrderResult> {
        // Set leverage with isolated margin — each position has its own
        // margin and liquidation price, preventing cascading liquidations.
        self.exchange
            .update_leverage(leverage as u32, coin, false, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to set leverage: {e:?}"))?;

        // Place market order via SDK (handles slippage internally)
        let result = self
            .exchange
            .market_open(MarketOrderParams {
                asset: coin,
                is_buy,
                sz,
                px: None,
                slippage: Some(0.05), // 5% slippage tolerance
                cloid: None,
                wallet: None,
            })
            .await;

        Self::parse_response(result)
    }

    /// Place a stop-loss trigger order via SDK.
    pub async fn stop_loss(
        &self,
        coin: &str,
        trigger_px: f64,
        sz: f64,
        is_buy: bool,
    ) -> Result<OrderResult> {
        let order = ClientOrderRequest {
            asset: coin.to_string(),
            is_buy,
            reduce_only: true,
            limit_px: trigger_px,
            sz,
            cloid: None,
            order_type: ClientOrder::Trigger(ClientTrigger {
                trigger_px,
                is_market: true,
                tpsl: "sl".to_string(),
            }),
        };

        let result = self.exchange.order(order, None).await;
        Self::parse_response(result)
    }

    /// Place a take-profit trigger order via SDK.
    pub async fn take_profit(
        &self,
        coin: &str,
        trigger_px: f64,
        sz: f64,
        is_buy: bool,
    ) -> Result<OrderResult> {
        let order = ClientOrderRequest {
            asset: coin.to_string(),
            is_buy,
            reduce_only: true,
            limit_px: trigger_px,
            sz,
            cloid: None,
            order_type: ClientOrder::Trigger(ClientTrigger {
                trigger_px,
                is_market: true,
                tpsl: "tp".to_string(),
            }),
        };

        let result = self.exchange.order(order, None).await;
        Self::parse_response(result)
    }

    /// Query current positions via InfoClient.
    pub async fn positions(&self) -> Result<Vec<HlPosition>> {
        let info = InfoClient::new(None, Some(BaseUrl::Mainnet))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create InfoClient: {e:?}"))?;

        let state = info
            .user_state(self.wallet.address())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query user state: {e:?}"))?;

        let positions = state
            .asset_positions
            .iter()
            .filter_map(|ap| {
                let szi: f64 = ap.position.szi.parse().ok()?;
                if szi.abs() < 1e-12 {
                    return None;
                }
                Some(HlPosition {
                    coin: ap.position.coin.clone(),
                    size: szi,
                    entry_px: ap.position.entry_px.as_ref()?.parse().ok()?,
                    unrealized_pnl: ap.position.unrealized_pnl.parse().unwrap_or(0.0),
                    leverage: ap.position.leverage.value as u8,
                })
            })
            .collect();

        Ok(positions)
    }

    /// Query total account equity: perp clearinghouse + spot USDC balance.
    /// Unified accounts keep most USDC in the spot clearinghouse while using
    /// Total account equity from HL's margin_summary.account_value.
    /// This already includes unrealized PnL — no manual addition needed.
    pub async fn account_equity(&self) -> Result<f64> {
        let info = InfoClient::new(None, Some(BaseUrl::Mainnet))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create InfoClient: {e:?}"))?;

        let state = info.user_state(self.wallet.address())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query user state: {e:?}"))?;

        let equity = state.margin_summary.account_value
            .parse::<f64>()
            .unwrap_or(0.0);

        info!(equity, "HL account equity");
        Ok(equity)
    }

    /// Close all positions (market sell/buy to flatten).
    pub async fn close_all(&self) -> Result<()> {
        let positions = self.positions().await?;
        let mut errors = Vec::new();
        for pos in positions {
            let is_buy = pos.size < 0.0;
            let sz = pos.size.abs();
            info!(coin = %pos.coin, size = sz, "Closing HL position");
            if let Err(e) = self.market_order(&pos.coin, is_buy, sz, pos.leverage).await {
                warn!(coin = %pos.coin, "Failed to close position: {e:#}");
                errors.push(format!("{}: {e}", pos.coin));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("Failed to close {} position(s): {}", errors.len(), errors.join(", "))
        }
    }

    fn parse_response(
        result: std::result::Result<ExchangeResponseStatus, hyperliquid_rust_sdk::Error>,
    ) -> Result<OrderResult> {
        match result {
            Ok(ExchangeResponseStatus::Ok(resp)) => {
                let statuses = resp.data.as_ref().map(|d| &d.statuses);
                let first = statuses.and_then(|s| s.first());

                let (order_id, filled_price) = match first {
                    Some(ExchangeDataStatus::Filled(f)) => (
                        Some(f.oid.to_string()),
                        f.avg_px.parse::<f64>().ok(),
                    ),
                    Some(ExchangeDataStatus::Resting(r)) => (Some(r.oid.to_string()), None),
                    Some(ExchangeDataStatus::WaitingForTrigger) => (None, None),
                    _ => (None, None),
                };

                let success = matches!(
                    first,
                    Some(ExchangeDataStatus::Filled(_))
                        | Some(ExchangeDataStatus::Resting(_))
                        | Some(ExchangeDataStatus::WaitingForTrigger)
                );

                Ok(OrderResult {
                    success,
                    order_id,
                    filled_price,
                    message: format!("{:?}", first),
                })
            }
            Ok(ExchangeResponseStatus::Err(msg)) => {
                warn!(msg = %msg, "HL order rejected");
                Ok(OrderResult {
                    success: false,
                    order_id: None,
                    filled_price: None,
                    message: msg,
                })
            }
            Err(e) => {
                warn!(error = %format!("{e:?}"), "HL order error");
                Ok(OrderResult {
                    success: false,
                    order_id: None,
                    filled_price: None,
                    message: format!("{e:?}"),
                })
            }
        }
    }
}
