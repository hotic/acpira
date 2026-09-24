//! The Acpira host runtime. Each module mirrors the src/host file of the same name

// Dependency seams are closures (`Arc<dyn Fn(..) -> BoxFuture<..>>`) like the TS deps objects they mirror
#![allow(clippy::type_complexity)]

pub mod accounts;
pub mod acp;
pub mod agent_ext;
pub mod agents_cli;
pub mod bridge_core;
pub mod external;
pub mod file_rank;
pub mod http;
pub mod i18n;
pub mod inventory;
pub mod json;
pub mod limits;
pub mod node_files;
pub mod runtime;
pub mod session_manager;
pub mod settings;
pub mod sidecar;
pub mod store;
pub mod util;
