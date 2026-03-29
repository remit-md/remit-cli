//! EIP-712 structured data hashing for the local signer.
//!
//! Takes domain, types, and value from a `/sign/typed-data` request
//! and produces the 32-byte digest: `keccak256(0x1901 || domainSeparator || structHash)`.
//!
//! This handles arbitrary EIP-712 types (Permit, APIRequest, etc.)
//! by dynamically building type hashes from the provided type definitions.
#![deny(unsafe_code)]

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::BTreeMap;

// ── Types (matching the HTTP API request shape) ────────────────────────────

/// EIP-712 domain fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedDataDomain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifying_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
}

/// A single field in an EIP-712 type definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeField {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

/// The full typed-data sign request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedDataRequest {
    pub domain: TypedDataDomain,
    pub types: BTreeMap<String, Vec<TypeField>>,
    pub value: serde_json::Value,
}

// ── Hashing ────────────────────────────────────────────────────────────────

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(data);
    h.finalize().into()
}

/// Compute the full EIP-712 hash for a typed-data request.
///
/// Returns the 32-byte digest: `keccak256(0x1901 || domainSeparator || structHash)`.
pub fn hash_typed_data(request: &TypedDataRequest) -> Result<[u8; 32]> {
    let domain_separator = encode_domain(&request.domain)?;
    let primary_type = find_primary_type(&request.types)?;
    let struct_hash = encode_struct(&primary_type, &request.value, &request.types)?;

    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.push(0x19);
    buf.push(0x01);
    buf.extend_from_slice(&domain_separator);
    buf.extend_from_slice(&struct_hash);

    Ok(keccak(&buf))
}

/// Extract the chain ID from a typed-data request (for policy evaluation).
#[allow(dead_code)]
pub fn extract_chain_id(domain: &TypedDataDomain) -> Option<u64> {
    domain.chain_id
}

/// Extract contract/spender/to address from the message value (for policy).
#[allow(dead_code)]
pub fn extract_contract(value: &serde_json::Value) -> Option<String> {
    // Try common field names used in EIP-712 messages
    for field in &["spender", "to", "recipient", "verifyingContract"] {
        if let Some(addr) = value.get(field).and_then(|v| v.as_str()) {
            if addr.starts_with("0x") {
                return Some(addr.to_string());
            }
        }
    }
    None
}

/// Extract USDC amount from the message value (for policy).
/// Assumes 6 decimals (USDC standard).
#[allow(dead_code)]
pub fn extract_amount_usdc(value: &serde_json::Value) -> Option<f64> {
    for field in &["value", "amount"] {
        if let Some(raw) = value.get(field) {
            let micro = match raw {
                serde_json::Value::Number(n) => n.as_u64(),
                serde_json::Value::String(s) => s.parse::<u64>().ok(),
                _ => None,
            };
            if let Some(micro_usdc) = micro {
                return Some(micro_usdc as f64 / 1_000_000.0);
            }
        }
    }
    None
}

// ── Domain encoding ────────────────────────────────────────────────────────

/// Encode the EIP-712 domain separator.
fn encode_domain(domain: &TypedDataDomain) -> Result<[u8; 32]> {
    // Build the EIP712Domain type string dynamically based on which fields are present
    let mut type_parts = Vec::new();
    if domain.name.is_some() {
        type_parts.push("string name");
    }
    if domain.version.is_some() {
        type_parts.push("string version");
    }
    if domain.chain_id.is_some() {
        type_parts.push("uint256 chainId");
    }
    if domain.verifying_contract.is_some() {
        type_parts.push("address verifyingContract");
    }
    if domain.salt.is_some() {
        type_parts.push("bytes32 salt");
    }
    let type_string = format!("EIP712Domain({})", type_parts.join(","));
    let type_hash = keccak(type_string.as_bytes());

    let mut encoded = Vec::with_capacity(6 * 32);
    encoded.extend_from_slice(&type_hash);

    if let Some(ref name) = domain.name {
        encoded.extend_from_slice(&keccak(name.as_bytes()));
    }
    if let Some(ref version) = domain.version {
        encoded.extend_from_slice(&keccak(version.as_bytes()));
    }
    if let Some(chain_id) = domain.chain_id {
        encoded.extend_from_slice(&abi_uint256_from_u64(chain_id));
    }
    if let Some(ref addr) = domain.verifying_contract {
        encoded.extend_from_slice(&abi_address(addr)?);
    }
    if let Some(ref salt) = domain.salt {
        let bytes =
            hex::decode(salt.trim_start_matches("0x")).map_err(|_| anyhow!("invalid salt hex"))?;
        if bytes.len() != 32 {
            return Err(anyhow!("salt must be 32 bytes"));
        }
        encoded.extend_from_slice(&bytes);
    }

    Ok(keccak(&encoded))
}

