use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Show USDC balance for your wallet.
#[derive(Args)]
pub struct BalanceArgs {
    /// Address to check (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
}

pub async fn run(_args: BalanceArgs, _ctx: Context) -> Result<()> {
    todo!("balance command — implemented in task 0.4")
}
