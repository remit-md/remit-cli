//! Local signer modules.
//!
//! Manages encrypted private keys and EIP-712 typed data hashing.

// eip712 + keystore functions will be used by `remit sign` (C0.5).
#![allow(dead_code)]

pub mod eip712;
pub mod keystore;
