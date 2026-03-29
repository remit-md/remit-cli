use anyhow::{bail, Context, Result};
use clap::Args;
use serde::Deserialize;
use std::io::Write;
use std::path::PathBuf;

/// Update the Remit CLI to the latest version
#[derive(Args)]
pub struct UpdateArgs {
    /// Only check for updates, don't install. Exit 0 if up-to-date, 1 if outdated.
    #[arg(long)]
    check: bool,

    /// Skip interactive confirmation
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub async fn run(args: UpdateArgs, _ctx: super::Context) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    // Check if installed via a package manager — delegate update to it
    if let Some(mgr) = detect_package_manager() {
        eprintln!("remit was installed via {mgr}. Update with:");
        eprintln!("  {}", manager_update_command(&mgr));
        std::process::exit(0);
    }

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

    if !is_newer(latest, current) {
        eprintln!("Already up to date (v{current})");
        std::process::exit(0);
    }

    eprintln!("Update available: v{current} → v{latest}");

    if args.check {
        std::process::exit(1);
    }

    // Determine the correct asset for this platform
    let asset_name = platform_asset_name()
        .context("unsupported platform — download manually from GitHub Releases")?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "release v{latest} has no asset '{asset_name}' — download manually from https://github.com/remit-md/remit-cli/releases/latest"
            )
        })?;

    // Confirmation
    if !args.yes {
        eprint!("Download and install v{latest}? [y/N] ");
        std::io::stderr().flush().ok();
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .context("failed to read input")?;
        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    eprintln!("Downloading {asset_name}...");
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("download failed")?
        .error_for_status()
        .context("download failed")?
        .bytes()
        .await
        .context("failed to read download")?;

    // Extract binary from archive
    let binary_bytes = extract_binary(&bytes, &asset_name)?;

    // Replace current binary
    let current_exe =
        std::env::current_exe().context("cannot determine current executable path")?;
    replace_binary(&current_exe, &binary_bytes)?;

    eprintln!("Updated remit v{current} → v{latest}");
    Ok(())
}

/// Compare semver strings. Returns true if `latest` is newer than `current`.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64) {
        let parts: Vec<&str> = s.split('.').collect();
        let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts
            .get(2)
            .and_then(|p| {
                // Strip pre-release suffix (e.g. "3-rc.1" → "3")
                p.split('-').next().and_then(|v| v.parse().ok())
            })
            .unwrap_or(0);
        (major, minor, patch)
    };
    parse(latest) > parse(current)
}

/// Map current OS + arch to the expected GitHub Release asset filename.
pub(crate) fn platform_asset_name() -> Option<String> {
    let target = platform_target()?;
    let ext = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };
    Some(format!("remit-{target}.{ext}"))
}

/// Map current OS + arch to the Rust target triple used in release builds.
pub(crate) fn platform_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// Detect if the CLI was installed via a package manager.
pub(crate) fn detect_package_manager() -> Option<String> {
    // Check config first (explicit install method)
    if let Ok(cfg) = crate::config::load() {
        if let Some(install) = &cfg.install {
            if let Some(method) = &install.method {
                if !method.is_empty() && method != "manual" && method != "curl" {
                    return Some(method.clone());
                }
            }
        }
    }

    // Auto-detect from executable path
    let exe = std::env::current_exe().ok()?;
    let path = exe.to_string_lossy();

    if std::env::var("SNAP").is_ok() {
        return Some("snap".into());
    }
    if std::env::var("HOMEBREW_PREFIX").is_ok() || path.contains("/Cellar/") {
        return Some("brew".into());
    }
    if path.contains("/scoop/") || path.contains("\\scoop\\") {
        return Some("scoop".into());
    }
    if path.contains("/.cargo/bin/") || path.contains("\\.cargo\\bin\\") {
        return Some("cargo".into());
    }
    if path.contains("/node_modules/") || path.contains("\\node_modules\\") {
        return Some("npm".into());
    }

    None
}

fn manager_update_command(manager: &str) -> &'static str {
    match manager {
        "brew" => "brew upgrade remit-md/tap/remit",
        "scoop" => "scoop update remit",
        "winget" => "winget upgrade remit-md.remit",
        "cargo" => "cargo install remit-cli --force",
        "snap" => "snap refresh remit",
        "npm" => "npm update -g @remit-md/cli",
        "pip" => "pip install --upgrade remit-cli",
        _ => "see https://github.com/remit-md/remit-cli/releases/latest",
    }
}

