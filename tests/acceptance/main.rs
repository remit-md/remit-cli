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

// ── Pay (direct payment) ────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cli_pay_direct_with_auto_permit() {
    let payer = TestWallet::generate();
    let recipient = TestWallet::generate();

    mint_usdc(&payer.address, 100.0).await;
    harness::wait_for_balance_change(&payer.address, 0.0).await;

    let payer_before = get_usdc_balance(&payer.address).await;
    let recipient_before = get_usdc_balance(&recipient.address).await;
    let fee_before = harness::get_fee_wallet_balance().await;

    let json = payer.run_cli_json(&["pay", &recipient.address, "10.00"]);
    assert_eq!(json["status"].as_str().unwrap(), "confirmed");
    assert!(json["tx_hash"].is_string(), "should have tx_hash");

    let payer_after = harness::wait_for_balance_change(&payer.address, payer_before).await;
    let recipient_after = get_usdc_balance(&recipient.address).await;
    let fee_after = harness::get_fee_wallet_balance().await;

    harness::assert_balance_change("payer", payer_before, payer_after, -10.10, 100);
    harness::assert_balance_change("recipient", recipient_before, recipient_after, 10.0, 10);
    harness::assert_balance_change("fee_wallet", fee_before, fee_after, 0.10, 100);
}

// ── Escrow lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cli_escrow_create_claim_start_release() {
    let payer = TestWallet::generate();
    let payee = TestWallet::generate();

    mint_usdc(&payer.address, 100.0).await;
    harness::wait_for_balance_change(&payer.address, 0.0).await;

    // Create escrow
    let create_json = payer.run_cli_json(&["escrow", "create", &payee.address, "5.00"]);
    let escrow_id = create_json["invoice_id"]
        .as_str()
        .expect("should have invoice_id");
    assert_eq!(create_json["status"].as_str().unwrap(), "funded");

    // Claim start (payee signals work done)
    let claim_json = payee.run_cli_json(&["escrow", "claim-start", escrow_id]);
    assert!(
        claim_json["claim_started"].as_bool().unwrap_or(false)
            || claim_json["status"].as_str().unwrap() == "claim_started"
    );

    // Release (payer approves)
    let release_json = payer.run_cli_json(&["escrow", "release", escrow_id]);
    assert!(
        release_json["status"].as_str().unwrap() == "released"
            || release_json["status"].as_str().unwrap() == "settled"
    );

    // Verify payee received funds
    let payee_balance = get_usdc_balance(&payee.address).await;
    assert!(
        payee_balance >= 4.9,
        "payee should have ~5 USDC, got {payee_balance}"
    );
}

// ── Tab lifecycle ───────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cli_tab_open_close() {
    let payer = TestWallet::generate();
    let provider = TestWallet::generate();

    mint_usdc(&payer.address, 100.0).await;
    harness::wait_for_balance_change(&payer.address, 0.0).await;

    // Open tab
    let open_json = payer.run_cli_json(&["tab", "open", &provider.address, "20.00"]);
    let tab_id = open_json["id"].as_str().expect("should have tab id");
    assert_eq!(open_json["status"].as_str().unwrap(), "open");

    // Close tab (provider settles)
    let close_json = provider.run_cli_json(&["tab", "close", tab_id]);
    assert!(
        close_json["status"].as_str().unwrap() == "closed"
            || close_json["status"].as_str().unwrap() == "settled"
    );
}

// ── Stream lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cli_stream_open_close() {
    let payer = TestWallet::generate();
    let payee = TestWallet::generate();

    mint_usdc(&payer.address, 100.0).await;
    harness::wait_for_balance_change(&payer.address, 0.0).await;

    // Open stream: 0.01 USDC/sec, max 5 USDC
    let open_json = payer.run_cli_json(&["stream", "open", &payee.address, "0.01", "5.00"]);
    let stream_id = open_json["id"].as_str().expect("should have stream id");
    assert_eq!(open_json["status"].as_str().unwrap(), "active");

    // Wait briefly for some accrual
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Close stream (payer)
    let close_json = payer.run_cli_json(&["stream", "close", stream_id]);
    assert!(
        close_json["status"].as_str().unwrap() == "closed"
            || close_json["status"].as_str().unwrap() == "settled"
    );
}

// ── Bounty lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cli_bounty_post_submit_award() {
    let poster = TestWallet::generate();
    let submitter = TestWallet::generate();

    mint_usdc(&poster.address, 100.0).await;
    harness::wait_for_balance_change(&poster.address, 0.0).await;

    // Post bounty
    let post_json = poster.run_cli_json(&["bounty", "post", "10.00", "test bounty from CLI"]);
    let bounty_id = post_json["id"].as_str().expect("should have bounty id");
    assert_eq!(post_json["status"].as_str().unwrap(), "open");

    // Submit (submitter provides evidence)
    let submit_json =
        submitter.run_cli_json(&["bounty", "submit", bounty_id, "0xdeadbeefcafebabe"]);
    let submission_id = submit_json["id"]
        .as_i64()
        .or_else(|| submit_json["id"].as_u64().map(|v| v as i64))
        .expect("should have submission id");

    // Award (poster awards to submitter)
    let award_json =
        poster.run_cli_json(&["bounty", "award", bounty_id, &submission_id.to_string()]);
    assert!(
        award_json["status"].as_str().unwrap() == "awarded"
            || award_json["status"].as_str().unwrap() == "settled"
    );

    // Verify submitter received funds
    let submitter_balance = get_usdc_balance(&submitter.address).await;
    assert!(
        submitter_balance >= 9.8,
        "submitter should have ~10 USDC, got {submitter_balance}"
    );
}

// ── Deposit lifecycle ───────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cli_deposit_create() {
    let depositor = TestWallet::generate();
    let provider = TestWallet::generate();

    mint_usdc(&depositor.address, 100.0).await;
    harness::wait_for_balance_change(&depositor.address, 0.0).await;

    // Create deposit
    let json = depositor.run_cli_json(&["deposit", "create", &provider.address, "15.00"]);
    assert!(json["id"].is_string(), "should have deposit id");
    assert_eq!(json["status"].as_str().unwrap(), "active");
}
