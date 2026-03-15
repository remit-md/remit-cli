use crate::commands::Context;
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum BountyAction {
    /// Post a new bounty
    Post(BountyPostArgs),
    /// Submit a claim for a bounty
    Submit(BountySubmitArgs),
    /// Award bounty to a submission
    Award(BountyAwardArgs),
}

#[derive(Args)]
pub struct BountyPostArgs {
    /// Bounty amount in USDC
    pub amount: String,
    /// Human-readable description
    pub description: String,
    /// Expiry in seconds from now
    #[arg(long)]
    pub expiry: Option<u64>,
}

#[derive(Args)]
pub struct BountySubmitArgs {
    /// Bounty ID
    pub bounty_id: String,
    /// Submission proof URL or hash
    pub proof: String,
}

#[derive(Args)]
pub struct BountyAwardArgs {
    /// Bounty ID
    pub bounty_id: String,
    /// Submission ID to award
    pub submission_id: String,
}

pub async fn run(_action: BountyAction, _ctx: Context) -> Result<()> {
    todo!("bounty commands — implemented in task 0.5")
}
