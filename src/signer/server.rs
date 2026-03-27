//! HTTP server for the local signer.
//!
//! Security invariants:
//! - Binds to 127.0.0.1 ONLY (invariant I6)
//! - Bearer token auth on every endpoint except GET /health (invariant I1)
//! - Policy evaluation BEFORE key decryption (invariant I3)
//! - Key material NEVER in responses (invariant I5)
//! - Concurrent sign requests serialized via mutex (invariant I8)
#![deny(unsafe_code)]

use crate::signer::{eip712, keystore, policy, tokens};
use alloy::signers::SignerSync;
use anyhow::{anyhow, Result};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Shared state ───────────────────────────────────────────────────────────

/// Shared application state for the axum server.
pub struct SignerState {
    pub keystore: keystore::Keystore,
    pub token_store: tokens::TokenStore,
    pub policy_store: policy::PolicyStore,
    /// Name of the wallet this signer manages.
    pub wallet_name: String,
    /// Cached wallet address (plaintext from key file, no decryption needed).
    pub address: String,
    /// Raw token used as passphrase for key decryption (single-secret mode).
    /// Stored in memory for the server's lifetime.
    passphrase: String,
    /// Signing mutex — serializes decrypt-sign-zeroize cycles.
    sign_lock: Mutex<()>,
    /// CLI version string for /health.
    pub version: String,
}

impl SignerState {
    /// Create a new signer state.
    pub fn new(
        keystore: keystore::Keystore,
        token_store: tokens::TokenStore,
        policy_store: policy::PolicyStore,
        wallet_name: String,
        address: String,
        passphrase: String,
        version: String,
    ) -> Self {
        Self {
            keystore,
            token_store,
            policy_store,
            wallet_name,
            address,
            passphrase,
            sign_lock: Mutex::new(()),
            version,
        }
    }
}

pub type AppState = Arc<SignerState>;

// ── Response types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    version: String,
    wallet: String,
}

#[derive(Serialize)]
struct AddressResponse {
    address: String,
}

#[derive(Serialize)]
struct SignatureResponse {
    signature: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

// ── Request types ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DigestRequest {
    digest: String,
}

// ── Router ─────────────────────────────────────────────────────────────────

/// Build the axum router for the signer server.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/address", get(get_address))
        .route("/sign/typed-data", post(sign_typed_data))
        .route("/sign/digest", post(sign_digest))
        .with_state(state)
}

/// Start the signer HTTP server on the given port.
///
/// Binds to `127.0.0.1:{port}` — NEVER `0.0.0.0` (invariant I6).
pub async fn start(state: AppState, port: u16) -> Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow!("cannot bind to {addr}: {e}"))?;

    eprintln!("Signer listening on http://{addr}");
    eprintln!("Wallet: {} ({})", state.wallet_name, state.address);

    axum::serve(listener, router(state))
        .await
        .map_err(|e| anyhow!("server error: {e}"))
}

// ── Handlers ───────────────────────────────────────────────────────────────

/// GET /health — no auth required.
async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: state.version.clone(),
        wallet: state.wallet_name.clone(),
    })
}

/// GET /address — requires auth.
async fn get_address(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AddressResponse>, (StatusCode, Json<ErrorResponse>)> {
    authenticate(&state, &headers)?;
    Ok(Json(AddressResponse {
        address: state.address.clone(),
    }))
}

/// POST /sign/typed-data — requires auth, content-aware policy.
async fn sign_typed_data(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<eip712::TypedDataRequest>,
) -> Result<Json<SignatureResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 1. Auth (invariant I1)
    authenticate(&state, &headers)?;

    // 2. Policy evaluation BEFORE key decryption (invariant I3)
    let mut policy_file = state
        .policy_store
        .load("default")
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "policy_error", None))?;

    let ctx = policy::TypedDataContext {
        chain_id: eip712::extract_chain_id(&request.domain).unwrap_or(0),
        contract: eip712::extract_contract(&request.value),
        amount_usdc: eip712::extract_amount_usdc(&request.value),
    };

    if let Err(denial) = policy::evaluate_typed_data(&mut policy_file, &ctx) {
        return Err(err(
            StatusCode::FORBIDDEN,
            "policy_denied",
            Some(denial.reason),
        ));
    }

    // 3. Hash the typed data into a 32-byte digest
    let digest = eip712::hash_typed_data(&request).map_err(|e| {
        err(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            Some(e.to_string()),
        )
    })?;

    // 4. Decrypt key, sign, zeroize (under mutex)
    let signature = sign_with_key(&state, &digest).await?;

    // 5. Record spend (after successful sign)
    if let Some(amount) = ctx.amount_usdc {
        policy::record_spend(&mut policy_file, amount);
        let _ = state.policy_store.save(&policy_file);
    }

    Ok(Json(SignatureResponse { signature }))
}

