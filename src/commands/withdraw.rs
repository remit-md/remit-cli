use anyhow::Result;
use clap::Args;

use crate::commands::Context;
use crate::output;

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

pub async fn run(args: WithdrawArgs, ctx: Context) -> Result<()> {
    super::require_init()?;
    let mut client = ctx.client()?;
    let resp = client
        .link_withdraw(args.amount.as_deref(), args.to.as_deref())
        .await?;

    if ctx.json {
        output::print_json(&resp);
    } else {
        output::info(&resp.url);
        if let Some(exp) = resp.expires_at {
            output::info(&format!("Expires: {exp}"));
        }
    }
    Ok(())
}
