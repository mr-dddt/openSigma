use anyhow::Result;
use tracing::info;

use crate::types::*;

/// Hyperliquid order execution client.
/// Handles signing and submitting orders via the Hyperliquid API.
pub struct HyperliquidExecutor {
    _private_key: String,
    // TODO: ethers Wallet for signing
}

impl HyperliquidExecutor {
    pub fn new(private_key: String) -> Self {
        Self {
            _private_key: private_key,
        }
    }

    /// Place a limit/market order on Hyperliquid.
    pub async fn place_order(&self, decision: &TradeDecision) -> Result<String> {
        let size = decision.adjusted_size.unwrap_or(decision.proposal.size_usd);
        let leverage = decision
            .adjusted_leverage
            .unwrap_or(decision.proposal.leverage);

        info!(
            symbol = %decision.proposal.symbol,
            direction = ?decision.proposal.direction,
            size_usd = size,
            leverage = leverage,
            "Placing order on Hyperliquid (Phase 0 stub)"
        );

        // TODO Phase 1:
        // 1. Build Hyperliquid order payload
        // 2. Sign with EOA private key
        // 3. POST to Hyperliquid exchange API
        // 4. Return order ID

        Ok("stub-order-id".to_string())
    }

    /// Cancel an existing order.
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        info!(order_id = order_id, "Cancelling order (Phase 0 stub)");
        Ok(())
    }

    /// Close all open positions (kill switch).
    pub async fn close_all_positions(&self) -> Result<()> {
        info!("Closing all positions (Phase 0 stub)");
        // TODO: query open positions, market close each
        Ok(())
    }

    /// Query current account balance.
    pub async fn get_balance(&self) -> Result<f64> {
        // TODO: query Hyperliquid info API
        Ok(0.0)
    }
}
