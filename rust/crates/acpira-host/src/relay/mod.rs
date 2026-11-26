//! Cross-harness subagents ("relay"): personas the user defines once (`roster.rs`, `~/.acpira/subagents.json`) that any
//! session can summon. The agent calls `ask_agent` on Acpira's MCP server (`host_mcp.rs`, a process the agent CLI
//! starts); that process reaches the sidecar over a loopback socket (`hub.rs`), and the session that owns the call runs
//! the persona's CLI as a child it drives itself (`acp/session/relay.rs`), shown as a `session` subagent node.
//!
//! Hub wire (`wire.rs`): one request per TCP connection, newline-delimited JSON. The request carries the entry's grant
//! token and the op; `list` answers the enabled personas, `ask` answers progress lines and then one final line

pub mod hub;
pub use crate::relay_roster as roster;
pub use crate::relay_wire as wire;
pub use crate::relay_wire::{ENV_ADDR, ENV_HOST_SERVER, ENV_TOKEN, MAX_DEPTH, MAX_RUNNING};
