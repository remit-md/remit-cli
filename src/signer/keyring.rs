//! OS keychain backend for private key storage.
//!
//! Stores raw 32-byte private keys in the platform's native credential store:
//! - macOS: Security.framework Keychain
//! - Windows: Credential Manager (DPAPI)
//! - Linux: Secret Service API (GNOME Keyring / KWallet)
//!
//! Security invariants:
//! - K1: Raw key returned in `Zeroizing<[u8; 32]>` — zeroed on drop.
//! - K2: Encrypted at rest by the OS. No extra password needed for interactive use.
//! - K3: No key material in error messages.
//! - K4: MetaFile contains ONLY public info (address, storage type). No secrets.
//! - K5: `is_available()` is non-destructive — never stores, loads, or deletes.
//! - K6: `store_key` takes `&[u8; 32]` — caller controls key lifetime.
#![deny(unsafe_code)]
#![deny(clippy::unwrap_used)]

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Service name used in all keychain entries.
const KEYCHAIN_SERVICE: &str = "remit";

// ── MetaFile ──────────────────────────────────────────────────────────────

/// On-disk metadata for keychain-stored keys.
///
/// Contains ONLY public information — no secret material.
/// Written to `~/.remit/keys/{name}.meta` when using the keychain path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaFile {
    pub version: u32,
    pub name: String,
    /// Wallet address (0x-prefixed, lowercase). Public info.
    pub address: String,
    /// Storage backend: "keychain" or "file".
    pub storage: String,
    pub created_at: String,
}

impl MetaFile {
    /// Path to the meta file for a given wallet name.
    pub fn path(name: &str) -> Result<PathBuf> {
        let home = dirs::home_dir().context("cannot locate home directory")?;
        Ok(home
            .join(".remit")
            .join("keys")
            .join(format!("{name}.meta")))
    }

    /// Write meta file to disk (creates parent dirs if needed).
    pub fn write_to_disk(&self) -> Result<()> {
        let path = Self::path(&self.name)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create keys directory: {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("cannot serialize meta file")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write meta file: {}", path.display()))
    }

    /// Load meta file from disk.
    pub fn load(name: &str) -> Result<Self> {
        let path = Self::path(name)?;
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read meta file: {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("cannot parse meta file: {}", path.display()))
    }

    /// Check if a meta file exists for the given wallet name.
    pub fn exists(name: &str) -> Result<bool> {
        Ok(Self::path(name)?.exists())
    }

    /// Delete the meta file from disk.
    pub fn delete(name: &str) -> Result<()> {
        let path = Self::path(name)?;
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot delete meta file: {}", path.display()))?;
        }
        Ok(())
    }
}

// ── Keychain operations ───────────────────────────────────────────────────

/// Check if the OS keychain is available for use.
///
/// Returns `true` if the platform keychain daemon is running and accessible.
/// Returns `false` in headless environments (SSH, Docker, CI, WSL without bridge).
///
/// K5: Non-destructive — never stores, loads, or deletes.
pub fn is_available() -> bool {
    // Try creating an entry — if the platform has no keychain backend, this fails.
    let entry = match keyring::Entry::new(KEYCHAIN_SERVICE, "__remit_probe__") {
        Ok(e) => e,
        Err(_) => return false,
    };

    // Attempt to read a non-existent key. NoEntry means keychain works.
    // Any other error means the keychain is not usable.
    match entry.get_secret() {
        Ok(_) => true,                        // Unexpected but fine — keychain works
        Err(keyring::Error::NoEntry) => true, // Expected — keychain works, no key stored
        Err(_) => false,                      // Keychain not available
    }
}

/// Store a raw 32-byte private key in the OS keychain.
///
/// Label format: wallet name (e.g., "default").
/// The keychain entry uses service="remit", user=label.
///
/// K6: Takes `&[u8; 32]` — caller controls key lifetime via Zeroizing.
pub fn store_key(label: &str, key: &[u8; 32]) -> Result<()> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, label).map_err(|e| anyhow!("keychain error: {e}"))?;
    entry
        .set_secret(key.as_ref())
        .map_err(|e| anyhow!("failed to store key in OS keychain: {e}"))
}

/// Retrieve a raw 32-byte private key from the OS keychain.
///
/// K1: Returns `Zeroizing<[u8; 32]>` — zeroed on drop.
/// K3: Error messages never include key material.
pub fn load_key(label: &str) -> Result<Zeroizing<[u8; 32]>> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, label).map_err(|e| anyhow!("keychain error: {e}"))?;

    let secret = entry.get_secret().map_err(|e| match e {
        keyring::Error::NoEntry => {
            anyhow!("no key '{label}' found in OS keychain. Run: remit signer init")
        }
        keyring::Error::Ambiguous(_) => {
            anyhow!("multiple keychain entries found for '{label}' — resolve manually")
        }
        other => anyhow!("failed to load key from OS keychain: {other}"),
    })?;

    if secret.len() != 32 {
        return Err(anyhow!(
            "keychain key has wrong length: expected 32 bytes, got {}",
            secret.len()
        ));
    }

    let mut arr = Zeroizing::new([0u8; 32]);
    arr.copy_from_slice(&secret);
    Ok(arr)
}

