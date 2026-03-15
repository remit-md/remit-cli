use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Request testnet USDC from the Remit faucet (testnet only).
#[derive(Args)]
pub struct FaucetArgs {
    /// Address to fund (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
}

pub async fn run(_args: FaucetArgs, _ctx: Context) -> Result<()> {
    todo!("faucet command — implemented in task 0.6")
}
