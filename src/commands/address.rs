//! `remit address` — print the wallet address from the keystore.
//!
//! No password needed — address is stored in plaintext in the keystore JSON.

use anyhow::Result;
use clap::Args;

use crate::signer::keystore;

#[derive(Args)]
pub struct AddressArgs {
    /// Path to keystore file (default: ~/.remit/keys/default.enc)
    #[arg(long)]
    pub keystore: Option<String>,
}

pub async fn run(args: AddressArgs) -> Result<()> {
    let keystore_path = match &args.keystore {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let ks = keystore::Keystore::open()?;
            ks.key_path("default")
        }
    };

    if !keystore_path.exists() {
        anyhow::bail!(
            "No keystore found at {}. Run: remit signer init",
            keystore_path.display()
        );
    }

    let key_file = keystore::load_file(&keystore_path)?;

    // Reject V1 keystores
    if key_file.version == 1 {
        anyhow::bail!("Keystore version 1 (V24). Run: remit signer migrate");
    }

    // Print address — no decryption needed
    print!("{}", key_file.address);

    Ok(())
}
