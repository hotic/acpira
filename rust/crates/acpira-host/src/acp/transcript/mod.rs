pub mod compaction_text;
pub mod diff;
pub mod failure;
pub mod normalize;
pub mod plan_snapshots;
pub mod plans;
pub mod questions;
pub mod restore_turns;
pub mod retry_text;
pub mod subagent_tree;

/// Fields `u` reports replace the ones `into` holds; the rest stay.
pub(crate) fn merge_usage(into: &mut acpira_shared::transcript::TurnUsage, u: acpira_shared::transcript::TurnUsage) {
  macro_rules! take {
    ($($f:ident),*) => { $( if u.$f.is_some() { into.$f = u.$f; } )* };
  }
  take!(input, output, cached_read, cached_write, reasoning, total, model_calls, decode_ms, model, request_id, context);
}
