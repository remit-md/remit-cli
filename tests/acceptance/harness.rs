#![allow(dead_code)]
//! CLI acceptance test harness.
//!
//! All operations go through the Remit API (via CLI or HTTP).
//! No direct RPC calls — balances come from `remit balance`, not eth_call.
//!
//! A single wallet pair (payer + provider) is shared across all tests.
//! Tests run sequentially (`--test-threads=1`) so nonces are ordered correctly.

use serde_json::Value;
use std::process::Command;
use std::sync::OnceLock;

// ── Config ──────────────────────────────────────────────────────────────────

pub const DEFAULT_API_URL: &str = "https://testnet.remit.md/api/v1";

/// Read API URL from env or use testnet default.
pub fn api_url() -> String {
    std::env::var("ACCEPTANCE_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string())
}

/// Print an `[ACCEPTANCE]` log line.
#[macro_export]
macro_rules! acceptance_log {
    ($($arg:tt)*) => {
        eprintln!("[ACCEPTANCE] {}", format!($($arg)*));
    };
}

// ── Shared wallet pair ──────────────────────────────────────────────────────

static WALLETS: OnceLock<(TestWallet, TestWallet)> = OnceLock::new();

/// Get the shared wallet pair (payer, provider). Created once, reused across all tests.
/// Mint happens in the `mint_and_balance` test — must run first.
pub fn shared_wallets() -> &'static (TestWallet, TestWallet) {
    WALLETS.get_or_init(|| {
        acceptance_log!("=== SHARED WALLET SETUP ===");
        let payer = TestWallet::generate();
        let provider = TestWallet::generate();
        acceptance_log!("payer:    {}", payer.address);
        acceptance_log!("provider: {}", provider.address);
        (payer, provider)
    })
}

// ── CLI output ──────────────────────────────────────────────────────────────

pub struct CliOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl CliOutput {
    /// Parse stdout as JSON. Panics if the output is not valid JSON.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "Failed to parse JSON from CLI output: {e}\nstdout: {}\nstderr: {}",
                self.stdout, self.stderr
            )
        })
    }
}

// ── Test wallet ─────────────────────────────────────────────────────────────

pub struct TestWallet {
    pub private_key: String,
    pub address: String,
}

impl TestWallet {
    /// Generate a fresh random wallet.
    pub fn generate() -> Self {
        use alloy::signers::local::PrivateKeySigner;
        use rand::RngCore;

        let mut key_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key_bytes);
        let private_key = format!("0x{}", hex::encode(key_bytes));

        let signer =
            PrivateKeySigner::from_bytes(&key_bytes.into()).expect("valid random private key");
        let address = format!("{:#x}", signer.address());

        acceptance_log!("wallet: {} (chain=84532)", address);

