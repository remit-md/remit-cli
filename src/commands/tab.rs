use anyhow::Result;
use clap::{Args, Subcommand};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

#[derive(Subcommand)]
pub enum TabAction {
    /// Open a new tab with a spending limit
    Open(TabOpenArgs),
    /// Charge an amount against an open tab
    Charge(TabChargeArgs),
    /// Close a tab and settle
    Close(TabCloseArgs),
    /// Show tab details
    Get(TabGetArgs),
    /// List open tabs
    List(TabListArgs),
}

#[derive(Args)]
pub struct TabOpenArgs {
    /// Provider wallet address (the party that charges the tab)
    pub provider: String,
    /// Maximum spending limit in USDC
    pub limit: String,
    /// Per-unit charge in USDC (default: same as limit)
    #[arg(long)]
    pub per_unit: Option<String>,
    /// Expiry in seconds from now (default: 86400 = 24h)
    #[arg(long, default_value = "86400")]
    pub expiry: u64,
}

#[derive(Args)]
pub struct TabChargeArgs {
    /// Tab ID
    pub tab_id: String,
    /// Amount to charge in USDC
    pub amount: String,
    /// Running cumulative total (default: same as amount for first charge)
    #[arg(long)]
    pub cumulative: Option<String>,
    /// Call count for this charge (default: 1)
    #[arg(long, default_value = "1")]
    pub call_count: i32,
}

#[derive(Args)]
pub struct TabCloseArgs {
    /// Tab ID
    pub tab_id: String,
    /// Final settlement amount in USDC
    #[arg(long)]
    pub final_amount: Option<String>,
}

#[derive(Args)]
pub struct TabGetArgs {
    /// Tab ID
    pub tab_id: String,
}

#[derive(Args)]
pub struct TabListArgs {
    #[arg(long, default_value = "20")]
    pub limit: u32,
}

pub async fn run(action: TabAction, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);

    match action {
        TabAction::Open(args) => {
            let per_unit = args.per_unit.unwrap_or_else(|| args.limit.clone());
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let expiry = now as i64 + args.expiry as i64;
            let tab = client
                .tab_open(&args.provider, &args.limit, &per_unit, expiry)
                .await?;
            if ctx.json {
                output::print_json(&tab);
            } else {
                output::success(&format!("Tab opened: {}", tab.id));
                print_tab(&tab);
            }
        }

        TabAction::Charge(args) => {
            let cumulative = args.cumulative.unwrap_or_else(|| args.amount.clone());
            let charge = client
                .tab_charge(&args.tab_id, &args.amount, &cumulative, args.call_count)
                .await?;
            if ctx.json {
                output::print_json(&charge);
            } else {
                output::success(&format!(
                    "Tab {} charged: {} USDC",
                    args.tab_id, args.amount
                ));
                output::print_kv(&[
                    ("Tab ID", args.tab_id.as_str()),
                    ("Amount", charge.amount.to_string().as_str()),
                    ("Cumulative", charge.cumulative.to_string().as_str()),
                ]);
            }
        }

        TabAction::Close(args) => {
            let total_charged = "0"; // server computes final from tab state
            let final_amount = args
                .final_amount
                .unwrap_or_else(|| total_charged.to_string());
            let tab = client.tab_close(&args.tab_id, &final_amount).await?;
            if ctx.json {
                output::print_json(&tab);
            } else {
                output::success(&format!("Tab {} closed", args.tab_id));
                print_tab(&tab);
            }
        }

        TabAction::Get(args) => {
            let tab = client.tab_get(&args.tab_id).await?;
            if ctx.json {
                output::print_json(&tab);
            } else {
                print_tab(&tab);
            }
        }

        TabAction::List(args) => {
            let paged = client.tabs_list(args.limit).await?;
            if ctx.json {
                output::print_json(&paged);
            } else if paged.data.is_empty() {
                output::info("No active tabs.");
            } else {
                output::print_table(
                    vec!["ID", "Provider", "Limit", "Charged", "Status"],
                    paged
                        .data
                        .iter()
                        .map(|t| {
                            vec![
                                t.id.clone(),
                                t.provider.clone().unwrap_or_else(|| "—".to_string()),
                                t.limit_amount
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "—".to_string()),
                                t.total_charged
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "0".to_string()),
                                t.status.clone(),
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

fn print_tab(tab: &crate::client::Tab) {
    output::print_kv(&[
        ("ID", tab.id.as_str()),
        ("Status", tab.status.as_str()),
        ("Provider", tab.provider.as_deref().unwrap_or("—")),
        (
            "Limit",
            &tab.limit_amount
                .as_ref()
                .map(|v| format!("{v} USDC"))
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "Charged",
            &tab.total_charged
                .as_ref()
                .map(|v| format!("{v} USDC"))
                .unwrap_or_else(|| "0 USDC".to_string()),
        ),
        ("Tx Hash", tab.tx_hash.as_deref().unwrap_or("—")),
    ]);
}
