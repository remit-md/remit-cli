// Client and types used by command handlers (tasks 0.4+).
#![allow(dead_code)]

/// Authenticated HTTP client for the Remit API.
///
/// Every request is signed via EIP-712 auth headers (auth module).
/// All endpoints return typed Rust structs. Amounts are USDC strings.
use anyhow::{anyhow, Context, Result};
use reqwest::{Client, Method, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::auth::{build_auth_headers, ChainConfig};

// ── Base URLs ─────────────────────────────────────────────────────────────────

pub const MAINNET_API: &str = "https://remit.md/api/v0";
pub const TESTNET_API: &str = "https://testnet.remit.md/api/v0";

// ── Client ────────────────────────────────────────────────────────────────────

pub struct RemitClient {
    http: Client,
    base: String,
    chain: ChainConfig,
}

impl RemitClient {
    pub fn new(testnet: bool) -> Self {
        let http = Client::builder()
            .user_agent(concat!("remit-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client");
        // Allow env var override (useful for acceptance tests pointing at remit.md)
        let base = std::env::var("REMITMD_API_URL")
            .unwrap_or_else(|_| (if testnet { TESTNET_API } else { MAINNET_API }).to_string());
        Self {
            http,
            base,
            chain: ChainConfig::for_network(testnet),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Execute an authenticated request, mapping HTTP errors to friendly messages.
    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let auth = build_auth_headers(
            method.as_str(),
            path,
            self.chain.chain_id,
            &self.chain.router,
        )
        .context("failed to build auth headers")?;

        let mut req: RequestBuilder = self.http.request(method, self.url(path));
        for (k, v) in &auth {
            req = req.header(k, v);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("Network error: {e}"))?;
        let status = resp.status();

        if status.is_success() {
            resp.json::<T>()
                .await
                .map_err(|e| anyhow!("Failed to parse server response: {e}"))
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v["message"]
                        .as_str()
                        .or(v["error"].as_str())
                        .map(str::to_owned)
                })
                .unwrap_or(body_text);
            Err(anyhow!("{status}: {msg}"))
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request(Method::GET, path, None).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> Result<T> {
        self.request(Method::POST, path, Some(body)).await
    }

    async fn delete_req(&self, path: &str) -> Result<()> {
        let auth = build_auth_headers("DELETE", path, self.chain.chain_id, &self.chain.router)
            .context("failed to build auth headers")?;

        let mut req = self.http.request(Method::DELETE, self.url(path));
        for (k, v) in &auth {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("Network error: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v["message"]
                        .as_str()
                        .or(v["error"].as_str())
                        .map(str::to_owned)
                })
                .unwrap_or(body_text);
            Err(anyhow!("{status}: {msg}"))
        }
    }

    // ── Public: contracts (unauthenticated) ────────────────────────────────────

    /// Fetch all deployed contract addresses. Unauthenticated — no key needed.
    pub async fn get_contracts(&self) -> Result<ContractsResponse> {
        let resp = self
            .http
            .get(self.url("/contracts"))
            .send()
            .await
            .map_err(|e| anyhow!("Network error: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            resp.json::<ContractsResponse>()
                .await
                .map_err(|e| anyhow!("Failed to parse /contracts response: {e}"))
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(anyhow!("GET /contracts failed ({status}): {body}"))
        }
    }

    // ── Public: status ────────────────────────────────────────────────────────

    pub async fn status(&self, wallet: &str) -> Result<WalletStatus> {
        self.get(&format!("/status/{wallet}")).await
    }

    // ── Public: health ────────────────────────────────────────────────────────

    pub async fn health(&self) -> Result<HealthResponse> {
        self.get("/health").await
    }

    // ── Webhooks ──────────────────────────────────────────────────────────────

    pub async fn webhook_create(
        &self,
        url: &str,
        events: &[String],
        chains: &[String],
    ) -> Result<Webhook> {
        self.post(
            "/webhooks",
            serde_json::json!({
                "url": url,
                "events": events,
                "chains": if chains.is_empty() { None } else { Some(chains) },
            }),
        )
        .await
    }

    pub async fn webhooks_list(&self) -> Result<Vec<Webhook>> {
        self.get("/webhooks").await
    }

    pub async fn webhook_delete(&self, id: &str) -> Result<()> {
        self.delete_req(&format!("/webhooks/{id}")).await
    }

    // ── Payment: direct ───────────────────────────────────────────────────────

    pub async fn pay_direct(
        &self,
        to: &str,
        amount: &str,
        memo: Option<&str>,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<TxResponse> {
        use rand::Rng;
        let nonce = hex::encode(rand::thread_rng().gen::<[u8; 16]>());
        let mut body = serde_json::json!({
            "to": to,
            "amount": amount,
            "task": memo.unwrap_or(""),
            "chain": chain_name(self.chain.chain_id),
            "nonce": nonce,
            "signature": "0x",
        });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value,
                "deadline": p.deadline,
                "v": p.v,
                "r": p.r,
                "s": p.s,
            });
        }
        self.post("/payments/direct", body).await
    }

    // ── Tabs ─────────────────────────────────────────────────────────────────

    pub async fn tab_open(
        &self,
        provider: &str,
        limit: &str,
        per_unit: &str,
        expiry_secs: i64,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Tab> {
        let mut body = serde_json::json!({
            "chain": chain_name(self.chain.chain_id),
            "provider": provider,
            "limit_amount": limit,
            "per_unit": per_unit,
            "expiry": expiry_secs,
        });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value, "deadline": p.deadline, "v": p.v, "r": p.r, "s": p.s,
            });
        }
        self.post("/tabs", body).await
    }

    pub async fn tab_charge(
        &self,
        tab_id: &str,
        amount: &str,
        cumulative: &str,
        call_count: i32,
    ) -> Result<TabCharge> {
        self.post(
            &format!("/tabs/{tab_id}/charge"),
            serde_json::json!({
                "amount": amount,
                "cumulative": cumulative,
                "call_count": call_count,
                "provider_sig": "0x",
            }),
        )
        .await
    }

    pub async fn tab_close(&self, tab_id: &str, final_amount: &str) -> Result<Tab> {
        self.post(
            &format!("/tabs/{tab_id}/close"),
            serde_json::json!({
                "final_amount": final_amount,
                "provider_sig": "0x",
            }),
        )
        .await
    }

    pub async fn tab_get(&self, tab_id: &str) -> Result<Tab> {
        self.get(&format!("/tabs/{tab_id}")).await
    }

    pub async fn tabs_list(&self, limit: u32) -> Result<PagedResponse<Tab>> {
        self.get(&format!("/tabs?limit={limit}")).await
    }

    // ── Streams ───────────────────────────────────────────────────────────────

    pub async fn stream_open(
        &self,
        payee: &str,
        rate_per_second: &str,
        max_total: &str,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Stream> {
        let mut body = serde_json::json!({
            "chain": chain_name(self.chain.chain_id),
            "payee": payee,
            "rate_per_second": rate_per_second,
            "max_total": max_total,
        });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value, "deadline": p.deadline, "v": p.v, "r": p.r, "s": p.s,
            });
        }
        self.post("/streams", body).await
    }

    pub async fn stream_close(&self, stream_id: &str) -> Result<Stream> {
        self.post(
            &format!("/streams/{stream_id}/close"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn streams_list(&self, limit: u32) -> Result<PagedResponse<Stream>> {
        self.get(&format!("/streams?limit={limit}")).await
    }

    // ── Escrows ───────────────────────────────────────────────────────────────

    pub async fn escrow_create(
        &self,
        payee: &str,
        amount: &str,
        timeout_secs: Option<i64>,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Escrow> {
        use rand::Rng;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = hex::encode(rand::thread_rng().gen::<[u8; 16]>());
        let invoice_id = format!("cli-{}", hex::encode(rand::thread_rng().gen::<[u8; 8]>()));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expiry = timeout_secs
            .map(|s| now as i64 + s)
            .unwrap_or(now as i64 + 86400);

        // Create invoice first
        self.post::<serde_json::Value>(
            "/invoices",
            serde_json::json!({
                "id": invoice_id,
                "chain": chain_name(self.chain.chain_id),
                "to_agent": payee,
                "amount": amount,
                "type": "escrow",
                "task": "",
                "nonce": nonce,
                "signature": "0x",
                "escrow_timeout": expiry,
            }),
        )
        .await
        .context("create invoice")?;

        // Fund escrow (with permit if provided)
        let mut body = serde_json::json!({ "invoice_id": invoice_id });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value, "deadline": p.deadline, "v": p.v, "r": p.r, "s": p.s,
            });
        }
        self.post("/escrows", body).await
    }

    pub async fn escrow_release(&self, escrow_id: &str) -> Result<Escrow> {
        self.post(
            &format!("/escrows/{escrow_id}/release"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn escrow_cancel(&self, escrow_id: &str) -> Result<Escrow> {
        self.post(
            &format!("/escrows/{escrow_id}/cancel"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn escrow_claim_start(&self, escrow_id: &str) -> Result<Escrow> {
        self.post(
            &format!("/escrows/{escrow_id}/claim-start"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn escrows_list(&self, limit: u32) -> Result<PagedResponse<Escrow>> {
        self.get(&format!("/escrows?limit={limit}")).await
    }

    // ── Bounties ──────────────────────────────────────────────────────────────

    pub async fn bounty_post(
        &self,
        amount: &str,
        description: &str,
        expiry_secs: i64,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Bounty> {
        let mut body = serde_json::json!({
            "chain": chain_name(self.chain.chain_id),
            "amount": amount,
            "task_description": description,
            "deadline": expiry_secs,
        });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value, "deadline": p.deadline, "v": p.v, "r": p.r, "s": p.s,
            });
        }
        self.post("/bounties", body).await
    }

    pub async fn bounty_submit(&self, bounty_id: &str, proof: &str) -> Result<BountySubmission> {
        self.post(
            &format!("/bounties/{bounty_id}/submit"),
            serde_json::json!({
                "evidence_hash": proof,
            }),
        )
        .await
    }

    pub async fn bounty_award(&self, bounty_id: &str, submission_id: i64) -> Result<Bounty> {
        self.post(
            &format!("/bounties/{bounty_id}/award"),
            serde_json::json!({ "submission_id": submission_id }),
        )
        .await
    }

    pub async fn bounties_list(&self, limit: u32) -> Result<PagedResponse<Bounty>> {
        self.get(&format!("/bounties?limit={limit}")).await
    }

    // ── Deposits ──────────────────────────────────────────────────────────────

    pub async fn deposit_create(
        &self,
        provider: &str,
        amount: &str,
        expiry_secs: i64,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Deposit> {
        let mut body = serde_json::json!({
            "chain": chain_name(self.chain.chain_id),
            "provider": provider,
            "amount": amount,
            "expiry": expiry_secs,
        });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value, "deadline": p.deadline, "v": p.v, "r": p.r, "s": p.s,
            });
        }
        self.post("/deposits", body).await
    }

    // ── Links ─────────────────────────────────────────────────────────────────

    pub async fn link_fund(&self) -> Result<CreateLinkResponse> {
        self.post("/links/fund", serde_json::json!({})).await
    }

    pub async fn link_withdraw(&self) -> Result<CreateLinkResponse> {
        self.post("/links/withdraw", serde_json::json!({})).await
    }

    // ── Mint (testnet only) ──────────────────────────────────────────────────

    /// Mint testnet USDC via `POST /mint`. This endpoint is unauthenticated,
    /// so we skip EIP-712 auth headers.
    pub async fn mint(&self, wallet: &str, amount: f64) -> Result<MintResponse> {
        let body = serde_json::json!({ "wallet": wallet, "amount": amount });
        let req = self
            .http
            .request(Method::POST, self.url("/mint"))
            .json(&body);

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("Network error: {e}"))?;
        let status = resp.status();

        if status.is_success() {
            resp.json::<MintResponse>()
                .await
                .map_err(|e| anyhow!("Failed to parse server response: {e}"))
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v["message"]
                        .as_str()
                        .or(v["error"].as_str())
                        .map(str::to_owned)
                })
                .unwrap_or(body_text);
            Err(anyhow!("{status}: {msg}"))
        }
    }

    // ── Faucet (DEPRECATED — use mint) ──────────────────────────────────────

    pub async fn faucet(&self, wallet: &str, amount: Option<&str>) -> Result<FaucetResponse> {
        let mut body = serde_json::json!({ "wallet": wallet });
        if let Some(amt) = amount {
            body["amount"] = serde_json::Value::String(amt.to_string());
        }
        self.post("/faucet", body).await
    }
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn chain_name(chain_id: u64) -> &'static str {
    match chain_id {
        8453 => "base",
        84532 => "base-sepolia",
        31337 => "localhost",
        _ => "base-sepolia",
    }
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct ContractsResponse {
    pub chain_id: u64,
    pub usdc: Option<String>,
    pub router: String,
    pub escrow: Option<String>,
    pub tab: Option<String>,
    pub stream: Option<String>,
    pub bounty: Option<String>,
    pub deposit: Option<String>,
    pub fee_calculator: Option<String>,
    pub key_registry: Option<String>,
    pub arbitration: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WalletStatus {
    pub wallet: String,
    pub balance: String,
    pub monthly_volume: Option<serde_json::Value>,
    pub tier: Option<String>,
    pub fee_rate_bps: Option<u32>,
    pub active_escrows: Option<i64>,
    pub active_tabs: Option<i64>,
    pub active_streams: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TxResponse {
    pub invoice_id: Option<String>,
    pub tx_hash: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Tab {
    pub id: String,
    pub chain: Option<String>,
    pub payer: Option<String>,
    pub provider: Option<String>,
    pub limit_amount: Option<serde_json::Value>,
    pub per_unit: Option<serde_json::Value>,
    pub total_charged: Option<serde_json::Value>,
    pub call_count: Option<i32>,
    pub status: String,
    pub expiry: Option<String>,
    pub tx_hash: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TabCharge {
    pub id: Option<i64>,
    pub tab_id: String,
    pub amount: serde_json::Value,
    pub cumulative: serde_json::Value,
    pub call_count: Option<i32>,
    pub charged_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Stream {
    pub id: String,
    pub chain: Option<String>,
    pub payer: Option<String>,
    pub payee: Option<String>,
    pub rate_per_second: Option<serde_json::Value>,
    pub max_total: Option<serde_json::Value>,
    pub withdrawn: Option<serde_json::Value>,
    pub status: String,
    pub started_at: Option<String>,
    pub closed_at: Option<String>,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Escrow {
    pub invoice_id: String,
    pub chain: Option<String>,
    pub tx_hash: Option<String>,
    pub status: String,
    pub payer: Option<String>,
    pub payee: Option<String>,
    pub amount: Option<serde_json::Value>,
    pub fee: Option<serde_json::Value>,
    pub timeout: Option<String>,
    pub claim_started: Option<bool>,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Bounty {
    pub id: String,
    pub chain: Option<String>,
    pub poster: Option<String>,
    pub amount: Option<serde_json::Value>,
    pub task_description: Option<String>,
    pub deadline: Option<String>,
    pub status: String,
    pub winner: Option<String>,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BountySubmission {
    pub id: Option<i64>,
    pub bounty_id: String,
    pub submitter: Option<String>,
    pub evidence_hash: String,
    pub status: String,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Deposit {
    pub id: String,
    pub chain: Option<String>,
    pub depositor: Option<String>,
    pub provider: Option<String>,
    pub amount: Option<serde_json::Value>,
    pub expiry: Option<String>,
    pub status: String,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateLinkResponse {
    pub url: String,
    pub token: Option<String>,
    pub expires_at: Option<String>,
    pub wallet_address: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MintResponse {
    pub tx_hash: String,
    pub balance: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FaucetResponse {
    pub tx_hash: Option<String>,
    pub amount: serde_json::Value,
    pub recipient: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Webhook {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub chains: Vec<String>,
    pub active: bool,
    pub secret: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PagedResponse<T> {
    pub data: Vec<T>,
    pub total: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
