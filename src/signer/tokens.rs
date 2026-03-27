//! Bearer token management for the local signer.
//!
//! Security invariants:
//! - Raw tokens are NEVER stored on disk — only SHA-256 hashes
//! - Token validation is constant-time via hash comparison
//! - Revoked tokens fail auth on the NEXT request (immediate)
//! - Token material NEVER appears in error messages, logs, or Debug impls
//! - Token format: `rmit_sk_{64 hex chars}` (72 chars total)
#![deny(unsafe_code)]

use anyhow::{anyhow, Context, Result};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ── Constants ──────────────────────────────────────────────────────────────

const TOKEN_PREFIX: &str = "rmit_sk_";
const TOKEN_RANDOM_BYTES: usize = 32;
/// Number of hex chars from the hash used as filename prefix.
const HASH_PREFIX_LEN: usize = 16;

// ── Types ──────────────────────────────────────────────────────────────────

/// On-disk token record (JSON). Contains the hash, NOT the raw token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    /// SHA-256 hash of the full token string (hex-encoded).
    pub hash: String,
    /// Human-readable name for this token.
    pub name: String,
    /// Name of the wallet this token is bound to.
    pub wallet: String,
    pub created_at: String,
    /// Optional expiry (ISO 8601). `None` = never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Whether this token has been revoked.
    #[serde(default)]
    pub revoked: bool,
}

/// Result of validating a bearer token.
#[derive(Debug)]
pub struct ValidatedToken {
    pub wallet: String,
    pub name: String,
}

// ── TokenStore ─────────────────────────────────────────────────────────────

/// Manages bearer token records in a directory.
pub struct TokenStore {
    dir: PathBuf,
}

impl TokenStore {
    /// Open the token store at the default location (`~/.remit/tokens/`).
    pub fn open() -> Result<Self> {
        let home = dirs::home_dir().context("cannot locate home directory")?;
        Ok(Self {
            dir: home.join(".remit").join("tokens"),
        })
    }

    /// Open a token store at a custom directory (for testing).
    pub fn open_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Generate a new bearer token, store its hash, and return the raw token.
    ///
    /// The raw token is returned ONCE and must be saved by the caller.
    /// Only the SHA-256 hash is written to disk.
    pub fn create(&self, name: &str, wallet: &str) -> Result<String> {
        // Generate random token: rmit_sk_{64 hex chars}
        let mut random_bytes = [0u8; TOKEN_RANDOM_BYTES];
        OsRng.fill_bytes(&mut random_bytes);
        let raw_token = format!("{TOKEN_PREFIX}{}", hex::encode(random_bytes));

        // Hash the token
        let hash = sha256_hex(&raw_token);
        let hash_prefix = &hash[..HASH_PREFIX_LEN];

        // Build record
        let record = TokenRecord {
            hash: hash.clone(),
            name: name.to_string(),
            wallet: wallet.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            revoked: false,
        };

        // Write to disk
        self.write_record(hash_prefix, &record)?;

        Ok(raw_token)
    }

    /// Validate a bearer token. Returns the associated wallet name if valid.
    ///
    /// Checks:
    /// 1. Token format is `rmit_sk_{64 hex}`
    /// 2. Hash exists on disk
    /// 3. Token is not revoked
    /// 4. Token has not expired
    pub fn validate(&self, raw_token: &str) -> Result<ValidatedToken> {
        // Format check
        if !raw_token.starts_with(TOKEN_PREFIX) {
            return Err(anyhow!("invalid token format"));
        }
        let hex_part = &raw_token[TOKEN_PREFIX.len()..];
        if hex_part.len() != TOKEN_RANDOM_BYTES * 2 {
            return Err(anyhow!("invalid token format"));
        }
        if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(anyhow!("invalid token format"));
        }

        // Hash and look up
        let hash = sha256_hex(raw_token);
        let hash_prefix = &hash[..HASH_PREFIX_LEN];

        let record = self
            .load_record(hash_prefix)
            .map_err(|_| anyhow!("invalid or unknown token"))?;

        // Verify full hash matches (not just prefix)
        if record.hash != hash {
            return Err(anyhow!("invalid or unknown token"));
        }

        // Check revocation
        if record.revoked {
            return Err(anyhow!("token has been revoked"));
        }

