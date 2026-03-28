use anyhow::Result;
use clap::Args;

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// Generate a fund link to top up your Remit wallet.
#[derive(Args)]
pub struct FundArgs {
    /// Pre-fill amount in USDC (optional)
    #[arg(long)]
    pub amount: Option<String>,

    /// Agent or person name shown on the fund page
    #[arg(long)]
    pub name: Option<String>,

    /// Message shown on the fund page explaining the request
    #[arg(long)]
    pub message: Option<String>,
}

pub async fn run(args: FundArgs, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet).await;
    let resp = client
        .link_fund(
            args.amount.as_deref(),
            args.name.as_deref(),
            args.message.as_deref(),
        )
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
