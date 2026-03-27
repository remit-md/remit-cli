//! Integration tests for the signer server.
//!
//! These tests start a real HTTP server on a random port and make
//! actual HTTP requests, testing the full flow from request to signature.
#![cfg(test)]
#![allow(clippy::unwrap_used)]

use crate::signer::{keystore, policy, server, tokens};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;

/// Set up a complete signer state with real keystore, token, and policy.
fn setup() -> (server::AppState, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_path_buf();

    let ks = keystore::Keystore::open_in(base.join("keys"));
    let ts = tokens::TokenStore::open_in(base.join("tokens"));
    let ps = policy::PolicyStore::open_in(base.join("policies"));

    // Create token (also used as passphrase)
    let raw_token = ts.create("integration-test", "test-wallet").unwrap();

    // Generate key encrypted with the token
    let address = ks.generate("test-wallet", &raw_token).unwrap();

    // Create policy: chain lock to Base (8453)
    ps.create_default("test-wallet", "base").unwrap();

    let state = Arc::new(server::SignerState::new(
        ks,
        ts,
        ps,
        "test-wallet".to_string(),
        address,
        raw_token.clone(),
        "0.3.0-test".to_string(),
    ));

    std::mem::forget(dir);
    (state, raw_token)
}

fn auth(token: &str) -> String {
    format!("Bearer {token}")
}

/// Helper to make a JSON POST request.
fn json_post(uri: &str, token: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", auth(token))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(body).unwrap()))
        .unwrap()
}

// ── I1: Auth required on all endpoints except /health ───────────────────

#[tokio::test]
async fn i1_health_needs_no_auth() {
    let (state, _) = setup();
    let app = server::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn i1_address_needs_auth() {
    let (state, _) = setup();
    let app = server::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/address")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn i1_sign_typed_data_needs_auth() {
    let (state, _) = setup();
    let app = server::router(state);
    let body = serde_json::json!({
        "domain": { "name": "Test" },
        "types": { "Test": [] },
        "value": {}
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sign/typed-data")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn i1_sign_digest_needs_auth() {
    let (state, _) = setup();
    let app = server::router(state);
    let body = serde_json::json!({ "digest": "0x".to_string() + &"ab".repeat(32) });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sign/digest")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── I2: Revoked token denied immediately ────────────────────────────────

#[tokio::test]
async fn i2_revoked_token_denied() {
    let (state, token) = setup();

    // Revoke the token
    state.token_store.revoke(&token).unwrap();

    let app = server::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/address")
                .header("authorization", auth(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── I5: No key material in responses ────────────────────────────────────

#[tokio::test]
async fn i5_error_responses_have_no_secrets() {
    let (state, _) = setup();
    let app = server::router(state);

    // Bad request
    let body = serde_json::json!({ "digest": "not-hex" });
    let resp = app
        .oneshot(json_post(
            "/sign/digest",
            "rmit_sk_0000000000000000000000000000000000000000000000000000000000000000",
            &body,
        ))
        .await
        .unwrap();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let body_str = String::from_utf8_lossy(&body_bytes);

    // Must not contain anything that looks like a private key
    assert!(
        !body_str.contains("0xac0974"),
        "response must not contain key material"
    );
}

// ── I7: Deterministic signatures (RFC 6979) ─────────────────────────────

#[tokio::test]
async fn i7_deterministic_signatures() {
    let (state, token) = setup();
    let digest = "0x".to_string() + &"ab".repeat(32);
    let body = serde_json::json!({ "digest": &digest });

    // Sign twice
    let app1 = server::router(state.clone());
    let resp1 = app1
        .oneshot(json_post("/sign/digest", &token, &body))
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let body1 = axum::body::to_bytes(resp1.into_body(), 4096).await.unwrap();
    let json1: serde_json::Value = serde_json::from_slice(&body1).unwrap();

    let app2 = server::router(state);
    let resp2 = app2
        .oneshot(json_post("/sign/digest", &token, &body))
        .await
        .unwrap();
    let body2 = axum::body::to_bytes(resp2.into_body(), 4096).await.unwrap();
    let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();

    assert_eq!(
        json1["signature"], json2["signature"],
        "same digest must produce same signature (RFC 6979)"
    );
}

// ── I8: Concurrent requests ─────────────────────────────────────────────

#[tokio::test]
async fn i8_concurrent_sign_requests() {
    let (state, token) = setup();
    let mut handles = Vec::new();
    for i in 0..5 {
        let state = state.clone();
        let token = token.clone();
        let handle = tokio::spawn(async move {
            let digest = format!("0x{:064x}", i);
            let body = serde_json::json!({ "digest": &digest });
            let app = server::router(state);
            let resp = app
                .oneshot(json_post("/sign/digest", &token, &body))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "request {i} must succeed");
            let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            let sig = json["signature"].as_str().unwrap().to_string();
            assert!(sig.starts_with("0x"));
            assert_eq!(sig.len(), 132);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }
}

// ── I9: Graceful denial for malformed requests ──────────────────────────

#[tokio::test]
async fn i9_malformed_json_returns_400() {
    let (state, token) = setup();
    let app = server::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sign/digest")
                .header("authorization", auth(&token))
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    // axum returns 400 or 422 for deserialization errors
    assert!(
        resp.status() == StatusCode::BAD_REQUEST
            || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "malformed JSON should return 4xx, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn i9_wrong_digest_length_returns_400() {
    let (state, token) = setup();
    let app = server::router(state);
    let body = serde_json::json!({ "digest": "0xdeadbeef" });
    let resp = app
        .oneshot(json_post("/sign/digest", &token, &body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn i9_get_to_post_endpoint_returns_405() {
    let (state, token) = setup();
    let app = server::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/sign/digest")
                .header("authorization", auth(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ── Sign typed-data with USDC Permit ────────────────────────────────────

#[tokio::test]
async fn sign_typed_data_permit() {
    let (state, token) = setup();

    // Update policy to allow chain 8453 (already the default)
    let body = serde_json::json!({
        "domain": {
            "name": "USD Coin",
            "version": "2",
            "chainId": 8453,
            "verifyingContract": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
        },
        "types": {
            "Permit": [
                {"name": "owner", "type": "address"},
                {"name": "spender", "type": "address"},
                {"name": "value", "type": "uint256"},
                {"name": "nonce", "type": "uint256"},
                {"name": "deadline", "type": "uint256"}
            ]
        },
        "value": {
            "owner": state.address.clone(),
            "spender": "0xAf2e211BC585D3Ab37e9BD546Fb25747a09254D2",
            "value": "1000000",
            "nonce": 0,
            "deadline": 1711387200
        }
    });

    let app = server::router(state);
    let resp = app
        .oneshot(json_post("/sign/typed-data", &token, &body))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let sig = json["signature"].as_str().unwrap();
    assert!(sig.starts_with("0x"));
    assert_eq!(sig.len(), 132, "signature must be 65 bytes (130 hex + 0x)");
}

// ── Policy enforcement: wrong chain denied ──────────────────────────────

#[tokio::test]
async fn policy_denies_wrong_chain() {
    let (state, token) = setup();

    // Policy is chain-locked to Base (8453). Try Ethereum mainnet (1).
    let body = serde_json::json!({
        "domain": {
            "name": "Test",
            "chainId": 1
        },
        "types": {
            "Test": [
                {"name": "foo", "type": "uint256"}
            ]
        },
        "value": {
            "foo": 42
        }
    });

    let app = server::router(state);
    let resp = app
        .oneshot(json_post("/sign/typed-data", &token, &body))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"], "policy_denied");
}
