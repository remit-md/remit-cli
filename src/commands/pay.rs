use anyhow::Result;
use clap::Args;

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

/// Send a one-time USDC payment to an address.
#[derive(Args)]
pub struct PayArgs {
    /// Recipient wallet address (0x...)
    pub to: String,
    /// Amount in USDC (e.g., 10.50)
    pub amount: String,
    /// Optional memo attached to the payment
    #[arg(long)]
    pub memo: Option<String>,
}

pub async fn run(args: PayArgs, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);
    let resp = client
        .pay_direct(&args.to, &args.amount, args.memo.as_deref())
        .await?;

    if ctx.json {
        output::print_json(&resp);
    } else {
        output::success(&format!("Payment sent: {} USDC → {}", args.amount, args.to));
        output::print_kv(&[
            ("Status", resp.status.as_str()),
            ("Invoice", resp.invoice_id.as_deref().unwrap_or("—")),
            ("Tx Hash", resp.tx_hash.as_deref().unwrap_or("pending")),
        ]);
    }
    Ok(())
}
