use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::{build_auth_headers, ChainConfig};
use crate::commands::Context;
use crate::output;

// ─── Subcommand ───────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum A2AAction {
    /// Discover an A2A agent — fetch and display its agent card
    Discover(DiscoverArgs),
    /// Send a payment via A2A message/send
    Pay(A2APayArgs),
    /// Show the Remit agent card (mainnet or testnet)
    Card(CardArgs),
}

#[derive(Args)]
pub struct DiscoverArgs {
    /// Base URL of the agent (e.g., https://remit.md)
    pub url: String,
}

#[derive(Args)]
pub struct A2APayArgs {
    /// Base URL of the A2A agent to pay through
    pub url: String,
    /// Amount in USDC (e.g., 10.50)
    pub amount: String,
    /// Recipient wallet address (0x...)
    #[arg(long)]
    pub to: String,
    /// Optional memo
    #[arg(long)]
    pub memo: Option<String>,
}

#[derive(Args)]
pub struct CardArgs {}

// ─── A2A types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct AgentCard {
    #[serde(rename = "protocolVersion")]
    protocol_version: Option<String>,
    name: Option<String>,
    description: Option<String>,
    url: Option<String>,
    version: Option<String>,
    #[serde(rename = "documentationUrl")]
    documentation_url: Option<String>,
    skills: Option<Vec<Skill>>,
    x402: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Skill {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct A2ATaskResult {
    id: Option<String>,
    status: Option<A2AStatus>,
    artifacts: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct A2AStatus {
    state: Option<String>,
    message: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct JsonRpcResponse {
    result: Option<A2ATaskResult>,
    error: Option<Value>,
}

// ─── Dispatch ─────────────────────────────────────────────────────────────────

pub async fn run(action: A2AAction, ctx: Context) -> Result<()> {
    match action {
        A2AAction::Discover(args) => discover(args, ctx).await,
        A2AAction::Pay(args) => pay(args, ctx).await,
        A2AAction::Card(args) => card(args, ctx).await,
    }
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn discover(args: DiscoverArgs, ctx: Context) -> Result<()> {
    let card = fetch_card(&args.url).await?;

    if ctx.json {
        output::print_json(&card);
    } else {
        output::success(&format!(
            "Agent: {}",
            card.name.as_deref().unwrap_or("unknown")
        ));
        output::print_kv(&[
            ("Name", card.name.as_deref().unwrap_or("—")),
            ("Protocol", card.protocol_version.as_deref().unwrap_or("—")),
            ("Version", card.version.as_deref().unwrap_or("—")),
            ("Endpoint", card.url.as_deref().unwrap_or("—")),
            ("Docs", card.documentation_url.as_deref().unwrap_or("—")),
        ]);
        if let Some(skills) = &card.skills {
            println!("\nSkills ({}):", skills.len());
            for skill in skills {
                println!(
                    "  {:20} {}",
                    skill.id.as_deref().unwrap_or("?"),
                    skill.description.as_deref().unwrap_or("")
                );
            }
        }
    }
    Ok(())
}

async fn pay(args: A2APayArgs, ctx: Context) -> Result<()> {
    // Discover the agent card to find the A2A endpoint URL.
    let card = fetch_card(&args.url).await?;
    let a2a_url = card
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("Agent card has no 'url' field"))?
        .to_string();

    // Extract the path for EIP-712 signing (server verifies against OriginalUri).
    let parsed = url::Url::parse(&a2a_url).map_err(|e| anyhow!("Invalid A2A endpoint URL: {e}"))?;
    let path = parsed.path().to_string();

    // Build A2A JSON-RPC payload.
    let nonce = hex::encode(rand::thread_rng().gen::<[u8; 16]>());
    let message_id = hex::encode(rand::thread_rng().gen::<[u8; 16]>());
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": message_id,
        "method": "message/send",
        "params": {
            "message": {
                "messageId": message_id,
                "role": "user",
                "parts": [{
                    "kind": "data",
                    "data": {
                        "model": "direct",
                        "to": args.to,
                        "amount": args.amount,
                        "memo": args.memo.unwrap_or_default(),
                        "nonce": nonce,
                    }
                }]
            }
        }
    });

    // Sign and send.
    let chain = ChainConfig::for_network(ctx.testnet);
    let auth_headers = build_auth_headers("POST", &path, chain.chain_id, &chain.router)?;

    let http = reqwest::Client::builder()
        .user_agent(concat!("remit-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap();

    let mut req = http
        .post(&a2a_url)
        .header("Content-Type", "application/json")
        .json(&payload);
    for (k, v) in &auth_headers {
        req = req.header(k, v);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow!("Network error: {e}"))?;

    let status = resp.status();
    let body: JsonRpcResponse = resp
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse A2A response: {e}"))?;

    if let Some(err) = &body.error {
        return Err(anyhow!(
            "A2A error (HTTP {}): {}",
            status,
            serde_json::to_string(err).unwrap_or_default()
        ));
    }

    let task = body.result.unwrap_or_default();

    if ctx.json {
        output::print_json(&task);
    } else {
        let state = task
            .status
            .as_ref()
            .and_then(|s| s.state.as_deref())
            .unwrap_or("unknown");
        let tx_hash = extract_tx_hash(&task);
        output::success(&format!(
            "A2A payment {} ({} USDC → {})",
            state, args.amount, args.to
        ));
        output::print_kv(&[
            ("Task ID", task.id.as_deref().unwrap_or("—")),
            ("State", state),
            ("Tx Hash", tx_hash.as_deref().unwrap_or("pending")),
        ]);
    }
    Ok(())
}

async fn card(_args: CardArgs, ctx: Context) -> Result<()> {
    let base = if ctx.testnet {
        "https://testnet.remit.md"
    } else {
        "https://remit.md"
    };
    let card = fetch_card(base).await?;

    if ctx.json {
        output::print_json(&card);
    } else {
        output::success(&format!(
            "Remit Agent Card ({})",
            if ctx.testnet { "testnet" } else { "mainnet" }
        ));
        output::print_kv(&[
            ("Name", card.name.as_deref().unwrap_or("—")),
            ("Protocol", card.protocol_version.as_deref().unwrap_or("—")),
            ("Version", card.version.as_deref().unwrap_or("—")),
            ("Endpoint", card.url.as_deref().unwrap_or("—")),
        ]);
        if let Some(skills) = &card.skills {
            println!("\nSkills ({}):", skills.len());
            for skill in skills {
                println!(
                    "  {:20} {}",
                    skill.id.as_deref().unwrap_or("?"),
                    skill.name.as_deref().unwrap_or("")
                );
            }
        }
    }
    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

async fn fetch_card(base_url: &str) -> Result<AgentCard> {
    let url = format!(
        "{}/.well-known/agent-card.json",
        base_url.trim_end_matches('/')
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow!("Network error: {e}"))?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "Agent card discovery failed: HTTP {}",
            resp.status()
        ));
    }
    resp.json()
        .await
        .map_err(|e| anyhow!("Failed to parse agent card: {e}"))
}

fn extract_tx_hash(task: &A2ATaskResult) -> Option<String> {
    let artifacts = task.artifacts.as_ref()?;
    for artifact in artifacts {
        if let Some(parts) = artifact.get("parts").and_then(|p| p.as_array()) {
            for part in parts {
                if let Some(tx) = part
                    .get("data")
                    .and_then(|d| d.get("txHash"))
                    .and_then(|h| h.as_str())
                {
                    return Some(tx.to_string());
                }
            }
        }
    }
    None
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tx_hash_present() {
        let task = A2ATaskResult {
            id: Some("task_1".to_string()),
            status: Some(A2AStatus {
                state: Some("completed".to_string()),
                message: None,
            }),
            artifacts: Some(vec![serde_json::json!({
                "parts": [{"kind": "data", "data": {"txHash": "0xdeadbeef"}}]
            })]),
        };
        assert_eq!(extract_tx_hash(&task), Some("0xdeadbeef".to_string()));
    }

    #[test]
    fn test_extract_tx_hash_absent() {
        let task = A2ATaskResult {
            id: Some("task_2".to_string()),
            status: None,
            artifacts: Some(vec![]),
        };
        assert_eq!(extract_tx_hash(&task), None);
    }

    #[test]
    fn test_a2a_pay_args_url_parsing() {
        let url = "https://remit.md/a2a";
        let parsed = url::Url::parse(url).unwrap();
        assert_eq!(parsed.path(), "/a2a");
        assert_eq!(
            format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or("")),
            "https://remit.md"
        );
    }
}
