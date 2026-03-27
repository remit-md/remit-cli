//! Policy engine for the local signer.
//!
//! Security invariants:
//! - Policy evaluation happens BEFORE key decryption (invariant I3)
//! - Content-aware policy for typed-data (inspect chain, contract, amount)
//! - Rate-limit-only for digest signing (opaque hash, no content inspection)
//! - Spending tracker resets on UTC date rollover
//! - Policy denials return structured reasons (no key material)
#![deny(unsafe_code)]

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Types ──────────────────────────────────────────────────────────────────

/// On-disk policy file (JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFile {
    pub version: u32,
    pub name: String,
    /// Name of the wallet this policy applies to.
    pub wallet: String,
    pub rules: PolicyRules,
    pub spending: SpendingTracker,
}

/// Policy rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRules {
    /// Allowed chain IDs in CAIP-2 format (e.g., `["eip155:8453"]`).
    /// Empty = allow all chains.
    pub chain_ids: Vec<String>,
    /// Allowed contract addresses (lowercase, 0x-prefixed).
    /// Empty = allow all contracts.
    #[serde(default)]
    pub allowed_contracts: Vec<String>,
    /// Maximum USDC per transaction. `None` = unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tx_usdc: Option<f64>,
    /// Maximum USDC per UTC day. `None` = unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_limit_usdc: Option<f64>,
}

/// Tracks cumulative daily spending. Persisted to disk with the policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingTracker {
    /// Cumulative USDC spent today.
    pub daily_total_usdc: f64,
    /// UTC date of the current tracking period (YYYY-MM-DD).
    pub date: String,
}

/// Deny reason returned to the caller.
#[derive(Debug, Clone)]
pub struct PolicyDenial {
    pub reason: String,
}

/// Context extracted from a typed-data sign request for content-aware policy.
pub struct TypedDataContext {
    /// Chain ID from the EIP-712 domain (e.g., 8453).
    pub chain_id: u64,
    /// Target contract address (from message fields like `spender`, `to`).
    pub contract: Option<String>,
    /// USDC amount (in human units, e.g., 10.5 = $10.50).
    pub amount_usdc: Option<f64>,
}

// ── PolicyStore ────────────────────────────────────────────────────────────

/// Manages policy files in a directory.
pub struct PolicyStore {
    dir: PathBuf,
}

impl PolicyStore {
    /// Open the policy store at the default location (`~/.remit/policies/`).
    pub fn open() -> Result<Self> {
        let home = dirs::home_dir().context("cannot locate home directory")?;
        Ok(Self {
            dir: home.join(".remit").join("policies"),
        })
    }

    /// Open a policy store at a custom directory (for testing).
    pub fn open_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Load a policy file by name.
    pub fn load(&self, name: &str) -> Result<PolicyFile> {
        let path = self.dir.join(format!("{name}.json"));
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read policy file: {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("cannot parse policy file: {}", path.display()))
    }

    /// Save a policy file.
    pub fn save(&self, policy: &PolicyFile) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create policies directory: {}", self.dir.display()))?;
        let path = self.dir.join(format!("{}.json", policy.name));
        let json = serde_json::to_string_pretty(policy).context("cannot serialize policy")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write policy file: {}", path.display()))?;
        Ok(())
    }

    /// Create a default chain-lock-only policy.
    pub fn create_default(&self, wallet: &str, chain: &str) -> Result<PolicyFile> {
        let caip2 = match chain {
            "base" => "eip155:8453",
            "base-sepolia" => "eip155:84532",
            _ => return Err(anyhow!("unknown chain: {chain}")),
        };

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let policy = PolicyFile {
            version: 1,
            name: "default".to_string(),
            wallet: wallet.to_string(),
            rules: PolicyRules {
                chain_ids: vec![caip2.to_string()],
                allowed_contracts: vec![],
                max_tx_usdc: None,
                daily_limit_usdc: None,
            },
            spending: SpendingTracker {
                daily_total_usdc: 0.0,
                date: today,
            },
        };
        self.save(&policy)?;
        Ok(policy)
    }

    /// List all policy names.
    pub fn list(&self) -> Result<Vec<String>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("cannot read policies directory: {}", self.dir.display()))?
        {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_string_lossy().strip_suffix(".json") {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }
}

// ── Policy evaluation ──────────────────────────────────────────────────────

