use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Send a one-time USDC payment to an address.
#[derive(Args)]
pub struct PayArgs {
    /// Recipient wallet address (0x...)
    pub to: String,
    /// Amount in USDC (e.g., 10.50)
    pub amount: String,
    /// Optional memo attached to the payment
    #[arg(long)]
    pub memo: Option<String>,
}

pub async fn run(_args: PayArgs, _ctx: Context) -> Result<()> {
    todo!("pay command — implemented in task 0.4")
}
