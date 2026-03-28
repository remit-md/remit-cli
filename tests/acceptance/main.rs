//! CLI acceptance tests — run against live Base Sepolia API.
//!
//! All operations go through the Remit API (via CLI or HTTP).
//! No direct RPC calls — balances come from `remit balance`, not eth_call.
//!
//! A single wallet pair (payer + provider) is shared across all flow tests.
//! Tests run sequentially (`--test-threads=1`) with proper nonce ordering.
//!
//! Run with: `cargo test --test acceptance -- --ignored --test-threads=1`

#[macro_use]
mod harness;

use harness::{get_contracts, mint_usdc, shared_wallets, TestWallet};

// ── 1. Wallet generation (standalone) ───────────────────────────────────────

#[test]
#[ignore]
fn t01_wallet_generation() {
    acceptance_log!("=== TEST: t01_wallet_generation ===");
    let wallet = TestWallet::generate();
    assert!(wallet.address.starts_with("0x"));
    assert_eq!(wallet.address.len(), 42);
    assert!(wallet.private_key.starts_with("0x"));
    assert_eq!(wallet.private_key.len(), 66);
    acceptance_log!("PASS");
}

// ── 2. Mint + balance (funds the shared payer) ──────────────────────────────

#[tokio::test]
#[ignore]
async fn t02_mint_and_balance() {
    acceptance_log!("=== TEST: t02_mint_and_balance ===");
    let (payer, _) = shared_wallets();

    mint_usdc(&payer.address, 200.0).await;
    let bal = payer.wait_for_balance(0.0).await;
    assert!(bal >= 199.99, "payer should have ~200 USDC, got {bal}");
    acceptance_log!("PASS: payer funded with 200 USDC");
}

// ── 3. Status ───────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t03_status() {
    acceptance_log!("=== TEST: t03_status ===");
    let (payer, _) = shared_wallets();

    let json = payer.run_cli_json(&["status"]);
    assert_eq!(
        json["wallet"].as_str().unwrap().to_lowercase(),
        payer.address.to_lowercase()
    );
    assert!(json["balance"].is_string(), "status should include balance");
    acceptance_log!(
        "status: wallet={}, balance={}",
        json["wallet"],
        json["balance"]
    );
    acceptance_log!("PASS");
}

// ── 4. Direct payment ──────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t04_pay_direct() {
    acceptance_log!("=== TEST: t04_pay_direct ===");
    let (payer, provider) = shared_wallets();

    let payer_before = payer.balance();
    acceptance_log!(
        "direct | 10.00 USDC {} -> {}",
        payer.address,
        provider.address
    );
    let json = payer.run_cli_json(&["pay", &provider.address, "10.00"]);
    let status = json["status"].as_str().unwrap();
    assert!(
        status == "confirmed" || status == "pending",
        "expected confirmed or pending, got {status}"
    );
    assert!(json["tx_hash"].is_string(), "should have tx_hash");

    // Verify payer balance decreased
    let payer_after = payer.wait_for_balance(payer_before).await;
    harness::assert_balance_change("payer", payer_before, payer_after, -10.0, 100);

    // Verify provider received funds
    let provider_balance = provider.balance();
    assert!(
        provider_balance >= 9.89,
        "provider should have ~10 USDC (minus fee), got {provider_balance}"
    );
    acceptance_log!("PASS");
}

// ── 5. Escrow lifecycle ────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t05_escrow_lifecycle() {
    acceptance_log!("=== TEST: t05_escrow_lifecycle ===");
    let (payer, provider) = shared_wallets();

    acceptance_log!(
        "escrow | create 5.00 USDC {} -> {}",
        payer.address,
        provider.address
    );
    let create_json = payer.run_cli_json(&["escrow", "create", &provider.address, "5.00"]);
    let escrow_id = create_json["invoice_id"]
        .as_str()
        .expect("should have invoice_id");
    assert_eq!(create_json["status"].as_str().unwrap(), "funded");

    // Claim start (provider signals work done)
    acceptance_log!("escrow | claim-start {escrow_id}");
    let claim_json = provider.run_cli_json(&["escrow", "claim-start", escrow_id]);
    assert!(
        claim_json["claim_started"].as_bool().unwrap_or(false)
            || claim_json["status"].as_str().unwrap() == "claim_started"
    );

    // Release (payer approves)
    acceptance_log!("escrow | release {escrow_id}");
    let release_json = payer.run_cli_json(&["escrow", "release", escrow_id]);
    let status = release_json["status"].as_str().unwrap();
    assert!(
        status == "released" || status == "settled" || status == "completed",
        "expected released/settled/completed, got {status}"
    );
    acceptance_log!("PASS");
}

// ── 6. Tab lifecycle (open → charge → close) ──────────────────────────────