/// Evaluate content-aware policy for a typed-data sign request.
///
/// This is called BEFORE key decryption (invariant I3).
/// Returns `Ok(())` if the request is allowed, or `Err(PolicyDenial)` if denied.
pub fn evaluate_typed_data(
    policy: &mut PolicyFile,
    ctx: &TypedDataContext,
) -> Result<(), PolicyDenial> {
    // Reset spending tracker if date has rolled over
    reset_spending_if_new_day(&mut policy.spending);

    // 1. Chain ID check
    if !policy.rules.chain_ids.is_empty() {
        let caip2 = format!("eip155:{}", ctx.chain_id);
        if !policy.rules.chain_ids.contains(&caip2) {
            return Err(PolicyDenial {
                reason: format!(
                    "chain {} is not in the allowed list: {:?}",
                    ctx.chain_id, policy.rules.chain_ids
                ),
            });
        }
    }

    // 2. Contract allowlist check
    if !policy.rules.allowed_contracts.is_empty() {
        if let Some(ref contract) = ctx.contract {
            let contract_lower = contract.to_lowercase();
            let allowed = policy
                .rules
                .allowed_contracts
                .iter()
                .any(|c| c.to_lowercase() == contract_lower);
            if !allowed {
                return Err(PolicyDenial {
                    reason: format!("contract {contract} is not in the allowlist"),
                });
            }
        }
    }

    // 3. Per-transaction limit
    if let (Some(max_tx), Some(amount)) = (policy.rules.max_tx_usdc, ctx.amount_usdc) {
        if amount > max_tx {
            return Err(PolicyDenial {
                reason: format!(
                    "transaction amount ${:.2} exceeds per-transaction limit ${:.2}",
                    amount, max_tx
                ),
            });
        }
    }

    // 4. Daily spending limit
    if let (Some(daily_limit), Some(amount)) = (policy.rules.daily_limit_usdc, ctx.amount_usdc) {
        let projected = policy.spending.daily_total_usdc + amount;
        if projected > daily_limit {
            return Err(PolicyDenial {
                reason: format!(
                    "transaction ${:.2} would exceed daily limit ${:.2} (spent today: ${:.2})",
                    amount, daily_limit, policy.spending.daily_total_usdc
                ),
            });
        }
    }

    Ok(())
}

/// Record a successful spend (called AFTER signing succeeds).
pub fn record_spend(policy: &mut PolicyFile, amount_usdc: f64) {
    reset_spending_if_new_day(&mut policy.spending);
    policy.spending.daily_total_usdc += amount_usdc;
}

/// Evaluate rate-limit-only policy for a digest sign request.
///
/// Digest signing is opaque — we can't inspect chain, contract, or amount.
/// Only basic rate limiting is possible. For now, this is a no-op that
/// logs a warning. Future: implement request-per-minute throttling.
pub fn evaluate_digest(_policy: &PolicyFile) -> Result<(), PolicyDenial> {
    // Digest signing is opaque — no content-aware policy possible.
    // Rate limiting will be added when the HTTP server tracks request counts.
    Ok(())
}

