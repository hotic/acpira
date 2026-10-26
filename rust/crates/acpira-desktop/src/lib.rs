//! The desktop app's logic without its UI toolkit (apps/desktop is the Tauri shell around it): the engine client
//! and its launch, the app's own stores, and the system side of the IDE actions. Nothing here depends on Tauri, so the
//! workspace's standard gates (cargo test, clippy) cover it. See docs/dev/desktop.md

pub mod browser;
pub mod client;
pub mod confine;
pub mod context_api;
pub mod dirs;
pub mod editors;
pub mod engine;
pub mod json_file;
pub mod locator;
pub mod login_env;
pub mod menu;
pub mod page;
pub mod platform;
pub mod process;
pub mod registry;
pub mod settings;
pub mod shared_settings;
pub mod single_instance;
pub mod terminal;
pub mod update;
pub mod vscode_settings;
pub mod window_state;
pub mod wire;
