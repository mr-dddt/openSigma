use anyhow::{Context, Result};
use ethers::core::k256::ecdsa::SigningKey;
use ethers::signers::{LocalWallet, Wallet};
use ethers::types::{H160, U256};
use ethers::utils::keccak256;

use super::hyperliquid_types::SignaturePayload;

// ---------------------------------------------------------------------------
// EIP-712 domain and types for Hyperliquid phantom agent signing
// ---------------------------------------------------------------------------

// Domain: { name: "Exchange", version: "1", chainId: 1337, verifyingContract: 0x0 }
const EIP712_DOMAIN_TYPE_HASH: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const AGENT_TYPE_HASH: &str = "Agent(string source,bytes32 connectionId)";
const DOMAIN_NAME: &str = "Exchange";
const DOMAIN_VERSION: &str = "1";
const CHAIN_ID: u64 = 1337;

/// Compute the EIP-712 domain separator for Hyperliquid Exchange.
fn domain_separator() -> [u8; 32] {
    let domain_type_hash = keccak256(EIP712_DOMAIN_TYPE_HASH.as_bytes());
    let name_hash = keccak256(DOMAIN_NAME.as_bytes());
    let version_hash = keccak256(DOMAIN_VERSION.as_bytes());

    let mut buf = Vec::with_capacity(5 * 32);
    buf.extend_from_slice(&domain_type_hash);
    buf.extend_from_slice(&name_hash);
    buf.extend_from_slice(&version_hash);
    // chainId = 1337, left-padded to 32 bytes
    let mut chain_id_bytes = [0u8; 32];
    U256::from(CHAIN_ID).to_big_endian(&mut chain_id_bytes);
    buf.extend_from_slice(&chain_id_bytes);
    // verifyingContract = address(0), left-padded to 32 bytes
    let mut addr_bytes = [0u8; 32];
    let zero_addr = H160::zero();
    addr_bytes[12..].copy_from_slice(zero_addr.as_bytes());
    buf.extend_from_slice(&addr_bytes);

    keccak256(&buf)
}

/// Compute the struct hash for Agent { source, connectionId }.
fn agent_struct_hash(source: &str, connection_id: [u8; 32]) -> [u8; 32] {
    let type_hash = keccak256(AGENT_TYPE_HASH.as_bytes());
    let source_hash = keccak256(source.as_bytes());

    let mut buf = Vec::with_capacity(3 * 32);
    buf.extend_from_slice(&type_hash);
    buf.extend_from_slice(&source_hash);
    buf.extend_from_slice(&connection_id);

    keccak256(&buf)
}

/// Build the connection_id from an action, nonce, and optional vault address.
///
/// Steps:
/// 1. Serialize the action value to MessagePack bytes
/// 2. Append nonce as 8-byte big-endian
/// 3. Append vault_address flag: 1 if present, 0 if absent
/// 4. If vault_address present, append the 20-byte address
/// 5. keccak256 the whole buffer → connection_id
pub fn build_connection_id(
    action: &serde_json::Value,
    nonce: u64,
    vault_address: Option<&str>,
) -> Result<[u8; 32]> {
    let msgpack_bytes =
        rmp_serde::to_vec_named(action).context("Failed to serialize action to MessagePack")?;

    let mut buf = Vec::with_capacity(msgpack_bytes.len() + 8 + 1 + 20);
    buf.extend_from_slice(&msgpack_bytes);
    buf.extend_from_slice(&nonce.to_be_bytes());

    match vault_address {
        Some(addr) => {
            buf.push(1u8);
            let addr_bytes = parse_hex_address(addr)?;
            buf.extend_from_slice(&addr_bytes);
        }
        None => {
            buf.push(0u8);
        }
    }

    Ok(keccak256(&buf))
}

/// Sign an L1 action for Hyperliquid exchange API.
///
/// Returns a SignaturePayload { r, s, v } suitable for the exchange request.
pub async fn sign_l1_action(
    wallet: &Wallet<SigningKey>,
    action: &serde_json::Value,
    nonce: u64,
    vault_address: Option<&str>,
    is_mainnet: bool,
) -> Result<SignaturePayload> {
    let connection_id = build_connection_id(action, nonce, vault_address)?;

    // source: "a" for mainnet, "b" for testnet
    let source = if is_mainnet { "a" } else { "b" };

    let struct_hash = agent_struct_hash(source, connection_id);
    let domain_sep = domain_separator();

    // EIP-712 hash: keccak256("\x19\x01" || domainSeparator || structHash)
    let mut digest_input = Vec::with_capacity(2 + 32 + 32);
    digest_input.push(0x19);
    digest_input.push(0x01);
    digest_input.extend_from_slice(&domain_sep);
    digest_input.extend_from_slice(&struct_hash);
    let digest = keccak256(&digest_input);

    // Sign the digest with the wallet
    let signature = wallet
        .sign_hash(ethers::types::H256::from(digest))
        .context("Failed to sign EIP-712 digest")?;

    Ok(SignaturePayload {
        r: format!("0x{}", {
            let mut bytes = [0u8; 32];
            signature.r.to_big_endian(&mut bytes);
            hex::encode(bytes)
        }),
        s: format!("0x{}", {
            let mut bytes = [0u8; 32];
            signature.s.to_big_endian(&mut bytes);
            hex::encode(bytes)
        }),
        v: signature.v as u8,
    })
}

