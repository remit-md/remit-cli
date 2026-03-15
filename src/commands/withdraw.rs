use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Generate a withdraw link to move funds out of your Remit wallet.
#[derive(Args)]
pub struct WithdrawArgs {
    /// Amount to withdraw in USDC (optional)
    #[arg(long)]
    pub amount: Option<String>,
    /// Destination address (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub to: Option<String>,
}

pub async fn run(_args: WithdrawArgs, _ctx: Context) -> Result<()> {
    todo!("withdraw command — implemented in task 0.6")
}
