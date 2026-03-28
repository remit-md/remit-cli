use anyhow::{anyhow, Result};
use clap::Subcommand;
use std::io::IsTerminal;

use crate::output;
use crate::signer::keyring;
use crate::signer::keystore;

#[derive(Subcommand)]
pub enum SignerAction {
    /// Initialize a new local signer wallet
    Init(SignerInitArgs),
    /// Import an existing private key into the signer
    Import(SignerImportArgs),
    /// Export the private key (for backup or migration)
    Export(SignerExportArgs),
    /// Migrate a V24 (token-based) keystore to V25 (password-based)
    Migrate(SignerMigrateArgs),
}

#[derive(clap::Args)]
pub struct SignerExportArgs {
    /// Wallet name (default: "default")
    #[arg(long)]
    pub name: Option<String>,
    /// Path to keystore file (overrides name)
    #[arg(long)]
    pub keystore: Option<String>,
}

#[derive(clap::Args)]
pub struct SignerInitArgs {
    /// Wallet name (default: "default")
    #[arg(long)]
    pub name: Option<String>,
    /// Force password-encrypted .enc file instead of OS keychain
    #[arg(long)]
    pub no_keychain: bool,
}

#[derive(clap::Args)]
pub struct SignerImportArgs {
    /// Private key to import (hex, with or without 0x prefix)
    #[arg(long)]
    pub key: String,
    /// Wallet name (default: "default")
    #[arg(long)]
    pub name: Option<String>,
    /// Force password-encrypted .enc file instead of OS keychain
    #[arg(long)]
    pub no_keychain: bool,
}

#[derive(clap::Args)]
pub struct SignerMigrateArgs {
    /// Path to keystore file (default: ~/.remit/keys/default.enc)
    #[arg(long)]
    pub keystore: Option<String>,
}

pub async fn run(action: SignerAction, ctx: crate::commands::Context) -> Result<()> {
    match action {
        SignerAction::Init(args) => run_init(args, ctx).await,
        SignerAction::Import(args) => run_import(args, ctx).await,
        SignerAction::Export(args) => run_export(args).await,
        SignerAction::Migrate(args) => run_migrate(args).await,
    }
}

// ── Wallet existence check ───────────────────────────────────────────────

/// Check if a wallet already exists (either .enc or .meta file).
fn wallet_exists(name: &str) -> Result<bool> {
    let ks = keystore::Keystore::open()?;
    if ks.exists(name) {
        return Ok(true);
    }
    keyring::MetaFile::exists(name)
}

// ── Password acquisition ─────────────────────────────────────────────────

/// Acquire a password for keystore encryption.
///
/// Priority:
///   1. REMIT_SIGNER_KEY env var (non-interactive, used as-is)
///   2. REMIT_KEY_PASSWORD env var (deprecated fallback)
///   3. Interactive prompt on stderr (twice for confirmation)
///   4. Error if non-interactive and no env var
///
/// SECURITY: Password never appears in CLI arguments (visible in ps aux).
fn acquire_password_with_confirmation() -> Result<String> {
    // 1. New env var
    if let Ok(password) = std::env::var("REMIT_SIGNER_KEY") {
        if password.is_empty() {
            return Err(anyhow!(
                "REMIT_SIGNER_KEY is set but empty. Password must be non-empty."
            ));
        }
        return Ok(password);
    }

    // 2. Deprecated env var (backwards compat — remove in V28)
    if let Ok(password) = std::env::var("REMIT_KEY_PASSWORD") {
        if password.is_empty() {
            return Err(anyhow!(
                "REMIT_KEY_PASSWORD is set but empty. Password must be non-empty."
            ));
        }
        eprintln!(
            "\u{26a0} REMIT_KEY_PASSWORD is deprecated and will be removed in a future release.\n  \
             Set REMIT_SIGNER_KEY instead."
        );
        return Ok(password);
    }

    // 3. Interactive prompt (must be terminal)
    if !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "No password source available.\n\
             Set REMIT_SIGNER_KEY in your environment, or run interactively."
        ));
    }

    loop {
        let password = rpassword::prompt_password("Choose a password: ")
            .map_err(|e| anyhow!("failed to read password: {e}"))?;

        if password.is_empty() {
            eprintln!("Password must not be empty. Try again.");
            continue;
        }

        let confirm = rpassword::prompt_password("Confirm password: ")
            .map_err(|e| anyhow!("failed to read password confirmation: {e}"))?;

        if password != confirm {
            eprintln!("Passwords do not match. Try again.");
            continue;
        }

        return Ok(password);
    }
}

