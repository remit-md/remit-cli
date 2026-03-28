//! `remit address` — print the wallet address.
//!
//! No password or keychain access needed — address is public info stored in
//! either `.meta` (keychain) or `.enc` (encrypted keystore) files.

use anyhow::Result;
use clap::Args;

use crate::signer::{keyring, keystore};

#[derive(Args)]
pub struct AddressArgs {
    /// Path to keystore file (default: auto-detect from ~/.remit/keys/)
    #[arg(long)]
    pub keystore: Option<String>,
}

pub async fn run(args: AddressArgs) -> Result<()> {
    // Explicit --keystore flag: use that .enc file directly
    if let Some(ref path) = args.keystore {
        let key_file = keystore::load_file(std::path::Path::new(path))?;
        if key_file.version == 1 {
            anyhow::bail!("Keystore version 1 (V24). Run: remit signer migrate");
        }
        print!("{}", key_file.address);
        return Ok(());
    }

    // Check .meta file first (keychain — has address in plaintext)
    if keyring::MetaFile::exists("default")? {
        let meta = keyring::MetaFile::load("default")?;
        print!("{}", meta.address);
        return Ok(());
    }

    // Fall back to .enc file
    let ks = keystore::Keystore::open()?;
    let enc_path = ks.key_path("default");
    if enc_path.exists() {
        let key_file = keystore::load_file(&enc_path)?;
        if key_file.version == 1 {
            anyhow::bail!("Keystore version 1 (V24). Run: remit signer migrate");
        }
        print!("{}", key_file.address);
        return Ok(());
    }

    anyhow::bail!("No wallet found. Run: remit signer init")
}
