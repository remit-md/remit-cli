//! EIP-2612 permit signing for USDC.
//!
//! Signs a gasless approval that the server submits on-chain before the
//! payment contract calls `transferFrom`. This avoids a separate `approve()`
//! transaction and is the primary mechanism for agent-wallet payments.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::RemitClient;

use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;

// ── Permit result ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermitSignature {
    pub value: u64,
    pub deadline: u64,
    pub v: u8,
    pub r: String,
    pub s: String,
}

// ── Permit nonce fetch (API-first, RPC fallback) ───────────────────────────

const MAINNET_RPC_URL: &str = "https://mainnet.base.org";
const TESTNET_RPC_URL: &str = "https://sepolia.base.org";

/// Return the RPC URL for the given chain ID, respecting `REMITMD_RPC_URL` override.
fn rpc_url_for_chain(chain_id: u64) -> Result<String> {
    if let Ok(url) = std::env::var("REMITMD_RPC_URL") {
        return Ok(url);
    }
    match chain_id {
        8453 => Ok(MAINNET_RPC_URL.to_string()),
        84532 => Ok(TESTNET_RPC_URL.to_string()),
        _ => Err(anyhow!(
            "Unknown chain_id {chain_id}. Supported: 8453 (Base), 84532 (Base Sepolia). \
             Set REMITMD_RPC_URL for custom chains."
        )),
    }
}

/// Fetch the EIP-2612 permit nonce via the API (/status/{address}).
/// Falls back to direct RPC if the API is unavailable or doesn't return the nonce.
pub async fn fetch_permit_nonce(
    client: &RemitClient,
    owner: &str,
    usdc_address: &str,
    chain_id: u64,
) -> Result<u64> {
    // Try API first.
    if let Ok(status) = client.status(owner).await {
        if let Some(nonce) = status.permit_nonce {
            return Ok(nonce);
        }
    }
    // Fall back to direct RPC.
    fetch_usdc_nonce_rpc(owner, usdc_address, chain_id).await
}

/// Fallback: fetch the EIP-2612 nonce directly from the USDC contract via JSON-RPC eth_call.
/// Uses `nonces(address)` selector = 0x7ecebe00.
async fn fetch_usdc_nonce_rpc(owner: &str, usdc_address: &str, chain_id: u64) -> Result<u64> {
    let padded = owner.to_lowercase().trim_start_matches("0x").to_string();
    let padded = format!("{:0>64}", padded);
    let data = format!("0x7ecebe00{padded}");

    let rpc_url = rpc_url_for_chain(chain_id)?;
    let http = reqwest::Client::new();
    let resp = http
        .post(&rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{ "to": usdc_address, "data": data }, "latest"],
        }))
        .send()
        .await
        .map_err(|e| anyhow!("RPC nonce fetch failed: {e}"))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("RPC nonce response parse failed: {e}"))?;

    if let Some(err) = json.get("error") {
        return Err(anyhow!("RPC nonces() error: {err}"));
    }

    let hex_result = json["result"]
        .as_str()
        .ok_or_else(|| anyhow!("RPC returned null result for nonce query — USDC contract may not exist at {usdc_address}"))?;
    let nonce = u64::from_str_radix(hex_result.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow!("Failed to parse nonce hex '{hex_result}': {e}"))?;
    Ok(nonce)
}

// ── EIP-712 permit signing ──────────────────────────────────────────────────

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(data);
    h.finalize().into()
}

fn abi_uint256(v: u64) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[24..32].copy_from_slice(&v.to_be_bytes());
    slot
}

fn abi_address(addr_hex: &str) -> Result<[u8; 32]> {
    let clean = addr_hex.trim_start_matches("0x");
    let bytes = hex::decode(clean).map_err(|e| anyhow!("invalid address hex: {e}"))?;
    if bytes.len() != 20 {
        return Err(anyhow!("address must be 20 bytes, got {}", bytes.len()));
    }
    let mut slot = [0u8; 32];
    slot[12..32].copy_from_slice(&bytes);
    Ok(slot)
}

