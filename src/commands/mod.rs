pub mod a2a;
pub mod address;
pub mod balance;
pub mod bounty;
pub mod config_cmd;
pub mod deposit;
pub mod escrow;
pub mod fund;
pub mod init;
pub mod mint;
pub mod pay;
pub mod sign;
pub mod signer;
pub mod status;
pub mod stream;
pub mod tab;
pub mod update;
pub mod wallet;
pub mod webhook;
pub mod withdraw;

/// Per-invocation context passed to every command handler.
#[derive(Clone)]
pub struct Context {
    pub json: bool,
    pub testnet: bool,
    pub config: crate::config::Config,
}

/// Create a `RemitClient` from the context, returning `Result`.
impl Context {
    pub fn client(&self) -> anyhow::Result<crate::client::RemitClient> {
        crate::client::RemitClient::new(self.testnet, &self.config)
    }
}

/// Check that a signing wallet is configured.
/// Payment commands must call this before proceeding.
///
/// A wallet is considered configured if ANY of the following exist:
///   1. REMITMD_KEY env var is set (raw private key)
///   2. ~/.remit/keys/default.meta exists (OS keychain wallet)
///   3. ~/.remit/keys/default.enc exists (encrypted keystore)
pub fn require_init() -> anyhow::Result<()> {
    // 1. Raw private key in env
    if std::env::var("REMITMD_KEY").is_ok() {
        return Ok(());
    }

    // 2. OS keychain (.meta file)
    if let Ok(true) = crate::signer::keyring::MetaFile::exists("default") {
        return Ok(());
    }

    // 3. Encrypted keystore (.enc file)
    if let Ok(ks) = crate::signer::keystore::Keystore::open() {
        if ks.exists("default") {
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "No wallet configured. Run `remit signer init` to create one,\n\
         or set REMITMD_KEY in your environment."
    ))
}

/// Validate that a USDC amount string is a positive number.
/// Returns the parsed f64 for further use, or an error if invalid or non-positive.
pub fn validate_positive_amount(amount: &str, field_name: &str) -> anyhow::Result<f64> {
    let parsed: f64 = amount
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid {field_name}: {amount}"))?;
    if parsed <= 0.0 {
        return Err(anyhow::anyhow!(
            "{field_name} must be positive, got {amount}"
        ));
    }
    Ok(parsed)
}

/// Validate that an Ethereum address is well-formed: 42 chars, 0x-prefixed, hex.
pub fn validate_address(addr: &str, field_name: &str) -> anyhow::Result<()> {
    if addr.len() != 42 {
        return Err(anyhow::anyhow!(
            "{field_name} must be 42 characters (0x + 40 hex), got {} chars",
            addr.len()
        ));
    }
    if !addr.starts_with("0x") && !addr.starts_with("0X") {
        return Err(anyhow::anyhow!(
            "{field_name} must start with 0x, got: {}",
            &addr[..std::cmp::min(4, addr.len())]
        ));
    }
    if !addr[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow::anyhow!(
            "{field_name} contains non-hex characters: {addr}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_positive_amount_valid() {
        assert!(validate_positive_amount("10.50", "amount").is_ok());
        assert!(validate_positive_amount("0.01", "amount").is_ok());
        assert!(validate_positive_amount("1000", "amount").is_ok());
    }

    #[test]
    fn test_validate_positive_amount_zero() {
        assert!(validate_positive_amount("0", "amount").is_err());
        assert!(validate_positive_amount("0.0", "amount").is_err());
    }

    #[test]
    fn test_validate_positive_amount_negative() {
        assert!(validate_positive_amount("-1", "amount").is_err());
        assert!(validate_positive_amount("-0.5", "amount").is_err());
    }

    #[test]
    fn test_validate_positive_amount_invalid() {
        assert!(validate_positive_amount("abc", "amount").is_err());
        assert!(validate_positive_amount("", "amount").is_err());
    }

    #[test]
    fn test_validate_address_valid() {
        assert!(validate_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "to").is_ok());
        assert!(validate_address("0x0000000000000000000000000000000000000001", "to").is_ok());
    }

    #[test]
    fn test_validate_address_wrong_length() {
        assert!(validate_address("0x1234", "to").is_err());
        assert!(validate_address("0x", "to").is_err());
    }

    #[test]
    fn test_validate_address_no_prefix() {
        assert!(validate_address("f39Fd6e51aad88F6F4ce6aB8827279cffFb922660", "to").is_err());
    }

    #[test]
    fn test_validate_address_non_hex() {
        assert!(validate_address("0xGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG", "to").is_err());
    }
}
