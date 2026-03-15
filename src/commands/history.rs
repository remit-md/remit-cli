use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// List recent transactions for your wallet.
#[derive(Args)]
pub struct HistoryArgs {
    /// Maximum number of entries to show
    #[arg(long, default_value = "20")]
    pub limit: u32,
}

pub async fn run(_args: HistoryArgs, _ctx: Context) -> Result<()> {
    todo!("history command — implemented in task 0.4")
}