/// Parse a hex address string (with or without 0x prefix) into 20 bytes.
fn parse_hex_address(addr: &str) -> Result<[u8; 20]> {
    let clean = addr.strip_prefix("0x").unwrap_or(addr);
    let bytes = hex::decode(clean).context("Invalid hex address")?;
    if bytes.len() != 20 {
        anyhow::bail!("Address must be 20 bytes, got {}", bytes.len());
    }
    let mut result = [0u8; 20];
    result.copy_from_slice(&bytes);
    Ok(result)
}

/// Create a LocalWallet from a hex private key string.
pub fn wallet_from_key(private_key: &str) -> Result<LocalWallet> {
    let clean = private_key.strip_prefix("0x").unwrap_or(private_key);
    let key_bytes = hex::decode(clean).context("Invalid hex private key")?;
    let wallet = LocalWallet::from_bytes(&key_bytes).context("Invalid private key bytes")?;
    Ok(wallet)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::signers::Signer;

    const TEST_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn domain_separator_is_deterministic() {
        let sep1 = domain_separator();
        let sep2 = domain_separator();
        assert_eq!(sep1, sep2);
        // Should be 32 bytes
        assert_eq!(sep1.len(), 32);
    }

    #[test]
    fn agent_struct_hash_differs_by_source() {
        let connection_id = [0u8; 32];
        let hash_a = agent_struct_hash("a", connection_id);
        let hash_b = agent_struct_hash("b", connection_id);
        assert_ne!(hash_a, hash_b, "Mainnet and testnet hashes should differ");
    }

    #[test]
    fn build_connection_id_deterministic() {
        let action = serde_json::json!({"type": "order", "orders": [], "grouping": "na"});
        let id1 = build_connection_id(&action, 1000, None).unwrap();
        let id2 = build_connection_id(&action, 1000, None).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn build_connection_id_differs_by_nonce() {
        let action = serde_json::json!({"type": "order", "orders": [], "grouping": "na"});
        let id1 = build_connection_id(&action, 1000, None).unwrap();
        let id2 = build_connection_id(&action, 1001, None).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn build_connection_id_differs_with_vault() {
        let action = serde_json::json!({"type": "order", "orders": [], "grouping": "na"});
        let id_no_vault = build_connection_id(&action, 1000, None).unwrap();
        let id_with_vault = build_connection_id(
            &action,
            1000,
            Some("0x1234567890abcdef1234567890abcdef12345678"),
        )
        .unwrap();
        assert_ne!(id_no_vault, id_with_vault);
    }

    #[test]
    fn wallet_from_key_works() {
        let wallet = wallet_from_key(TEST_KEY).unwrap();
        let addr = format!("{:?}", wallet.address());
        // Known address for private key = 1
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 42);
    }

    #[test]
    fn wallet_from_key_without_prefix() {
        let key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet = wallet_from_key(key).unwrap();
        let wallet2 = wallet_from_key(TEST_KEY).unwrap();
        assert_eq!(wallet.address(), wallet2.address());
    }

    #[test]
    fn wallet_from_key_invalid() {
        assert!(wallet_from_key("not-a-key").is_err());
        assert!(wallet_from_key("0x123").is_err());
    }

    #[tokio::test]
    async fn sign_l1_action_produces_valid_signature() {
        let wallet = wallet_from_key(TEST_KEY).unwrap();
        let action = serde_json::json!({"type": "order", "orders": [], "grouping": "na"});
        let sig = sign_l1_action(&wallet, &action, 1000, None, false)
            .await
            .unwrap();

        // r and s should be 0x-prefixed 64-char hex strings
        assert!(sig.r.starts_with("0x"));
        assert!(sig.s.starts_with("0x"));
        assert_eq!(sig.r.len(), 66); // 0x + 64 hex chars
        assert_eq!(sig.s.len(), 66);
        // v should be 27 or 28
        assert!(sig.v == 27 || sig.v == 28);
    }

    #[tokio::test]
    async fn sign_l1_action_mainnet_vs_testnet_differ() {
        let wallet = wallet_from_key(TEST_KEY).unwrap();
        let action = serde_json::json!({"type": "order", "orders": [], "grouping": "na"});

        let sig_mainnet = sign_l1_action(&wallet, &action, 1000, None, true)
            .await
            .unwrap();
        let sig_testnet = sign_l1_action(&wallet, &action, 1000, None, false)
            .await
            .unwrap();

        // Different source should produce different signatures
        assert!(
            sig_mainnet.r != sig_testnet.r || sig_mainnet.s != sig_testnet.s,
            "Mainnet and testnet signatures should differ"
        );
    }

    #[test]
    fn parse_hex_address_valid() {
        let addr = parse_hex_address("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(addr.len(), 20);
        assert_eq!(addr[0], 0x12);
    }

    #[test]
    fn parse_hex_address_invalid_length() {
        assert!(parse_hex_address("0x1234").is_err());
    }
}
