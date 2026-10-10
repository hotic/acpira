//! The JSON-RPC peer ACP runs on. It lives apart from acpira-host so the built-in agent (acpira-agent, which the host
//! binary depends on) can serve ACP over the same connection type without a dependency cycle

pub mod cancel;
pub mod rpc;
