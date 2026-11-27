//! Normalized transcript shape (mirror of packages/shared/transcript.ts): the host reduces ACP session/update into these
//! blocks and the webview only understands these. Optional TS fields are `Option` + `skip_serializing_if`, so the JSON
//! this side writes is the JSON the TS side wrote (absent, never `null`)

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::model_shapes::{ModelMemory, ModelShapes};
use crate::num::{Num, lenient_opt_i64, lenient_opt_u64};
use crate::subagents::SubagentSummary;

pub type AgentId = String;
pub type StrMap = BTreeMap<String, String>;

pub(crate) fn is_false(v: &bool) -> bool {
  !*v
}

mod agent;
mod blocks;
mod diff;
mod turns;
mod session;

pub use agent::*;
pub use blocks::*;
pub use diff::*;
pub use turns::*;
pub use session::*;
