use crate::commands::Context;
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum EscrowAction {
    /// Create a new escrow
    Create(EscrowCreateArgs),
    /// Release escrowed funds to recipient
    Release(EscrowIdArgs),
    /// Cancel escrow and return funds
    Cancel(EscrowIdArgs),
    /// Start a claim process (dispute)
    #[command(name = "claim-start")]
    ClaimStart(EscrowIdArgs),
}

#[derive(Args)]
pub struct EscrowCreateArgs {
    /// Recipient wallet address
    pub recipient: String,
    /// Amount in USDC
    pub amount: String,
    /// Optional arbitrator address
    #[arg(long)]
    pub arbitrator: Option<String>,
}

#[derive(Args)]
pub struct EscrowIdArgs {
    /// Escrow ID
    pub escrow_id: String,
}

pub async fn run(_action: EscrowAction, _ctx: Context) -> Result<()> {
    todo!("escrow commands — implemented in task 0.5")
}
