//! User plugins (`docs/dev/plugins.md`): folders under `~/.acpira/plugins/` that add tools, MCP servers, hooks, a theme,
//! prompt sections or an agent without a build or a restart. `store.rs` keeps their frozen copies and the state of
//! what is enabled; `manifest.rs` reads `plugin.json`; `host.rs` is what the engine has loaded, the plugin page and
//! activation; `run.rs` runs a script tool; `mcp.rs` turns `mcp` entries into servers a session gets

pub mod cli;
pub mod guide;
pub mod host;
pub mod manifest;
pub mod mcp;
pub mod run;
pub mod store;

/// The plugins folder under the data root
pub const DIR: &str = "plugins";
