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

// ── RPC nonce fetch ─────────────────────────────────────────────────────────

const RPC_URL: &str = "https://sepolia.base.org";

/// Fetch the current EIP-2612 nonce for `owner` from the USDC contract.
/// Uses `nonces(address)` selector = 0x7ecebe00.
pub async fn fetch_usdc_nonce(owner: &str, usdc_address: &str) -> Result<u64> {
    let padded = owner.to_lowercase().trim_start_matches("0x").to_string();
    let padded = format!("{:0>64}", padded);
    let data = format!("0x7ecebe00{padded}");

    let client = reqwest::Client::new();
    let resp = client
        .post(RPC_URL)
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

    let hex_result = json["result"].as_str().unwrap_or("0x0");
    Ok(u64::from_str_radix(hex_result.trim_start_matches("0x"), 16).unwrap_or(0))
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

/// Sign an EIP-2612 USDC permit.
///
/// - `private_key`: hex string (with or without 0x prefix)
/// - `spender`: the contract that will call transferFrom (e.g. Router)
/// - `amount_usdc`: amount in USDC (e.g. 10.50)
/// - `chain_id`: chain ID (84532 for Base Sepolia)
/// - `usdc_address`: USDC contract address
pub async fn sign_usdc_permit(
    private_key: &str,
    spender: &str,
    amount_usdc: f64,
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

    // Value in USDC smallest unit (6 decimals)
    let value = (amount_usdc * 1_000_000.0).round() as u64;

    // Fetch on-chain nonce
    let nonce = fetch_usdc_nonce(&owner, usdc_address).await?;

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
/// Returns the permit or an error.
pub async fn auto_permit(
    client: &RemitClient,
    amount_usdc: f64,
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

    sign_usdc_permit(&key, &spender, amount_usdc, contracts.chain_id, usdc).await
}
