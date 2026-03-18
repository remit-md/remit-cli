use anyhow::Result;
use clap::{Args, Subcommand};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;
use crate::permit;

#[derive(Subcommand)]
pub enum BountyAction {
    /// Post a new bounty
    Post(BountyPostArgs),
    /// Submit a claim for a bounty
    Submit(BountySubmitArgs),
    /// Award bounty to a submission
    Award(BountyAwardArgs),
    /// List bounties
    List(BountyListArgs),
}

#[derive(Args)]
pub struct BountyPostArgs {
    /// Bounty amount in USDC
    pub amount: String,
    /// Human-readable description of the task
    pub description: String,
    /// Expiry in seconds from now (default: 604800 = 7 days)
    #[arg(long, default_value = "604800")]
    pub expiry: u64,
}

#[derive(Args)]
pub struct BountySubmitArgs {
    /// Bounty ID
    pub bounty_id: String,
    /// Submission proof (URL or content hash)
    pub proof: String,
}

#[derive(Args)]
pub struct BountyAwardArgs {
    /// Bounty ID
    pub bounty_id: String,
    /// Submission ID to award
    pub submission_id: i64,
}

#[derive(Args)]
pub struct BountyListArgs {
    #[arg(long, default_value = "20")]
    pub limit: u32,
}

pub async fn run(action: BountyAction, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);

    match action {
        BountyAction::Post(args) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let deadline = now as i64 + args.expiry as i64;
            let amount_f64: f64 = args
                .amount
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid amount: {}", args.amount))?;
            let permit_sig = permit::auto_permit(&client, amount_f64, "bounty").await?;
            let bounty = client
                .bounty_post(&args.amount, &args.description, deadline, Some(&permit_sig))
                .await?;
            if ctx.json {
                output::print_json(&bounty);
            } else {
                output::success(&format!("Bounty posted: {}", bounty.id));
                print_bounty(&bounty);
            }
        }

        BountyAction::Submit(args) => {
            let submission = client.bounty_submit(&args.bounty_id, &args.proof).await?;
            if ctx.json {
                output::print_json(&submission);
            } else {
                output::success(&format!(
                    "Submission {} on bounty {}",
                    submission.id.unwrap_or(0),
                    args.bounty_id
                ));
                output::print_kv(&[
                    ("Status", submission.status.as_str()),
                    ("Proof", submission.evidence_hash.as_str()),
                ]);
            }
        }

        BountyAction::Award(args) => {
            let bounty = client
                .bounty_award(&args.bounty_id, args.submission_id)
                .await?;
            if ctx.json {
                output::print_json(&bounty);
            } else {
                output::success(&format!(
                    "Bounty {} awarded to submission {}",
                    args.bounty_id, args.submission_id
                ));
                print_bounty(&bounty);
            }
        }

        BountyAction::List(args) => {
            let paged = client.bounties_list(args.limit).await?;
            if ctx.json {
                output::print_json(&paged);
            } else if paged.data.is_empty() {
                output::info("No bounties found.");
            } else {
                output::print_table(
                    vec!["ID", "Amount", "Status", "Deadline"],
                    paged
                        .data
                        .iter()
                        .map(|b| {
                            vec![
                                b.id.clone(),
                                b.amount
                                    .as_ref()
                                    .map(|v| format!("{v} USDC"))
                                    .unwrap_or_else(|| "—".to_string()),
                                b.status.clone(),
                                b.deadline.clone().unwrap_or_else(|| "—".to_string()),
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

fn print_bounty(b: &crate::client::Bounty) {
    output::print_kv(&[
        ("ID", b.id.as_str()),
        ("Status", b.status.as_str()),
        (
            "Amount",
            &b.amount
                .as_ref()
                .map(|v| format!("{v} USDC"))
                .unwrap_or_else(|| "—".to_string()),
        ),
        ("Description", b.task_description.as_deref().unwrap_or("—")),
        ("Winner", b.winner.as_deref().unwrap_or("—")),
        ("Tx Hash", b.tx_hash.as_deref().unwrap_or("—")),
    ]);
}