// ── Init ───────────────────────────────────────────────────────────────────

async fn run_init(args: SignerInitArgs, ctx: crate::commands::Context) -> Result<()> {
    let wallet_name = args.name.unwrap_or_else(|| "default".to_string());

    if wallet_exists(&wallet_name)? {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name.",
            wallet_name,
        ));
    }

    let use_keychain = !args.no_keychain && keyring::is_available();

    if use_keychain {
        run_init_keychain(&wallet_name, &ctx)
    } else {
        run_init_enc(&wallet_name, &ctx)
    }
}

/// Init with OS keychain: generate key, store in keychain, write .meta file.
fn run_init_keychain(wallet_name: &str, ctx: &crate::commands::Context) -> Result<()> {
    use alloy::signers::local::PrivateKeySigner;
    use rand::rngs::OsRng;
    use zeroize::Zeroizing;

    // Generate keypair
    let signer = PrivateKeySigner::random_with(&mut OsRng);
    let address = format!("{:#x}", signer.address());

    // Extract raw key bytes
    let field_bytes = signer.credential().to_bytes();
    let mut raw_key = Zeroizing::new([0u8; 32]);
    raw_key.copy_from_slice(field_bytes.as_slice());

    // Store in OS keychain
    keyring::store_key(wallet_name, &raw_key)?;

    // Write .meta file (public info only)
    let meta = keyring::MetaFile {
        version: 2,
        name: wallet_name.to_string(),
        address: address.clone(),
        storage: "keychain".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    meta.write_to_disk()?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "storage": "keychain",
        }));
    } else {
        eprintln!();
        output::print_kv(&[
            ("Wallet", wallet_name),
            ("Address", &address),
            ("Key stored in", "OS keychain (encrypted by your login)"),
        ]);
        eprintln!();
        eprintln!("No password or env vars needed. Just use `remit pay`, `remit sign`, etc.");
    }

    Ok(())
}

/// Init with encrypted .enc file: generate key, prompt password, write .enc.
fn run_init_enc(wallet_name: &str, ctx: &crate::commands::Context) -> Result<()> {
    let ks = keystore::Keystore::open()?;
    let password = acquire_password_with_confirmation()?;
    let address = ks.generate(wallet_name, &password)?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "keystore": ks.key_path(wallet_name).display().to_string(),
        }));
    } else {
        eprintln!();
        output::print_kv(&[
            ("Wallet", wallet_name),
            ("Address", &address),
            ("Keystore", &ks.key_path(wallet_name).display().to_string()),
        ]);
        eprintln!();
        eprintln!("Set your password for non-interactive signing:");
        eprintln!("{}", crate::platform::password_hint());
    }

    Ok(())
}

// ── Import ─────────────────────────────────────────────────────────────────

async fn run_import(args: SignerImportArgs, ctx: crate::commands::Context) -> Result<()> {
    let wallet_name = args.name.unwrap_or_else(|| "default".to_string());

    if wallet_exists(&wallet_name)? {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name.",
            wallet_name
        ));
    }

    let use_keychain = !args.no_keychain && keyring::is_available();

    if use_keychain {
        run_import_keychain(&wallet_name, &args.key, &ctx)
    } else {
        run_import_enc(&wallet_name, &args.key, &ctx)
    }
}