/// Parse a USDC amount string (e.g. "10.50") into micro-USDC (6 decimals)
/// using integer math to avoid float precision issues.
fn parse_usdc_to_micro(amount: &str) -> Result<u64> {
    let amount = amount.trim();
    if let Some(dot_pos) = amount.find('.') {
        let integer_part = &amount[..dot_pos];
        let frac_part = &amount[dot_pos + 1..];

        // Pad or truncate fractional part to exactly 6 digits
        let frac_padded = if frac_part.len() > 6 {
            &frac_part[..6]
        } else {
            frac_part
        };
        let frac_padded = format!("{:0<6}", frac_padded);

        let int_val: u64 = if integer_part.is_empty() {
            0
        } else {
            integer_part
                .parse()
                .map_err(|_| anyhow!("invalid amount integer part: {}", amount))?
        };
        let frac_val: u64 = frac_padded
            .parse()
            .map_err(|_| anyhow!("invalid amount fractional part: {}", amount))?;

        Ok(int_val * 1_000_000 + frac_val)
    } else {
        // No decimal point — whole USDC
        let int_val: u64 = amount
            .parse()
            .map_err(|_| anyhow!("invalid amount: {}", amount))?;
        Ok(int_val * 1_000_000)
    }
}

/// Sign an EIP-2612 USDC permit.
///
/// - `client`: RemitClient for API-based nonce fetch
/// - `private_key`: hex string (with or without 0x prefix)
/// - `spender`: the contract that will call transferFrom (e.g. Router)
/// - `amount_usdc`: amount in USDC as string (e.g. "10.50")
/// - `chain_id`: chain ID (84532 for Base Sepolia)
/// - `usdc_address`: USDC contract address
pub async fn sign_usdc_permit(
    client: &RemitClient,
    private_key: &str,
    spender: &str,
    amount_usdc: &str,
    chain_id: u64,
    usdc_address: &str,
) -> Result<PermitSignature> {
    let key_hex = private_key.trim_start_matches("0x");
    let key_bytes = hex::decode(key_hex).map_err(|e| anyhow!("invalid key hex: {e}"))?;
    let key_arr: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow!("key must be 32 bytes"))?;
    let signer =
        PrivateKeySigner::from_bytes(&key_arr.into()).map_err(|e| anyhow!("invalid key: {e}"))?;
    let owner = format!("{:#x}", signer.address());

    // Value in USDC smallest unit (6 decimals) — integer math, no floats
    let value = parse_usdc_to_micro(amount_usdc)?;

    // Fetch nonce: API first, RPC fallback
    let nonce = fetch_permit_nonce(client, &owner, usdc_address, chain_id).await?;

    // Deadline: 1 hour from now
    let deadline = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() + 3600)
        .map_err(|_| anyhow!("clock error"))?;

    // EIP-712 domain for USDC: name="USD Coin", version="2"
    let domain_typehash = keccak(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak(b"USD Coin");
    let version_hash = keccak(b"2");
    let chain_slot = abi_uint256(chain_id);
    let usdc_slot = abi_address(usdc_address)?;

    let mut domain_encoded = Vec::with_capacity(5 * 32);
    domain_encoded.extend_from_slice(&domain_typehash);
    domain_encoded.extend_from_slice(&name_hash);
    domain_encoded.extend_from_slice(&version_hash);
    domain_encoded.extend_from_slice(&chain_slot);
    domain_encoded.extend_from_slice(&usdc_slot);
    let domain_separator = keccak(&domain_encoded);

    // Permit struct hash
    let permit_typehash = keccak(
        b"Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
    );
    let owner_slot = abi_address(&owner)?;
    let spender_slot = abi_address(spender)?;
    let value_slot = abi_uint256(value);
    let nonce_slot = abi_uint256(nonce);
    let deadline_slot = abi_uint256(deadline);

    let mut struct_encoded = Vec::with_capacity(6 * 32);
    struct_encoded.extend_from_slice(&permit_typehash);
    struct_encoded.extend_from_slice(&owner_slot);
    struct_encoded.extend_from_slice(&spender_slot);
    struct_encoded.extend_from_slice(&value_slot);
    struct_encoded.extend_from_slice(&nonce_slot);
    struct_encoded.extend_from_slice(&deadline_slot);
    let struct_hash = keccak(&struct_encoded);

    // Final EIP-712 hash: keccak256("\x19\x01" || domainSeparator || structHash)
    let mut final_buf = Vec::with_capacity(2 + 32 + 32);
    final_buf.push(0x19u8);
    final_buf.push(0x01u8);
    final_buf.extend_from_slice(&domain_separator);
    final_buf.extend_from_slice(&struct_hash);
    let hash = keccak(&final_buf);

    // Sign
    let sig = signer
        .sign_hash_sync(&hash.into())
        .map_err(|e| anyhow!("permit signing failed: {e}"))?;
    let sig_bytes = sig.as_bytes();

    // Split: first 32 bytes = r, next 32 = s, last byte = v
    let r = format!("0x{}", hex::encode(&sig_bytes[..32]));
    let s = format!("0x{}", hex::encode(&sig_bytes[32..64]));
    let v = sig_bytes[64];

    Ok(PermitSignature {
        value,
        deadline,
        v,
        r,
        s,
    })
}

