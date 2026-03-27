use anyhow::{anyhow, Result};
use clap::Subcommand;
use std::io::IsTerminal;

use crate::output;
use crate::signer::keystore;

#[derive(Subcommand)]
pub enum SignerAction {
    /// Initialize a new local signer wallet
    Init(SignerInitArgs),
    /// Import an existing private key into the signer
    Import(SignerImportArgs),
    /// Migrate a V24 (token-based) keystore to V25 (password-based)
    Migrate(SignerMigrateArgs),
}

#[derive(clap::Args)]
pub struct SignerInitArgs {
    /// Wallet name (default: "default")
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(clap::Args)]
pub struct SignerImportArgs {
    /// Private key to import (hex, with or without 0x prefix)
    #[arg(long)]
    pub key: String,
    /// Wallet name (default: "default")
    #[arg(long)]
    pub name: Option<String>,
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
        SignerAction::Migrate(args) => run_migrate(args).await,
    }
}

// ── Password acquisition ─────────────────────────────────────────────────

/// Acquire a password for keystore encryption.
///
/// Priority:
///   1. REMIT_KEY_PASSWORD env var (non-interactive, used as-is)
///   2. Interactive prompt on stderr (twice for confirmation)
///   3. Error if non-interactive and no env var
///
/// SECURITY: Password never appears in CLI arguments (visible in ps aux).
fn acquire_password_with_confirmation() -> Result<String> {
    // 1. Check env var
    if let Ok(password) = std::env::var("REMIT_KEY_PASSWORD") {
        if password.is_empty() {
            return Err(anyhow!(
                "REMIT_KEY_PASSWORD is set but empty. Password must be non-empty."
            ));
        }
        return Ok(password);
    }

    // 2. Interactive prompt (must be terminal)
    if !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "No password source available.\n\
             Set REMIT_KEY_PASSWORD in your environment, or run interactively."
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
    let ks = keystore::Keystore::open()?;
    let wallet_name = args.name.unwrap_or_else(|| "default".to_string());

    if ks.exists(&wallet_name) {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name or delete ~/.remit/keys/{}.enc",
            wallet_name,
            wallet_name
        ));
    }

    let password = acquire_password_with_confirmation()?;
    let address = ks.generate(&wallet_name, &password)?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "keystore": keystore::Keystore::open()?.key_path(&wallet_name).display().to_string(),
        }));
    } else {
        eprintln!();
        output::print_kv(&[
            ("Wallet", &wallet_name),
            ("Address", &address),
            (
                "Keystore",
                &keystore::Keystore::open()?
                    .key_path(&wallet_name)
                    .display()
                    .to_string(),
            ),
        ]);
        eprintln!();
        eprintln!("Set your password for non-interactive signing:");
        eprintln!("{}", crate::platform::password_hint());
    }

    Ok(())
}

// ── Import ─────────────────────────────────────────────────────────────────

async fn run_import(args: SignerImportArgs, ctx: crate::commands::Context) -> Result<()> {
    let ks = keystore::Keystore::open()?;
    let wallet_name = args.name.unwrap_or_else(|| "default".to_string());

    if ks.exists(&wallet_name) {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name.",
            wallet_name
        ));
    }

    let password = acquire_password_with_confirmation()?;
    let address = ks.import(&wallet_name, &args.key, &password)?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "keystore": keystore::Keystore::open()?.key_path(&wallet_name).display().to_string(),
        }));
    } else {
        eprintln!();
        output::print_kv(&[
            ("Wallet", &wallet_name),
            ("Address", &address),
            (
                "Keystore",
                &keystore::Keystore::open()?
                    .key_path(&wallet_name)
                    .display()
                    .to_string(),
            ),
        ]);
        eprintln!();
        eprintln!("Set your password for non-interactive signing:");
        eprintln!("{}", crate::platform::password_hint());
    }

    Ok(())
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
