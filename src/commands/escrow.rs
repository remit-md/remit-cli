use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;
use crate::permit;

#[derive(Subcommand)]
pub enum EscrowAction {
    /// Create and fund a new escrow
    Create(EscrowCreateArgs),
    /// Release escrowed funds to recipient
    Release(EscrowIdArgs),
    /// Cancel escrow and return funds to payer
    Cancel(EscrowIdArgs),
    /// Start a claim process
    #[command(name = "claim-start")]
    ClaimStart(EscrowIdArgs),
    /// List escrows
    List(EscrowListArgs),
}

#[derive(Args)]
pub struct EscrowCreateArgs {
    /// Recipient wallet address
    pub payee: String,
    /// Amount in USDC
    pub amount: String,
    /// Timeout in seconds (default: 86400 = 24h)
    #[arg(long)]
    pub timeout: Option<i64>,
}

#[derive(Args)]
pub struct EscrowIdArgs {
    /// Escrow ID (invoice_id)
    pub escrow_id: String,
}

#[derive(Args)]
pub struct EscrowListArgs {
    #[arg(long, default_value = "20")]
    pub limit: u32,
}

pub async fn run(action: EscrowAction, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);

    match action {
        EscrowAction::Create(args) => {
            let amount: f64 = args
                .amount
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid amount: {}", args.amount))?;
            let permit_sig = permit::auto_permit(&client, amount, "escrow").await?;
            let escrow = client
                .escrow_create(&args.payee, &args.amount, args.timeout, Some(&permit_sig))
                .await?;
            if ctx.json {
                output::print_json(&escrow);
            } else {
                output::success(&format!("Escrow created: {}", escrow.invoice_id));
                print_escrow(&escrow);
            }
        }

        EscrowAction::Release(args) => {
            let escrow = client.escrow_release(&args.escrow_id).await?;
            if ctx.json {
                output::print_json(&escrow);
            } else {
                output::success(&format!("Escrow {} released", args.escrow_id));
                print_escrow(&escrow);
            }
        }

        EscrowAction::Cancel(args) => {
            let escrow = client.escrow_cancel(&args.escrow_id).await?;
            if ctx.json {
                output::print_json(&escrow);
            } else {
                output::success(&format!("Escrow {} cancelled", args.escrow_id));
                print_escrow(&escrow);
            }
        }

        EscrowAction::ClaimStart(args) => {
            let escrow = client.escrow_claim_start(&args.escrow_id).await?;
            if ctx.json {
                output::print_json(&escrow);
            } else {
                output::success(&format!("Claim started on escrow {}", args.escrow_id));
                print_escrow(&escrow);
            }
        }

        EscrowAction::List(args) => {
            let paged = client.escrows_list(args.limit).await?;
            if ctx.json {
                output::print_json(&paged);
            } else if paged.data.is_empty() {
                output::info("No active escrows.");
            } else {
                output::print_table(
                    vec!["ID", "Payee", "Amount", "Status", "Timeout"],
                    paged
                        .data
                        .iter()
                        .map(|e| {
                            vec![
                                e.invoice_id.clone(),
                                e.payee.clone().unwrap_or_else(|| "—".to_string()),
                                e.amount
                                    .as_ref()
                                    .map(|v| format!("{v} USDC"))
                                    .unwrap_or_else(|| "—".to_string()),
                                e.status.clone(),
                                e.timeout.clone().unwrap_or_else(|| "—".to_string()),
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

fn print_escrow(e: &crate::client::Escrow) {
    output::print_kv(&[
        ("ID", e.invoice_id.as_str()),
        ("Status", e.status.as_str()),
        ("Payee", e.payee.as_deref().unwrap_or("—")),
        (
            "Amount",
            &e.amount
                .as_ref()
                .map(|v| format!("{v} USDC"))
                .unwrap_or_else(|| "—".to_string()),
        ),
        ("Timeout", e.timeout.as_deref().unwrap_or("—")),
        ("Tx Hash", e.tx_hash.as_deref().unwrap_or("—")),
    ]);
}
