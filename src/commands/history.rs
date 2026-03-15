use anyhow::Result;
use clap::Args;

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// List recent events and transactions for your wallet.
#[derive(Args)]
pub struct HistoryArgs {
    /// Maximum number of entries to show
    #[arg(long, default_value = "20")]
    pub limit: u32,
    /// Show events after this Unix timestamp
    #[arg(long)]
    pub since: Option<i64>,
}

pub async fn run(args: HistoryArgs, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);
    let events = client.events(args.since, Some(args.limit)).await?;

    if ctx.json {
        output::print_json(&events);
    } else if events.is_empty() {
        output::info("No recent events.");
    } else {
        output::print_table(
            vec!["Event", "ID", "Chain", "When"],
            events
                .iter()
                .map(|e| {
                    vec![
                        e.event_type.clone(),
                        e.invoice_id.clone().unwrap_or_else(|| "—".to_string()),
                        e.chain.clone().unwrap_or_else(|| "—".to_string()),
                        e.created_at.clone().unwrap_or_else(|| "—".to_string()),
                    ]
                })
                .collect(),
        );
    }
    Ok(())
}
