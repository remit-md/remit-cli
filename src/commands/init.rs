use anyhow::{Context as _, Result};
use clap::Args;

use crate::commands::Context;
use crate::output;

/// Generate a new keypair and configure REMITMD_KEY.
#[derive(Args)]
pub struct InitArgs {
    /// Write the key to .env in the current directory (default: print to stdout only)
    #[arg(long)]
    pub write_env: bool,
}

pub async fn run(args: InitArgs, ctx: Context) -> Result<()> {
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
            // Append (don't overwrite existing .env)
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(env_path)
                .context("opening .env")?;
            file.write_all(line.as_bytes()).context("writing to .env")?;
            output::success("Appended REMITMD_KEY to .env");
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
        println!("⚠  Back up your private key. It cannot be recovered if lost.");
        println!("   Set it in your environment:");
        println!("   export REMITMD_KEY={private_key}");
        if !args.write_env {
            println!();
            println!("   Or run with --write-env to write it to .env automatically.");
        }
    }

    Ok(())
}
