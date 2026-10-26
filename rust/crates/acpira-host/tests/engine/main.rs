//! Engine tests, one module per former TS suite (test/<name>.test.ts), built into a single test binary

#![allow(clippy::cloned_ref_to_slice_refs, reason = "`[x.clone()]` reads better than `slice::from_ref(&x)` on the expected side of an assertion")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::string_slice, reason = "test code")]

mod support;

mod accounts;
mod acp_session;
mod agent_process;
mod bridge;
mod builtin_agent;
mod chatgpt;
mod compaction;
mod failures;
mod golden;
mod harness;
mod inventory;
mod local_accounts;
mod normalize;
mod plans;
mod preview;
mod questions;
mod registry;
mod relay;
mod remote_connections;
mod remote_peer;
mod remote_workspace;
mod remote_helper;
mod session_library;
mod session_manager;
mod store;
mod subagents;
