//! Contracts and pure logic shared by the Acpira host and webview. Each module mirrors the src/shared file of the
//! same name; the TypeScript side stays the source for the webview, this crate is what the Rust host speaks

// Wire enums mirror TS unions variant for variant; boxing large variants would only reshape the serde layer
#![allow(clippy::large_enum_variant)]

pub mod agent_order;
pub mod appearance;
pub mod attachments;
pub mod chatgpt_integration;
pub mod composer_controls;
pub mod export_transcript;
pub mod i18n;
pub mod inventory;
pub mod model_catalog;
pub mod model_shapes;
pub mod model_sources;
pub mod models;
pub mod num;
pub mod plan_execution;
pub mod protocol;
pub mod settings;
pub mod sidecar;
pub mod slash_commands;
pub mod subagents;
pub mod todo_tools;
pub mod transcript;
pub mod turn_errors;
pub mod turn_settings;
