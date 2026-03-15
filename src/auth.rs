#![allow(dead_code)]

use anyhow::{anyhow, Result};
use std::env;

/// Load the private key from REMITMD_KEY env var.
/// Returns the hex-encoded private key (with or without 0x prefix).
pub fn load_private_key() -> Result<String> {
    env::var("REMITMD_KEY").map_err(|_| {
        anyhow!(
            "REMITMD_KEY not set.\n\
            Set it in your environment or .env file:\n\
            export REMITMD_KEY=0x<your-private-key>\n\
            Run `remit init` to generate a new keypair."
        )
    })
}

/// Return the wallet address derived from REMITMD_KEY.
pub fn wallet_address() -> Result<String> {
    use alloy::signers::local::PrivateKeySigner;

    let key = load_private_key()?;
    let key = key.trim_start_matches("0x");
    let bytes = hex::decode(key).map_err(|e| anyhow!("REMITMD_KEY is not valid hex: {e}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("REMITMD_KEY must be exactly 32 bytes"))?;

    let signer = PrivateKeySigner::from_bytes(&bytes.into())
        .map_err(|e| anyhow!("Invalid private key: {e}"))?;

    Ok(format!("{:#x}", signer.address()))
}

/// Sign a message using EIP-191 personal_sign (for request authentication).
/// Returns (signature_hex, address).
pub fn sign_message(message: &[u8]) -> Result<(String, String)> {
    use alloy::signers::local::PrivateKeySigner;
    use alloy::signers::SignerSync;

    let key = load_private_key()?;
    let key = key.trim_start_matches("0x");
    let bytes = hex::decode(key).map_err(|e| anyhow!("REMITMD_KEY is not valid hex: {e}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("REMITMD_KEY must be exactly 32 bytes"))?;

    let signer = PrivateKeySigner::from_bytes(&bytes.into())
        .map_err(|e| anyhow!("Invalid private key: {e}"))?;

    let sig = signer
        .sign_message_sync(message)
        .map_err(|e| anyhow!("Signing failed: {e}"))?;

    let address = format!("{:#x}", signer.address());
    let sig_hex = format!("0x{}", hex::encode(sig.as_bytes()));

    Ok((sig_hex, address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_key_error() {
        // Temporarily unset the env var (if set) to verify error message
        let original = std::env::var("REMITMD_KEY").ok();
        std::env::remove_var("REMITMD_KEY");

        let result = load_private_key();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("REMITMD_KEY not set"));

        // Restore
        if let Some(val) = original {
            std::env::set_var("REMITMD_KEY", val);
        }
    }
}
