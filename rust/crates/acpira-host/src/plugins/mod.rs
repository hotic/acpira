//! User plugins (`docs/dev/plugins.md`): folders under `~/.acpira/plugins/` that add tools, MCP servers, hooks, a theme,
//! prompt sections or an agent without a build or a restart. `store.rs` keeps their frozen copies and the state of
//! what is enabled; `manifest.rs` reads `plugin.json`

pub mod manifest;
pub mod store;

/// The plugins folder under the data root
pub const DIR: &str = "plugins";
