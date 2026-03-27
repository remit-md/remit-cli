//! EIP-2612 permit signing for USDC.
//!
//! Signs a gasless approval that the server submits on-chain before the
//! payment contract calls `transferFrom`. This avoids a separate `approve()`
//! transaction and is the primary mechanism for agent-wallet payments.
//!
//! Supports two signing backends (resolved via `crate::auth`):
//!   1. Local — REMITMD_KEY private key, signs in-process
//!   2. HTTP  — REMIT_SIGNER_URL signer server, sends only the 32-byte hash

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::RemitClient;

// ── Permit result ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermitSignature {
    pub value: u64,
    pub deadline: u64,
    pub v: u8,
    pub r: String,
    pub s: String,
}

// ── Permit nonce fetch ───────────────────────────────────────────────────────

/// Fetch the EIP-2612 permit nonce via the API (/status/{address}).
pub async fn fetch_permit_nonce(client: &RemitClient, owner: &str) -> Result<u64> {
    let status = client
        .status(owner)
        .await
        .context("failed to fetch wallet status for permit nonce")?;
    status
        .permit_nonce
        .ok_or_else(|| anyhow!("server did not return permit_nonce for {owner}"))
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

/// Compute the EIP-712 permit hash for USDC.
///
/// This is always computed locally — only the resulting 32-byte hash is
/// ever sent to a remote signer. The signer never sees amounts, addresses,
/// or any permit details.
fn compute_permit_hash(
    owner: &str,
    spender: &str,
    value: u64,
    nonce: u64,
    deadline: u64,
    chain_id: u64,
    usdc_address: &str,
) -> Result<[u8; 32]> {
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
    let owner_slot = abi_address(owner)?;
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

    Ok(keccak(&final_buf))
}

// ── Auto-permit helper for commands ─────────────────────────────────────────

/// Sign a USDC permit for the given spender contract.
///
/// Uses the resolved signing backend (local key or HTTP signer server).
/// The EIP-712 permit hash is always computed locally — only the 32-byte
/// hash is sent to a remote signer if using the HTTP backend.
///
/// `spender_field` is the contract name in the /contracts response (e.g., "router", "escrow").
/// `amount_usdc` is the USDC amount as a string (e.g., "10.50").
/// Returns the permit or an error.
pub async fn auto_permit(
    client: &RemitClient,
    amount_usdc: &str,
    spender_field: &str,
) -> Result<PermitSignature> {
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

    // Get the owner address from the signing backend (local key or HTTP signer)
    let owner = crate::auth::wallet_address().await?;

    // Value in USDC smallest unit (6 decimals)
    let value = parse_usdc_to_micro(amount_usdc)?;

    // Fetch permit nonce from API
    let nonce = fetch_permit_nonce(client, &owner).await?;

    // Deadline: 1 hour from now
    let deadline = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() + 3600)
        .map_err(|_| anyhow!("clock error"))?;

    // Compute EIP-712 hash locally — signer only sees the opaque 32-byte digest
    let hash = compute_permit_hash(
        &owner,
        &spender,
        value,
        nonce,
        deadline,
        contracts.chain_id,
        usdc,
    )?;

    // Sign via the resolved backend (local or HTTP)
    let (sig_bytes, _addr) = crate::auth::sign_digest(&hash).await?;

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
    fn test_compute_permit_hash_deterministic() {
        // Same inputs should always produce the same hash
        let hash1 = compute_permit_hash(
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            10_000_000,
            0,
            1741400000,
            84532,
            "0x2d846325766921935f37d5b4478196d3ef93707c",
        )
        .unwrap();

        let hash2 = compute_permit_hash(
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            10_000_000,
            0,
            1741400000,
            84532,
            "0x2d846325766921935f37d5b4478196d3ef93707c",
        )
        .unwrap();

        assert_eq!(hash1, hash2, "permit hash must be deterministic");
        assert_ne!(hash1, [0u8; 32], "permit hash must not be zero");
    }
}