/// Import with OS keychain: parse key, store in keychain, write .meta file.
fn run_import_keychain(
    wallet_name: &str,
    private_key_hex: &str,
    ctx: &crate::commands::Context,
) -> Result<()> {
    use alloy::signers::local::PrivateKeySigner;
    use zeroize::Zeroizing;

    // Parse and validate key
    let hex_clean = private_key_hex.trim_start_matches("0x");
    let raw_bytes = hex::decode(hex_clean).map_err(|_| anyhow!("private key is not valid hex"))?;
    if raw_bytes.len() != 32 {
        return Err(anyhow!(
            "private key must be exactly 32 bytes (64 hex chars)"
        ));
    }

    let mut raw_key = Zeroizing::new([0u8; 32]);
    raw_key.copy_from_slice(&raw_bytes);

    // Derive address
    let signer = PrivateKeySigner::from_bytes(&(*raw_key).into())
        .map_err(|_| anyhow!("not a valid secp256k1 private key"))?;
    let address = format!("{:#x}", signer.address());

    // Store in OS keychain
    keyring::store_key(wallet_name, &raw_key)?;

    // Write .meta file
    let meta = keyring::MetaFile {
        version: 2,
        name: wallet_name.to_string(),
        address: address.clone(),
        storage: "keychain".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    meta.write_to_disk()?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "storage": "keychain",
        }));
    } else {
        eprintln!();
        output::print_kv(&[
            ("Wallet", wallet_name),
            ("Address", &address),
            ("Key stored in", "OS keychain (encrypted by your login)"),
        ]);
        eprintln!();
        eprintln!("No password or env vars needed. Just use `remit pay`, `remit sign`, etc.");
    }

    Ok(())
}

/// Import with encrypted .enc file: parse key, prompt password, write .enc.
fn run_import_enc(
    wallet_name: &str,
    private_key_hex: &str,
    ctx: &crate::commands::Context,
) -> Result<()> {
    let ks = keystore::Keystore::open()?;
    let password = acquire_password_with_confirmation()?;
    let address = ks.import(wallet_name, private_key_hex, &password)?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "keystore": ks.key_path(wallet_name).display().to_string(),
        }));
    } else {
        eprintln!();
        output::print_kv(&[
            ("Wallet", wallet_name),
            ("Address", &address),
            ("Keystore", &ks.key_path(wallet_name).display().to_string()),
        ]);
        eprintln!();
        eprintln!("Set your password for non-interactive signing:");
        eprintln!("{}", crate::platform::password_hint());
    }

    Ok(())
}

// ── Export ─────────────────────────────────────────────────────────────────
//
// SECURITY: Displays the raw private key. Requires interactive confirmation.

async fn run_export(args: SignerExportArgs) -> Result<()> {
    let wallet_name = args.name.unwrap_or_else(|| "default".to_string());

    // Safety: require interactive confirmation
    if !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "remit signer export requires an interactive terminal for confirmation."
        ));
    }

    eprintln!("WARNING: This will display your private key in plaintext.");
    eprintln!("Anyone who sees it can steal your funds.");
    eprint!("Continue? [y/N] ");

    let mut confirm = String::new();
    std::io::stdin()
        .read_line(&mut confirm)
        .map_err(|e| anyhow!("failed to read confirmation: {e}"))?;
    if confirm.trim().to_lowercase() != "y" {
        eprintln!("Aborted.");
        return Ok(());
    }

    // Resolve private key — keychain or .enc
    if let Some(ref keystore_path) = args.keystore {
        // Explicit .enc file
        let key_file = keystore::load_file(std::path::Path::new(keystore_path))?;
        let password = acquire_password_for_decrypt()?;
        let signer = keystore::decrypt(&key_file, &password)?;
        let key_bytes = signer.credential().to_bytes();
        println!("0x{}", hex::encode(key_bytes));
        return Ok(());
    }

    // Check .meta (keychain) first
    if keyring::MetaFile::exists(&wallet_name)? {
        let meta = keyring::MetaFile::load(&wallet_name)?;
        if meta.storage == "keychain" {
            let raw_key = keyring::load_key(&wallet_name)?;
            println!("0x{}", hex::encode(*raw_key));
            return Ok(());
        }
    }

    // Fall back to .enc
    let ks = keystore::Keystore::open()?;
    if ks.exists(&wallet_name) {
        let key_file = ks.load(&wallet_name)?;
        let password = acquire_password_for_decrypt()?;
        let signer = keystore::decrypt(&key_file, &password)?;
        let key_bytes = signer.credential().to_bytes();
        println!("0x{}", hex::encode(key_bytes));
        return Ok(());
    }

    Err(anyhow!(
        "No wallet '{}' found. Run: remit signer init",
        wallet_name
    ))
}

