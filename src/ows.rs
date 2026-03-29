//! OWS (Open Wallet Standard) integration helpers.
//!
//! Maps Remit chain names to CAIP-2 identifiers and provides
//! convenience wrappers around ows-lib for the CLI.

use anyhow::{anyhow, Context, Result};

/// Map a Remit chain name to its CAIP-2 identifier.
pub fn chain_to_caip2(chain: &str) -> Result<String> {
    match chain {
        "base" => Ok("eip155:8453".to_string()),
        "base-sepolia" => Ok("eip155:84532".to_string()),
        _ => Err(anyhow!(
            "unknown chain: {chain}. Use 'base' or 'base-sepolia'."
        )),
    }
}

/// Map a Remit chain name to a numeric chain ID.
#[allow(dead_code)]
pub fn chain_to_id(chain: &str) -> Result<u64> {
    match chain {
        "base" => Ok(8453),
        "base-sepolia" => Ok(84532),
        _ => Err(anyhow!(
            "unknown chain: {chain}. Use 'base' or 'base-sepolia'."
        )),
    }
}

/// Detect chain from env var or default to "base".
pub fn detect_chain() -> String {
    std::env::var("REMITMD_CHAIN").unwrap_or_else(|_| "base".to_string())
}

/// Generate a wallet name from the hostname: `remit-{hostname}`.
pub fn default_wallet_name() -> String {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "agent".to_string());
    format!("remit-{host}")
}

/// Remit contract addresses for Base Sepolia (for policy allowlists).
#[allow(dead_code)]
pub const TESTNET_CONTRACTS: &[&str] = &[
    "0x3120f396ff6a9afc5a9d92e28796082f1429e024", // Router
    "0x47de7cdd757e3765d36c083dab59b2c5a9d249f2", // Escrow
    "0x9415f510d8c6199e0f66bde927d7d88de391f5e8", // Tab
    "0x20d413e0eac0f5da3c8630667fd16a94fcd7231a", // Stream
    "0xb3868471c3034280cce3a56dd37c6154c3bb0b32", // Bounty
    "0x7e0ae37df62e93c1c16a5661a7998bd174331554", // Deposit
    "0x2d846325766921935f37d5b4478196d3ef93707c", // USDC (MockUSDC)
];

/// Remit contract addresses for Base Mainnet.
#[allow(dead_code)]
pub const MAINNET_CONTRACTS: &[&str] = &[
    "0xAf2e211BC585D3Ab37e9BD546Fb25747a09254D2", // Router
    "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", // USDC
];

/// Get contracts for a given chain name.
#[allow(dead_code)]
pub fn contracts_for_chain(chain: &str) -> &'static [&'static str] {
    match chain {
        "base" => MAINNET_CONTRACTS,
        "base-sepolia" => TESTNET_CONTRACTS,
        _ => &[],
    }
}

/// Check if OWS is available by attempting to list wallets.
pub fn is_ows_available() -> bool {
    ows_lib::list_wallets(None).is_ok()
}

/// Create a wallet, returning wallet info. Errors if name already exists.
pub fn create_wallet(name: &str) -> Result<ows_lib::WalletInfo> {
    ows_lib::create_wallet(name, None, None, None)
        .map_err(|e| anyhow!("failed to create OWS wallet: {e}"))
}

/// Get a wallet by name or ID.
pub fn get_wallet(name_or_id: &str) -> Result<ows_lib::WalletInfo> {
    ows_lib::get_wallet(name_or_id, None).map_err(|e| anyhow!("failed to get OWS wallet: {e}"))
}

/// List all OWS wallets.
pub fn list_wallets() -> Result<Vec<ows_lib::WalletInfo>> {
    ows_lib::list_wallets(None).map_err(|e| anyhow!("failed to list OWS wallets: {e}"))
}

/// Create an OWS policy with chain lock only (no spending limits).
pub fn create_chain_policy(chain: &str) -> Result<ows_core::Policy> {
    let caip2 = chain_to_caip2(chain)?;
    let id = format!("remit-{chain}");
    let now = chrono::Utc::now().to_rfc3339();

    let policy = ows_core::Policy {
        id: id.clone(),
        name: format!("Remit {chain} chain lock"),
        version: 1,
        created_at: now,
        rules: vec![ows_core::PolicyRule::AllowedChains {
            chain_ids: vec![caip2],
        }],
        executable: None,
        config: None,
        action: ows_core::PolicyAction::Deny,
    };

    ows_lib::policy_store::save_policy(&policy, None)
        .map_err(|e| anyhow!("failed to save policy: {e}"))?;

    Ok(policy)
}