/// POST /sign/digest — requires auth, rate-limit policy only.
async fn sign_digest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DigestRequest>,
) -> Result<Json<SignatureResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 1. Auth
    authenticate(&state, &headers)?;

    // 2. Rate-limit policy (digest is opaque)
    if let Ok(pf) = state.policy_store.load("default") {
        if let Err(denial) = policy::evaluate_digest(&pf) {
            return Err(err(
                StatusCode::FORBIDDEN,
                "policy_denied",
                Some(denial.reason),
            ));
        }
    }

    // 3. Parse digest (must be exactly 32 bytes)
    let digest = parse_digest(&request.digest)?;

    // 4. Sign
    let signature = sign_with_key(&state, &digest).await?;

    Ok(Json(SignatureResponse { signature }))
}

// ── Auth ───────────────────────────────────────────────────────────────────

/// Extract and validate the bearer token from request headers.
fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<tokens::ValidatedToken, (StatusCode, Json<ErrorResponse>)> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized", None))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized", None))?;

    state
        .token_store
        .validate(token)
        .map_err(|_| err(StatusCode::UNAUTHORIZED, "unauthorized", None))
}

// ── Signing ────────────────────────────────────────────────────────────────

/// Decrypt key, sign a 32-byte digest, zeroize, return hex signature.
///
/// Uses a mutex to serialize signing operations (invariant I8).
async fn sign_with_key(
    state: &AppState,
    digest: &[u8; 32],
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let _lock = state.sign_lock.lock().await;

    let key_file = state.keystore.load(&state.wallet_name).map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "key_error",
            Some("cannot load key file".to_string()),
        )
    })?;

    // Decrypt key using passphrase (= raw bearer token in single-secret mode).
    // The PrivateKeySigner internally zeroizes the key on drop.
    let signer = keystore::decrypt(&key_file, &state.passphrase).map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "key_error",
            Some("cannot decrypt key".to_string()),
        )
    })?;

    let sig = signer.sign_hash_sync(&(*digest).into()).map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "sign_error",
            Some("ECDSA signing failed".to_string()),
        )
    })?;

    // 0x-prefixed hex: 65 bytes = 130 hex chars
    Ok(format!("0x{}", hex::encode(sig.as_bytes())))
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Parse a hex-encoded digest into exactly 32 bytes.
fn parse_digest(hex_str: &str) -> Result<[u8; 32], (StatusCode, Json<ErrorResponse>)> {
    let clean = hex_str.trim_start_matches("0x");
    let bytes = hex::decode(clean).map_err(|_| {
        err(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            Some("digest must be valid hex".to_string()),
        )
    })?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| {
        err(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            Some("digest must be exactly 32 bytes".to_string()),
        )
    })?;
    Ok(arr)
}

/// Build a standard error response tuple.
fn err(
    status: StatusCode,
    error: &str,
    reason: Option<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
            reason,
        }),
    )
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Create a test signer state with a real keystore, token store, and policy.
    fn test_state() -> (AppState, String) {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().to_path_buf();

        let ks = keystore::Keystore::open_in(base.join("keys"));
        let ts = tokens::TokenStore::open_in(base.join("tokens"));
        let ps = policy::PolicyStore::open_in(base.join("policies"));

        // Generate a key and token
        let passphrase = "test-passphrase";
        let token = ts.create("test-token", "test-wallet").unwrap();

        // We use a fixed passphrase (not the token) for testing simplicity
        let address = ks.generate("test-wallet", passphrase).unwrap();
        ps.create_default("test-wallet", "base").unwrap();

        let state = Arc::new(SignerState::new(
            ks,
            ts,
            ps,
            "test-wallet".to_string(),
            address,
            passphrase.to_string(),
            "0.3.0-test".to_string(),
        ));

        // Leak the TempDir so it persists for the test
        std::mem::forget(dir);

        (state, token)
    }

    #[tokio::test]
    async fn health_requires_no_auth() {
        let (state, _token) = test_state();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["wallet"], "test-wallet");
    }

    #[tokio::test]
    async fn address_requires_auth() {
        let (state, _token) = test_state();
        let app = router(state);

        // No auth header → 401
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/address")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn address_with_valid_token() {
        let (state, token) = test_state();
        let app = router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/address")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["address"].as_str().unwrap().starts_with("0x"));
    }

    #[tokio::test]
    async fn sign_digest_works() {
        let (state, token) = test_state();
        let app = router(state);

        let digest = "0x".to_string() + &"ab".repeat(32);
        let body = serde_json::json!({ "digest": digest });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign/digest")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sig = json["signature"].as_str().unwrap();
        assert!(sig.starts_with("0x"));
        // 0x + 130 hex chars (65 bytes)
        assert_eq!(sig.len(), 132);
    }

    #[tokio::test]
    async fn sign_digest_bad_length() {
        let (state, token) = test_state();
        let app = router(state);

        let body = serde_json::json!({ "digest": "0xdeadbeef" });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign/digest")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_digest_no_auth() {
        let (state, _token) = test_state();
        let app = router(state);

        let digest = "0x".to_string() + &"ab".repeat(32);
        let body = serde_json::json!({ "digest": digest });

        let response = app
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

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
