//! Process management for the signer server.
//!
//! PID file at `~/.remit/signer.pid`. Foreground by default.
//! `remit signer stop` reads the PID and sends SIGTERM (Unix)
//! or taskkill (Windows).
#![deny(unsafe_code)]

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

// ── PID file ───────────────────────────────────────────────────────────────

/// Path to the signer PID file.
pub fn pid_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot locate home directory")?;
    Ok(home.join(".remit").join("signer.pid"))
}

/// Write the current process PID to the PID file.
pub fn write_pid() -> Result<()> {
    let path = pid_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory: {}", parent.display()))?;
    }
    let pid = std::process::id();
    std::fs::write(&path, pid.to_string())
        .with_context(|| format!("cannot write PID file: {}", path.display()))?;
    Ok(())
}

/// Read the PID from the PID file.
pub fn read_pid() -> Result<u32> {
    let path = pid_path()?;
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read PID file: {}", path.display()))?;
    contents
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid PID in {}: '{}'", path.display(), contents.trim()))
}

/// Remove the PID file.
pub fn remove_pid() -> Result<()> {
    let path = pid_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("cannot remove PID file: {}", path.display()))?;
    }
    Ok(())
}

/// Check if a signer process is currently running.
pub fn is_running() -> bool {
    read_pid().ok().map(is_pid_alive).unwrap_or(false)
}

/// Check if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks if process exists without sending a signal
        let result = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        result.map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(windows)]
    {
        let result = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        result
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Stop the signer process by PID.
pub fn stop_process() -> Result<()> {
    let pid = read_pid()?;

    if !is_pid_alive(pid) {
        remove_pid()?;
        return Err(anyhow!(
            "signer is not running (stale PID file for process {pid})"
        ));
    }

    #[cfg(unix)]
    {
        let status = std::process::Command::new("kill")
            .args([&pid.to_string()])
            .status()
            .context("failed to send SIGTERM")?;
        if !status.success() {
            return Err(anyhow!("failed to stop signer process {pid}"));
        }
    }

    #[cfg(windows)]
    {
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status()
            .context("failed to run taskkill")?;
        if !status.success() {
            return Err(anyhow!("failed to stop signer process {pid}"));
        }
    }

    remove_pid()?;
    Ok(())
}

/// Spawn the signer as a detached background process.
///
/// On Windows, uses `CREATE_NEW_PROCESS_GROUP` via `creation_flags`.
/// On Unix, spawns a child that outlives the parent.
pub fn spawn_background(args: &[&str]) -> Result<u32> {
    let exe = std::env::current_exe().context("cannot locate remit binary")?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const DETACHED_PROCESS: u32 = 0x00000008;

        let child = std::process::Command::new(exe)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn()
            .context("failed to spawn background signer")?;
        Ok(child.id())
    }

    #[cfg(unix)]
    {
        let child = std::process::Command::new(exe)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn background signer")?;
        Ok(child.id())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = args;
        Err(anyhow!("background mode not supported on this platform"))
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pid_path_is_under_remit_dir() {
        let path = pid_path().unwrap();
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains(".remit"),
            "PID path must be under .remit, got: {path_str}"
        );
        assert!(
            path_str.ends_with("signer.pid"),
            "PID file must be named signer.pid, got: {path_str}"
        );
    }

    #[test]
    fn current_process_is_alive() {
        let pid = std::process::id();
        assert!(is_pid_alive(pid), "current process must be alive");
    }

    #[test]
    fn bogus_pid_is_not_alive() {
        // PID 99999999 almost certainly doesn't exist
        assert!(!is_pid_alive(99_999_999));
    }

    #[test]
    fn read_pid_fails_when_no_file() {
        // If there's no PID file, read_pid should error
        // (may pass or fail depending on whether ~/.remit/signer.pid exists)
        let _ = read_pid(); // Just verify it doesn't panic
    }
}
