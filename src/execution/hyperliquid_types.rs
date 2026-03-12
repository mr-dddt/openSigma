use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Info API — POST /info
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug)]
#[serde(tag = "type")]
pub enum InfoRequest {
    #[serde(rename = "clearinghouseState")]
    ClearinghouseState { user: String },
    #[serde(rename = "meta")]
    Meta,
    #[serde(rename = "openOrders")]
    OpenOrders { user: String },
}

// -- clearinghouseState response --

#[derive(Deserialize, Debug)]
pub struct ClearinghouseState {
    #[serde(rename = "marginSummary")]
    pub margin_summary: MarginSummary,
    #[serde(rename = "crossMarginSummary")]
    pub cross_margin_summary: MarginSummary,
    #[serde(rename = "assetPositions")]
    pub asset_positions: Vec<AssetPosition>,
    pub withdrawable: String,
}

#[derive(Deserialize, Debug)]
pub struct MarginSummary {
    #[serde(rename = "accountValue")]
    pub account_value: String,
    #[serde(rename = "totalNtlPos")]
    pub total_ntl_pos: String,
    #[serde(rename = "totalRawUsd")]
    pub total_raw_usd: String,
    #[serde(rename = "totalMarginUsed")]
    pub total_margin_used: String,
}

#[derive(Deserialize, Debug)]
pub struct AssetPosition {
    pub position: PositionData,
    #[serde(rename = "type")]
    pub position_type: String,
}

#[derive(Deserialize, Debug)]
pub struct PositionData {
    pub coin: String,
    pub szi: String,
    #[serde(rename = "entryPx")]
    pub entry_px: Option<String>,
    pub leverage: LeverageInfo,
    #[serde(rename = "positionValue")]
    pub position_value: String,
    #[serde(rename = "unrealizedPnl")]
    pub unrealized_pnl: String,
    #[serde(rename = "liquidationPx")]
    pub liquidation_px: Option<String>,
    #[serde(rename = "marginUsed")]
    pub margin_used: String,
}

#[derive(Deserialize, Debug)]
pub struct LeverageInfo {
    #[serde(rename = "type")]
    pub leverage_type: String,
    pub value: u32,
}

// -- meta response --

#[derive(Deserialize, Debug)]
pub struct Meta {
    pub universe: Vec<AssetMeta>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AssetMeta {
    pub name: String,
    #[serde(rename = "szDecimals")]
    pub sz_decimals: u32,
    #[serde(rename = "maxLeverage")]
    pub max_leverage: u32,
}

// -- openOrders response --

#[derive(Deserialize, Debug)]
pub struct OpenOrder {
    pub coin: String,
    #[serde(rename = "limitPx")]
    pub limit_px: String,
    pub oid: u64,
    pub side: String,
    pub sz: String,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Exchange API — POST /exchange
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct OrderAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub orders: Vec<OrderWire>,
    pub grouping: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct OrderWire {
    pub a: u32,
    pub b: bool,
    pub p: String,
    pub s: String,
    pub r: bool,
    pub t: OrderType,
}

#[derive(Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum OrderType {
    Limit(LimitOrderType),
    Trigger(TriggerOrderType),
}

#[derive(Serialize, Clone, Debug)]
pub struct LimitOrderType {
    pub limit: LimitOrder,
}

#[derive(Serialize, Clone, Debug)]
pub struct LimitOrder {
    pub tif: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct TriggerOrderType {
    pub trigger: TriggerOrder,
}

#[derive(Serialize, Clone, Debug)]
pub struct TriggerOrder {
    #[serde(rename = "isMarket")]
    pub is_market: bool,
    #[serde(rename = "triggerPx")]
    pub trigger_px: String,
    pub tpsl: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CancelAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub cancels: Vec<CancelWire>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CancelWire {
    pub a: u32,
    pub o: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct UpdateLeverageAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub asset: u32,
    #[serde(rename = "isCross")]
    pub is_cross: bool,
    pub leverage: u32,
}

// -- Exchange request envelope --

#[derive(Serialize, Debug)]
pub struct ExchangeRequest {
    pub action: serde_json::Value,
    pub nonce: u64,
    pub signature: SignaturePayload,
    #[serde(rename = "vaultAddress", skip_serializing_if = "Option::is_none")]
    pub vault_address: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct SignaturePayload {
    pub r: String,
    pub s: String,
    pub v: u8,
}

// -- Exchange response --

#[derive(Deserialize, Debug)]
pub struct ExchangeResponse {
    pub status: String,
    pub response: Option<ExchangeResponseData>,
}

#[derive(Deserialize, Debug)]
pub struct ExchangeResponseData {
    #[serde(rename = "type")]
    pub response_type: String,
    pub data: Option<OrderResponseData>,
}

#[derive(Deserialize, Debug)]
pub struct OrderResponseData {
    pub statuses: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a float for Hyperliquid API: no trailing zeros.
pub fn format_float(value: f64, max_decimals: u32) -> String {
    let s = format!("{:.prec$}", value, prec = max_decimals as usize);
    if s.contains('.') {
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    } else {
        s
    }
}

/// Format size with the asset's szDecimals precision.
pub fn format_size(size: f64, sz_decimals: u32) -> String {
    format_float(size, sz_decimals)
}

/// Format price (up to 6 significant decimals for most assets).
pub fn format_price(price: f64) -> String {
    format_float(price, 6)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_float_removes_trailing_zeros() {
        assert_eq!(format_float(50000.0, 6), "50000");
        assert_eq!(format_float(50000.50, 6), "50000.5");
        assert_eq!(format_float(0.12300, 5), "0.123");
        assert_eq!(format_float(1.0, 2), "1");
        assert_eq!(format_float(0.001, 4), "0.001");
    }

    #[test]
    fn format_price_works() {
        assert_eq!(format_price(84200.0), "84200");
        assert_eq!(format_price(84200.5), "84200.5");
        assert_eq!(format_price(3200.123456), "3200.123456");
    }

    #[test]
    fn format_size_works() {
        assert_eq!(format_size(0.001, 3), "0.001");
        assert_eq!(format_size(0.0010, 4), "0.001");
        assert_eq!(format_size(1.0, 0), "1");
    }

    #[test]
    fn info_request_serialization() {
        let req = InfoRequest::ClearinghouseState {
            user: "0xabc".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"clearinghouseState\""));
        assert!(json.contains("\"user\":\"0xabc\""));
    }