// ── Struct encoding ────────────────────────────────────────────────────────

/// Find the primary type (the type that's not EIP712Domain and not referenced
/// by any other type — or just the first non-EIP712Domain type).
fn find_primary_type(types: &BTreeMap<String, Vec<TypeField>>) -> Result<String> {
    // Simple heuristic: first key that isn't EIP712Domain
    for key in types.keys() {
        if key != "EIP712Domain" {
            return Ok(key.clone());
        }
    }
    Err(anyhow!("no primary type found in types definition"))
}

/// Build the type hash for a struct: `keccak256("TypeName(type1 name1,type2 name2,...)")`.
fn type_hash(type_name: &str, types: &BTreeMap<String, Vec<TypeField>>) -> Result<[u8; 32]> {
    let type_string = encode_type_string(type_name, types)?;
    Ok(keccak(type_string.as_bytes()))
}

/// Build the full type string including referenced types (sorted alphabetically).
fn encode_type_string(type_name: &str, types: &BTreeMap<String, Vec<TypeField>>) -> Result<String> {
    let fields = types
        .get(type_name)
        .ok_or_else(|| anyhow!("type '{}' not found in types definition", type_name))?;

    let primary = format!(
        "{}({})",
        type_name,
        fields
            .iter()
            .map(|f| format!("{} {}", f.type_name, f.name))
            .collect::<Vec<_>>()
            .join(",")
    );

    // Collect referenced types (sub-structs), sorted alphabetically
    let mut referenced = Vec::new();
    for field in fields {
        let base_type = field.type_name.trim_end_matches("[]");
        if types.contains_key(base_type) && base_type != type_name {
            referenced.push(base_type.to_string());
        }
    }
    referenced.sort();
    referenced.dedup();

    let mut result = primary;
    for ref_type in &referenced {
        let sub = encode_type_string(ref_type, types)?;
        result.push_str(&sub);
    }

    Ok(result)
}

/// Encode a struct value: `keccak256(typeHash || encodeData(...))`.
fn encode_struct(
    type_name: &str,
    value: &serde_json::Value,
    types: &BTreeMap<String, Vec<TypeField>>,
) -> Result<[u8; 32]> {
    let fields = types
        .get(type_name)
        .ok_or_else(|| anyhow!("type '{}' not found", type_name))?;
    let th = type_hash(type_name, types)?;

    let mut encoded = Vec::with_capacity((1 + fields.len()) * 32);
    encoded.extend_from_slice(&th);

    for field in fields {
        let field_value = value.get(&field.name);
        let slot = encode_field(&field.type_name, field_value, types)?;
        encoded.extend_from_slice(&slot);
    }

    Ok(keccak(&encoded))
}

