//! Encrypted key storage for the local signer.
//!
//! Security invariants:
//! - Private keys encrypted at rest with AES-256-GCM (authenticated encryption)
//! - Encryption key derived from passphrase via scrypt (n=32768, r=8, p=1)
//! - Decrypted key material wrapped in `Zeroizing<>` — zeroed on drop
//! - Plaintext address stored alongside ciphertext (public info, avoids
//!   decryption for GET /address)
//! - Key material NEVER appears in error messages, logs, or serialized output
//!
//! Note: short-lived stack temporaries during B256 conversion are not
//! zeroized (would require unsafe). This matches alloy's own internals.
#![deny(unsafe_code)]

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use alloy::signers::local::PrivateKeySigner;
use anyhow::{anyhow, Context, Result};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

// ── Constants ──────────────────────────────────────────────────────────────

const KEY_FILE_VERSION: u32 = 2;
const SCRYPT_LOG_N: u8 = 15; // n = 2^15 = 32768
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const SCRYPT_DKLEN: usize = 32; // AES-256 key length
const AES_NONCE_LEN: usize = 12; // 96-bit nonce for GCM

// ── Types ──────────────────────────────────────────────────────────────────

/// On-disk encrypted key file (JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedKeyFile {
    pub version: u32,
    pub name: String,
    /// Wallet address (plaintext — this is public information).
    pub address: String,
    pub created_at: String,
    pub encryption: EncryptionParams,
}

/// Encryption parameters stored alongside the ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionParams {
    pub algorithm: String,
    pub kdf: String,
    pub kdf_params: KdfParams,
    /// Hex-encoded scrypt salt.
    pub salt: String,
    /// Hex-encoded AES-GCM nonce (12 bytes).
    pub nonce: String,
    /// Hex-encoded AES-GCM ciphertext (includes 16-byte auth tag).
    pub ciphertext: String,
}

/// Scrypt KDF parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub n: u32,
    pub r: u32,
    pub p: u32,
    pub dklen: u32,
}

// ── Keystore ───────────────────────────────────────────────────────────────

/// Manages encrypted key files in a directory.
pub struct Keystore {
    dir: PathBuf,
}

impl Keystore {
    /// Open the keystore at the default location (`~/.remit/keys/`).
    pub fn open() -> Result<Self> {
        let home = dirs::home_dir().context("cannot locate home directory")?;
        Ok(Self {
            dir: home.join(".remit").join("keys"),
        })
    }

    /// Open a keystore at a custom directory (for testing).
    #[allow(dead_code)]
    pub fn open_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Path to a specific key file.
    pub fn key_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.enc"))
    }

    /// The directory this keystore reads/writes to.
    #[allow(dead_code)]
    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    /// Generate a new secp256k1 key, encrypt it, and write to disk.
    ///
    /// Returns the wallet address (0x-prefixed, lowercase).
    /// Errors if a key with the given name already exists.
    pub fn generate(&self, name: &str, passphrase: &str) -> Result<String> {
        let path = self.key_path(name);
        if path.exists() {
            return Err(anyhow!(
                "key '{}' already exists at {}",
                name,
                path.display()
            ));
        }

        // Generate keypair
        let signer = PrivateKeySigner::random_with(&mut OsRng);
        let address = format!("{:#x}", signer.address());

        // Extract key bytes into a Zeroizing buffer
        let field_bytes = signer.credential().to_bytes();
        let raw_key = Zeroizing::new(field_bytes.as_slice().to_vec());

        // Encrypt and write
        let encryption = encrypt_key(&raw_key, passphrase)?;
        let key_file = EncryptedKeyFile {
            version: KEY_FILE_VERSION,
            name: name.to_string(),
            address: address.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            encryption,
        };
        self.write(&key_file)?;

        Ok(address)
    }

    /// Import an existing private key, encrypt it, and write to disk.
    ///
    /// Returns the wallet address. Errors if a key with the given name
    /// already exists.
    pub fn import(&self, name: &str, private_key_hex: &str, passphrase: &str) -> Result<String> {
        let path = self.key_path(name);
        if path.exists() {
            return Err(anyhow!(
                "key '{}' already exists at {}",
                name,
                path.display()
            ));
        }

        // Parse and validate key (Zeroizing the decoded bytes)
        let hex_clean = private_key_hex.trim_start_matches("0x");
        let raw_key = Zeroizing::new(
            hex::decode(hex_clean).map_err(|_| anyhow!("private key is not valid hex"))?,
        );
        if raw_key.len() != 32 {
            return Err(anyhow!(
                "private key must be exactly 32 bytes (64 hex chars)"
            ));
        }

        // Derive address from key
        let key_arr: [u8; 32] = raw_key
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("private key must be exactly 32 bytes"))?;
        let signer = PrivateKeySigner::from_bytes(&key_arr.into())
            .map_err(|_| anyhow!("not a valid secp256k1 private key"))?;
        let address = format!("{:#x}", signer.address());

        // Encrypt and write
        let encryption = encrypt_key(&raw_key, passphrase)?;
        let key_file = EncryptedKeyFile {
            version: KEY_FILE_VERSION,
            name: name.to_string(),
            address: address.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            encryption,
        };
        self.write(&key_file)?;

        Ok(address)
    }

    /// Load an encrypted key file from disk.
    pub fn load(&self, name: &str) -> Result<EncryptedKeyFile> {
        let path = self.key_path(name);
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read key file: {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("cannot parse key file: {}", path.display()))
    }

    /// List all key names (from `.enc` files in the keys directory).
    #[allow(dead_code)]
    pub fn list(&self) -> Result<Vec<String>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("cannot read keys directory: {}", self.dir.display()))?
        {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_string_lossy().strip_suffix(".enc") {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Check if a key with the given name exists on disk.
    pub fn exists(&self, name: &str) -> bool {
        self.key_path(name).exists()
    }

    /// Write an encrypted key file to disk (create dirs if needed).
    fn write(&self, key_file: &EncryptedKeyFile) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create keys directory: {}", self.dir.display()))?;
        let path = self.key_path(&key_file.name);
        let json = serde_json::to_string_pretty(key_file).context("cannot serialize key file")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write key file: {}", path.display()))?;
        Ok(())
    }
}

