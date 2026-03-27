use anyhow::{anyhow, Result};
use clap::Subcommand;
use std::sync::Arc;

use crate::output;
use crate::signer::{daemon, keystore, policy, server, tokens};

/// Default port for the signer HTTP server.
const DEFAULT_PORT: u16 = 7402;

#[derive(Subcommand)]
pub enum SignerAction {
    /// Initialize a new local signer wallet
    Init(SignerInitArgs),
    /// Start the signer HTTP server
    Start(SignerStartArgs),
    /// Stop the running signer server
    Stop,
    /// Show signer status
    Status,
    /// Import an existing private key into the signer
    Import(SignerImportArgs),
    /// Manage bearer tokens
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
}

#[derive(clap::Args)]
pub struct SignerInitArgs {
    /// Wallet name (default: remit-{hostname})
    #[arg(long)]
    pub name: Option<String>,
    /// Chain: "base" (mainnet) or "base-sepolia" (testnet)
    #[arg(long)]
    pub chain: Option<String>,
}

#[derive(clap::Args)]
pub struct SignerStartArgs {
    /// Port to bind (default: 7402)
    #[arg(long)]
    pub port: Option<u16>,
    /// Run in background (daemon mode)
    #[arg(long)]
    pub daemon: bool,
    /// Bearer token for authentication (required on first start, then cached)
    #[arg(long, env = "REMIT_SIGNER_TOKEN")]
    pub token: Option<String>,
}

#[derive(clap::Args)]
pub struct SignerImportArgs {
    /// Private key to import (hex, with or without 0x prefix)
    #[arg(long)]
    pub key: String,
    /// Wallet name (default: remit-{hostname})
    #[arg(long)]
    pub name: Option<String>,
    /// Chain for default policy: "base" or "base-sepolia"
    #[arg(long)]
    pub chain: Option<String>,
}

#[derive(Subcommand)]
pub enum TokenAction {
    /// Create a new bearer token
    Create(TokenCreateArgs),
    /// List all bearer tokens
    List,
    /// Revoke a bearer token
    Revoke(TokenRevokeArgs),
}

#[derive(clap::Args)]
pub struct TokenCreateArgs {
    /// Token name (for identification)
    #[arg(long, default_value = "default")]
    pub name: String,
}

#[derive(clap::Args)]
pub struct TokenRevokeArgs {
    /// Token hash prefix (from `remit signer token list`)
    #[arg(long)]
    pub prefix: String,
}

pub async fn run(action: SignerAction, ctx: crate::commands::Context) -> Result<()> {
    match action {
        SignerAction::Init(args) => run_init(args, ctx).await,
        SignerAction::Start(args) => run_start(args, ctx).await,
        SignerAction::Stop => run_stop(ctx).await,
        SignerAction::Status => run_status(ctx).await,
        SignerAction::Import(args) => run_import(args, ctx).await,
        SignerAction::Token { action } => run_token(action, ctx).await,
    }
}

// ── Init ───────────────────────────────────────────────────────────────────

async fn run_init(args: SignerInitArgs, ctx: crate::commands::Context) -> Result<()> {
    let ks = keystore::Keystore::open()?;
    let ts = tokens::TokenStore::open()?;
    let ps = policy::PolicyStore::open()?;

    let wallet_name = args.name.unwrap_or_else(crate::ows::default_wallet_name);
    let chain = args.chain.unwrap_or_else(crate::ows::detect_chain);

    // Check if wallet already exists
    if ks.exists(&wallet_name) {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name or delete ~/.remit/keys/{}.enc",
            wallet_name,
            wallet_name
        ));
    }

    // 1. Generate bearer token (this is also the encryption passphrase)
    let raw_token = ts.create("default", &wallet_name)?;

    // 2. Generate and encrypt key
    let address = ks.generate(&wallet_name, &raw_token)?;

    // 3. Create default policy
    ps.create_default(&wallet_name, &chain)?;

    // 4. Output
    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "chain": chain,
            "token": raw_token,
            "signer_url": format!("http://127.0.0.1:{DEFAULT_PORT}"),
        }));
    } else {
        output::print_kv(&[
            ("Wallet", &wallet_name),
            ("Address", &address),
            ("Chain", &chain),
        ]);
        println!();
        eprintln!("Token (save this — shown once):");
        eprintln!("  {raw_token}");
        println!();
        eprintln!("Start the signer:");
        eprintln!("  remit signer start --token {raw_token}");
        println!();
        eprintln!("Environment variables for your agent:");
        eprintln!("  REMIT_SIGNER_URL=http://127.0.0.1:{DEFAULT_PORT}");
        eprintln!("  REMIT_SIGNER_TOKEN={raw_token}");
        println!();
        eprintln!("MCP config:");
        let mcp = serde_json::json!({
            "mcpServers": {
                "remit": {
                    "command": "npx",
                    "args": ["@remitmd/mcp-server"],
                    "env": {
                        "REMIT_SIGNER_URL": format!("http://127.0.0.1:{DEFAULT_PORT}"),
                        "REMIT_SIGNER_TOKEN": "$REMIT_SIGNER_TOKEN",
                        "REMITMD_CHAIN": chain,
                    }
                }
            }
        });
        eprintln!("{}", serde_json::to_string_pretty(&mcp).unwrap_or_default());
    }

    Ok(())
}

