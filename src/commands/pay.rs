use anyhow::Result;
use clap::Args;

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;
use crate::permit;

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
    /// Skip auto-permit (use existing on-chain USDC approval instead)
    #[arg(long)]
    pub no_permit: bool,
}

pub async fn run(args: PayArgs, ctx: Context) -> Result<()> {
    super::validate_positive_amount(&args.amount, "amount")?;
    super::validate_address(&args.to, "to")?;
    let client = RemitClient::new(ctx.testnet).await;

    let permit_sig = if args.no_permit {
        None
    } else {
        Some(permit::auto_permit(&client, &args.amount, "router").await?)
    };

    let resp = client
        .pay_direct(
            &args.to,
            &args.amount,
            args.memo.as_deref(),
            permit_sig.as_ref(),
        )
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
