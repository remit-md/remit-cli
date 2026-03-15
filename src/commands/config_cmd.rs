use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use crate::commands::Context;
use crate::config;
use crate::output;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a config value
    Set(ConfigSetArgs),
    /// Get a config value
    Get(ConfigGetArgs),
    /// Show all config values
    Show,
}

#[derive(Args)]
pub struct ConfigSetArgs {
    /// Config key: network | output_format | api_base
    pub key: String,
    /// Value to set
    pub value: String,
}

#[derive(Args)]
pub struct ConfigGetArgs {
    /// Config key to retrieve
    pub key: String,
}

pub async fn run(action: ConfigAction, ctx: Context) -> Result<()> {
    match action {
        ConfigAction::Set(args) => {
            let mut cfg = config::load()?;
            match args.key.as_str() {
                "network" => {
                    if args.value != "mainnet" && args.value != "testnet" {
                        return Err(anyhow!(
                            "network must be 'mainnet' or 'testnet', got '{}'",
                            args.value
                        ));
                    }
                    cfg.network = Some(args.value.clone());
                }
                "output_format" => {
                    if args.value != "table" && args.value != "json" {
                        return Err(anyhow!(
                            "output_format must be 'table' or 'json', got '{}'",
                            args.value
                        ));
                    }
                    cfg.output_format = Some(args.value.clone());
                }
                "api_base" => {
                    cfg.api_base = Some(args.value.clone());
                }
                key => {
                    return Err(anyhow!(
                        "Unknown config key: '{key}'. Valid keys: network, output_format, api_base"
                    ))
                }
            }
            config::save(&cfg)?;
            if ctx.json {
                output::print_json(&serde_json::json!({ "key": args.key, "value": args.value }));
            } else {
                output::success(&format!("{} = {}", args.key, args.value));
            }
        }

        ConfigAction::Get(args) => {
            let cfg = config::load()?;
            let value = match args.key.as_str() {
                "network" => cfg.network.as_deref().unwrap_or("mainnet").to_string(),
                "output_format" => cfg.output_format.as_deref().unwrap_or("table").to_string(),
                "api_base" => cfg.api_base.clone().unwrap_or_default(),
                key => {
                    return Err(anyhow!(
                        "Unknown config key: '{key}'. Valid keys: network, output_format, api_base"
                    ))
                }
            };
            if ctx.json {
                output::print_json(&serde_json::json!({ "key": args.key, "value": value }));
            } else {
                println!("{value}");
            }
        }

        ConfigAction::Show => {
            let cfg = config::load()?;
            let path = config::config_path()?;
            if ctx.json {
                output::print_json(&cfg);
            } else {
                output::print_kv(&[
                    ("Config file", path.to_string_lossy().as_ref()),
                    (
                        "network",
                        cfg.network.as_deref().unwrap_or("mainnet (default)"),
                    ),
                    (
                        "output_format",
                        cfg.output_format.as_deref().unwrap_or("table (default)"),
                    ),
                    (
                        "api_base",
                        cfg.api_base
                            .as_deref()
                            .unwrap_or("(not set — uses network default)"),
                    ),
                ]);
            }
        }
    }
    Ok(())
}
