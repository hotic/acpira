// The JSON-RPC peer and the cancel signal live in acpira-rpc; re-exported here so call sites keep their paths
pub use acpira_rpc::{cancel, rpc};

pub mod process;
pub mod wire;