/// Acquire a password for decryption (no confirmation needed).
fn acquire_password_for_decrypt() -> Result<String> {
    // New env var
    if let Ok(password) = std::env::var("REMIT_SIGNER_KEY") {
        if !password.is_empty() {
            return Ok(password);
        }
    }

    // Deprecated env var (backwards compat — remove in V28)
    if let Ok(password) = std::env::var("REMIT_KEY_PASSWORD") {
        if !password.is_empty() {
            eprintln!(
                "\u{26a0} REMIT_KEY_PASSWORD is deprecated and will be removed in a future release.\n  \
                 Set REMIT_SIGNER_KEY instead."
            );
            return Ok(password);
        }
    }

    if !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "No password source. Set REMIT_SIGNER_KEY or run interactively."
        ));
    }

    rpassword::prompt_password("Enter password: ")
        .map_err(|e| anyhow!("failed to read password: {e}"))
}

// ── Migrate ────────────────────────────────────────────────────────────────
//
// SECURITY INVARIANTS:
//   I10: Write .enc.bak backup BEFORE overwriting original.
//   I11: Password entered twice for confirmation.
//   I2:  Key in Zeroizing<> only, dropped after re-encryption.

async fn run_migrate(args: SignerMigrateArgs) -> Result<()> {
    let keystore_path = match &args.keystore {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let ks = keystore::Keystore::open()?;
            ks.key_path("default")
        }
    };

    if !keystore_path.exists() {
        return Err(anyhow!(
            "No keystore found at {}. Nothing to migrate.",
            keystore_path.display()
        ));
    }

    let key_file = keystore::load_file(&keystore_path)?;

    if key_file.version != 1 {
        return Err(anyhow!(
            "Keystore is already version {} — no migration needed.",
            key_file.version
        ));
    }

    eprintln!("Migrating keystore from V24 (token-based) to V25 (password-based).");
    eprintln!("Address: {}", key_file.address);
    eprintln!();

    // Prompt for old passphrase (the V24 bearer token)
    let old_passphrase = if let Ok(pw) = std::env::var("REMIT_SIGNER_TOKEN") {
        if pw.is_empty() {
            rpassword::prompt_password("Enter the V24 bearer token used to encrypt this keystore: ")
                .map_err(|e| anyhow!("failed to read old passphrase: {e}"))?
        } else {
            pw
        }
    } else {
        rpassword::prompt_password("Enter the V24 bearer token used to encrypt this keystore: ")
            .map_err(|e| anyhow!("failed to read old passphrase: {e}"))?
    };

    // Decrypt with old passphrase
    let signer = keystore::decrypt(&key_file, &old_passphrase)
        .map_err(|_| anyhow!("Decryption failed — wrong bearer token?"))?;

    // Get new password
    eprintln!();
    let new_password = acquire_password_with_confirmation()?;

    // Extract raw key bytes for re-encryption
    let field_bytes = signer.credential().to_bytes();
    let raw_key = zeroize::Zeroizing::new(field_bytes.as_slice().to_vec());

    // I10: Write backup BEFORE overwriting
    let backup_path = keystore_path.with_extension("enc.bak");
    std::fs::copy(&keystore_path, &backup_path)
        .map_err(|e| anyhow!("Failed to create backup at {}: {e}", backup_path.display()))?;

    // Re-encrypt with new password and write as version 2
    let encryption = keystore::encrypt_key(&raw_key, &new_password)?;
    let new_key_file = keystore::EncryptedKeyFile {
        version: 2,
        name: key_file.name.clone(),
        address: key_file.address.clone(),
        created_at: key_file.created_at.clone(),
        encryption,
    };

    let json =
        serde_json::to_string_pretty(&new_key_file).map_err(|e| anyhow!("serialize: {e}"))?;
    std::fs::write(&keystore_path, json)
        .map_err(|e| anyhow!("Failed to write migrated keystore: {e}"))?;

    eprintln!();
    eprintln!("Migration complete!");
    eprintln!("  Backup: {}", backup_path.display());
    eprintln!("  Keystore: {}", keystore_path.display());
    eprintln!();
    eprintln!("Set your password for non-interactive signing:");
    eprintln!("{}", crate::platform::password_hint());

    Ok(())
}
