//! test/normalize.test.ts

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::acp::transcript::diff::diff_lines;
use acpira_host::acp::transcript::normalize::*;
use acpira_host::acp::transport::wire::{AsyncTaskEvent, TaskEventKind};
use acpira_shared::transcript::*;

use crate::support::{Clock, expect_absent, expect_eq, expect_match, v};

fn state() -> NormalizeState {
  NormalizeState::new(vec![])
}

fn apply(s: &mut NormalizeState, u: Value) -> bool {
  apply_update(s, &u)
}

fn turn(j: Value) -> Turn {
  serde_json::from_value(j).expect("turn")
}

fn block(s: &NormalizeState, turn: usize, i: usize) -> Value {
  v(&s.turns[turn])["blocks"][i].clone()
}

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

mod content;
mod execution;
mod controls;
mod images;
mod tools;
mod permissions;
mod tasks;
mod prose;

fn saver(f: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static) -> Option<ImageSaver> {
  Some(Arc::new(f))
}

fn chunk(text: &str) -> Value {
  json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } })
}