#[tokio::test]
#[ignore]
async fn t06_tab_lifecycle() {
    acceptance_log!("=== TEST: t06_tab_lifecycle ===");
    let (payer, provider) = shared_wallets();

    // Open tab
    acceptance_log!(
        "tab | open limit=20.00 per-unit=2.50 {} -> {}",
        payer.address,
        provider.address
    );
    let open_json = payer.run_cli_json(&[
        "tab",
        "open",
        &provider.address,
        "20.00",
        "--per-unit",
        "2.50",
    ]);
    let tab_id = open_json["id"].as_str().expect("should have tab id");
    assert_eq!(open_json["status"].as_str().unwrap(), "open");

    // Get tab contract address for EIP-712 signing
    let contracts = get_contracts().await;
    let tab_contract = contracts["tab"]
        .as_str()
        .expect("should have tab contract address");

    // Provider signs TabCharge EIP-712 for charge
    let charge_units: u64 = 2_500_000; // 2.50 USDC in 6-decimal units
    let call_count: u32 = 1;
    let charge_sig = provider.sign_tab_charge(tab_contract, tab_id, charge_units, call_count);

    // Charge tab (provider charges 2.50 with EIP-712 sig)
    acceptance_log!("tab | charge {tab_id} 2.50");
    let charge_json = provider.run_cli_json(&[
        "tab",
        "charge",
        tab_id,
        "2.50",
        "--cumulative",
        "2.50",
        "--call-count",
        "1",
    ]);
    acceptance_log!(
        "tab | charged: {} cumulative: {}",
        charge_json["amount"],
        charge_json["cumulative"]
    );

    // Provider signs final state for close (same cumulative)
    let close_sig = provider.sign_tab_charge(tab_contract, tab_id, charge_units, call_count);

    // Close tab (payer closes with provider's EIP-712 signature)
    acceptance_log!("tab | close {tab_id} final_amount=2.50");
    let close_json = payer.run_cli_json(&[
        "tab",
        "close",
        tab_id,
        "--final-amount",
        "2.50",
        "--provider-sig",
        &close_sig,
    ]);
    let status = close_json["status"].as_str().unwrap();
    assert!(
        status == "closed" || status == "settled",
        "expected closed or settled, got {status}"
    );
    acceptance_log!("PASS");
}

// ── 7. Stream lifecycle ────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t07_stream_lifecycle() {
    acceptance_log!("=== TEST: t07_stream_lifecycle ===");
    let (payer, provider) = shared_wallets();

    // Open stream: 0.01 USDC/sec, max 5 USDC
    acceptance_log!(
        "stream | open rate=0.01/s max=5.00 {} -> {}",
        payer.address,
        provider.address
    );
    let open_json = payer.run_cli_json(&["stream", "open", &provider.address, "0.01", "5.00"]);
    let stream_id = open_json["id"].as_str().expect("should have stream id");
    assert_eq!(open_json["status"].as_str().unwrap(), "active");

    // Wait for accrual
    acceptance_log!("stream | waiting 3s for accrual...");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Close stream
    acceptance_log!("stream | close {stream_id}");
    let close_json = payer.run_cli_json(&["stream", "close", stream_id]);
    let status = close_json["status"].as_str().unwrap();
    assert!(
        status == "closed" || status == "settled",
        "expected closed or settled, got {status}"
    );
    acceptance_log!("PASS");
}

// ── 8. Bounty lifecycle ────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t08_bounty_lifecycle() {
    acceptance_log!("=== TEST: t08_bounty_lifecycle ===");
    let (payer, provider) = shared_wallets();

    // Post bounty (payer)
    acceptance_log!("bounty | post 10.00 USDC from {}", payer.address);
    let post_json = payer.run_cli_json(&["bounty", "post", "10.00", "test bounty from CLI"]);
    let bounty_id = post_json["id"].as_str().expect("should have bounty id");
    assert_eq!(post_json["status"].as_str().unwrap(), "open");

    // Submit (provider provides evidence — 32-byte hash)
    let evidence = format!("0x{}", "ab".repeat(32));
    acceptance_log!("bounty | submit {bounty_id} from {}", provider.address);
    let submit_json = provider.run_cli_json(&["bounty", "submit", bounty_id, &evidence]);
    let submission_id = submit_json["id"]
        .as_i64()
        .or_else(|| submit_json["id"].as_u64().map(|v| v as i64))
        .expect("should have submission id");

    // Award (payer awards to provider)
    acceptance_log!("bounty | award {bounty_id} submission={submission_id}");
    let award_json =
        payer.run_cli_json(&["bounty", "award", bounty_id, &submission_id.to_string()]);
    let status = award_json["status"].as_str().unwrap();
    assert!(
        status == "awarded" || status == "settled",
        "expected awarded or settled, got {status}"
    );
    acceptance_log!("PASS");
}

// ── 9. Deposit ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t09_deposit_create() {
    acceptance_log!("=== TEST: t09_deposit_create ===");
    let (payer, provider) = shared_wallets();

    acceptance_log!(
        "deposit | create 15.00 USDC {} -> {}",
        payer.address,
        provider.address
    );
    let json = payer.run_cli_json(&["deposit", "create", &provider.address, "15.00"]);
    assert!(json["id"].is_string(), "should have deposit id");
    let status = json["status"].as_str().unwrap();
    assert!(
        status == "active" || status == "locked",
        "expected active or locked, got {status}"
    );
    acceptance_log!("PASS");
}

// ── 10. CLI mint (separate wallet) ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn t10_cli_mint() {
    acceptance_log!("=== TEST: t10_cli_mint ===");
    let wallet = TestWallet::generate();
    let output = wallet.run_cli(&["mint", "50"]);
    assert!(output.success, "remit mint failed: {}", output.stderr);
    let json = output.json();
    assert!(json["tx_hash"].is_string(), "response should have tx_hash");
    acceptance_log!("PASS");
}
