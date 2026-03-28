//! `remit sign` — sign data using the encrypted keystore.
//!
//! SECURITY INVARIANTS:
//!   I1: No network calls. Pure local computation.
//!   I2: Decrypted key in Zeroizing<> only, dropped before function returns.
//!   I3: Password never in CLI args (env var or file only).
//!   I4: stdout = hex signature, stderr = JSON errors. Nothing else.
//!   I5: Deterministic signatures (RFC 6979).
//!   I6: Fail loud — exit 1 + JSON error, never exit 0 with bad output.
//!   I7: stdin fully consumed before key decryption.
//!   I8: Non-interactive detection — error if no password source and stdin is pipe.
//!   I9: V1 keystore rejection with migration instructions.

use std::io::{self, Read, Write};

use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use anyhow::Result;
use clap::Args;
use zeroize::Zeroizing;

use crate::signer::{eip712, keyring, keystore};

#[derive(Args)]
pub struct SignArgs {
    /// Sign EIP-712 typed data (JSON on stdin with domain, types, message)
    #[arg(long, conflicts_with = "digest")]
    pub eip712: bool,

    /// Sign a raw 32-byte digest (hex on stdin)
    #[arg(long, conflicts_with = "eip712")]
    pub digest: bool,

    /// Path to keystore file (default: ~/.remit/keys/default.enc)
    #[arg(long)]
    pub keystore: Option<String>,

    /// Path to file containing the password
    #[arg(long)]
    pub password_file: Option<String>,
}

/// Structured error output on stderr. Never returns.
fn emit_error(code: &str, reason: &str) -> ! {
    let err = serde_json::json!({
        "error": code,
        "reason": reason,
    });
    eprintln!("{}", serde_json::to_string(&err).unwrap_or_default());
    std::process::exit(1);
}

pub async fn run(args: SignArgs) -> Result<()> {
    // Validate flags: exactly one of --eip712 or --digest must be set.
    if !args.eip712 && !args.digest {
        emit_error(
            "missing_flag",
            "Specify --eip712 or --digest. Usage: echo '...' | remit sign --eip712",
        );
    }

    // I7: Read stdin FULLY before any key operations.
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        emit_error("stdin_error", &format!("Failed to read stdin: {e}"));
    }

    let input = input.trim();
    if input.is_empty() {
        emit_error("invalid_input", "stdin is empty");
    }

    // Compute the digest to sign BEFORE loading the key (I7).
    let digest: [u8; 32] = if args.eip712 {
        compute_eip712_digest(input)
    } else {
        parse_hex_digest(input)
    };

    // I2: Resolve signer — keychain first, then .enc file.
    let signer = resolve_signer(&args);

    // I5: Sign with RFC 6979 deterministic nonce.
    let sig = match signer.sign_hash_sync(&digest.into()) {
        Ok(s) => s,
        Err(e) => emit_error("sign_failed", &format!("Signing failed: {e}")),
    };

    // I4: Only signature on stdout, nothing else.
    let sig_hex = format!("0x{}", hex::encode(sig.as_bytes()));
    if let Err(e) = io::stdout().write_all(sig_hex.as_bytes()) {
        emit_error("output_error", &format!("Failed to write signature: {e}"));
    }

    // Key is dropped here (Zeroize via PrivateKeySigner::drop).
    Ok(())
}

/// Resolve the private key signer.
///
/// Priority:
///   1. --keystore flag → load that specific .enc file (password required)
///   2. ~/.remit/keys/default.meta → load from OS keychain (no password)
///   3. ~/.remit/keys/default.enc → load from encrypted file (password required)
///   4. Error: no key found
fn resolve_signer(args: &SignArgs) -> PrivateKeySigner {
    if let Some(ref path) = args.keystore {
        // Explicit keystore path — always use .enc decryption
        return resolve_signer_from_enc(&std::path::PathBuf::from(path), &args.password_file);
    }

    // Check for .meta file (keychain path) first
    let meta_exists = keyring::MetaFile::exists("default").unwrap_or(false);
    if meta_exists {
        match keyring::MetaFile::load("default") {
            Ok(meta) if meta.storage == "keychain" => {
                // Load raw key from OS keychain
                let raw_key = match keyring::load_key("default") {
                    Ok(k) => k,
                    Err(e) => emit_error(
                        "keychain_error",
                        &format!("Failed to load key from OS keychain: {e}"),
                    ),
                };
                return signer_from_raw_key(&raw_key);
            }
            Ok(_) => {}  // storage != "keychain", fall through to .enc
            Err(_) => {} // corrupt meta file, fall through
        }
    }

    // Check for .enc file
    let Ok(ks) = keystore::Keystore::open() else {
        emit_error("keystore_error", "Cannot open keystore directory");
    };
    let enc_path = ks.key_path("default");
    if enc_path.exists() {
        return resolve_signer_from_enc(&enc_path, &args.password_file);
    }

    emit_error("no_keystore", "No wallet found. Run: remit signer init");
}