/// Create an OWS policy with chain lock + spending limits.
pub fn create_spending_policy(
    chain: &str,
    max_tx_usdc: Option<f64>,
    daily_limit_usdc: Option<f64>,
) -> Result<ows_core::Policy> {
    let caip2 = chain_to_caip2(chain)?;
    let id = format!("remit-{chain}-limits");
    let now = chrono::Utc::now().to_rfc3339();

    // Build config for the executable policy
    let mut config = serde_json::Map::new();
    config.insert("chain_ids".to_string(), serde_json::json!([&caip2]));
    if let Some(max_tx) = max_tx_usdc {
        config.insert("max_tx_usdc".to_string(), serde_json::json!(max_tx));
    }
    if let Some(daily) = daily_limit_usdc {
        config.insert("daily_limit_usdc".to_string(), serde_json::json!(daily));
    }

    // The executable is the remit-policy npm package
    let policy = ows_core::Policy {
        id: id.clone(),
        name: format!("Remit {chain} spending policy"),
        version: 1,
        created_at: now,
        rules: vec![ows_core::PolicyRule::AllowedChains {
            chain_ids: vec![caip2],
        }],
        executable: Some("npx @remitmd/ows-policy".to_string()),
        config: Some(serde_json::Value::Object(config)),
        action: ows_core::PolicyAction::Deny,
    };

    ows_lib::policy_store::save_policy(&policy, None)
        .map_err(|e| anyhow!("failed to save policy: {e}"))?;

    Ok(policy)
}

/// Create an API key bound to a wallet and policy.
/// Returns the raw token (shown once) and the key metadata.
pub fn create_api_key(wallet_id: &str, policy_id: &str) -> Result<(String, ows_core::ApiKeyFile)> {
    // No passphrase — wallets created without passphrase use empty string
    ows_lib::key_ops::create_api_key(
        "remit-agent",
        &[wallet_id.to_string()],
        &[policy_id.to_string()],
        "",
        None, // no expiry
        None, // default vault
    )
    .map_err(|e| anyhow!("failed to create API key: {e}"))
}

/// List all OWS policies.
#[allow(dead_code)]
pub fn list_policies() -> Result<Vec<ows_core::Policy>> {
    ows_lib::policy_store::list_policies(None).map_err(|e| anyhow!("failed to list policies: {e}"))
}

/// Get the EVM address for a wallet.
pub fn wallet_evm_address(wallet: &ows_lib::WalletInfo) -> Option<String> {
    wallet
        .accounts
        .iter()
        .find(|a| a.chain_id.starts_with("eip155:") || a.chain_id == "evm")
        .map(|a| a.address.clone())
}

/// Generate the MCP config JSON for the user to add to their config.
pub fn mcp_config_json(wallet_name: &str, chain: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "remit": {
                "command": "npx",
                "args": ["@remitmd/mcp-server"],
                "env": {
                    "OWS_WALLET_ID": wallet_name,
                    "OWS_API_KEY": "$OWS_API_KEY",
                    "REMITMD_CHAIN": chain,
                }
            }
        }
    }))
    .expect("JSON serialization cannot fail for static structure")
}

/// Resolve the vault path for display purposes.
pub fn vault_path_display() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(".ows").display().to_string()
}