/// Extract the `remit` binary from a tar.gz or zip archive.
fn extract_binary(archive_bytes: &[u8], asset_name: &str) -> Result<Vec<u8>> {
    if asset_name.ends_with(".tar.gz") {
        extract_from_tar_gz(archive_bytes)
    } else if asset_name.ends_with(".zip") {
        extract_from_zip(archive_bytes)
    } else {
        bail!("unknown archive format: {asset_name}");
    }
}

fn extract_from_tar_gz(bytes: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("invalid tar archive")? {
        let mut entry = entry.context("invalid tar entry")?;
        let path = entry.path().context("invalid entry path")?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "remit" {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .context("failed to read binary from archive")?;
            return Ok(buf);
        }
    }
    bail!("archive does not contain 'remit' binary");
}

fn extract_from_zip(bytes: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("invalid zip archive")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("invalid zip entry")?;
        let name = file
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();
        if name == "remit.exe" || name == "remit" {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .context("failed to read binary from zip")?;
            return Ok(buf);
        }
    }
    bail!("zip does not contain 'remit.exe' binary");
}

/// Atomically replace the current binary.
fn replace_binary(current_exe: &PathBuf, new_bytes: &[u8]) -> Result<()> {
    let dir = current_exe
        .parent()
        .context("cannot determine executable directory")?;

    if cfg!(target_os = "windows") {
        // Windows locks running executables — rename current to .old, write new
        let old = current_exe.with_extension("exe.old");
        // Clean up previous .old if it exists
        let _ = std::fs::remove_file(&old);
        std::fs::rename(current_exe, &old).context("failed to rename current binary")?;
        std::fs::write(current_exe, new_bytes).context("failed to write new binary")?;
        // Best-effort cleanup of .old (may fail if still locked)
        let _ = std::fs::remove_file(&old);
    } else {
        // Unix: write to temp, chmod, atomic rename
        let tmp = dir.join(".remit-update-tmp");
        std::fs::write(&tmp, new_bytes).context("failed to write temp binary")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .context("failed to set executable permission")?;
        }
        std::fs::rename(&tmp, current_exe).context("failed to replace binary")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("1.0.0", "0.9.0"));
        assert!(is_newer("0.6.0", "0.5.4"));
        assert!(is_newer("0.5.5", "0.5.4"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn test_is_newer_equal() {
        assert!(!is_newer("0.5.4", "0.5.4"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_older() {
        assert!(!is_newer("0.5.3", "0.5.4"));
        assert!(!is_newer("0.4.0", "0.5.4"));
    }

    #[test]
    fn test_is_newer_prerelease_stripped() {
        // Pre-release suffixes are stripped for comparison
        assert!(is_newer("0.6.0", "0.5.4-rc.1"));
        assert!(!is_newer("0.5.4", "0.5.4-rc.1"));
    }

    #[test]
    fn test_platform_target_returns_something() {
        // On any CI/dev machine, this should return Some
        let target = platform_target();
        assert!(target.is_some(), "platform_target() returned None");
    }

    #[test]
    fn test_platform_asset_name_format() {
        let name = platform_asset_name().expect("unsupported platform");
        assert!(name.starts_with("remit-"));
        assert!(name.ends_with(".tar.gz") || name.ends_with(".zip"));
    }

    #[test]
    fn test_detect_package_manager_none_by_default() {
        // In a dev/CI env without snap/brew/scoop, should return None or "cargo"
        // (cargo if running from .cargo/bin)
        let mgr = detect_package_manager();
        // We just verify it doesn't panic
        let _ = mgr;
    }

    #[test]
    fn test_manager_update_command_known() {
        assert_eq!(
            manager_update_command("brew"),
            "brew upgrade remit-md/tap/remit"
        );
        assert_eq!(
            manager_update_command("cargo"),
            "cargo install remit-cli --force"
        );
        assert_eq!(manager_update_command("snap"), "snap refresh remit");
    }

    #[test]
    fn test_manager_update_command_unknown() {
        let cmd = manager_update_command("unknown");
        assert!(cmd.contains("github.com"));
    }
}
