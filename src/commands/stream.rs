use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::RemitClient;
use crate::commands::Context;
use crate::output;

#[derive(Subcommand)]
pub enum StreamAction {
    /// Open a payment stream
    Open(StreamOpenArgs),
    /// Close a payment stream
    Close(StreamCloseArgs),
    /// List active streams
    List(StreamListArgs),
}

#[derive(Args)]
pub struct StreamOpenArgs {
    /// Recipient wallet address
    pub payee: String,
    /// USDC per second (e.g., 0.001)
    pub rate: String,
    /// Maximum total USDC to stream
    pub max: String,
}

#[derive(Args)]
pub struct StreamCloseArgs {
    /// Stream ID
    pub stream_id: String,
}

#[derive(Args)]
pub struct StreamListArgs {
    #[arg(long, default_value = "20")]
    pub limit: u32,
}

pub async fn run(action: StreamAction, ctx: Context) -> Result<()> {
    let client = RemitClient::new(ctx.testnet);

    match action {
        StreamAction::Open(args) => {
            let stream = client
                .stream_open(&args.payee, &args.rate, &args.max)
                .await?;
            if ctx.json {
                output::print_json(&stream);
            } else {
                output::success(&format!("Stream opened: {}", stream.id));
                print_stream(&stream);
            }
        }

        StreamAction::Close(args) => {
            let stream = client.stream_close(&args.stream_id).await?;
            if ctx.json {
                output::print_json(&stream);
            } else {
                output::success(&format!("Stream {} closed", args.stream_id));
                print_stream(&stream);
            }
        }

        StreamAction::List(args) => {
            let paged = client.streams_list(args.limit).await?;
            if ctx.json {
                output::print_json(&paged);
            } else if paged.data.is_empty() {
                output::info("No active streams.");
            } else {
                output::print_table(
                    vec!["ID", "Payee", "Rate/s", "Max", "Withdrawn", "Status"],
                    paged
                        .data
                        .iter()
                        .map(|s| {
                            vec![
                                s.id.clone(),
                                s.payee.clone().unwrap_or_else(|| "—".to_string()),
                                s.rate_per_second
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "—".to_string()),
                                s.max_total
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "—".to_string()),
                                s.withdrawn
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "0".to_string()),
                                s.status.clone(),
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

fn print_stream(stream: &crate::client::Stream) {
    output::print_kv(&[
        ("ID", stream.id.as_str()),
        ("Status", stream.status.as_str()),
        ("Payee", stream.payee.as_deref().unwrap_or("—")),
        (
            "Rate/s",
            &stream
                .rate_per_second
                .as_ref()
                .map(|v| format!("{v} USDC"))
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "Max total",
            &stream
                .max_total
                .as_ref()
                .map(|v| format!("{v} USDC"))
                .unwrap_or_else(|| "—".to_string()),
        ),
        ("Tx Hash", stream.tx_hash.as_deref().unwrap_or("—")),
    ]);
}
