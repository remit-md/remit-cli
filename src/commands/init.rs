use anyhow::{Context as _, Result};
use clap::Args;

use crate::commands;
use crate::output;
use crate::ows;

/// Initialize a Remit agent wallet.
///
/// Default: creates a local signer wallet (encrypted key + bearer token).
/// Use --ows for OWS wallet, --legacy for raw keypair.
#[derive(Args)]
pub struct InitArgs {
    /// Wallet name (default: remit-{hostname})
    #[arg(long)]
    pub name: Option<String>,

    /// Chain: "base" (mainnet) or "base-sepolia" (testnet)
    #[arg(long)]
    pub chain: Option<String>,

    /// Write REMITMD_KEY to .env (legacy mode only)
    #[arg(long)]
    pub write_env: bool,

    /// Use OWS (Open Wallet Standard) instead of local signer
    #[arg(long, conflicts_with = "legacy")]
    pub ows: bool,

    /// Use legacy raw keypair instead of local signer
    #[arg(long, conflicts_with = "ows")]
    pub legacy: bool,
}

pub async fn run(args: InitArgs, ctx: commands::Context) -> Result<()> {
    if args.legacy {
        return run_legacy(args, ctx).await;
    }
    if args.ows {
        return run_ows(args, ctx).await;
    }

    // Default: local signer
    run_signer(args, ctx).await
}

/// Default init: local signer (V24).
async fn run_signer(args: InitArgs, ctx: commands::Context) -> Result<()> {
    // Delegate to `remit signer init` with the same args
    let signer_args = crate::commands::signer::SignerInitArgs {
        name: args.name,
        chain: args.chain,
    };
    crate::commands::signer::run(
        crate::commands::signer::SignerAction::Init(signer_args),
        ctx,
    )
    .await
}

async fn run_ows(args: InitArgs, ctx: commands::Context) -> Result<()> {
    let chain = args.chain.unwrap_or_else(ows::detect_chain);
    let wallet_name = args.name.unwrap_or_else(ows::default_wallet_name);

    // Validate chain
    ows::chain_to_caip2(&chain)?;

    // Step 1: Check if OWS is installed
    if !ows::is_ows_available() {
        output::info("OWS not detected. Installing via npm...");
        ows::install_ows_via_npm().context("failed to install OWS")?;

        // Verify it worked
        if !ows::is_ows_available() {
            return Err(anyhow::anyhow!(
                "OWS installation failed. Install manually: npm install -g @open-wallet-standard/core\n\
                 Or use `remit init` (without --ows) for the local signer."
            ));
        }
        output::success("OWS installed");
    }

    // Step 2: Create wallet (no passphrase — API key auth only)
    output::info(&format!("Creating wallet '{wallet_name}'..."));
    let wallet = ows::create_wallet(&wallet_name)?;
    let address = ows::wallet_evm_address(&wallet)
        .ok_or_else(|| anyhow::anyhow!("wallet has no EVM account"))?;
    output::success(&format!("Wallet created: {address}"));

    // Step 3: Create chain-lock policy
    let policy = ows::create_chain_policy(&chain)?;
    output::success(&format!(
        "Policy created: {} (chain lock: {})",
        policy.id, chain
    ));

    // Step 4: Create API key bound to wallet + policy
    let (token, _key_file) = ows::create_api_key(&wallet.id, &policy.id)?;
    output::success("API key created");

    // Step 5: Output
    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet_id": wallet.id,
            "wallet_name": wallet.name,
            "address": address,
            "chain": chain,
            "policy_id": policy.id,
            "api_key": token,
            "mcp_config": serde_json::from_str::<serde_json::Value>(
                &ows::mcp_config_json(&wallet_name, &chain)
            ).unwrap_or_default(),
        }));
    } else {
        println!();
        output::print_kv(&[
            ("Wallet", &wallet_name),
            ("Address", &address),
            ("Chain", &chain),
            ("Policy", &policy.id),
            ("Vault", &ows::vault_path_display()),
        ]);
        println!();
        eprintln!("API Key (save this — shown once):");
        eprintln!("  {token}");
        println!();
        eprintln!("MCP config (add to your claude_desktop_config.json):");
        eprintln!("{}", ows::mcp_config_json(&wallet_name, &chain));
        println!();
        eprintln!("Set OWS_API_KEY={token} in your environment.");
        eprintln!("Then your agent can use Remit via MCP with OWS-secured signing.");
    }

    Ok(())
}

/// Legacy init: generate a raw keypair (no OWS, no signer).
async fn run_legacy(args: InitArgs, ctx: commands::Context) -> Result<()> {
    use alloy::signers::local::PrivateKeySigner;
    use rand::rngs::OsRng;

    let signer = PrivateKeySigner::random_with(&mut OsRng);
    let address = format!("{:#x}", signer.address());
    let key_bytes = signer.credential().to_bytes();
    let private_key = format!("0x{}", hex::encode(key_bytes));

    if args.write_env {
        let env_path = std::path::Path::new(".env");
        let line = format!("REMITMD_KEY={private_key}\n");

        if env_path.exists() {
            let contents = std::fs::read_to_string(env_path).context("reading .env")?;

            let has_key = contents
                .lines()
                .any(|l| l.starts_with("REMITMD_KEY=") || l.starts_with("REMITMD_KEY ="));

            if has_key {
                let new_contents: String = contents
                    .lines()
                    .map(|l| {
                        if l.starts_with("REMITMD_KEY=") || l.starts_with("REMITMD_KEY =") {
                            format!("REMITMD_KEY={private_key}")
                        } else {
                            l.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n";
                std::fs::write(env_path, new_contents).context("writing .env")?;
                output::success("Replaced REMITMD_KEY in .env");
            } else {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(env_path)
                    .context("opening .env")?;
                file.write_all(line.as_bytes()).context("writing to .env")?;
                output::success("Appended REMITMD_KEY to .env");
            }
        } else {
            std::fs::write(env_path, &line).context("writing .env")?;
            output::success("Created .env with REMITMD_KEY");
        }
    }

    if ctx.json {
        output::print_json(&serde_json::json!({
            "address": address,
            "private_key": private_key,
        }));
    } else {
        output::print_kv(&[
            ("Address", address.as_str()),
            ("Private key", private_key.as_str()),
        ]);
        println!();
        println!("Back up your private key. It cannot be recovered if lost.");
        println!("   Set it in your environment:");
        println!("   export REMITMD_KEY={private_key}");
        if !args.write_env {
            println!();
            println!("   Or run with --write-env to write it to .env automatically.");
        }
        println!();
        println!("   For better security, use `remit init` (default) for the local signer.");
    }

    Ok(())
}