/// Encode a single field value to a 32-byte ABI slot.
fn encode_field(
    type_name: &str,
    value: Option<&serde_json::Value>,
    types: &BTreeMap<String, Vec<TypeField>>,
) -> Result<[u8; 32]> {
    let val = value.ok_or_else(|| anyhow!("missing field value for type {type_name}"))?;

    match type_name {
        "address" => {
            let addr = val
                .as_str()
                .ok_or_else(|| anyhow!("address must be a string"))?;
            abi_address(addr)
        }
        "uint256" | "uint128" | "uint64" | "uint32" | "uint16" | "uint8" => {
            let n = parse_uint(val)?;
            Ok(abi_uint256_from_u128(n))
        }
        "int256" | "int128" | "int64" | "int32" | "int16" | "int8" => {
            let n = parse_int(val)?;
            Ok(abi_int256(n))
        }
        "bool" => {
            let b = val.as_bool().unwrap_or_else(|| {
                // Handle string "true"/"false" and numeric 0/1
                match val {
                    serde_json::Value::String(s) => s == "true" || s == "1",
                    serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) != 0,
                    _ => false,
                }
            });
            let mut slot = [0u8; 32];
            if b {
                slot[31] = 1;
            }
            Ok(slot)
        }
        "bytes32" => {
            let hex_str = val
                .as_str()
                .ok_or_else(|| anyhow!("bytes32 must be a hex string"))?;
            let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                .map_err(|_| anyhow!("invalid bytes32 hex"))?;
            if bytes.len() != 32 {
                return Err(anyhow!("bytes32 must be 32 bytes"));
            }
            let mut slot = [0u8; 32];
            slot.copy_from_slice(&bytes);
            Ok(slot)
        }
        "string" => {
            let s = val
                .as_str()
                .ok_or_else(|| anyhow!("string field must be a string"))?;
            Ok(keccak(s.as_bytes()))
        }
        "bytes" => {
            let hex_str = val
                .as_str()
                .ok_or_else(|| anyhow!("bytes must be a hex string"))?;
            let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                .map_err(|_| anyhow!("invalid bytes hex"))?;
            Ok(keccak(&bytes))
        }
        _ => {
            // Check if it's a custom struct type
            if types.contains_key(type_name) {
                return encode_struct(type_name, val, types);
            }
            // Array types
            if type_name.ends_with("[]") {
                let base = type_name.trim_end_matches("[]");
                let arr = val
                    .as_array()
                    .ok_or_else(|| anyhow!("array field must be a JSON array"))?;
                let mut concat = Vec::with_capacity(arr.len() * 32);
                for item in arr {
                    let slot = encode_field(base, Some(item), types)?;
                    concat.extend_from_slice(&slot);
                }
                return Ok(keccak(&concat));
            }
            Err(anyhow!("unsupported EIP-712 type: {type_name}"))
        }
    }
}

// ── ABI encoding helpers ───────────────────────────────────────────────────

fn abi_uint256_from_u64(v: u64) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[24..32].copy_from_slice(&v.to_be_bytes());
    slot
}

fn abi_uint256_from_u128(v: u128) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[16..32].copy_from_slice(&v.to_be_bytes());
    slot
}

fn abi_int256(v: i128) -> [u8; 32] {
    let mut slot = if v < 0 { [0xffu8; 32] } else { [0u8; 32] };
    slot[16..32].copy_from_slice(&v.to_be_bytes());
    slot
}

fn abi_address(addr_hex: &str) -> Result<[u8; 32]> {
    let clean = addr_hex.trim_start_matches("0x");
    let bytes =
        hex::decode(clean).map_err(|_| anyhow!("invalid address hex: {}", &addr_hex[..8]))?;
    if bytes.len() != 20 {
        return Err(anyhow!("address must be 20 bytes, got {}", bytes.len()));
    }
    let mut slot = [0u8; 32];
    slot[12..32].copy_from_slice(&bytes);
    Ok(slot)
}

/// Parse a JSON value as an unsigned integer (handles string and number).
fn parse_uint(val: &serde_json::Value) -> Result<u128> {
    match val {
        serde_json::Value::Number(n) => n
            .as_u64()
            .map(u128::from)
            .ok_or_else(|| anyhow!("uint value out of range")),
        serde_json::Value::String(s) => s
            .parse::<u128>()
            .map_err(|_| anyhow!("invalid uint string: {s}")),
        _ => Err(anyhow!("uint must be a number or string")),
    }
}

