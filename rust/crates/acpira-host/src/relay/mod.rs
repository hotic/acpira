//! Cross-harness subagents ("relay"): personas the user defines once (`roster.rs`, `~/.acpira/subagents.json`) that any
//! session can summon. The agent calls `ask_agent` on Acpira's MCP server (`host_mcp.rs`, a process the agent CLI
//! starts); that process reaches the sidecar over a loopback socket (`hub.rs`), and the session that owns the call runs
//! the persona's CLI as a child it drives itself (`acp/session/relay.rs`), shown as a `session` subagent node.
//!
//! Hub wire (`wire.rs`): one request per TCP connection, newline-delimited JSON. The request carries the entry's grant
//! token and the op; `list` answers the enabled personas, `ask` answers progress lines and then one final line

pub mod hub;
pub mod roster;
pub mod wire;

/// How deep summoned children may summon in turn (the root session's own call is depth 0)
pub const MAX_DEPTH: u32 = 2;
/// Summoned rounds running at once in one session, nested ones included (each is a CLI process of its own)
pub const MAX_RUNNING: usize = 6;

/// Environment variables of the MCP server entry that lead back to the hub. The token is a grant minted for that one
/// entry: the hub knows from it which session, which summoned thread and which depth the caller is, so none of these
/// travel in a request where a caller could forge them
pub const ENV_ADDR: &str = "ACPIRA_RELAY";
pub const ENV_TOKEN: &str = "ACPIRA_RELAY_TOKEN";
