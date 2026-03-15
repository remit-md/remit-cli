#![deny(warnings)]

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod completions;
mod config;
mod output;

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

    /// Use testnet (Base Sepolia) instead of mainnet
    #[arg(long, global = true)]
    testnet: bool,

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
    /// List recent transactions
    History(commands::history::HistoryArgs),
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
    /// Request testnet USDC from faucet
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
    /// Generate shell completion scripts
    Completions(completions::CompletionsArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let ctx = commands::Context {
        json: cli.json,
        testnet: cli.testnet,
    };

    match cli.command {
        Commands::Pay(args) => commands::pay::run(args, ctx).await,
        Commands::Status(args) => commands::status::run(args, ctx).await,
        Commands::Balance(args) => commands::balance::run(args, ctx).await,
        Commands::History(args) => commands::history::run(args, ctx).await,
        Commands::Tab { action } => commands::tab::run(action, ctx).await,
        Commands::Stream { action } => commands::stream::run(action, ctx).await,
        Commands::Escrow { action } => commands::escrow::run(action, ctx).await,
        Commands::Bounty { action } => commands::bounty::run(action, ctx).await,
        Commands::Deposit { action } => commands::deposit::run(action, ctx).await,
        Commands::Fund(args) => commands::fund::run(args, ctx).await,
        Commands::Withdraw(args) => commands::withdraw::run(args, ctx).await,
        Commands::Faucet(args) => commands::faucet::run(args, ctx).await,
        Commands::Init(args) => commands::init::run(args, ctx).await,
        Commands::Config { action } => commands::config_cmd::run(action, ctx).await,
        Commands::A2A { action } => commands::a2a::run(action, ctx).await,
        Commands::Completions(args) => {
            completions::run(args, &mut Cli::command());
            Ok(())
        }
    }
}