/// Reset the spending tracker if the UTC date has changed.
fn reset_spending_if_new_day(spending: &mut SpendingTracker) {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    if spending.date != today {
        spending.daily_total_usdc = 0.0;
        spending.date = today;
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_store() -> (PolicyStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("cannot create temp dir");
        let store = PolicyStore::open_in(dir.path().join("policies"));
        (store, dir)
    }

    fn base_policy(wallet: &str) -> PolicyFile {
        PolicyFile {
            version: 1,
            name: "test".to_string(),
            wallet: wallet.to_string(),
            rules: PolicyRules {
                chain_ids: vec!["eip155:8453".to_string()],
                allowed_contracts: vec![],
                max_tx_usdc: None,
                daily_limit_usdc: None,
            },
            spending: SpendingTracker {
                daily_total_usdc: 0.0,
                date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            },
        }
    }

    // ── Chain lock ──────────────────────────────────────────────────────

    #[test]
    fn allows_correct_chain() {
        let mut policy = base_policy("w");
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: None,
        };
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
    }

    #[test]
    fn denies_wrong_chain() {
        let mut policy = base_policy("w");
        let ctx = TypedDataContext {
            chain_id: 1,
            contract: None,
            amount_usdc: None,
        };
        let result = evaluate_typed_data(&mut policy, &ctx);
        assert!(result.is_err());
        let denial = result.unwrap_err();
        assert!(
            denial.reason.contains("chain 1"),
            "denial must mention the chain"
        );
    }

    #[test]
    fn empty_chain_list_allows_all() {
        let mut policy = base_policy("w");
        policy.rules.chain_ids = vec![];
        let ctx = TypedDataContext {
            chain_id: 999,
            contract: None,
            amount_usdc: None,
        };
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
    }

    // ── Contract allowlist ──────────────────────────────────────────────

    #[test]
    fn allows_contract_in_allowlist() {
        let mut policy = base_policy("w");
        policy.rules.allowed_contracts =
            vec!["0xabcdef1234567890abcdef1234567890abcdef12".to_string()];
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: Some("0xAbCdEf1234567890abcdef1234567890abcdef12".to_string()),
            amount_usdc: None,
        };
        // Case-insensitive match
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
    }

    #[test]
    fn denies_contract_not_in_allowlist() {
        let mut policy = base_policy("w");
        policy.rules.allowed_contracts = vec!["0xaaaa".to_string()];
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: Some("0xbbbb".to_string()),
            amount_usdc: None,
        };
        let result = evaluate_typed_data(&mut policy, &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().reason.contains("allowlist"));
    }

    #[test]
    fn empty_allowlist_allows_all_contracts() {
        let mut policy = base_policy("w");
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: Some("0xanything".to_string()),
            amount_usdc: None,
        };
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
    }

    // ── Per-transaction limit ───────────────────────────────────────────

    #[test]
    fn allows_within_per_tx_limit() {
        let mut policy = base_policy("w");
        policy.rules.max_tx_usdc = Some(500.0);
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(100.0),
        };
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
    }

    #[test]
    fn denies_over_per_tx_limit() {
        let mut policy = base_policy("w");
        policy.rules.max_tx_usdc = Some(500.0);
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(501.0),
        };
        let result = evaluate_typed_data(&mut policy, &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().reason.contains("per-transaction limit"));
    }

    // ── Daily spending limit ────────────────────────────────────────────

    #[test]
    fn allows_within_daily_limit() {
        let mut policy = base_policy("w");
        policy.rules.daily_limit_usdc = Some(1000.0);
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(400.0),
        };
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
    }

    #[test]
    fn denies_over_daily_limit() {
        let mut policy = base_policy("w");
        policy.rules.daily_limit_usdc = Some(1000.0);
        policy.spending.daily_total_usdc = 700.0;
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(400.0),
        };
        let result = evaluate_typed_data(&mut policy, &ctx);
        assert!(result.is_err());
        let reason = result.unwrap_err().reason;
        assert!(reason.contains("daily limit"), "reason: {reason}");
    }

    #[test]
    fn spending_sequence_tracks_cumulative(/* invariant I10 */) {
        let mut policy = base_policy("w");
        policy.rules.daily_limit_usdc = Some(1000.0);

        // First $400 — allowed
        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(400.0),
        };
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
        record_spend(&mut policy, 400.0);

        // Second $400 — allowed ($800 total)
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
        record_spend(&mut policy, 400.0);

        // Third $400 — denied ($1200 would exceed $1000 limit)
        let result = evaluate_typed_data(&mut policy, &ctx);
        assert!(result.is_err());
    }

    // ── Spending reset on date rollover ─────────────────────────────────

    #[test]
    fn spending_resets_on_new_day() {
        let mut policy = base_policy("w");
        policy.rules.daily_limit_usdc = Some(100.0);
        policy.spending.daily_total_usdc = 99.0;
        // Set date to yesterday
        policy.spending.date = "2020-01-01".to_string();

        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(50.0),
        };
        // Should succeed because date rolled over, resetting to 0
        assert!(evaluate_typed_data(&mut policy, &ctx).is_ok());
        assert_eq!(
            policy.spending.daily_total_usdc, 0.0,
            "spending must reset on date rollover"
        );
    }

    // ── PolicyStore CRUD ────────────────────────────────────────────────

    #[test]
    fn create_default_and_load() {
        let (store, _dir) = test_store();
        let policy = store.create_default("my-wallet", "base").unwrap();
        assert_eq!(policy.wallet, "my-wallet");
        assert_eq!(policy.rules.chain_ids, vec!["eip155:8453"]);

        let loaded = store.load("default").unwrap();
        assert_eq!(loaded.wallet, "my-wallet");
    }

    #[test]
    fn create_default_testnet() {
        let (store, _dir) = test_store();
        let policy = store.create_default("w", "base-sepolia").unwrap();
        assert_eq!(policy.rules.chain_ids, vec!["eip155:84532"]);
    }

    #[test]
    fn create_default_unknown_chain_fails() {
        let (store, _dir) = test_store();
        assert!(store.create_default("w", "ethereum").is_err());
    }

    #[test]
    fn list_policies() {
        let (store, _dir) = test_store();
        assert!(store.list().unwrap().is_empty());

        store.create_default("w", "base").unwrap();
        let names = store.list().unwrap();
        assert_eq!(names, vec!["default"]);
    }

    #[test]
    fn save_and_reload_preserves_spending() {
        let (store, _dir) = test_store();
        let mut policy = store.create_default("w", "base").unwrap();
        policy.spending.daily_total_usdc = 42.5;
        store.save(&policy).unwrap();

        let reloaded = store.load("default").unwrap();
        assert!((reloaded.spending.daily_total_usdc - 42.5).abs() < f64::EPSILON);
    }

    // ── Frame condition: evaluate doesn't modify key/token state ────────

    #[test]
    fn evaluate_only_modifies_spending() {
        let mut policy = base_policy("w");
        policy.rules.daily_limit_usdc = Some(1000.0);
        let original_name = policy.name.clone();
        let original_wallet = policy.wallet.clone();
        let original_rules = policy.rules.chain_ids.clone();

        let ctx = TypedDataContext {
            chain_id: 8453,
            contract: None,
            amount_usdc: Some(100.0),
        };
        evaluate_typed_data(&mut policy, &ctx).unwrap();
        record_spend(&mut policy, 100.0);

        // Only spending should change
        assert_eq!(policy.name, original_name);
        assert_eq!(policy.wallet, original_wallet);
        assert_eq!(policy.rules.chain_ids, original_rules);
        assert!(policy.spending.daily_total_usdc > 0.0);
    }

    // ── Digest policy ───────────────────────────────────────────────────

    #[test]
    fn digest_policy_allows_by_default() {
        let policy = base_policy("w");
        assert!(evaluate_digest(&policy).is_ok());
    }
}
