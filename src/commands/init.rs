use crate::commands::Context;
use anyhow::Result;
use clap::Args;

/// Generate a new keypair and configure REMITMD_KEY.
#[derive(Args)]
pub struct InitArgs {
    /// Write the key to .env in the current directory
    #[arg(long)]
    pub write_env: bool,
}

pub async fn run(_args: InitArgs, _ctx: Context) -> Result<()> {
    todo!("init command — implemented in task 0.8")
}
