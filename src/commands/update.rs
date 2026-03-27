use anyhow::{Context, Result};
use clap::Args;
use serde::Deserialize;

use crate::config;

/// Update the Remit CLI to the latest version
#[derive(Args)]
pub struct UpdateArgs;

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

pub async fn run(_args: UpdateArgs, _ctx: super::Context) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    // Check latest release
    eprintln!("Checking for updates...");
    let client = reqwest::Client::builder().user_agent("remit-cli").build()?;
    let release: GitHubRelease = client
        .get("https://api.github.com/repos/remit-md/remit-cli/releases/latest")
        .send()
        .await
        .context("failed to check for updates")?
        .error_for_status()
        .context("GitHub API error")?
        .json()
        .await
        .context("invalid release response")?;

    let latest = release.tag_name.trim_start_matches('v');

    if latest == current {
        eprintln!("Already up to date (v{current})");
        return Ok(());
    }

    eprintln!("Update available: v{current} → v{latest}");

    // Determine install method from config
    let cfg = config::load()?;
    let method = cfg
        .install
        .as_ref()
        .and_then(|i| i.method.as_deref())
        .unwrap_or("manual");

    match method {
        "brew" => {
            eprintln!("Updating via Homebrew...");
            let status = std::process::Command::new("brew")
                .args(["upgrade", "remit"])
                .status()
                .context("failed to run brew upgrade")?;
            if !status.success() {
                anyhow::bail!("brew upgrade failed");
            }
        }
        "winget" => {
            eprintln!("Updating via winget...");
            let status = std::process::Command::new("winget")
                .args(["upgrade", "remit-md.remit"])
                .status()
                .context("failed to run winget upgrade")?;
            if !status.success() {
                anyhow::bail!("winget upgrade failed");
            }
        }
        "scoop" => {
            eprintln!("Updating via Scoop...");
            let status = std::process::Command::new("scoop")
                .args(["update", "remit"])
                .status()
                .context("failed to run scoop update")?;
            if !status.success() {
                anyhow::bail!("scoop update failed");
            }
        }
        "cargo" => {
            eprintln!("Updating via cargo...");
            let status = std::process::Command::new("cargo")
                .args(["install", "remit-cli", "--force"])
                .status()
                .context("failed to run cargo install")?;
            if !status.success() {
                anyhow::bail!("cargo install failed");
            }
        }
        "curl" => {
            eprintln!("Updating via install script...");
            let shell = if cfg!(windows) { "cmd" } else { "sh" };
            let args: Vec<&str> = if cfg!(windows) {
                vec!["/C", "echo Update from https://remit.md/install.sh"]
            } else {
                vec!["-c", "curl -fsSL https://remit.md/install.sh | sh"]
            };
            let status = std::process::Command::new(shell)
                .args(&args)
                .status()
                .context("failed to run install script")?;
            if !status.success() {
                anyhow::bail!("install script failed");
            }
        }
        _ => {
            eprintln!("Update manually from:");
            eprintln!("  https://github.com/remit-md/remit-cli/releases/latest");
            eprintln!();
            eprintln!("Or reinstall with a package manager for automatic updates:");
            if cfg!(target_os = "macos") {
                eprintln!("  brew install remit-md/tap/remit");
            } else if cfg!(target_os = "windows") {
                eprintln!("  scoop bucket add remit-md https://github.com/remit-md/scoop-bucket");
                eprintln!("  scoop install remit");
            } else {
                eprintln!("  curl -fsSL https://remit.md/install.sh | sh");
            }
        }
    }

    Ok(())
}
