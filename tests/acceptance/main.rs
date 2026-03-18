//! CLI acceptance tests — run against live Base Sepolia API.
//!
//! These tests are marked `#[ignore]` so they don't run in normal CI.
//! Run with: `cargo test --test acceptance -- --ignored`
//! Or in CI: the acceptance workflow sets the flag.

mod harness;

use harness::{get_usdc_balance, mint_usdc, TestWallet};

// ── Harness verification ────────────────────────────────────────────────────

#[test]
#[ignore]
fn wallet_generation_produces_valid_address() {
    let wallet = TestWallet::generate();
    assert!(
        wallet.address.starts_with("0x"),
        "address must start with 0x"
    );
    assert_eq!(
        wallet.address.len(),
        42,
        "address must be 42 chars (0x + 40 hex)"
    );
    assert!(
        wallet.private_key.starts_with("0x"),
        "key must start with 0x"
    );
    assert_eq!(
        wallet.private_key.len(),
        66,
        "key must be 66 chars (0x + 64 hex)"
    );
}

#[tokio::test]
#[ignore]
async fn mint_funds_wallet_and_balance_reflects() {
    let wallet = TestWallet::generate();
    let before = get_usdc_balance(&wallet.address).await;
    assert!(
        (before - 0.0).abs() < 0.01,
        "fresh wallet should have 0 USDC"
    );

    mint_usdc(&wallet.address, 100.0).await;
    let after = harness::wait_for_balance_change(&wallet.address, before).await;
    assert!(
        after >= 99.99,
        "wallet should have ~100 USDC after mint, got {after}"
    );
}

#[tokio::test]
#[ignore]
async fn cli_mint_command_works() {
    let wallet = TestWallet::generate();
    let output = wallet.run_cli(&["mint", "50"]);
    assert!(output.success, "remit mint failed: {}", output.stderr,);
    let json = output.json();
    assert!(json["tx_hash"].is_string(), "response should have tx_hash");
}

#[tokio::test]
#[ignore]
async fn cli_status_returns_wallet_info() {
    let wallet = TestWallet::generate();
    mint_usdc(&wallet.address, 100.0).await;
    harness::wait_for_balance_change(&wallet.address, 0.0).await;

    let json = wallet.run_cli_json(&["status"]);
    assert_eq!(
        json["wallet"].as_str().unwrap().to_lowercase(),
        wallet.address.to_lowercase()
    );
    assert!(json["balance"].is_string(), "status should include balance");
}
