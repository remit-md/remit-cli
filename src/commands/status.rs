use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Show wallet status (address, balance, network).
#[derive(Args)]
pub struct StatusArgs {}

pub async fn run(_args: StatusArgs, _ctx: Context) -> Result<()> {
    todo!("status command — implemented in task 0.4")
}