// ── Auto-permit helper for commands ─────────────────────────────────────────

/// Sign a USDC permit for the given spender contract.
///
/// `spender_field` is the contract name in the /contracts response (e.g., "router", "escrow").
/// `amount_usdc` is the USDC amount as a string (e.g., "10.50").
/// Returns the permit or an error.
pub async fn auto_permit(
    client: &RemitClient,
    amount_usdc: &str,
    spender_field: &str,
) -> Result<PermitSignature> {
    let key = crate::auth::load_private_key()?;
    let contracts = client.get_contracts().await.context("fetch contracts")?;

    let spender = match spender_field {
        "router" => contracts.router.clone(),
        "escrow" => contracts
            .escrow
            .clone()
            .ok_or_else(|| anyhow!("server did not return escrow address"))?,
        "tab" => contracts
            .tab
            .clone()
            .ok_or_else(|| anyhow!("server did not return tab address"))?,
        "stream" => contracts
            .stream
            .clone()
            .ok_or_else(|| anyhow!("server did not return stream address"))?,
        "bounty" => contracts
            .bounty
            .clone()
            .ok_or_else(|| anyhow!("server did not return bounty address"))?,
        "deposit" => contracts
            .deposit
            .clone()
            .ok_or_else(|| anyhow!("server did not return deposit address"))?,
        _ => return Err(anyhow!("unknown spender field: {spender_field}")),
    };

    let usdc = contracts
        .usdc
        .as_deref()
        .ok_or_else(|| anyhow!("server did not return USDC address"))?;

    sign_usdc_permit(client, &key, &spender, amount_usdc, contracts.chain_id, usdc).await
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_usdc_whole_number() {
        assert_eq!(parse_usdc_to_micro("10").unwrap(), 10_000_000);
        assert_eq!(parse_usdc_to_micro("0").unwrap(), 0);
        assert_eq!(parse_usdc_to_micro("1").unwrap(), 1_000_000);
    }

    #[test]
    fn test_parse_usdc_with_decimals() {
        assert_eq!(parse_usdc_to_micro("10.50").unwrap(), 10_500_000);
        assert_eq!(parse_usdc_to_micro("0.01").unwrap(), 10_000);
        assert_eq!(parse_usdc_to_micro("0.000001").unwrap(), 1);
        assert_eq!(parse_usdc_to_micro("1.5").unwrap(), 1_500_000);
        assert_eq!(parse_usdc_to_micro("100.123456").unwrap(), 100_123_456);
    }

    #[test]
    fn test_parse_usdc_precision_no_float_error() {
        // 0.1 + 0.2 != 0.3 in float, but our integer math handles it correctly
        assert_eq!(parse_usdc_to_micro("0.1").unwrap(), 100_000);
        assert_eq!(parse_usdc_to_micro("0.2").unwrap(), 200_000);
        assert_eq!(parse_usdc_to_micro("0.3").unwrap(), 300_000);
    }

    #[test]
    fn test_parse_usdc_truncates_beyond_6_decimals() {
        // More than 6 decimal places — truncated to 6
        assert_eq!(parse_usdc_to_micro("1.1234567").unwrap(), 1_123_456);
    }

    #[test]
    fn test_parse_usdc_invalid() {
        assert!(parse_usdc_to_micro("abc").is_err());
        assert!(parse_usdc_to_micro("").is_err());
        assert!(parse_usdc_to_micro("1.2.3").is_err());
    }

    #[test]
    fn test_rpc_url_for_chain_mainnet() {
        // Without env var override, mainnet should use base.org
        std::env::remove_var("REMITMD_RPC_URL");
        assert_eq!(rpc_url_for_chain(8453).unwrap(), MAINNET_RPC_URL);
    }

    #[test]
    fn test_rpc_url_for_chain_testnet() {
        std::env::remove_var("REMITMD_RPC_URL");
        assert_eq!(rpc_url_for_chain(84532).unwrap(), TESTNET_RPC_URL);
    }

    #[test]
    fn test_rpc_url_for_chain_unknown_errors() {
        std::env::remove_var("REMITMD_RPC_URL");
        assert!(rpc_url_for_chain(12345).is_err());
    }
}