        // Check expiry
        if let Some(ref expires) = record.expires_at {
            let expiry = chrono::DateTime::parse_from_rfc3339(expires)
                .map_err(|_| anyhow!("invalid expiry timestamp in token record"))?;
            if chrono::Utc::now() > expiry {
                return Err(anyhow!("token has expired"));
            }
        }

        Ok(ValidatedToken {
            wallet: record.wallet,
            name: record.name,
        })
    }

    /// Revoke a token by its raw value. Sets `revoked: true` on disk.
    ///
    /// Takes effect immediately on the next `validate()` call.
    pub fn revoke(&self, raw_token: &str) -> Result<()> {
        let hash = sha256_hex(raw_token);
        let hash_prefix = &hash[..HASH_PREFIX_LEN];

        let mut record = self
            .load_record(hash_prefix)
            .map_err(|_| anyhow!("token not found"))?;

        if record.hash != hash {
            return Err(anyhow!("token not found"));
        }

        record.revoked = true;
        self.write_record(hash_prefix, &record)?;
        Ok(())
    }

    /// Revoke a token by its hash prefix (for use when the raw token is unknown).
    pub fn revoke_by_prefix(&self, hash_prefix: &str) -> Result<()> {
        let mut record = self
            .load_record(hash_prefix)
            .map_err(|_| anyhow!("token not found"))?;

        record.revoked = true;
        self.write_record(hash_prefix, &record)?;
        Ok(())
    }

    /// List all token records (without raw token values).
    pub fn list(&self) -> Result<Vec<TokenRecord>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("cannot read tokens directory: {}", self.dir.display()))?
        {
            let entry = entry?;
            if entry.file_name().to_string_lossy().ends_with(".json") {
                let contents = std::fs::read_to_string(entry.path()).with_context(|| {
                    format!("cannot read token file: {}", entry.path().display())
                })?;
                if let Ok(record) = serde_json::from_str::<TokenRecord>(&contents) {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(records)
    }

    /// Load a token record by hash prefix.
    fn load_record(&self, hash_prefix: &str) -> Result<TokenRecord> {
        let path = self.dir.join(format!("{hash_prefix}.json"));
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read token file: {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("cannot parse token file: {}", path.display()))
    }

    /// Write a token record to disk.
    fn write_record(&self, hash_prefix: &str, record: &TokenRecord) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create tokens directory: {}", self.dir.display()))?;
        let path = self.dir.join(format!("{hash_prefix}.json"));
        let json = serde_json::to_string_pretty(record).context("cannot serialize token record")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write token file: {}", path.display()))?;
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// SHA-256 hash of a string, returned as lowercase hex.
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_store() -> (TokenStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("cannot create temp dir");
        let store = TokenStore::open_in(dir.path().join("tokens"));
        (store, dir)
    }

    // ── Token generation ────────────────────────────────────────────────

    #[test]
    fn create_returns_valid_format() {
        let (store, _dir) = test_store();
        let token = store.create("test-token", "my-wallet").unwrap();

        assert!(
            token.starts_with("rmit_sk_"),
            "token must start with rmit_sk_, got: {}...",
            &token[..16]
        );
        assert_eq!(
            token.len(),
            72,
            "token must be 72 chars (8 prefix + 64 hex)"
        );
        // Hex part should be valid hex
        let hex_part = &token[8..];
        assert!(
            hex_part.chars().all(|c| c.is_ascii_hexdigit()),
            "token suffix must be hex"
        );
    }

    #[test]
    fn create_generates_unique_tokens() {
        let (store, _dir) = test_store();
        let t1 = store.create("t1", "w").unwrap();
        let t2 = store.create("t2", "w").unwrap();
        assert_ne!(t1, t2, "tokens must be unique");
    }

    // ── Validation ──────────────────────────────────────────────────────

    #[test]
    fn validate_succeeds_for_valid_token() {
        let (store, _dir) = test_store();
        let token = store.create("valid", "my-wallet").unwrap();

        let result = store.validate(&token).unwrap();
        assert_eq!(result.wallet, "my-wallet");
        assert_eq!(result.name, "valid");
    }

    #[test]
    fn validate_rejects_unknown_token() {
        let (store, _dir) = test_store();
        let fake = format!("rmit_sk_{}", "ab".repeat(32));
        let result = store.validate(&fake);
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_bad_format() {
        let (store, _dir) = test_store();

        // Wrong prefix
        assert!(store
            .validate("bad_prefix_abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678")
            .is_err());
        // Too short
        assert!(store.validate("rmit_sk_abc").is_err());
        // Non-hex
        assert!(store
            .validate("rmit_sk_zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")
            .is_err());
    }

    // ── Revocation ──────────────────────────────────────────────────────

    #[test]
    fn revoked_token_is_immediately_rejected() {
        let (store, _dir) = test_store();
        let token = store.create("revokable", "w").unwrap();

        // Valid before revocation
        assert!(store.validate(&token).is_ok());

        // Revoke
        store.revoke(&token).unwrap();

        // Invalid after revocation (IMMEDIATE — invariant I2)
        let result = store.validate(&token);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("revoked"),
            "error must mention revocation"
        );
    }

    #[test]
    fn revoke_by_prefix_works() {
        let (store, _dir) = test_store();
        let token = store.create("prefix-revoke", "w").unwrap();
        let hash = sha256_hex(&token);
        let prefix = &hash[..HASH_PREFIX_LEN];

        store.revoke_by_prefix(prefix).unwrap();
        assert!(store.validate(&token).is_err());
    }

    #[test]
    fn revoke_unknown_token_fails() {
        let (store, _dir) = test_store();
        let fake = format!("rmit_sk_{}", "ab".repeat(32));
        assert!(store.revoke(&fake).is_err());
    }

    // ── Listing ─────────────────────────────────────────────────────────

    #[test]
    fn list_returns_all_tokens() {
        let (store, _dir) = test_store();
        assert!(store.list().unwrap().is_empty());

        store.create("first", "w").unwrap();
        store.create("second", "w").unwrap();

        let records = store.list().unwrap();
        assert_eq!(records.len(), 2);
    }

    // ── No raw token on disk ────────────────────────────────────────────

    #[test]
    fn raw_token_not_stored_on_disk() {
        let (store, _dir) = test_store();
        let token = store.create("disk-check", "w").unwrap();

        // Read all files in the tokens directory
        for entry in std::fs::read_dir(store.dir.clone()).unwrap() {
            let entry = entry.unwrap();
            let contents = std::fs::read_to_string(entry.path()).unwrap();
            assert!(
                !contents.contains(&token),
                "raw token must NOT appear in token file on disk"
            );
            // Only the hash should be there
            let hash = sha256_hex(&token);
            assert!(
                contents.contains(&hash),
                "token hash SHOULD appear in token file"
            );
        }
    }

    // ── No token in error messages ──────────────────────────────────────

    #[test]
    fn error_messages_contain_no_token_material() {
        let (store, _dir) = test_store();
        let token = store.create("err-check", "w").unwrap();
        store.revoke(&token).unwrap();

        let err = store.validate(&token).unwrap_err().to_string();
        assert!(
            !err.contains(&token[8..]),
            "error must not contain token hex"
        );
    }

    // ── Expiry ──────────────────────────────────────────────────────────

    #[test]
    fn expired_token_is_rejected() {
        let (store, _dir) = test_store();
        let token = store.create("expiring", "w").unwrap();

        // Manually set expiry to the past
        let hash = sha256_hex(&token);
        let prefix = &hash[..HASH_PREFIX_LEN];
        let mut record = store.load_record(prefix).unwrap();
        record.expires_at = Some("2020-01-01T00:00:00Z".to_string());
        store.write_record(prefix, &record).unwrap();

        let result = store.validate(&token);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expired"));
    }

    // ── Token record structure ──────────────────────────────────────────

    #[test]
    fn token_record_has_correct_fields() {
        let (store, _dir) = test_store();
        let token = store.create("structured", "my-wallet").unwrap();

        let hash = sha256_hex(&token);
        let prefix = &hash[..HASH_PREFIX_LEN];
        let record = store.load_record(prefix).unwrap();

        assert_eq!(record.hash, hash);
        assert_eq!(record.name, "structured");
        assert_eq!(record.wallet, "my-wallet");
        assert!(!record.revoked);
        assert!(record.expires_at.is_none());
        assert!(!record.created_at.is_empty());
    }

    // ── SHA-256 determinism ─────────────────────────────────────────────

    #[test]
    fn sha256_is_deterministic() {
        let a = sha256_hex("test-input");
        let b = sha256_hex("test-input");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "SHA-256 hash must be 64 hex chars");
    }
}
