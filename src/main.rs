#![deny(warnings)]

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod completions;
mod config;
mod output;
mod ows;
mod permit;
mod signer;

/// Remit — USDC payment protocol CLI for AI agents
#[derive(Parser)]
#[command(
    name = "remit",
    version,
    about = "Send USDC payments, open tabs, run streams, and more — from the command line.",
    long_about = None,
    arg_required_else_help = true,
)]
struct Cli {
    /// Output raw JSON (default: human-readable table)
    #[arg(long, global = true)]
    json: bool,

    /// Use testnet (Base Sepolia)
    #[arg(long, global = true)]
    testnet: bool,

    /// Use mainnet (Base). Required for fund-moving commands when not set in config.
    #[arg(long, global = true, conflicts_with = "testnet")]
    mainnet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a one-time USDC payment
    Pay(commands::pay::PayArgs),
    /// Check your wallet status and balance
    Status(commands::status::StatusArgs),
    /// Show your USDC balance
    Balance(commands::balance::BalanceArgs),
    /// Tab operations (open, charge, close)
    Tab {
        #[command(subcommand)]
        action: commands::tab::TabAction,
    },
    /// Stream operations (open, close)
    Stream {
        #[command(subcommand)]
        action: commands::stream::StreamAction,
    },
    /// Escrow operations (create, release, cancel, claim-start)
    Escrow {
        #[command(subcommand)]
        action: commands::escrow::EscrowAction,
    },
    /// Bounty operations (post, submit, award)
    Bounty {
        #[command(subcommand)]
        action: commands::bounty::BountyAction,
    },
    /// Deposit operations (create)
    Deposit {
        #[command(subcommand)]
        action: commands::deposit::DepositAction,
    },
    /// Generate a fund link
    Fund(commands::fund::FundArgs),
    /// Generate a withdraw link
    Withdraw(commands::withdraw::WithdrawArgs),
    /// Mint testnet USDC (max 2500 per request, 1/hr rate limit)
    Mint(commands::mint::MintArgs),
    /// Request testnet USDC from faucet (DEPRECATED — use `mint`)
    Faucet(commands::faucet::FaucetArgs),
    /// Generate a new keypair and configure auth
    Init(commands::init::InitArgs),
    /// Set or get config values
    Config {
        #[command(subcommand)]
        action: commands::config_cmd::ConfigAction,
    },
    /// A2A / AP2 operations (discover, pay, card)
    A2A {
        #[command(subcommand)]
        action: commands::a2a::A2AAction,
    },
    /// OWS wallet operations (list, fund, set-policy)
    Wallet {
        #[command(subcommand)]
        action: commands::wallet::WalletAction,
    },
    /// Webhook operations (create, list, delete)
    Webhook {
        #[command(subcommand)]
        action: commands::webhook::WebhookAction,
    },
    /// Print the wallet address from the keystore (no password needed)
    Address(commands::address::AddressArgs),
    /// Sign data using the encrypted keystore (stdin → stdout)
    Sign(commands::sign::SignArgs),
    /// Local signer operations (init, import, migrate)
    Signer {
        #[command(subcommand)]
        action: commands::signer::SignerAction,
    },
    /// Generate shell completion scripts
    Completions(completions::CompletionsArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    // Load config — propagate parse errors (only default when config file is absent).
    let cfg = config::load()?;
    let json = cli.json
        || cfg
            .output_format
            .as_deref()
            .map(|f| f == "json")
            .unwrap_or(false);
    // Network: explicit flags > config > default to testnet (safe direction)
    let testnet = if cli.mainnet {
        false
    } else if cli.testnet {
        true
    } else {
        let config_network = cfg.network.as_deref().unwrap_or("");
        match config_network {
            "mainnet" => false,
            "testnet" => true,
            _ => {
                eprintln!("warning: no network specified, defaulting to testnet. Use --mainnet for mainnet.");
                true
            }
        }
    };

    let ctx = commands::Context { json, testnet };

    match cli.command {
        Commands::Pay(args) => commands::pay::run(args, ctx).await,
        Commands::Status(args) => commands::status::run(args, ctx).await,
        Commands::Balance(args) => commands::balance::run(args, ctx).await,
        Commands::Tab { action } => commands::tab::run(action, ctx).await,
        Commands::Stream { action } => commands::stream::run(action, ctx).await,
        Commands::Escrow { action } => commands::escrow::run(action, ctx).await,
        Commands::Bounty { action } => commands::bounty::run(action, ctx).await,
        Commands::Deposit { action } => commands::deposit::run(action, ctx).await,
        Commands::Fund(args) => commands::fund::run(args, ctx).await,
        Commands::Withdraw(args) => commands::withdraw::run(args, ctx).await,
        Commands::Mint(args) => commands::mint::run(args, ctx).await,
        Commands::Faucet(args) => commands::faucet::run(args, ctx).await,
        Commands::Init(args) => commands::init::run(args, ctx).await,
        Commands::Config { action } => commands::config_cmd::run(action, ctx).await,
        Commands::A2A { action } => commands::a2a::run(action, ctx).await,
        Commands::Wallet { action } => commands::wallet::run(action, ctx).await,
        Commands::Webhook { action } => commands::webhook::run(action, ctx).await,
        Commands::Address(args) => commands::address::run(args).await,
        Commands::Sign(args) => commands::sign::run(args).await,
        Commands::Signer { action } => commands::signer::run(action, ctx).await,
        Commands::Completions(args) => {
            completions::run(args, &mut Cli::command());
            Ok(())
        }
    }
}
