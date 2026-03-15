pub mod a2a;
pub mod balance;
pub mod bounty;
pub mod config_cmd;
pub mod deposit;
pub mod escrow;
pub mod faucet;
pub mod fund;
pub mod history;
pub mod init;
pub mod pay;
pub mod status;
pub mod stream;
pub mod tab;
pub mod withdraw;

/// Per-invocation context passed to every command handler.
// Fields used by command handlers as they are implemented (tasks 0.4+).
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Context {
    pub json: bool,
    pub testnet: bool,
}
