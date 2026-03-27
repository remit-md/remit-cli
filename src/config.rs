#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User config stored in ~/.remit/config.toml
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub network: Option<String>, // "mainnet" | "testnet"
    #[serde(default)]
    pub output_format: Option<String>, // "table" | "json"
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub install: Option<InstallConfig>,
}

/// Tracks how the CLI was installed, for `remit update`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct InstallConfig {
    pub method: Option<String>, // "brew" | "winget" | "scoop" | "cargo" | "curl" | "manual"
    pub installed_at: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot locate home directory")?;
    Ok(home.join(".remit").join("config.toml"))
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&contents).context("parsing config.toml")
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
