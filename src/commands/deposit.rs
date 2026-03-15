use crate::commands::Context;
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum DepositAction {
    /// Create a deposit address / invoice
    Create(DepositCreateArgs),
}

#[derive(Args)]
pub struct DepositCreateArgs {
    /// Expected amount in USDC (optional — open-ended if omitted)
    #[arg(long)]
    pub amount: Option<String>,
    /// Optional memo
    #[arg(long)]
    pub memo: Option<String>,
}

pub async fn run(_action: DepositAction, _ctx: Context) -> Result<()> {
    todo!("deposit commands — implemented in task 0.5")
}
