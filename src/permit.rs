//! EIP-2612 permit signing for USDC via server-side `/permits/prepare`.
//!
//! The server computes the EIP-712 hash, manages nonces, and resolves
//! contract addresses. The CLI only signs the 32-byte hash.
//!
//! Supports two signing backends (resolved via `crate::auth`):
//!   1. Local — REMITMD_KEY private key, signs in-process
//!   2. CLI signer — `remit sign` subprocess, receives only the 32-byte hash

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

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

// ── Server response from /permits/prepare ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PermitPrepareResponse {
    pub hash: String,
    pub value: u64,
    pub deadline: u64,
    // nonce, spender, chain_id, usdc also returned but unused by CLI
}

// ── Contract name → flow name mapping ───────────────────────────────────────

fn contract_to_flow(contract: &str) -> Result<&'static str> {
    match contract {
        "router" => Ok("direct"),
        "escrow" => Ok("escrow"),
        "tab" => Ok("tab"),
        "stream" => Ok("stream"),
        "bounty" => Ok("bounty"),
        "deposit" => Ok("deposit"),
        _ => Err(anyhow!("unknown contract name: {contract}")),
    }
}

// ── Auto-permit helper for commands ─────────────────────────────────────────

/// Sign a USDC permit via the server's `/permits/prepare` endpoint.
///
/// The server computes the EIP-712 hash, manages nonces, and resolves
/// the spender contract address. The CLI only signs the 32-byte hash.
///
/// `spender_field` is the contract name (e.g., "router", "escrow").
/// `amount_usdc` is the USDC amount as a string (e.g., "10.50").
pub async fn auto_permit(
    client: &mut RemitClient,
    amount_usdc: &str,
    spender_field: &str,
) -> Result<PermitSignature> {
    let flow = contract_to_flow(spender_field)?;
    let owner = crate::auth::wallet_address().await?;

    let prepared: PermitPrepareResponse = client
        .permit_prepare(flow, amount_usdc, &owner)
        .await
        .context("POST /permits/prepare failed")?;

    // Decode the 32-byte hash from hex
    let hash_hex = prepared.hash.trim_start_matches("0x");
    let hash_bytes =
        hex::decode(hash_hex).map_err(|e| anyhow!("invalid hash from /permits/prepare: {e}"))?;
    if hash_bytes.len() != 32 {
        return Err(anyhow!(
            "expected 32-byte hash from /permits/prepare, got {}",
            hash_bytes.len()
        ));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hash_bytes);

    // Sign via the resolved backend (local key or CLI signer)
    let (sig_bytes, _addr) = crate::auth::sign_digest(&hash).await?;

    // Split: first 32 bytes = r, next 32 = s, last byte = v
    let r = format!("0x{}", hex::encode(&sig_bytes[..32]));
    let s = format!("0x{}", hex::encode(&sig_bytes[32..64]));
    let v = sig_bytes[64];

    Ok(PermitSignature {
        value: prepared.value,
        deadline: prepared.deadline,
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
    fn test_contract_to_flow() {
        assert_eq!(contract_to_flow("router").unwrap(), "direct");
        assert_eq!(contract_to_flow("escrow").unwrap(), "escrow");
        assert_eq!(contract_to_flow("tab").unwrap(), "tab");
        assert_eq!(contract_to_flow("stream").unwrap(), "stream");
        assert_eq!(contract_to_flow("bounty").unwrap(), "bounty");
        assert_eq!(contract_to_flow("deposit").unwrap(), "deposit");
        assert!(contract_to_flow("unknown").is_err());
    }
}
