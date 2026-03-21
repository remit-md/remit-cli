#![allow(dead_code)]
//! CLI acceptance test harness.
//!
//! Provides helpers for running the `remit` binary against the live Base Sepolia API,
//! generating test wallets, minting USDC, and reading on-chain balances via RPC.

use serde_json::Value;
use std::process::Command;

// ── Config ──────────────────────────────────────────────────────────────────

pub const API_URL: &str = "https://remit.md/api/v1";
pub const RPC_URL: &str = "https://sepolia.base.org";
pub const USDC_ADDRESS: &str = "0x142aD61B8d2edD6b3807D9266866D97C35Ee0317";
pub const FEE_WALLET: &str = "0xd3f721BDF92a2bB5Dd8d2FE2AFC03aFE5629B420";

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

        let output = Command::new(bin)
            .args(&full_args)
            .env("REMITMD_KEY", &self.private_key)
            .output()
            .expect("failed to execute remit binary");

        CliOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        }
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
}

// ── Funding (direct HTTP, bypasses CLI) ─────────────────────────────────────

/// Mint testnet USDC to a wallet via the /mint API (unauthenticated).
pub async fn mint_usdc(address: &str, amount: f64) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{API_URL}/mint"))
        .json(&serde_json::json!({ "wallet": address, "amount": amount }))
        .send()
        .await
        .expect("mint request failed");

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert!(status.is_success(), "POST /mint failed ({status}): {body}");
}

// ── On-chain balance via RPC ────────────────────────────────────────────────

/// Read USDC balance via RPC `eth_call` to `balanceOf(address)`. Returns USD.
pub async fn get_usdc_balance(address: &str) -> f64 {
    let padded = address.to_lowercase().trim_start_matches("0x").to_string();
    let padded = format!("{:0>64}", padded);
    // balanceOf(address) selector = 0x70a08231
    let data = format!("0x70a08231{padded}");

    let client = reqwest::Client::new();
    let resp = client
        .post(RPC_URL)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{ "to": USDC_ADDRESS, "data": data }, "latest"],
        }))
        .send()
        .await
        .expect("RPC call failed");

    let json: Value = resp.json().await.expect("RPC response parse failed");
    if let Some(err) = json.get("error") {
        panic!("RPC balanceOf error: {err}");
    }

    let hex_result = json["result"].as_str().unwrap_or("0x0");
    let raw = u64::from_str_radix(hex_result.trim_start_matches("0x"), 16).unwrap_or(0);
    raw as f64 / 1_000_000.0
}

/// Read the fee wallet's USDC balance.
pub async fn get_fee_wallet_balance() -> f64 {
    get_usdc_balance(FEE_WALLET).await
}

/// Poll until balance differs from `before_balance`. Returns the new balance.
pub async fn wait_for_balance_change(address: &str, before_balance: f64) -> f64 {
    let max_wait_ms: u64 = 30_000;
    let poll_interval_ms: u64 = 2_000;
    let start = std::time::Instant::now();

    while start.elapsed().as_millis() < max_wait_ms as u128 {
        let current = get_usdc_balance(address).await;
        if (current - before_balance).abs() > 0.0001 {
            return current;
        }
        tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
    }

    get_usdc_balance(address).await
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
