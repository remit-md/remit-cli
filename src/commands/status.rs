use anyhow::Result;
use clap::Args;

use crate::auth::wallet_address;
use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// Show wallet status (address, balance, fee tier, active positions).
#[derive(Args)]
pub struct StatusArgs {
    /// Wallet address to query (defaults to your REMITMD_KEY wallet)
    #[arg(long)]
    pub address: Option<String>,
}

pub async fn run(args: StatusArgs, ctx: Context) -> Result<()> {
    let addr = match args.address {
        Some(a) => a,
        None => wallet_address().await?,
    };

    let client = RemitClient::new(ctx.testnet).await;
    let resp = client.status(&addr).await?;

    if ctx.json {
        output::print_json(&resp);
    } else {
        let fee = resp
            .fee_rate_bps
            .map(|bps| format!("{}bps ({}%)", bps, bps as f64 / 100.0))
            .unwrap_or_else(|| "—".to_string());

        output::print_kv(&[
            ("Wallet", resp.wallet.as_str()),
            ("Balance", &format!("{} USDC", resp.balance)),
            ("Fee rate", &fee),
            ("Tier", resp.tier.as_deref().unwrap_or("standard")),
            (
                "Active escrows",
                &resp.active_escrows.unwrap_or(0).to_string(),
            ),
            ("Active tabs", &resp.active_tabs.unwrap_or(0).to_string()),
            (
                "Active streams",
                &resp.active_streams.unwrap_or(0).to_string(),
            ),
        ]);
    }
    Ok(())
}
