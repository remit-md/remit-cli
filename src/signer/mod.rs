//! Local signer server modules.
//!
//! The signer manages encrypted private keys, bearer tokens, policies,
//! and an HTTP server for remote signing on localhost.
// Some methods are only used from tests or integration tests (C0.9).
#![allow(dead_code)]

pub mod daemon;
pub mod eip712;
#[cfg(test)]
mod integration_tests;
pub mod keystore;
pub mod policy;
pub mod server;
pub mod tokens;
