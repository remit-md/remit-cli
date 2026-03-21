// Functions used by the client module (task 0.3+) and command handlers (task 0.4+).
#![allow(dead_code)]

/// Authentication for the Remit API.
///
/// Every request requires four headers computed from the caller's private key:
///   X-Remit-Signature: <hex_sig>     — EIP-712 signature over the request
///   X-Remit-Agent:     <address>     — signer's wallet address
///   X-Remit-Timestamp: <unix_secs>   — current time (server rejects ±5 min)
///   X-Remit-Nonce:     <0x_hex>      — 32-byte random nonce (replay prevention)
///
/// The signed EIP-712 struct is:
///   domain: { name: "remit.md", version: "0.1", chainId, verifyingContract }
///   type: APIRequest { method: string, path: string, timestamp: uint256, nonce: bytes32 }
use anyhow::{anyhow, Result};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;

// ── Chain config ──────────────────────────────────────────────────────────────

/// Known chain configurations keyed by testnet flag.
pub struct ChainConfig {
    pub chain_id: u64,
    pub router: String,
}

impl ChainConfig {
    /// Mainnet — Base (chain 8453). Router address TBD until mainnet deploy.
    pub fn mainnet() -> Self {
        Self {
            chain_id: 8453,
            // Will be set at mainnet deploy. For now the server runs on Base Sepolia.
            router: std::env::var("REMITMD_ROUTER")
                .unwrap_or_else(|_| "0x0000000000000000000000000000000000000000".to_string()),
        }
    }

    /// Testnet — Base Sepolia (chain 84532).
    pub fn testnet() -> Self {
        Self {
            chain_id: 84532,
            router: std::env::var("REMITMD_ROUTER")
                .unwrap_or_else(|_| "0xb3E96ebE54138d1c0caea00Ae098309C7E0138eC".to_string()),
        }
    }

    /// Select chain config for the given testnet flag.
    /// Currently both use Base Sepolia as mainnet deploy hasn't happened.
    pub fn for_network(testnet: bool) -> Self {
        if testnet {
            Self::testnet()
        } else {
            // Production server currently runs on Base Sepolia
            Self::testnet()
        }
    }
}

// ── Key loading ───────────────────────────────────────────────────────────────

/// Load the private key from REMITMD_KEY env var (with or without 0x prefix).
pub fn load_private_key() -> Result<String> {
    env::var("REMITMD_KEY").map_err(|_| {
        anyhow!(
            "REMITMD_KEY not set.\n\
            Set it in your environment or .env file:\n  export REMITMD_KEY=0x<your-private-key>\n\
            Run `remit init` to generate a new keypair."
        )
    })
}

fn parse_signer() -> Result<PrivateKeySigner> {
    let key = load_private_key()?;
    signer_from_hex(&key)
}

fn signer_from_hex(key_hex: &str) -> Result<PrivateKeySigner> {
    let key = key_hex.trim_start_matches("0x");
    let bytes = hex::decode(key).map_err(|e| anyhow!("private key is not valid hex: {e}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("private key must be exactly 32 bytes (64 hex chars)"))?;
    PrivateKeySigner::from_bytes(&bytes.into()).map_err(|e| anyhow!("Invalid private key: {e}"))
}

/// Return the wallet address (lowercase, 0x-prefixed) derived from REMITMD_KEY.
pub fn wallet_address() -> Result<String> {
    let signer = parse_signer()?;
    Ok(format!("{:#x}", signer.address()))
}

// ── EIP-712 hash ──────────────────────────────────────────────────────────────

const DOMAIN_TYPEHASH_STR: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const REQUEST_TYPEHASH_STR: &str =
    "APIRequest(string method,string path,uint256 timestamp,bytes32 nonce)";

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(data);
    h.finalize().into()
}

/// Encode a uint256 value (u64) as a 32-byte big-endian ABI slot.
fn abi_uint256(v: u64) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[24..32].copy_from_slice(&v.to_be_bytes());
    slot
}

/// Encode a 20-byte Ethereum address as a 32-byte ABI slot (left-padded).
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

/// Compute the EIP-712 domain separator.
fn domain_separator(chain_id: u64, router: &str) -> Result<[u8; 32]> {
    let domain_typehash = keccak(DOMAIN_TYPEHASH_STR.as_bytes());
    let name_hash = keccak(b"remit.md");
    let version_hash = keccak(b"0.1");
    let chain_id_slot = abi_uint256(chain_id);
    let router_slot = abi_address(router)?;

    let mut encoded = Vec::with_capacity(5 * 32);
    encoded.extend_from_slice(&domain_typehash);
    encoded.extend_from_slice(&name_hash);
    encoded.extend_from_slice(&version_hash);
    encoded.extend_from_slice(&chain_id_slot);
    encoded.extend_from_slice(&router_slot);

    Ok(keccak(&encoded))
}