/// Parse a JSON value as a signed integer.
fn parse_int(val: &serde_json::Value) -> Result<i128> {
    match val {
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .ok_or_else(|| anyhow!("int value out of range")),
        serde_json::Value::String(s) => s
            .parse::<i128>()
            .map_err(|_| anyhow!("invalid int string: {s}")),
        _ => Err(anyhow!("int must be a number or string")),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Build a USDC Permit typed-data request for testing.
    fn permit_request() -> TypedDataRequest {
        let types_json = serde_json::json!({
            "Permit": [
                {"name": "owner", "type": "address"},
                {"name": "spender", "type": "address"},
                {"name": "value", "type": "uint256"},
                {"name": "nonce", "type": "uint256"},
                {"name": "deadline", "type": "uint256"}
            ]
        });
        let types: BTreeMap<String, Vec<TypeField>> = serde_json::from_value(types_json).unwrap();

        TypedDataRequest {
            domain: TypedDataDomain {
                name: Some("USD Coin".to_string()),
                version: Some("2".to_string()),
                chain_id: Some(84532),
                verifying_contract: Some("0x2d846325766921935f37d5b4478196d3ef93707c".to_string()),
                salt: None,
            },
            types,
            value: serde_json::json!({
                "owner": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                "spender": "0x3120f396ff6a9afc5a9d92e28796082f1429e024",
                "value": "1000000",
                "nonce": 0,
                "deadline": 1711387200
            }),
        }
    }

    // ── Basic hashing ───────────────────────────────────────────────────

    #[test]
    fn hash_typed_data_produces_32_bytes() {
        let request = permit_request();
        let hash = hash_typed_data(&request).unwrap();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn hash_typed_data_is_deterministic() {
        let request = permit_request();
        let h1 = hash_typed_data(&request).unwrap();
        let h2 = hash_typed_data(&request).unwrap();
        assert_eq!(h1, h2, "same input must produce same hash");
    }

    #[test]
    fn different_values_produce_different_hashes() {
        let r1 = permit_request();
        let mut r2 = permit_request();
        r2.value["value"] = serde_json::json!("2000000");

        let h1 = hash_typed_data(&r1).unwrap();
        let h2 = hash_typed_data(&r2).unwrap();
        assert_ne!(h1, h2, "different values must produce different hashes");
    }

    #[test]
    fn different_chains_produce_different_hashes() {
        let r1 = permit_request();
        let mut r2 = permit_request();
        r2.domain.chain_id = Some(8453);

        let h1 = hash_typed_data(&r1).unwrap();
        let h2 = hash_typed_data(&r2).unwrap();
        assert_ne!(h1, h2, "different chains must produce different hashes");
    }

    // ── Domain encoding ─────────────────────────────────────────────────

    #[test]
    fn domain_with_all_fields() {
        let domain = TypedDataDomain {
            name: Some("Test".to_string()),
            version: Some("1".to_string()),
            chain_id: Some(1),
            verifying_contract: Some("0x0000000000000000000000000000000000000001".to_string()),
            salt: None,
        };
        let sep = encode_domain(&domain).unwrap();
        assert_eq!(sep.len(), 32);
    }

    #[test]
    fn domain_with_only_name() {
        let domain = TypedDataDomain {
            name: Some("Minimal".to_string()),
            version: None,
            chain_id: None,
            verifying_contract: None,
            salt: None,
        };
        let sep = encode_domain(&domain).unwrap();
        assert_eq!(sep.len(), 32);
    }

    // ── Extraction helpers ──────────────────────────────────────────────

    #[test]
    fn extract_chain_id_from_domain() {
        let request = permit_request();
        assert_eq!(extract_chain_id(&request.domain), Some(84532));
    }

    #[test]
    fn extract_contract_from_value() {
        let request = permit_request();
        let contract = extract_contract(&request.value);
        assert_eq!(
            contract,
            Some("0x3120f396ff6a9afc5a9d92e28796082f1429e024".to_string())
        );
    }

    #[test]
    fn extract_amount_usdc_from_value() {
        let request = permit_request();
        let amount = extract_amount_usdc(&request.value);
        assert!((amount.unwrap() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_amount_usdc_string_value() {
        let value = serde_json::json!({"value": "5000000"});
        let amount = extract_amount_usdc(&value);
        assert!((amount.unwrap() - 5.0).abs() < f64::EPSILON);
    }

    // ── Type string encoding ────────────────────────────────────────────

    #[test]
    fn type_string_for_permit() {
        let request = permit_request();
        let ts = encode_type_string("Permit", &request.types).unwrap();
        assert_eq!(
            ts,
            "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"
        );
    }

    // ── Error cases ─────────────────────────────────────────────────────

    #[test]
    fn missing_type_fails() {
        let request = TypedDataRequest {
            domain: TypedDataDomain {
                name: Some("Test".to_string()),
                version: None,
                chain_id: None,
                verifying_contract: None,
                salt: None,
            },
            types: BTreeMap::new(),
            value: serde_json::json!({}),
        };
        let result = hash_typed_data(&request);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_address_fails() {
        let result = abi_address("not-an-address");
        assert!(result.is_err());
    }

    #[test]
    fn wrong_length_address_fails() {
        let result = abi_address("0xdead");
        assert!(result.is_err());
    }

    // ── uint parsing ────────────────────────────────────────────────────

    #[test]
    fn parse_uint_from_number() {
        let val = serde_json::json!(42);
        assert_eq!(parse_uint(&val).unwrap(), 42);
    }

    #[test]
    fn parse_uint_from_string() {
        let val = serde_json::json!("1000000000000000000");
        assert_eq!(parse_uint(&val).unwrap(), 1_000_000_000_000_000_000u128);
    }
}
