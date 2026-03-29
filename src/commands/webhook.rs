use anyhow::Result;
use clap::{Args, Subcommand};

use crate::commands::Context;
use crate::output;

#[derive(Subcommand)]
pub enum WebhookAction {
    /// Register a new webhook endpoint
    Create(WebhookCreateArgs),
    /// List all registered webhooks
    List(WebhookListArgs),
    /// Update a webhook registration
    Update(WebhookUpdateArgs),
    /// Delete a webhook registration
    Delete(WebhookDeleteArgs),
}

#[derive(Args)]
pub struct WebhookCreateArgs {
    /// Webhook delivery URL (https://...)
    pub url: String,
    /// Event types to subscribe to (repeat for multiple)
    #[arg(long = "event", required = true)]
    pub events: Vec<String>,
    /// Chain filter — omit to receive all chains (repeat for multiple)
    #[arg(long = "chain")]
    pub chains: Vec<String>,
}

#[derive(Args)]
pub struct WebhookListArgs {}

#[derive(Args)]
pub struct WebhookUpdateArgs {
    /// Webhook ID to update
    pub webhook_id: String,
    /// New delivery URL
    #[arg(long)]
    pub url: Option<String>,
    /// New event types (repeat for multiple)
    #[arg(long = "event")]
    pub events: Option<Vec<String>>,
    /// Set active/inactive
    #[arg(long)]
    pub active: Option<bool>,
}

#[derive(Args)]
pub struct WebhookDeleteArgs {
    /// Webhook ID to delete
    pub id: String,
}

pub async fn run(action: WebhookAction, ctx: Context) -> Result<()> {
    let mut client = ctx.client()?;

    match action {
        WebhookAction::Create(args) => {
            let wh = client
                .webhook_create(&args.url, &args.events, &args.chains)
                .await?;
            if ctx.json {
                output::print_json(&wh);
            } else {
                output::success(&format!("Webhook registered: {}", wh.id));
                output::print_kv(&[
                    ("ID", wh.id.as_str()),
                    ("URL", wh.url.as_str()),
                    ("Events", &wh.events.join(", ")),
                    (
                        "Chains",
                        &if wh.chains.is_empty() {
                            "all".to_string()
                        } else {
                            wh.chains.join(", ")
                        },
                    ),
                    ("Active", if wh.active { "yes" } else { "no" }),
                    ("Secret", wh.secret.as_deref().unwrap_or("(not returned)")),
                ]);
                if wh.secret.is_some() {
                    output::info("Save the secret — it will not be shown again.");
                }
            }
        }
        WebhookAction::List(_) => {
            let webhooks = client.webhooks_list().await?;
            if ctx.json {
                output::print_json(&webhooks);
            } else if webhooks.is_empty() {
                output::info("No webhooks registered.");
            } else {
                output::print_table(
                    vec!["ID", "URL", "Events", "Active"],
                    webhooks
                        .iter()
                        .map(|w| {
                            vec![
                                w.id.clone(),
                                w.url.clone(),
                                w.events.join(", "),
                                if w.active {
                                    "yes".to_string()
                                } else {
                                    "no".to_string()
                                },
                            ]
                        })
                        .collect(),
                );
            }
        }
        WebhookAction::Update(args) => {
            let wh = client
                .update_webhook(&args.webhook_id, args.url, args.events, args.active)
                .await?;
            if ctx.json {
                output::print_json(&wh);
            } else {
                output::success(&format!("Webhook updated: {}", wh.id));
                output::print_kv(&[
                    ("ID", wh.id.as_str()),
                    ("URL", wh.url.as_str()),
                    ("Events", &wh.events.join(", ")),
                    (
                        "Chains",
                        &if wh.chains.is_empty() {
                            "all".to_string()
                        } else {
                            wh.chains.join(", ")
                        },
                    ),
                    ("Active", if wh.active { "yes" } else { "no" }),
                ]);
            }
        }
        WebhookAction::Delete(args) => {
            client.webhook_delete(&args.id).await?;
            if ctx.json {
                output::print_json(&serde_json::json!({ "deleted": true, "id": args.id }));
            } else {
                output::success(&format!("Webhook deleted: {}", args.id));
            }
        }
    }
    Ok(())
}
