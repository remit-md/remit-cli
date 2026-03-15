use crate::commands::Context;
use anyhow::Result;
use clap::{Args, Subcommand};

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
    /// Config key (network, output_format, api_base)
    pub key: String,
    /// Value to set
    pub value: String,
}

#[derive(Args)]
pub struct ConfigGetArgs {
    /// Config key to retrieve
    pub key: String,
}

pub async fn run(_action: ConfigAction, _ctx: Context) -> Result<()> {
    todo!("config command — implemented in task 1.4")
}
