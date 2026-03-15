use anyhow::{anyhow, Result};
use clap::Args;

use crate::auth::wallet_address;
use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// Request testnet USDC from the Remit faucet (testnet only).
#[derive(Args)]
pub struct FaucetArgs {
    /// Address to fund (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
    /// Amount to request in USDC (max 1000, default 100)
    #[arg(long)]
    pub amount: Option<String>,
}

pub async fn run(args: FaucetArgs, ctx: Context) -> Result<()> {
    if !ctx.testnet {
        return Err(anyhow!(
            "Faucet is only available on testnet.\nUse: remit --testnet faucet"
        ));
    }

    let addr = match args.address {
        Some(a) => a,
        None => wallet_address()?,
    };

    let client = RemitClient::new(ctx.testnet);
    let resp = client.faucet(&addr, args.amount.as_deref()).await?;

    if ctx.json {
        output::print_json(&resp);
    } else {
        output::success(&format!("Faucet dripped {} USDC to {}", resp.amount, addr));
        if let Some(tx) = resp.tx_hash {
            output::print_kv(&[("Tx Hash", tx.as_str())]);
        }
    }
    Ok(())
}