/// Delete a key from the OS keychain.
pub fn delete_key(label: &str) -> Result<()> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, label).map_err(|e| anyhow!("keychain error: {e}"))?;
    entry.delete_credential().map_err(|e| match e {
        keyring::Error::NoEntry => {
            anyhow!("no key '{label}' found in OS keychain")
        }
        other => anyhow!("failed to delete key from OS keychain: {other}"),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ── MetaFile tests ────────────────────────────────────────────────────

    #[test]
    fn meta_file_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let meta_path = dir.path().join("test.meta");

        let meta = MetaFile {
            version: 2,
            name: "test".to_string(),
            address: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            storage: "keychain".to_string(),
            created_at: "2026-03-28T14:30:00Z".to_string(),
        };

        // Write directly to temp path (bypass home dir resolution)
        let json = serde_json::to_string_pretty(&meta).unwrap();
        std::fs::write(&meta_path, &json).unwrap();

        // Read back
        let contents = std::fs::read_to_string(&meta_path).unwrap();
        let loaded: MetaFile = serde_json::from_str(&contents).unwrap();

        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.name, "test");
        assert_eq!(loaded.address, "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        assert_eq!(loaded.storage, "keychain");
    }

    #[test]
    fn meta_file_has_no_secret_fields() {
        let meta = MetaFile {
            version: 2,
            name: "default".to_string(),
            address: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            storage: "keychain".to_string(),
            created_at: "2026-03-28T14:30:00Z".to_string(),
        };

        let json = serde_json::to_string(&meta).unwrap();

        // K4: No secret material in MetaFile
        assert!(
            !json.contains("private_key"),
            "meta file must not contain 'private_key'"
        );
        assert!(
            !json.contains("password"),
            "meta file must not contain 'password'"
        );
        assert!(
            !json.contains("secret"),
            "meta file must not contain 'secret'"
        );
        assert!(
            !json.contains("ciphertext"),
            "meta file must not contain 'ciphertext'"
        );
    }

    #[test]
    fn meta_file_path_uses_home_dir() {
        let path = MetaFile::path("default").unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".remit"));
        assert!(path_str.contains("keys"));
        assert!(path_str.ends_with("default.meta"));
    }

    // ── Keychain availability (platform-dependent) ────────────────────────

    #[test]
    fn is_available_does_not_panic() {
        // K5: is_available() is non-destructive, should never panic
        let _result = is_available();
    }

    // ── Keychain store/load/delete roundtrip ──────────────────────────────
    // These tests use the REAL OS keychain. They only run locally,
    // not in CI (where is_available() returns false).

    #[test]
    fn keychain_roundtrip_if_available() {
        if !is_available() {
            eprintln!("skipping keychain_roundtrip_if_available: no keychain available");
            return;
        }

        let test_label = "__remit_test_roundtrip__";
        let test_key: [u8; 32] = [0xab; 32];

        // Store
        store_key(test_label, &test_key).expect("store_key should succeed");

        // Load
        let loaded = load_key(test_label).expect("load_key should succeed");
        assert_eq!(*loaded, test_key, "loaded key must match stored key");

        // Delete
        delete_key(test_label).expect("delete_key should succeed");

        // Verify deleted
        let result = load_key(test_label);
        assert!(result.is_err(), "load after delete should fail");
    }

    #[test]
    fn load_nonexistent_key_fails() {
        if !is_available() {
            eprintln!("skipping load_nonexistent_key_fails: no keychain available");
            return;
        }

        let result = load_key("__remit_nonexistent_test_key__");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("no key"),
            "error should mention missing key: {err_msg}"
        );
    }

    #[test]
    fn delete_nonexistent_key_fails() {
        if !is_available() {
            eprintln!("skipping delete_nonexistent_key_fails: no keychain available");
            return;
        }

        let result = delete_key("__remit_nonexistent_test_key__");
        assert!(result.is_err());
    }

    #[test]
    fn error_messages_contain_no_key_material() {
        if !is_available() {
            return;
        }

        // Try loading a non-existent key
        let result = load_key("__remit_error_msg_test__");
        if let Err(e) = result {
            let msg = e.to_string();
            // K3: No hex-encoded key bytes in error messages
            assert!(
                !msg.contains("ab".repeat(16).as_str()),
                "error must not contain key material"
            );
        }
    }
}