/// Load a signer from an encrypted .enc file (requires password).
fn resolve_signer_from_enc(
    keystore_path: &std::path::Path,
    password_file: &Option<String>,
) -> PrivateKeySigner {
    if !keystore_path.exists() {
        emit_error(
            "no_keystore",
            &format!(
                "No keystore found at {}. Run: remit signer init",
                keystore_path.display()
            ),
        );
    }

    let key_file = match keystore::load_file(keystore_path) {
        Ok(f) => f,
        Err(e) => emit_error("keystore_error", &format!("Cannot read keystore: {e}")),
    };

    // I9: Reject V1 keystores.
    if key_file.version == 1 {
        emit_error(
            "v1_keystore",
            "Keystore version 1 (V24). Run: remit signer migrate",
        );
    }

    let password = resolve_password(password_file);
    match keystore::decrypt(&key_file, &password) {
        Ok(s) => s,
        Err(_) => emit_error("decrypt_failed", "Invalid password for keystore"),
    }
}

/// Create a PrivateKeySigner from raw 32-byte key material.
fn signer_from_raw_key(raw_key: &Zeroizing<[u8; 32]>) -> PrivateKeySigner {
    let key_bytes: [u8; 32] = **raw_key;
    match PrivateKeySigner::from_bytes(&key_bytes.into()) {
        Ok(s) => s,
        Err(_) => emit_error(
            "key_error",
            "Key from OS keychain is not a valid secp256k1 key",
        ),
    }
}

/// Resolve the password for keystore decryption.
///
/// Priority:
///   1. REMIT_SIGNER_KEY env var
///   2. REMIT_KEY_PASSWORD env var (deprecated fallback)
///   3. --password-file contents
///   4. Error (never interactive — stdin is used for payload)
fn resolve_password(password_file: &Option<String>) -> String {
    // 1. New env var
    if let Ok(pw) = std::env::var("REMIT_SIGNER_KEY") {
        if !pw.is_empty() {
            return pw;
        }
    }

    // 2. Deprecated env var (backwards compat — remove in V28)
    if let Ok(pw) = std::env::var("REMIT_KEY_PASSWORD") {
        if !pw.is_empty() {
            eprintln!(
                "\u{26a0} REMIT_KEY_PASSWORD is deprecated and will be removed in a future release.\n  \
                 Set REMIT_SIGNER_KEY instead."
            );
            return pw;
        }
    }

    // 3. Password file
    if let Some(path) = password_file {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let pw = contents.trim().to_string();
                if pw.is_empty() {
                    emit_error("no_password", "Password file is empty");
                }
                return pw;
            }
            Err(e) => emit_error(
                "no_password",
                &format!("Cannot read password file {path}: {e}"),
            ),
        }
    }

    // 4. No password source
    emit_error(
        "no_password",
        "No password source. Set REMIT_SIGNER_KEY or use --password-file",
    );
}

/// Parse EIP-712 JSON and compute the typed data hash.
///
/// Input JSON: { "domain": {...}, "types": {...}, "message": {...} }
/// The "message" field maps to eip712::TypedDataRequest.value.
fn compute_eip712_digest(input: &str) -> [u8; 32] {
    let parsed: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => emit_error("invalid_input", &format!("Invalid JSON: {e}")),
    };

    // Validate required fields exist
    let domain_val = match parsed.get("domain") {
        Some(d) => d,
        None => emit_error(
            "invalid_input",
            "Expected JSON with domain, types, message fields",
        ),
    };
    let types_val = match parsed.get("types") {
        Some(t) => t,
        None => emit_error(
            "invalid_input",
            "Expected JSON with domain, types, message fields",
        ),
    };
    let message_val = match parsed.get("message") {
        Some(m) => m,
        None => emit_error(
            "invalid_input",
            "Expected JSON with domain, types, message fields",
        ),
    };

    // Deserialize domain using serde (handles camelCase automatically)
    let domain: eip712::TypedDataDomain = match serde_json::from_value(domain_val.clone()) {
        Ok(d) => d,
        Err(e) => emit_error("invalid_input", &format!("Invalid domain: {e}")),
    };

    // Deserialize types
    let types: std::collections::BTreeMap<String, Vec<eip712::TypeField>> =
        match serde_json::from_value(types_val.clone()) {
            Ok(t) => t,
            Err(e) => emit_error("invalid_input", &format!("Invalid types: {e}")),
        };

    let request = eip712::TypedDataRequest {
        domain,
        types,
        value: message_val.clone(),
    };

    match eip712::hash_typed_data(&request) {
        Ok(h) => h,
        Err(e) => emit_error("hash_failed", &format!("EIP-712 hashing failed: {e}")),
    }
}

/// Parse a hex-encoded 32-byte digest from stdin.
fn parse_hex_digest(input: &str) -> [u8; 32] {
    let hex_str = input.trim_start_matches("0x");
    let bytes = match hex::decode(hex_str) {
        Ok(b) => b,
        Err(e) => emit_error("invalid_input", &format!("Digest is not valid hex: {e}")),
    };

    if bytes.len() != 32 {
        emit_error(
            "invalid_input",
            &format!(
                "Digest must be exactly 32 bytes (64 hex chars), got {} bytes",
                bytes.len()
            ),
        );
    }

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    arr
}
