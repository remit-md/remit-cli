use anyhow::{anyhow, Result};
use clap::Args;

use crate::auth::wallet_address;
use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// Mint testnet USDC to your wallet (testnet only, max 2500 USDC per request).
#[derive(Args)]
pub struct MintArgs {
    /// Amount of USDC to mint (max 2500)
    pub amount: String,
    /// Address to mint to (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
}

pub async fn run(args: MintArgs, ctx: Context) -> Result<()> {
    if !ctx.testnet {
        return Err(anyhow!(
            "Mint is only available on testnet.\nUse: remit --testnet mint <amount>"
        ));
    }

    let addr = match args.address {
        Some(a) => a,
        None => wallet_address()?,
    };

    let amount: f64 = args
        .amount
        .parse()
        .map_err(|_| anyhow!("Invalid amount: {}", args.amount))?;

    if amount <= 0.0 {
        return Err(anyhow!("Amount must be positive"));
    }
    if amount > 2500.0 {
        return Err(anyhow!("Amount must not exceed 2500 USDC per request"));
    }

    let client = RemitClient::new(ctx.testnet);
    let resp = client.mint(&addr, amount).await?;

    if ctx.json {
        output::print_json(&resp);
    } else {
        output::success(&format!("Minted {} USDC to {}", args.amount, addr));
        output::print_kv(&[
            ("Tx Hash", resp.tx_hash.as_str()),
            ("Balance", &resp.balance.to_string()),
        ]);
    }
    Ok(())
}
