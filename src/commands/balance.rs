use anyhow::Result;
use clap::Args;

use crate::auth::wallet_address;
use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// Show USDC balance for your wallet.
#[derive(Args)]
pub struct BalanceArgs {
    /// Address to check (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
}

pub async fn run(args: BalanceArgs, ctx: Context) -> Result<()> {
    let addr = match args.address {
        Some(a) => a,
        None => wallet_address()?,
    };

    let client = RemitClient::new(ctx.testnet);
    let resp = client.status(&addr).await?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "address": resp.wallet,
            "usdc": resp.balance,
            "network": if ctx.testnet { "base-sepolia" } else { "base" },
        }));
    } else {
        println!("{} USDC", resp.balance);
    }
    Ok(())
}