        Self {
            private_key,
            address,
        }
    }

    /// Run the `remit` CLI with `--testnet --json` and the given extra args.
    /// REMITMD_KEY is set to this wallet's private key.
    pub fn run_cli(&self, args: &[&str]) -> CliOutput {
        let bin = env!("CARGO_BIN_EXE_remit");
        let mut full_args = vec!["--testnet", "--json"];
        full_args.extend_from_slice(args);

        acceptance_log!("exec: remit {}", args.join(" "));

        let output = Command::new(bin)
            .args(&full_args)
            .env("REMITMD_KEY", &self.private_key)
            .output()
            .expect("failed to execute remit binary");

        let cli_output = CliOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        };

        if cli_output.success {
            if let Ok(json) = serde_json::from_str::<Value>(&cli_output.stdout) {
                if let Some(tx) = json["tx_hash"].as_str() {
                    acceptance_log!("  tx_hash: {tx}");
                }
                if let Some(status) = json["status"].as_str() {
                    acceptance_log!("  status: {status}");
                }
                if let Some(id) = json["id"].as_str().or(json["invoice_id"].as_str()) {
                    acceptance_log!("  id: {id}");
                }
            }
        } else {
            acceptance_log!(
                "  FAILED (exit={}): {}",
                cli_output.exit_code,
                cli_output.stderr.trim()
            );
        }

        cli_output
    }

    /// Run the CLI and parse JSON output. Panics on non-zero exit.
    pub fn run_cli_json(&self, args: &[&str]) -> Value {
        let output = self.run_cli(args);
        assert!(
            output.success,
            "CLI `remit {}` failed (exit={}):\nstderr: {}\nstdout: {}",
            args.join(" "),
            output.exit_code,
            output.stderr,
            output.stdout,
        );
        output.json()
    }

    /// Get USDC balance via `remit balance` (calls GET /status/{addr} on the API).
    pub fn balance(&self) -> f64 {
        let output = self.run_cli(&["balance"]);
        if !output.success {
            panic!(
                "remit balance failed: exit={}\nstderr: {}",
                output.exit_code, output.stderr
            );
        }
        let json = output.json();
        // `remit --json balance` returns {"address": "...", "network": "...", "usdc": "100.00"}
        json["usdc"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or_else(|| panic!("Could not parse balance from CLI output: {}", output.stdout))
    }

    /// Poll until balance differs from `before_balance`. Returns the new balance.
    /// Uses `remit balance` (API-backed), not direct RPC.
    pub async fn wait_for_balance(&self, before_balance: f64) -> f64 {
        let max_wait_ms: u64 = 30_000;
        let poll_interval_ms: u64 = 2_000;
        let start = std::time::Instant::now();

        while start.elapsed().as_millis() < max_wait_ms as u128 {
            let current = self.balance();
            if (current - before_balance).abs() > 0.0001 {
                acceptance_log!("  balance: {before_balance} -> {current}");
                return current;
            }
            tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
        }

        let final_balance = self.balance();
        acceptance_log!(
            "  balance poll timeout: still {final_balance} (expected change from {before_balance})"
        );
        final_balance
    }
    /// Sign an EIP-712 TabCharge message (for tab charge/close).
    /// Returns the hex-encoded signature.
    pub fn sign_tab_charge(
        &self,
        tab_contract: &str,
        tab_id: &str,
        total_charged_units: u64,
        call_count: u32,
    ) -> String {
        use alloy::primitives::{keccak256, Address, FixedBytes, U256};
        use alloy::signers::local::PrivateKeySigner;
        use alloy::signers::SignerSync;

        let key_bytes =
            hex::decode(self.private_key.trim_start_matches("0x")).expect("valid hex key");
        let signer = PrivateKeySigner::from_bytes(&FixedBytes::from_slice(&key_bytes))
            .expect("valid private key");

        // EIP-712 domain separator for RemitTab
        let chain_id: u64 = 84532; // Base Sepolia
        let contract: Address = tab_contract.parse().expect("valid tab contract address");

        // Domain hash: EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
        let domain_typehash = keccak256(
            "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );
        let name_hash = keccak256("RemitTab");
        let version_hash = keccak256("1");

        let mut domain_data = [0u8; 160];
        domain_data[0..32].copy_from_slice(domain_typehash.as_slice());
        domain_data[32..64].copy_from_slice(name_hash.as_slice());
        domain_data[64..96].copy_from_slice(version_hash.as_slice());
        domain_data[96..128].copy_from_slice(&U256::from(chain_id).to_be_bytes::<32>());
        domain_data[128..160]
            .copy_from_slice(&U256::from_be_slice(contract.as_slice()).to_be_bytes::<32>());
        let domain_separator = keccak256(domain_data);

        // TabCharge typehash
        let type_hash = keccak256("TabCharge(bytes32 tabId,uint96 totalCharged,uint32 callCount)");

        // UUID to bytes32 (ASCII left-padded)
        let mut tab_id_bytes = [0u8; 32];
        for (i, b) in tab_id.bytes().take(32).enumerate() {
            tab_id_bytes[i] = b;
        }

        // Encode struct: hash(typehash || tabId || totalCharged || callCount)
        let mut struct_data = [0u8; 128]; // 4 * 32 bytes
        struct_data[0..32].copy_from_slice(type_hash.as_slice());
        struct_data[32..64].copy_from_slice(&tab_id_bytes);
        struct_data[64..96].copy_from_slice(&U256::from(total_charged_units).to_be_bytes::<32>());
        struct_data[96..128].copy_from_slice(&U256::from(call_count).to_be_bytes::<32>());
        let struct_hash = keccak256(struct_data);

        // Final digest: \x19\x01 || domainSeparator || structHash
        let mut digest_input = [0u8; 66];
        digest_input[0] = 0x19;
        digest_input[1] = 0x01;
        digest_input[2..34].copy_from_slice(domain_separator.as_slice());
        digest_input[34..66].copy_from_slice(struct_hash.as_slice());
        let digest = keccak256(digest_input);

        let sig = signer
            .sign_hash_sync(&digest)
            .expect("signing should succeed");
        format!("0x{}", hex::encode(sig.as_bytes()))
    }
}

// ── Funding (via Remit API) ─────────────────────────────────────────────────

/// Fetch contract addresses from the API.
pub async fn get_contracts() -> Value {
    let url = api_url();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{url}/contracts"))
        .send()
        .await
        .expect("/contracts request failed");
    resp.json::<Value>()
        .await
        .expect("parse /contracts response")
}

/// Mint testnet USDC to a wallet via the /mint API (unauthenticated).
pub async fn mint_usdc(address: &str, amount: f64) {
    let url = api_url();
    acceptance_log!("mint: {amount} USDC -> {address}");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{url}/mint"))
        .json(&serde_json::json!({ "wallet": address, "amount": amount }))
        .send()
        .await
        .expect("mint request failed");

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert!(status.is_success(), "POST /mint failed ({status}): {body}");

    if let Ok(json) = serde_json::from_str::<Value>(&body) {
        if let Some(tx) = json["tx_hash"].as_str() {
            acceptance_log!("  mint tx_hash: {tx}");
        }
    }
}

/// Assert a balance changed by the expected delta (within tolerance).
pub fn assert_balance_change(
    label: &str,
    before: f64,
    after: f64,
    expected_delta: f64,
    tolerance_bps: u32,
) {
    let actual_delta = after - before;
    let tolerance = expected_delta.abs() * (tolerance_bps as f64 / 10_000.0);
    let diff = (actual_delta - expected_delta).abs();

    assert!(
        diff <= tolerance,
        "{label}: expected delta {expected_delta}, got {actual_delta} \
         (before={before}, after={after}, tolerance={tolerance})"
    );
}
