use crate::commands::Context;
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum TabAction {
    /// Open a new tab with a spending limit
    Open(TabOpenArgs),
    /// Charge an amount against an open tab
    Charge(TabChargeArgs),
    /// Close a tab and settle
    Close(TabCloseArgs),
}

#[derive(Args)]
pub struct TabOpenArgs {
    /// Counterparty wallet address
    pub counterparty: String,
    /// Maximum spending limit in USDC
    pub limit: String,
}

#[derive(Args)]
pub struct TabChargeArgs {
    /// Tab ID
    pub tab_id: String,
    /// Amount to charge in USDC
    pub amount: String,
    /// Optional description
    #[arg(long)]
    pub memo: Option<String>,
}

#[derive(Args)]
pub struct TabCloseArgs {
    /// Tab ID
    pub tab_id: String,
}

pub async fn run(_action: TabAction, _ctx: Context) -> Result<()> {
    todo!("tab commands — implemented in task 0.5")
}
