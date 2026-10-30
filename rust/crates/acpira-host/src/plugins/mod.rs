//! User plugins (`docs/dev/plugins.md`): folders under `~/.acpira/plugins/` that add tools, MCP servers, hooks, a theme,
//! prompt sections or an agent without a build or a restart. `store.rs` keeps their frozen copies and the state of
//! what is enabled; `manifest.rs` reads `plugin.json`; `host.rs` is what the engine has loaded, the plugin page and
//! activation; `run.rs` runs a script tool; `runtime.rs` keeps the resident processes; `build.rs` compiles TypeScript / TSX sources; `sdk.rs` ships their SDK and writes new plugin folders; `mcp.rs` turns `mcp` entries into servers a session gets

pub mod build;
pub mod cli;
pub mod guide;
pub mod host;
pub mod host_api;
pub mod manifest;
pub mod mcp;
pub mod run;
pub mod runtime;
pub mod schedule;
pub mod sdk;
pub mod store;

/// The plugins folder under the data root
pub const DIR: &str = "plugins";
