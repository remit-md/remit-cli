//! Platform detection utilities for shell-specific output.

/// Detect the current shell environment.
fn detect_shell() -> &'static str {
    if cfg!(windows) {
        // PSModulePath is set in PowerShell but not cmd
        if std::env::var("PSModulePath").is_ok() {
            "powershell"
        } else {
            "cmd"
        }
    } else {
        "unix" // bash, zsh — all use export
    }
}

/// Return a platform-aware hint for setting an environment variable.
pub fn env_var_hint(name: &str, placeholder: &str) -> String {
    match detect_shell() {
        "powershell" => format!("  $env:{name} = \"{placeholder}\""),
        "cmd" => format!("  set {name}={placeholder}"),
        _ => format!("  export {name}={placeholder}"),
    }
}

/// Return platform-aware instructions for setting the signer key password.
pub fn password_hint() -> String {
    env_var_hint("REMIT_SIGNER_KEY", "<your-password>")
}

/// Return platform-aware instructions for setting a raw private key.
#[allow(dead_code)]
pub fn raw_key_hint() -> String {
    env_var_hint("REMITMD_KEY", "0x<your-private-key>")
}
