use crate::commands::Context;
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum StreamAction {
    /// Open a payment stream
    Open(StreamOpenArgs),
    /// Close a payment stream
    Close(StreamCloseArgs),
}

#[derive(Args)]
pub struct StreamOpenArgs {
    /// Recipient wallet address
    pub recipient: String,
    /// Total USDC to stream
    pub amount: String,
    /// Duration in seconds
    pub duration: u64,
}

#[derive(Args)]
pub struct StreamCloseArgs {
    /// Stream ID
    pub stream_id: String,
}

pub async fn run(_action: StreamAction, _ctx: Context) -> Result<()> {
    todo!("stream commands — implemented in task 0.5")
}
