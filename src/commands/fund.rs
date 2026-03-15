use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Generate a fund link to top up your Remit wallet.
#[derive(Args)]
pub struct FundArgs {
    /// Pre-fill amount in USDC (optional)
    #[arg(long)]
    pub amount: Option<String>,
}

pub async fn run(_args: FundArgs, _ctx: Context) -> Result<()> {
    todo!("fund command — implemented in task 0.6")
}
