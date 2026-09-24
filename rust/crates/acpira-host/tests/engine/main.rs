//! Engine tests, one module per former TS suite (test/<name>.test.ts), built into a single test binary

// `[x.clone()]` reads better than `slice::from_ref(&x)` on the expected side of an assertion
#![allow(clippy::cloned_ref_to_slice_refs)]

mod support;

mod accounts;
mod acp_session;
mod agent_process;
mod bridge;
mod chatgpt;
mod compaction;
mod failures;
mod golden;
mod harness;
mod inventory;
mod local_accounts;
mod normalize;
mod plans;
mod questions;
mod registry;
mod session_manager;
mod store;
mod subagents;
