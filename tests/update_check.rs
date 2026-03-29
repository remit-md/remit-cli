//! Integration test: `remit update --check`
//!
//! Verifies the update command can reach the GitHub API and report version status.
//! Not ignored — runs in normal CI since it only hits a public endpoint.

use std::process::Command;

fn cargo_bin() -> String {
    let output = Command::new("cargo")
        .args(["build", "--quiet"])
        .output()
        .expect("cargo build failed");
    assert!(output.status.success(), "cargo build failed");

    // Locate the built binary
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let bin = if cfg!(target_os = "windows") {
        format!("{target_dir}/debug/remit.exe")
    } else {
        format!("{target_dir}/debug/remit")
    };
    assert!(
        std::path::Path::new(&bin).exists(),
        "binary not found at {bin}"
    );
    bin
}

#[test]
fn update_check_exits_cleanly() {
    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["update", "--check"])
        .output()
        .expect("failed to run remit update --check");

    // Exit 0 = up to date, Exit 1 = update available. Both are valid.
    let code = output.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "expected exit code 0 or 1, got {code}. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should print either "Already up to date" or "Update available"
    assert!(
        stderr.contains("up to date") || stderr.contains("Update available"),
        "unexpected output: {stderr}"
    );
}