// ── Start ──────────────────────────────────────────────────────────────────

async fn run_start(args: SignerStartArgs, ctx: crate::commands::Context) -> Result<()> {
    let port = args.port.unwrap_or(DEFAULT_PORT);

    // Check if already running
    if daemon::is_running() {
        return Err(anyhow!(
            "signer is already running (PID {})",
            daemon::read_pid().unwrap_or(0)
        ));
    }

    // Get the token (passphrase)
    let raw_token = args.token.ok_or_else(|| {
        anyhow!(
            "bearer token required. Use --token or set REMIT_SIGNER_TOKEN.\n\
             Run `remit signer init` to create a new wallet and token."
        )
    })?;

    // Validate the token
    let ts = tokens::TokenStore::open()?;
    let validated = ts
        .validate(&raw_token)
        .map_err(|_| anyhow!("invalid or revoked token. Check your REMIT_SIGNER_TOKEN value."))?;

    let wallet_name = validated.wallet;

    // Load the key file to get the address (no decryption needed)
    let ks = keystore::Keystore::open()?;
    let key_file = ks.load(&wallet_name)?;
    let address = key_file.address.clone();

    // Verify we can decrypt (fail fast if passphrase is wrong)
    keystore::decrypt(&key_file, &raw_token)
        .map_err(|_| anyhow!("cannot decrypt key file — token may not match the wallet"))?;

    // Daemon mode: spawn background process and exit
    if args.daemon {
        let port_str = port.to_string();
        let token_arg = format!("--token={raw_token}");
        let spawn_args: Vec<&str> = vec!["signer", "start", "--port", &port_str, &token_arg];

        let pid = daemon::spawn_background(&spawn_args)?;

        if ctx.json {
            output::print_json(&serde_json::json!({
                "status": "started",
                "pid": pid,
                "port": port,
                "wallet": wallet_name,
                "address": address,
            }));
        } else {
            output::success(&format!(
                "Signer started in background (PID {pid}, port {port})"
            ));
            output::print_kv(&[("Wallet", &wallet_name), ("Address", &address)]);
        }
        return Ok(());
    }

    // Foreground mode
    let ps = policy::PolicyStore::open()?;
    let state = Arc::new(server::SignerState::new(
        ks,
        ts,
        ps,
        wallet_name,
        address,
        raw_token,
        env!("CARGO_PKG_VERSION").to_string(),
    ));

    // Write PID file
    daemon::write_pid()?;

    // Install ctrl-c handler to clean up PID file
    let cleanup = tokio::spawn(async {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\nShutting down...");
        daemon::remove_pid().ok();
        std::process::exit(0);
    });

    // Start server (blocks until shutdown)
    let result = server::start(state, port).await;

    // Cleanup
    cleanup.abort();
    daemon::remove_pid().ok();

    result
}

// ── Stop ───────────────────────────────────────────────────────────────────

async fn run_stop(ctx: crate::commands::Context) -> Result<()> {
    let pid = daemon::read_pid()
        .map_err(|_| anyhow!("no signer PID file found. Is the signer running?"))?;

    daemon::stop_process()?;

    if ctx.json {
        output::print_json(&serde_json::json!({
            "status": "stopped",
            "pid": pid,
        }));
    } else {
        output::success(&format!("Signer stopped (PID {pid})"));
    }

    Ok(())
}

// ── Status ─────────────────────────────────────────────────────────────────

