use anyhow::{anyhow, Result};
use clap::Args;

use crate::commands::Context;

/// Request testnet USDC from the Remit faucet (DEPRECATED — use `mint`).
#[derive(Args)]
pub struct FaucetArgs {
    /// Address to fund (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
    /// Amount to request in USDC
    #[arg(long)]
    pub amount: Option<String>,
}

pub async fn run(_args: FaucetArgs, _ctx: Context) -> Result<()> {
    Err(anyhow!(
        "The `faucet` command has been removed. Use `mint` instead:\n  \
         remit --testnet mint <amount>\n  \
         remit --testnet mint 100 --address 0x..."
    ))
}
