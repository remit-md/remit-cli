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

pub async fn run(action: SignerAction, ctx: crate::commands::Context) -> Result<()> {
    match action {
        SignerAction::Init(args) => run_init(args, ctx).await,
        SignerAction::Import(args) => run_import(args, ctx).await,
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
        eprintln!("  export REMIT_KEY_PASSWORD=<your-password>");
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
        eprintln!("  export REMIT_KEY_PASSWORD=<your-password>");
    }

    Ok(())
}
