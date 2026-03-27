use anyhow::{anyhow, Result};
use clap::Subcommand;

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

// ── Init ───────────────────────────────────────────────────────────────────
// TODO(V25 C0.3): Rewrite with password-based encryption (not token-based).

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

    // Temporary: generate with a placeholder passphrase.
    // C0.3 will replace this with password-based encryption.
    let address = ks.generate(&wallet_name, "temporary-passphrase")?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
        }));
    } else {
        output::print_kv(&[("Wallet", &wallet_name), ("Address", &address)]);
        eprintln!();
        eprintln!("NOTE: This wallet uses a temporary passphrase.");
        eprintln!("V25 password-based init is not yet implemented.");
    }

    Ok(())
}

// ── Import ─────────────────────────────────────────────────────────────────
// TODO(V25 C0.4): Rewrite with password-based encryption.

async fn run_import(args: SignerImportArgs, ctx: crate::commands::Context) -> Result<()> {
    let ks = keystore::Keystore::open()?;
    let wallet_name = args.name.unwrap_or_else(|| "default".to_string());

    if ks.exists(&wallet_name) {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name.",
            wallet_name
        ));
    }

    // Temporary: import with placeholder passphrase.
    // C0.4 will replace this with password-based encryption.
    let address = ks.import(&wallet_name, &args.key, "temporary-passphrase")?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
        }));
    } else {
        output::print_kv(&[("Wallet", &wallet_name), ("Address", &address)]);
        eprintln!();
        eprintln!("NOTE: This wallet uses a temporary passphrase.");
        eprintln!("V25 password-based import is not yet implemented.");
    }

    Ok(())
}