/// Check if OWS CLI is installed by running `ows --version`.
#[allow(dead_code)]
pub fn ows_cli_version() -> Option<String> {
    std::process::Command::new("ows")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

/// Install OWS globally via npm (silent).
pub fn install_ows_via_npm() -> Result<()> {
    let status = std::process::Command::new("npm")
        .args(["install", "-g", "@open-wallet-standard/core"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run npm install")?;

    if !status.success() {
        return Err(anyhow!(
            "npm install -g @open-wallet-standard/core failed (exit code: {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Chain mapping tests ──────────────────────────────────────────────

    #[test]
    fn test_chain_to_caip2_base() {
        assert_eq!(chain_to_caip2("base").unwrap(), "eip155:8453");
    }

    #[test]
    fn test_chain_to_caip2_base_sepolia() {
        assert_eq!(chain_to_caip2("base-sepolia").unwrap(), "eip155:84532");
    }

    #[test]
    fn test_chain_to_caip2_unknown() {
        assert!(chain_to_caip2("ethereum").is_err());
        assert!(chain_to_caip2("").is_err());
        assert!(chain_to_caip2("arbitrum").is_err());
    }

    #[test]
    fn test_chain_to_id_base() {
        assert_eq!(chain_to_id("base").unwrap(), 8453);
    }

    #[test]
    fn test_chain_to_id_base_sepolia() {
        assert_eq!(chain_to_id("base-sepolia").unwrap(), 84532);
    }

    #[test]
    fn test_chain_to_id_unknown() {
        assert!(chain_to_id("solana").is_err());
    }

    // ── Wallet name tests ────────────────────────────────────────────────

    #[test]
    fn test_default_wallet_name_has_prefix() {
        let name = default_wallet_name();
        assert!(
            name.starts_with("remit-"),
            "wallet name must start with 'remit-', got: {name}"
        );
        assert!(name.len() > 6, "wallet name must include hostname");
    }

    // ── Contract address tests ───────────────────────────────────────────

    #[test]
    fn test_testnet_contracts_count() {
        assert_eq!(TESTNET_CONTRACTS.len(), 7);
    }

    #[test]
    fn test_mainnet_contracts_count() {
        assert_eq!(MAINNET_CONTRACTS.len(), 2);
    }

    #[test]
    fn test_contracts_for_chain_base() {
        assert_eq!(contracts_for_chain("base").len(), 2);
    }

    #[test]
    fn test_contracts_for_chain_testnet() {
        assert_eq!(contracts_for_chain("base-sepolia").len(), 7);
    }

    #[test]
    fn test_contracts_for_chain_unknown() {
        assert!(contracts_for_chain("unknown").is_empty());
    }

    #[test]
    fn test_all_contracts_are_valid_addresses() {
        for addr in TESTNET_CONTRACTS.iter().chain(MAINNET_CONTRACTS.iter()) {
            assert!(addr.starts_with("0x"), "address must start with 0x: {addr}");
            assert_eq!(addr.len(), 42, "address must be 42 chars: {addr}");
        }
    }

    // ── MCP config tests ─────────────────────────────────────────────────

    #[test]
    fn test_mcp_config_json_structure() {
        let json = mcp_config_json("remit-test", "base");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let server = &parsed["mcpServers"]["remit"];
        assert_eq!(server["command"], "npx");
        assert_eq!(server["args"][0], "@remitmd/mcp-server");
        assert_eq!(server["env"]["OWS_WALLET_ID"], "remit-test");
        assert_eq!(server["env"]["REMITMD_CHAIN"], "base");
        assert_eq!(server["env"]["OWS_API_KEY"], "$OWS_API_KEY");
    }

    #[test]
    fn test_mcp_config_json_testnet() {
        let json = mcp_config_json("remit-agent", "base-sepolia");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["mcpServers"]["remit"]["env"]["REMITMD_CHAIN"],
            "base-sepolia"
        );
    }

    // ── Vault path test ──────────────────────────────────────────────────

    #[test]
    fn test_vault_path_display_ends_with_ows() {
        let path = vault_path_display();
        assert!(
            path.contains(".ows"),
            "vault path must contain .ows, got: {path}"
        );
    }

    // ── EVM address extraction ───────────────────────────────────────────

    #[test]
    fn test_wallet_evm_address_found() {
        let wallet = ows_lib::WalletInfo {
            id: "test-id".to_string(),
            name: "test-wallet".to_string(),
            created_at: "2026-03-25T00:00:00Z".to_string(),
            accounts: vec![ows_lib::AccountInfo {
                chain_id: "eip155:8453".to_string(),
                address: "0xdeadbeef".to_string(),
                derivation_path: "m/44'/60'/0'/0/0".to_string(),
            }],
        };
        assert_eq!(wallet_evm_address(&wallet), Some("0xdeadbeef".to_string()));
    }

    #[test]
    fn test_wallet_evm_address_evm_chain_id() {
        let wallet = ows_lib::WalletInfo {
            id: "test-id".to_string(),
            name: "test-wallet".to_string(),
            created_at: "2026-03-25T00:00:00Z".to_string(),
            accounts: vec![ows_lib::AccountInfo {
                chain_id: "evm".to_string(),
                address: "0xcafe".to_string(),
                derivation_path: "m/44'/60'/0'/0/0".to_string(),
            }],
        };
        assert_eq!(wallet_evm_address(&wallet), Some("0xcafe".to_string()));
    }

    #[test]
    fn test_wallet_evm_address_not_found() {
        let wallet = ows_lib::WalletInfo {
            id: "test-id".to_string(),
            name: "test-wallet".to_string(),
            created_at: "2026-03-25T00:00:00Z".to_string(),
            accounts: vec![ows_lib::AccountInfo {
                chain_id: "solana".to_string(),
                address: "SolAddr123".to_string(),
                derivation_path: "m/44'/501'/0'/0'".to_string(),
            }],
        };
        assert_eq!(wallet_evm_address(&wallet), None);
    }

    #[test]
    fn test_wallet_evm_address_empty_accounts() {
        let wallet = ows_lib::WalletInfo {
            id: "test-id".to_string(),
            name: "test-wallet".to_string(),
            created_at: "2026-03-25T00:00:00Z".to_string(),
            accounts: vec![],
        };
        assert_eq!(wallet_evm_address(&wallet), None);
    }

    // ── Integration tests (skip if OWS not available) ────────────────────

    #[test]
    fn test_ows_availability_does_not_panic() {
        // This just verifies the function doesn't panic, regardless of OWS state
        let _ = is_ows_available();
    }
}