async fn run_status(ctx: crate::commands::Context) -> Result<()> {
    let running = daemon::is_running();
    let pid = daemon::read_pid().ok();

    let ks = keystore::Keystore::open()?;
    let keys = ks.list().unwrap_or_default();

    if ctx.json {
        output::print_json(&serde_json::json!({
            "running": running,
            "pid": pid,
            "wallets": keys,
        }));
    } else {
        if running {
            output::success(&format!("Signer is running (PID {})", pid.unwrap_or(0)));
        } else {
            output::info("Signer is not running");
        }
        if keys.is_empty() {
            output::info("No wallets. Run `remit signer init` to create one.");
        } else {
            output::info(&format!("Wallets: {}", keys.join(", ")));
        }
    }

    Ok(())
}

// ── Import ─────────────────────────────────────────────────────────────────

async fn run_import(args: SignerImportArgs, ctx: crate::commands::Context) -> Result<()> {
    let ks = keystore::Keystore::open()?;
    let ts = tokens::TokenStore::open()?;
    let ps = policy::PolicyStore::open()?;

    let wallet_name = args.name.unwrap_or_else(crate::ows::default_wallet_name);
    let chain = args.chain.unwrap_or_else(crate::ows::detect_chain);

    if ks.exists(&wallet_name) {
        return Err(anyhow!(
            "wallet '{}' already exists. Use a different --name.",
            wallet_name
        ));
    }

    // 1. Generate token (passphrase)
    let raw_token = ts.create("default", &wallet_name)?;

    // 2. Import and encrypt key
    let address = ks.import(&wallet_name, &args.key, &raw_token)?;

    // 3. Default policy
    ps.create_default(&wallet_name, &chain)?;

    // 4. Output (same as init)
    if ctx.json {
        output::print_json(&serde_json::json!({
            "wallet": wallet_name,
            "address": address,
            "chain": chain,
            "token": raw_token,
            "signer_url": format!("http://127.0.0.1:{DEFAULT_PORT}"),
        }));
    } else {
        output::print_kv(&[
            ("Wallet", &wallet_name),
            ("Address", &address),
            ("Chain", &chain),
        ]);
        println!();
        eprintln!("Token (save this — shown once):");
        eprintln!("  {raw_token}");
        println!();
        eprintln!("Start the signer:");
        eprintln!("  remit signer start --token {raw_token}");
        println!();
        eprintln!("Environment variables for your agent:");
        eprintln!("  REMIT_SIGNER_URL=http://127.0.0.1:{DEFAULT_PORT}");
        eprintln!("  REMIT_SIGNER_TOKEN={raw_token}");
    }

    Ok(())
}

// ── Token management ───────────────────────────────────────────────────────

async fn run_token(action: TokenAction, ctx: crate::commands::Context) -> Result<()> {
    let ts = tokens::TokenStore::open()?;

    match action {
        TokenAction::Create(args) => {
            // Determine wallet from first key
            let ks = keystore::Keystore::open()?;
            let keys = ks.list()?;
            let wallet = keys
                .first()
                .ok_or_else(|| anyhow!("no wallets found. Run `remit signer init` first."))?;

            let raw_token = ts.create(&args.name, wallet)?;

            if ctx.json {
                output::print_json(&serde_json::json!({
                    "token": raw_token,
                    "name": args.name,
                    "wallet": wallet,
                }));
            } else {
                eprintln!("Token (save this — shown once):");
                eprintln!("  {raw_token}");
                output::print_kv(&[("Name", &args.name), ("Wallet", wallet)]);
            }
        }
        TokenAction::List => {
            let records = ts.list()?;
            if ctx.json {
                let entries: Vec<serde_json::Value> = records
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "hash_prefix": &r.hash[..16],
                            "name": r.name,
                            "wallet": r.wallet,
                            "created_at": r.created_at,
                            "revoked": r.revoked,
                        })
                    })
                    .collect();
                output::print_json(&entries);
            } else if records.is_empty() {
                output::info("No tokens. Run `remit signer token create` to create one.");
            } else {
                let headers = vec!["Prefix", "Name", "Wallet", "Created", "Status"];
                let rows: Vec<Vec<String>> = records
                    .iter()
                    .map(|r| {
                        vec![
                            r.hash[..16].to_string(),
                            r.name.clone(),
                            r.wallet.clone(),
                            r.created_at.clone(),
                            if r.revoked {
                                "revoked".to_string()
                            } else {
                                "active".to_string()
                            },
                        ]
                    })
                    .collect();
                output::print_table(headers, rows);
            }
        }
        TokenAction::Revoke(args) => {
            ts.revoke_by_prefix(&args.prefix)?;
            if ctx.json {
                output::print_json(&serde_json::json!({
                    "revoked": true,
                    "prefix": args.prefix,
                }));
            } else {
                output::success(&format!("Token {}... revoked", &args.prefix));
            }
        }
    }

    Ok(())
}