    #[test]
    fn info_request_meta_serialization() {
        let req = InfoRequest::Meta;
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"type":"meta"}"#);
    }

    #[test]
    fn order_action_serialization() {
        let action = OrderAction {
            action_type: "order".into(),
            orders: vec![OrderWire {
                a: 0,
                b: true,
                p: "50000".into(),
                s: "0.001".into(),
                r: false,
                t: OrderType::Limit(LimitOrderType {
                    limit: LimitOrder { tif: "Gtc".into() },
                }),
            }],
            grouping: "na".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"a\":0"));
        assert!(json.contains("\"b\":true"));
        assert!(json.contains("\"tif\":\"Gtc\""));
    }

    #[test]
    fn cancel_action_serialization() {
        let action = CancelAction {
            action_type: "cancel".into(),
            cancels: vec![CancelWire { a: 0, o: 12345 }],
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"type\":\"cancel\""));
        assert!(json.contains("\"o\":12345"));
    }

    #[test]
    fn clearinghouse_state_deserialization() {
        let json = r#"{
            "marginSummary": {
                "accountValue": "10000.0",
                "totalNtlPos": "500.0",
                "totalRawUsd": "9500.0",
                "totalMarginUsed": "100.0"
            },
            "crossMarginSummary": {
                "accountValue": "10000.0",
                "totalNtlPos": "500.0",
                "totalRawUsd": "9500.0",
                "totalMarginUsed": "100.0"
            },
            "assetPositions": [],
            "withdrawable": "9000.0"
        }"#;
        let state: ClearinghouseState = serde_json::from_str(json).unwrap();
        assert_eq!(state.margin_summary.account_value, "10000.0");
        assert_eq!(state.withdrawable, "9000.0");
        assert!(state.asset_positions.is_empty());
    }

    #[test]
    fn exchange_response_deserialization() {
        let json = r#"{"status":"ok","response":{"type":"order","data":{"statuses":[{"resting":{"oid":12345}}]}}}"#;
        let resp: ExchangeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "ok");
        assert!(resp.response.is_some());
    }

    #[test]
    fn exchange_error_deserialization() {
        let json = r#"{"status":"err","response":null}"#;
        let resp: ExchangeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "err");
        assert!(resp.response.is_none());
    }
}
