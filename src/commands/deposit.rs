use anyhow::Result;
use clap::{Args, Subcommand};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;
use crate::permit;

#[derive(Subcommand)]
pub enum DepositAction {
    /// Create a new deposit (refundable collateral)
    Create(DepositCreateArgs),
}

#[derive(Args)]
pub struct DepositCreateArgs {
    /// Provider wallet address
    pub provider: String,
    /// Amount in USDC
    pub amount: String,
    /// Expiry in seconds from now (default: 86400 = 24h)
    #[arg(long, default_value = "86400")]
    pub expiry: u64,
}

pub async fn run(action: DepositAction, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);

    match action {
        DepositAction::Create(args) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let expiry = now as i64 + args.expiry as i64;
            let amount_f64: f64 = args
                .amount
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid amount: {}", args.amount))?;
            let permit_sig = permit::auto_permit(&client, amount_f64, "deposit").await?;
            let deposit = client
                .deposit_create(&args.provider, &args.amount, expiry, Some(&permit_sig))
                .await?;
            if ctx.json {
                output::print_json(&deposit);
            } else {
                output::success(&format!("Deposit created: {}", deposit.id));
                output::print_kv(&[
                    ("ID", deposit.id.as_str()),
                    ("Status", deposit.status.as_str()),
                    ("Provider", deposit.provider.as_deref().unwrap_or("—")),
                    (
                        "Amount",
                        &deposit
                            .amount
                            .as_ref()
                            .map(|v| format!("{v} USDC"))
                            .unwrap_or_else(|| "—".to_string()),
                    ),
                    ("Expiry", deposit.expiry.as_deref().unwrap_or("—")),
                    ("Tx Hash", deposit.tx_hash.as_deref().unwrap_or("—")),
                ]);
            }
        }
    }
    Ok(())
}
