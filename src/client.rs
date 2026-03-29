// Client and types used by command handlers.

/// Authenticated HTTP client for the Remit API.
///
/// Every request is signed via EIP-712 auth headers (auth module).
/// All endpoints return typed Rust structs. Amounts are USDC strings.
use anyhow::{anyhow, Context, Result};
use reqwest::{Client, Method, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::auth::{build_auth_headers, ChainConfig};

// ── Base URLs ─────────────────────────────────────────────────────────────────

pub const MAINNET_API: &str = "https://remit.md/api/v1";
pub const TESTNET_API: &str = "https://testnet.remit.md/api/v1";

// ── Client ────────────────────────────────────────────────────────────────────

pub struct RemitClient {
    http: Client,
    base: String,
    chain: ChainConfig,
    sign_path_prefix: String,
    /// Whether the router address has been fetched from /contracts yet.
    router_fetched: bool,
}

impl RemitClient {
    pub fn new(testnet: bool, cfg: &crate::config::Config) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("remit-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;

        // Determine API base: env var wins, then config, then network default.
        let base = std::env::var("REMITMD_API_URL").unwrap_or_else(|_| {
            cfg.api_base
                .clone()
                .unwrap_or_else(|| (if testnet { TESTNET_API } else { MAINNET_API }).to_string())
        });

        let chain = ChainConfig::for_network(testnet);

        let sign_path_prefix = reqwest::Url::parse(&base)
            .map(|u| u.path().trim_end_matches('/').to_string())
            .unwrap_or_default();

        Ok(Self {
            http,
            base,
            chain,
            sign_path_prefix,
            router_fetched: false,
        })
    }

    /// Lazily fetch the router address from /contracts on first authenticated request.
    /// Falls back to the hardcoded address with a warning if the fetch fails.
    async fn ensure_router(&mut self) {
        if self.router_fetched {
            return;
        }
        self.router_fetched = true;

        if std::env::var("REMITMD_ROUTER_ADDRESS").is_ok() {
            return;
        }

        let contracts_url = format!("{}/contracts", self.base);
        match self.http.get(&contracts_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<ContractsResponse>().await {
                    if !data.router.is_empty() {
                        self.chain.router = data.router;
                    }
                }
            }
            _ => {
                eprintln!("warning: could not fetch /contracts — using hardcoded router address");
            }
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Execute an authenticated request, mapping HTTP errors to friendly messages.
    async fn request<T: DeserializeOwned>(
        &mut self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        self.ensure_router().await;
        let path_only = path.split('?').next().unwrap_or(path);
        let sign_path = format!("{}{}", self.sign_path_prefix, path_only);
        let auth = build_auth_headers(
            method.as_str(),
            &sign_path,
            self.chain.chain_id,
            &self.chain.router,
        )
        .await
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

    async fn get<T: DeserializeOwned>(&mut self, path: &str) -> Result<T> {
        self.request(Method::GET, path, None).await
    }

    async fn post<T: DeserializeOwned>(
        &mut self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        self.request(Method::POST, path, Some(body)).await
    }

    async fn delete_req(&mut self, path: &str) -> Result<()> {
        self.ensure_router().await;
        let path_only = path.split('?').next().unwrap_or(path);
        let sign_path = format!("{}{}", self.sign_path_prefix, path_only);
        let auth = build_auth_headers(
            "DELETE",
            &sign_path,
            self.chain.chain_id,
            &self.chain.router,
        )
        .await
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
    #[allow(dead_code)]
    pub async fn get_contracts(&mut self) -> Result<ContractsResponse> {
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

    // ── Permits: /permits/prepare ────────────────────────────────────────────

    pub async fn permit_prepare<T: DeserializeOwned>(
        &mut self,
        flow: &str,
        amount: &str,
        owner: &str,
    ) -> Result<T> {
        self.post(
            "/permits/prepare",
            serde_json::json!({ "flow": flow, "amount": amount, "owner": owner }),
        )
        .await
    }

    // ── Public: status ────────────────────────────────────────────────────────

    pub async fn status(&mut self, wallet: &str) -> Result<WalletStatus> {
        self.get(&format!("/status/{wallet}")).await
    }

    // ── Public: health ────────────────────────────────────────────────────────

    #[allow(dead_code)]
    pub async fn health(&mut self) -> Result<HealthResponse> {
        self.get("/health").await
    }

    // ── Webhooks ──────────────────────────────────────────────────────────────

    pub async fn webhook_create(
        &mut self,
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

    pub async fn webhooks_list(&mut self) -> Result<Vec<Webhook>> {
        self.get("/webhooks").await
    }

    pub async fn webhook_delete(&mut self, id: &str) -> Result<()> {
        self.delete_req(&format!("/webhooks/{id}")).await
    }

    // ── Payment: direct ───────────────────────────────────────────────────────

    pub async fn pay_direct(
        &mut self,
        to: &str,
        amount: &str,
        memo: Option<&str>,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<TxResponse> {
        use rand::Rng;
        let chain = chain_name(self.chain.chain_id)?;
        let nonce = hex::encode(rand::thread_rng().gen::<[u8; 16]>());
        let mut body = serde_json::json!({
            "to": to,
            "amount": amount,
            "task": memo.unwrap_or(""),
            "chain": chain,
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
        &mut self,
        provider: &str,
        limit: &str,
        per_unit: &str,
        expiry_secs: i64,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Tab> {
        let chain = chain_name(self.chain.chain_id)?;
        let mut body = serde_json::json!({
            "chain": chain,
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
        &mut self,
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

    pub async fn tab_close(
        &mut self,
        tab_id: &str,
        final_amount: Option<&str>,
        provider_sig: Option<&str>,
    ) -> Result<Tab> {
        let mut body = serde_json::json!({
            "provider_sig": provider_sig.unwrap_or("0x"),
        });
        if let Some(amt) = final_amount {
            body["final_amount"] = serde_json::Value::String(amt.to_string());
        }
        self.post(&format!("/tabs/{tab_id}/close"), body).await
    }

    pub async fn tab_get(&mut self, tab_id: &str) -> Result<Tab> {
        self.get(&format!("/tabs/{tab_id}")).await
    }

    pub async fn tabs_list(&mut self, limit: u32) -> Result<PagedResponse<Tab>> {
        self.get(&format!("/tabs?limit={limit}")).await
    }

    // ── Streams ───────────────────────────────────────────────────────────────

    pub async fn stream_open(
        &mut self,
        payee: &str,
        rate_per_second: &str,
        max_total: &str,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Stream> {
        let chain = chain_name(self.chain.chain_id)?;
        let mut body = serde_json::json!({
            "chain": chain,
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

    pub async fn stream_close(&mut self, stream_id: &str) -> Result<Stream> {
        self.post(
            &format!("/streams/{stream_id}/close"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn streams_list(&mut self, limit: u32) -> Result<PagedResponse<Stream>> {
        self.get(&format!("/streams?limit={limit}")).await
    }

    // ── Escrows ───────────────────────────────────────────────────────────────

    pub async fn escrow_create(
        &mut self,
        payee: &str,
        amount: &str,
        timeout_secs: Option<i64>,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Escrow> {
        let timeout = timeout_secs.unwrap_or(86400);
        let mut body = serde_json::json!({
            "to": payee,
            "amount": amount,
            "task": "",
            "timeout": timeout,
        });
        if let Some(p) = permit {
            body["permit"] = serde_json::json!({
                "value": p.value, "deadline": p.deadline, "v": p.v, "r": p.r, "s": p.s,
            });
        }
        self.post("/escrows", body).await
    }

    pub async fn escrow_release(&mut self, escrow_id: &str) -> Result<Escrow> {
        self.post(
            &format!("/escrows/{escrow_id}/release"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn escrow_cancel(&mut self, escrow_id: &str) -> Result<Escrow> {
        self.post(
            &format!("/escrows/{escrow_id}/cancel"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn escrow_claim_start(&mut self, escrow_id: &str) -> Result<Escrow> {
        self.post(
            &format!("/escrows/{escrow_id}/claim-start"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn escrows_list(&mut self, limit: u32) -> Result<PagedResponse<Escrow>> {
        self.get(&format!("/escrows?limit={limit}")).await
    }

    // ── Bounties ──────────────────────────────────────────────────────────────

    pub async fn bounty_post(
        &mut self,
        amount: &str,
        description: &str,
        expiry_secs: i64,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Bounty> {
        let chain = chain_name(self.chain.chain_id)?;
        let mut body = serde_json::json!({
            "chain": chain,
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

    pub async fn bounty_submit(
        &mut self,
        bounty_id: &str,
        proof: &str,
    ) -> Result<BountySubmission> {
        self.post(
            &format!("/bounties/{bounty_id}/submit"),
            serde_json::json!({
                "evidence_hash": proof,
            }),
        )
        .await
    }

    pub async fn bounty_award(&mut self, bounty_id: &str, submission_id: i64) -> Result<Bounty> {
        self.post(
            &format!("/bounties/{bounty_id}/award"),
            serde_json::json!({ "submission_id": submission_id }),
        )
        .await
    }

    pub async fn bounties_list(&mut self, limit: u32) -> Result<PagedResponse<Bounty>> {
        self.get(&format!("/bounties?limit={limit}")).await
    }

    // ── Deposits ──────────────────────────────────────────────────────────────

    pub async fn deposit_create(
        &mut self,
        provider: &str,
        amount: &str,
        expiry_secs: i64,
        permit: Option<&crate::permit::PermitSignature>,
    ) -> Result<Deposit> {
        let chain = chain_name(self.chain.chain_id)?;
        let mut body = serde_json::json!({
            "chain": chain,
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

    pub async fn link_fund(
        &mut self,
        amount: Option<&str>,
        name: Option<&str>,
        messages: &[&str],
    ) -> Result<CreateLinkResponse> {
        let mut body = serde_json::json!({});
        if let Some(amt) = amount {
            body["amount"] = serde_json::Value::String(amt.to_string());
        }
        if let Some(n) = name {
            body["agent_name"] = serde_json::Value::String(n.to_string());
        }
        // System intro first (fund page linkifies "Base L2"), then agent messages
        let mut msgs = vec![
            serde_json::json!({ "role": "system", "text": "Your agent is requesting funds. Wallet verified on Base L2. Choose a payment method to fund." }),
        ];
        for msg in messages {
            msgs.push(serde_json::json!({ "role": "agent", "text": msg }));
        }
        body["messages"] = serde_json::Value::Array(msgs);
        self.post("/links/fund", body).await
    }

    pub async fn link_withdraw(
        &mut self,
        amount: Option<&str>,
        to: Option<&str>,
    ) -> Result<CreateLinkResponse> {
        let mut body = serde_json::json!({});
        if let Some(amt) = amount {
            body["amount"] = serde_json::Value::String(amt.to_string());
        }
        if let Some(addr) = to {
            body["to"] = serde_json::Value::String(addr.to_string());
        }
        self.post("/links/withdraw", body).await
    }

    // ── Mint (testnet only) ──────────────────────────────────────────────────

    /// Mint testnet USDC via `POST /mint`. This endpoint is unauthenticated,
    /// so we skip EIP-712 auth headers.
    pub async fn mint(&mut self, wallet: &str, amount: f64) -> Result<MintResponse> {
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
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn chain_name(chain_id: u64) -> Result<&'static str> {
    match chain_id {
        8453 => Ok("base"),
        84532 => Ok("base-sepolia"),
        31337 => Ok("localhost"),
        _ => Err(anyhow!("unknown chain ID: {chain_id}")),
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
    pub permit_nonce: Option<u64>,
}

#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_name_known() {
        assert_eq!(chain_name(8453).unwrap(), "base");
        assert_eq!(chain_name(84532).unwrap(), "base-sepolia");
        assert_eq!(chain_name(31337).unwrap(), "localhost");
    }

    #[test]
    fn test_chain_name_unknown_errors() {
        assert!(chain_name(1).is_err());
        assert!(chain_name(0).is_err());
        assert!(chain_name(999999).is_err());
    }
}