/// Compute the APIRequest struct hash.
fn struct_hash(method: &str, path: &str, timestamp: u64, nonce: &[u8; 32]) -> [u8; 32] {
    let type_hash = keccak(REQUEST_TYPEHASH_STR.as_bytes());
    let method_hash = keccak(method.as_bytes());
    let path_hash = keccak(path.as_bytes());
    let ts_slot = abi_uint256(timestamp);

    let mut encoded = Vec::with_capacity(5 * 32);
    encoded.extend_from_slice(&type_hash);
    encoded.extend_from_slice(&method_hash);
    encoded.extend_from_slice(&path_hash);
    encoded.extend_from_slice(&ts_slot);
    encoded.extend_from_slice(nonce);

    keccak(&encoded)
}

/// Compute the final EIP-712 hash: keccak256("\x19\x01" || domain_sep || struct_hash).
pub fn eip712_hash(
    method: &str,
    path: &str,
    timestamp: u64,
    nonce: &[u8; 32],
    chain_id: u64,
    router: &str,
) -> Result<[u8; 32]> {
    let domain_sep = domain_separator(chain_id, router)?;
    let s_hash = struct_hash(method, path, timestamp, nonce);

    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.push(0x19u8);
    buf.push(0x01u8);
    buf.extend_from_slice(&domain_sep);
    buf.extend_from_slice(&s_hash);

    Ok(keccak(&buf))
}

// ── Auth headers ──────────────────────────────────────────────────────────────

/// Build the four auth headers for a request.
///
/// `path` must be the full path as sent to the server, e.g. `/api/v1/pay`.
pub fn build_auth_headers(
    method: &str,
    path: &str,
    chain_id: u64,
    router: &str,
) -> Result<HashMap<String, String>> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| anyhow!("system clock error"))?;

    // 32-byte random nonce
    let nonce_bytes: [u8; 32] = {
        use rand::RngCore;
        let mut buf = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut buf);
        buf
    };
    let nonce_hex = format!("0x{}", hex::encode(nonce_bytes));

    let hash = eip712_hash(method, path, timestamp, &nonce_bytes, chain_id, router)?;

    let signer = parse_signer()?;
    let sig = signer
        .sign_hash_sync(&hash.into())
        .map_err(|e| anyhow!("signing failed: {e}"))?;

    let address = format!("{:#x}", signer.address());
    let sig_hex = format!("0x{}", hex::encode(sig.as_bytes()));

    let mut headers = HashMap::new();
    headers.insert("X-Remit-Signature".to_string(), sig_hex);
    headers.insert("X-Remit-Agent".to_string(), address);
    headers.insert("X-Remit-Timestamp".to_string(), timestamp.to_string());
    headers.insert("X-Remit-Nonce".to_string(), nonce_hex);

    Ok(headers)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Canonical test vectors from remit-server/test-vectors/eip712.json
    // Vector 1: POST /api/v1/escrows
    // Anvil test wallet #0 private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const TEST_ROUTER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const TEST_CHAIN_ID: u64 = 84532;
    const TEST_NONCE_HEX: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    const TEST_TIMESTAMP: u64 = 1741400000;

    fn test_nonce() -> [u8; 32] {
        let b = hex::decode(TEST_NONCE_HEX).unwrap();
        b.try_into().unwrap()
    }

    #[test]
    fn test_eip712_hash_post_escrows() {
        let hash = eip712_hash(
            "POST",
            "/api/v1/escrows",
            TEST_TIMESTAMP,
            &test_nonce(),
            TEST_CHAIN_ID,
            TEST_ROUTER,
        )
        .unwrap();
        assert_eq!(
            hex::encode(hash),
            "f5a0d6dae638bc7974ebe98f0b0633746a39fb5ec338a7bc6c7695b7a476aa56",
            "POST /api/v1/escrows hash must match test vector"
        );
    }

    #[test]
    fn test_eip712_hash_get_escrows() {
        let hash = eip712_hash(
            "GET",
            "/api/v1/escrows",
            TEST_TIMESTAMP,
            &test_nonce(),
            TEST_CHAIN_ID,
            TEST_ROUTER,
        )
        .unwrap();
        assert_eq!(
            hex::encode(hash),
            "bdc05060d899f3c6a9396e2791fb6818bb8d69a6b2ab17909028bd1a793978dc",
            "GET /api/v1/escrows hash must match test vector"
        );
    }

    #[test]
    fn test_sign_and_recover() {
        // Sign with test key directly (no env var mutation — avoids test race conditions)
        let hash = eip712_hash(
            "POST",
            "/api/v1/escrows",
            TEST_TIMESTAMP,
            &test_nonce(),
            TEST_CHAIN_ID,
            TEST_ROUTER,
        )
        .unwrap();

        let signer = signer_from_hex(TEST_PRIVATE_KEY).unwrap();
        let sig = signer.sign_hash_sync(&hash.into()).unwrap();
        let address = format!("{:#x}", signer.address());

        assert_eq!(
            address, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "recovered address must be Anvil test wallet #0"
        );
        assert_eq!(sig.as_bytes().len(), 65, "signature must be 65 bytes");
    }

    #[test]
    fn test_missing_key_error() {
        let original = std::env::var("REMITMD_KEY").ok();
        unsafe {
            std::env::remove_var("REMITMD_KEY");
        }

        let result = load_private_key();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("REMITMD_KEY not set"));

        if let Some(val) = original {
            unsafe {
                std::env::set_var("REMITMD_KEY", val);
            }
        }
    }
}