/// Load an encrypted key file from an explicit path (not using Keystore directory).
pub fn load_file(path: &std::path::Path) -> Result<EncryptedKeyFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file: {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("cannot parse key file: {}", path.display()))
}

// ── Pure crypto functions ──────────────────────────────────────────────────

/// Encrypt a private key (arbitrary bytes, must be 32) with AES-256-GCM.
///
/// Derives the encryption key from the passphrase via scrypt.
pub fn encrypt_key(private_key: &[u8], passphrase: &str) -> Result<EncryptionParams> {
    if private_key.len() != 32 {
        return Err(anyhow!(
            "private key must be exactly 32 bytes, got {}",
            private_key.len()
        ));
    }

    // Random salt (32 bytes) and nonce (12 bytes) from OS entropy
    let mut salt = [0u8; 32];
    let mut nonce_bytes = [0u8; AES_NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce_bytes);

    // Derive encryption key (zeroized on drop)
    let derived = derive_key(passphrase, &salt)?;

    // AES-256-GCM encrypt (ciphertext includes 16-byte auth tag)
    let cipher =
        Aes256Gcm::new_from_slice(&derived).map_err(|e| anyhow!("AES-256-GCM init failed: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, private_key)
        .map_err(|e| anyhow!("encryption failed: {e}"))?;

    Ok(EncryptionParams {
        algorithm: "aes-256-gcm".to_string(),
        kdf: "scrypt".to_string(),
        kdf_params: KdfParams {
            n: 1u32 << SCRYPT_LOG_N,
            r: SCRYPT_R,
            p: SCRYPT_P,
            dklen: SCRYPT_DKLEN as u32,
        },
        salt: hex::encode(salt),
        nonce: hex::encode(nonce_bytes),
        ciphertext: hex::encode(ciphertext),
    })
}

/// Decrypt a key file and return a `PrivateKeySigner`.
///
/// All intermediate decrypted bytes are `Zeroizing<>` — zeroed on drop,
/// whether the function succeeds or fails.
pub fn decrypt(key_file: &EncryptedKeyFile, passphrase: &str) -> Result<PrivateKeySigner> {
    let salt =
        hex::decode(&key_file.encryption.salt).map_err(|_| anyhow!("invalid salt in key file"))?;
    let nonce_bytes = hex::decode(&key_file.encryption.nonce)
        .map_err(|_| anyhow!("invalid nonce in key file"))?;
    let ciphertext = hex::decode(&key_file.encryption.ciphertext)
        .map_err(|_| anyhow!("invalid ciphertext in key file"))?;

    if nonce_bytes.len() != AES_NONCE_LEN {
        return Err(anyhow!(
            "nonce must be {} bytes, got {}",
            AES_NONCE_LEN,
            nonce_bytes.len()
        ));
    }

    // Derive decryption key (zeroized on drop)
    let derived = derive_key(passphrase, &salt)?;

    // Decrypt (zeroized on drop)
    let cipher =
        Aes256Gcm::new_from_slice(&derived).map_err(|e| anyhow!("AES-256-GCM init failed: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| anyhow!("decryption failed: wrong passphrase or corrupted key file"))?,
    );

    if plaintext.len() != 32 {
        return Err(anyhow!("decrypted key has wrong length"));
    }

    // Parse into signer (alloy zeroizes its internal key on drop)
    let key_arr: [u8; 32] = plaintext
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("decrypted key has wrong length"))?;
    PrivateKeySigner::from_bytes(&key_arr.into())
        .map_err(|_| anyhow!("decrypted data is not a valid secp256k1 private key"))
}

/// Derive an AES-256 encryption key from a passphrase using scrypt.
///
/// Returns the derived key wrapped in `Zeroizing<>` (zeroed on drop).
fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let params = scrypt::Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, SCRYPT_DKLEN)
        .map_err(|e| anyhow!("invalid scrypt params: {e}"))?;
    let mut key = Zeroizing::new(vec![0u8; SCRYPT_DKLEN]);
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, key.as_mut_slice())
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloy::signers::SignerSync;

    fn test_keystore() -> (Keystore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("cannot create temp dir");
        let ks = Keystore::open_in(dir.path().join("keys"));
        (ks, dir)
    }

    // ── Round-trip tests ────────────────────────────────────────────────

    #[test]
    fn generate_load_decrypt_roundtrip() {
        let (ks, _dir) = test_keystore();
        let passphrase = "test-passphrase-123";

        let address = ks.generate("test-key", passphrase).unwrap();
        assert!(address.starts_with("0x"));
        assert_eq!(address.len(), 42);

        let loaded = ks.load("test-key").unwrap();
        assert_eq!(loaded.name, "test-key");
        assert_eq!(loaded.address, address);
        assert_eq!(loaded.version, 2);

        let signer = decrypt(&loaded, passphrase).unwrap();
        assert_eq!(format!("{:#x}", signer.address()), address);
    }

    #[test]
    fn import_load_decrypt_roundtrip() {
        let (ks, _dir) = test_keystore();
        let passphrase = "import-test";
        // Anvil test wallet #0
        let key_hex = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let expected_addr = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";

        let address = ks.import("imported", key_hex, passphrase).unwrap();
        assert_eq!(address, expected_addr);

        let loaded = ks.load("imported").unwrap();
        let signer = decrypt(&loaded, passphrase).unwrap();
        assert_eq!(format!("{:#x}", signer.address()), expected_addr);
    }

    // ── Wrong passphrase ────────────────────────────────────────────────

    #[test]
    fn decrypt_wrong_passphrase_fails() {
        let (ks, _dir) = test_keystore();
        ks.generate("secret", "correct-passphrase").unwrap();
        let loaded = ks.load("secret").unwrap();

        let result = decrypt(&loaded, "wrong-passphrase");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("decryption failed"),
            "error should mention decryption failure, got: {err_msg}"
        );
    }

    // ── Duplicate name ──────────────────────────────────────────────────

    #[test]
    fn generate_duplicate_name_fails() {
        let (ks, _dir) = test_keystore();
        ks.generate("dupe", "pass").unwrap();
        let result = ks.generate("dupe", "pass");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn import_duplicate_name_fails() {
        let (ks, _dir) = test_keystore();
        let key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        ks.import("dupe", key, "pass").unwrap();
        let result = ks.import("dupe", key, "pass");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    // ── Nonexistent key ─────────────────────────────────────────────────

    #[test]
    fn load_nonexistent_fails() {
        let (ks, _dir) = test_keystore();
        let result = ks.load("nonexistent");
        assert!(result.is_err());
    }

    // ── Key file structure ──────────────────────────────────────────────

    #[test]
    fn key_file_has_correct_structure() {
        let (ks, _dir) = test_keystore();
        ks.generate("structured", "pass123").unwrap();

        let raw = std::fs::read_to_string(ks.key_path("structured")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed["version"], 2);
        assert_eq!(parsed["name"], "structured");
        assert!(parsed["address"].as_str().unwrap().starts_with("0x"));
        assert!(parsed["created_at"].as_str().is_some());
        assert_eq!(parsed["encryption"]["algorithm"], "aes-256-gcm");
        assert_eq!(parsed["encryption"]["kdf"], "scrypt");
        assert_eq!(parsed["encryption"]["kdf_params"]["n"], 32768);
        assert_eq!(parsed["encryption"]["kdf_params"]["r"], 8);
        assert_eq!(parsed["encryption"]["kdf_params"]["p"], 1);
        assert_eq!(parsed["encryption"]["kdf_params"]["dklen"], 32);
    }

    // ── List keys ───────────────────────────────────────────────────────

    #[test]
    fn list_keys_returns_sorted_names() {
        let (ks, _dir) = test_keystore();
        assert!(ks.list().unwrap().is_empty());

        ks.generate("bob", "pass").unwrap();
        ks.generate("alice", "pass").unwrap();

        let keys = ks.list().unwrap();
        assert_eq!(keys, vec!["alice", "bob"]);
    }

    // ── Exists ──────────────────────────────────────────────────────────

    #[test]
    fn exists_reflects_disk_state() {
        let (ks, _dir) = test_keystore();
        assert!(!ks.exists("nope"));
        ks.generate("yep", "pass").unwrap();
        assert!(ks.exists("yep"));
    }

    // ── Frame condition: no plaintext key on disk ───────────────────────

    #[test]
    fn no_plaintext_key_on_disk() {
        let (ks, _dir) = test_keystore();
        let key_hex = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        ks.import("frame-test", key_hex, "pass").unwrap();

        let raw = std::fs::read_to_string(ks.key_path("frame-test")).unwrap();
        assert!(
            !raw.contains(key_hex),
            "key file must not contain plaintext private key"
        );
    }

    // ── No key material in error messages ───────────────────────────────

    #[test]
    fn error_messages_contain_no_secrets() {
        let (ks, _dir) = test_keystore();
        let key_hex = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let passphrase = "my-secret-passphrase";
        ks.import("err-test", key_hex, passphrase).unwrap();
        let loaded = ks.load("err-test").unwrap();

        let err = decrypt(&loaded, "wrong").unwrap_err().to_string();
        assert!(!err.contains(key_hex), "error must not contain private key");
        assert!(
            !err.contains(passphrase),
            "error must not contain passphrase"
        );
    }

    // ── Corrupted ciphertext is detected (AES-GCM auth tag) ────────────

    #[test]
    fn corrupted_ciphertext_detected() {
        let (ks, _dir) = test_keystore();
        ks.generate("corrupt", "pass").unwrap();
        let mut loaded = ks.load("corrupt").unwrap();

        // Tamper with ciphertext — flip first byte
        let mut ct_bytes = hex::decode(&loaded.encryption.ciphertext).unwrap();
        ct_bytes[0] ^= 0xff;
        loaded.encryption.ciphertext = hex::encode(&ct_bytes);

        let result = decrypt(&loaded, "pass");
        assert!(result.is_err(), "corrupted ciphertext must be detected");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("decryption failed"));
    }

    // ── Import validation ───────────────────────────────────────────────

    #[test]
    fn import_invalid_hex_fails() {
        let (ks, _dir) = test_keystore();
        let result = ks.import("bad", "not-hex-at-all", "pass");
        assert!(result.is_err());
    }

    #[test]
    fn import_wrong_length_fails() {
        let (ks, _dir) = test_keystore();
        let result = ks.import("short", "0xdeadbeef", "pass");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("32 bytes"));
    }

    // ── Same key imported twice → same address ──────────────────────────

    #[test]
    fn same_key_produces_same_address() {
        let (ks1, _dir1) = test_keystore();
        let (ks2, _dir2) = test_keystore();
        let key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

        let addr1 = ks1.import("k1", key, "pass1").unwrap();
        let addr2 = ks2.import("k2", key, "pass2").unwrap();
        assert_eq!(addr1, addr2, "same key must produce same address");
    }

    // ── Deterministic: decrypt returns a signer that signs correctly ────

    #[test]
    fn decrypted_signer_produces_valid_signatures() {
        let (ks, _dir) = test_keystore();
        let key_hex = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let passphrase = "sign-test";

        ks.import("signer", key_hex, passphrase).unwrap();
        let loaded = ks.load("signer").unwrap();

        let signer = decrypt(&loaded, passphrase).unwrap();
        // Sign a known hash
        let hash = [0xabu8; 32];
        let sig = signer.sign_hash_sync(&hash.into()).unwrap();
        assert_eq!(sig.as_bytes().len(), 65, "signature must be 65 bytes");

        // Sign same hash again — must be deterministic (RFC 6979)
        let sig2 = signer.sign_hash_sync(&hash.into()).unwrap();
        assert_eq!(
            sig.as_bytes(),
            sig2.as_bytes(),
            "same key+hash must produce same signature (RFC 6979)"
        );
    }
}
