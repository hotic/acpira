//! The Acpira host runtime. Each module mirrors the apps/vscode/src file of the same name

#![allow(clippy::type_complexity, reason = "dependency seams are closures (`Arc<dyn Fn(..) -> BoxFuture<..>>`) like the TS deps objects they mirror")]

pub mod accounts;
pub mod acp;
pub mod agent_ext;
pub mod agents_cli;
pub mod bridge_core;
pub mod bus;
pub mod cli;
pub mod external;
pub mod file_rank;
pub mod handoff;
pub mod hooks;
pub mod host_mcp;
pub mod http;
pub mod i18n;
pub mod inventory;
pub mod json;
pub mod limits;
pub mod model_catalog;
pub mod net_proxy;
pub mod node_files;
pub mod platform;
pub mod plugins;
pub mod preview;
pub mod project_files;
pub mod providers;
pub mod relay;
pub mod remote;
pub mod runtime;
pub mod session_library;
pub mod session_manager;
pub mod settings;
pub mod shared_config;
pub mod sidecar;
pub mod store;
pub mod terminal;
pub mod util;
pub mod workspace;
