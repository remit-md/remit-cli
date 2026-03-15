#![allow(dead_code)]

use anyhow::{anyhow, Result};
use reqwest::{Client, RequestBuilder, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub const MAINNET_API: &str = "https://remit.md/api/v0";
pub const TESTNET_API: &str = "https://testnet.remit.md/api/v0";

/// Build the API base URL from the testnet flag.
pub fn api_base(testnet: bool) -> &'static str {
    if testnet {
        TESTNET_API
    } else {
        MAINNET_API
    }
}

/// HTTP client wrapper for the Remit API.
pub struct RemitClient {
    pub http: Client,
    pub base: String,
    pub address: Option<String>,
}

impl RemitClient {
    pub fn new(testnet: bool) -> Self {
        let http = Client::builder()
            .user_agent(concat!("remit-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            base: api_base(testnet).to_string(),
            address: None,
        }
    }

    pub fn with_address(mut self, address: String) -> Self {
        self.address = Some(address);
        self
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Execute a request, returning a typed response or a user-friendly error.
    pub async fn execute<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp: Response = req
            .send()
            .await
            .map_err(|e| anyhow!("Network error: {e}"))?;

        let status = resp.status();
        if status.is_success() {
            resp.json::<T>()
                .await
                .map_err(|e| anyhow!("Failed to parse response: {e}"))
        } else {
            let body = resp.text().await.unwrap_or_default();
            // Try to parse as JSON error object
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                let msg = v["error"].as_str().unwrap_or(&body);
                Err(anyhow!("API error {status}: {msg}"))
            } else {
                Err(anyhow!("API error {status}: {body}"))
            }
        }
    }
}

// ── Shared response types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct StatusResponse {
    pub address: String,
    pub balance: Option<String>,
    pub network: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TxResponse {
    pub tx_hash: Option<String>,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BalanceResponse {
    pub address: String,
    pub usdc: String,
    pub network: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryEntry {
    pub id: String,
    pub kind: String,
    pub amount: Option<String>,
    pub status: String,
    pub created_at: String,
    pub counterparty: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryResponse {
    pub transactions: Vec<HistoryEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LinkResponse {
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FaucetResponse {
    pub tx_hash: Option<String>,
    pub amount: String,
    pub message: String,
}
