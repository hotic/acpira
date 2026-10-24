//! `_session/steering`: the ACP steering extension both JetBrains-maintained adapters implement. The request injects a prompt
//! into the running turn; its output streams as part of that turn and the pending `session/prompt` settles once, after it
//! (verified 2026-09-29 with `scripts/probe/steering.ts`: claude-agent-acp 0.83.0 on Sonnet, codex-acp 1.13.0 on GPT-6 Astra).
//! Idle, claude-agent-acp honours `idleBehavior: promptRequired`; codex-acp ignores it and starts a turn of its own
//! (`startedNewTurn`) that no `session/prompt` owns. Codex brackets every turn, that one included, with
//! `session_info_update` `_meta.codex.threadStatus` `active` … `idle`, the idle one ahead of the prompt response.

use serde_json::{Value, json};

pub const METHOD: &str = "_session/steering";

/// How the peer answered a steering request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
  /// Joined the running turn
  Injected,
  /// No turn was running; the content stays with the host, which sends it as a normal `session/prompt`
  PromptRequired,
  /// No turn was running and the peer started one on its own, outside any host `session/prompt`
  StartedNewTurn,
}

/// The request params: with `idleBehavior: promptRequired` an idle peer hands the prompt back instead of starting a turn
/// the host never sees settle
pub fn params(session_id: &str, prompt: &[Value]) -> Value {
  json!({ "sessionId": session_id, "prompt": prompt, "_meta": { "steering": { "idleBehavior": "promptRequired" } } })
}

pub fn outcome_of(v: &Value) -> Option<Outcome> {
  match v.get("outcome")?.as_str()? {
    "injected" => Some(Outcome::Injected),
    "promptRequired" => Some(Outcome::PromptRequired),
    "startedNewTurn" => Some(Outcome::StartedNewTurn),
    _ => None,
  }
}

/// Advertised at initialize (`_meta.steering.supported`)
pub fn supported(init: &Value) -> bool {
  init.pointer("/_meta/steering/supported").and_then(Value::as_bool) == Some(true)
}

/// claude-agent-acp drops the steered messages Claude Code has not picked up yet when the turn is cancelled
/// (`cancel` → `dropQueuedMessages`, from 0.86.0; source read 2026-10-15 in 0.86.0 and 0.90.0). Earlier adapters leave
/// them queued: once the interrupt lands Claude Code starts the next one at once, no idle comes, and the stop only
/// settles at the adapter's 30 s force-cancel floor (seen 2026-10-15 on 0.83.0). Only the dropping adapters may have the
/// host send those messages again
pub fn cancel_drops_unanswered(init: &Value) -> bool {
  init.pointer("/agentInfo/version").and_then(Value::as_str).and_then(version_triple).is_some_and(|v| v >= (0, 86, 0))
}

/// `major.minor.patch`, a pre-release or build suffix ignored
fn version_triple(v: &str) -> Option<(u64, u64, u64)> {
  let core = v.split(['-', '+']).next()?;
  let mut parts = core.split('.').map(str::parse::<u64>);
  Some((parts.next()?.ok()?, parts.next()?.ok()?, parts.next().unwrap_or(Ok(0)).ok()?))
}

/// Codex's turn bracket on a root `session_info_update`: `Some(true)` when its thread went idle, `Some(false)` when active
pub fn thread_idle(update: &Value) -> Option<bool> {
  match update.pointer("/_meta/codex/threadStatus/type")?.as_str()? {
    "idle" => Some(true),
    "active" => Some(false),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_advertising_peers_are_steered() {
    assert!(supported(&json!({ "_meta": { "steering": { "supported": true } } })));
    assert!(!supported(&json!({ "agentInfo": { "name": "devin" } })));
    assert!(!supported(&json!({ "_meta": { "steering": { "supported": "yes" } } })));
  }

  #[test]
  fn codex_thread_status_reads_as_the_turn_bracket() {
    let status = |t: &str| json!({ "sessionUpdate": "session_info_update", "_meta": { "codex": { "threadStatus": { "type": t } } } });
    assert_eq!(thread_idle(&status("idle")), Some(true));
    assert_eq!(thread_idle(&status("active")), Some(false));
    assert_eq!(thread_idle(&status("notLoaded")), None);
    assert_eq!(thread_idle(&json!({ "sessionUpdate": "session_info_update", "title": "x" })), None);
  }

  #[test]
  fn only_adapters_from_0_86_drop_unanswered_steers_on_cancel() {
    let at = |v: &str| cancel_drops_unanswered(&json!({ "agentInfo": { "name": "@agentclientprotocol/claude-agent-acp", "version": v } }));
    assert!(!at("0.83.0"));
    assert!(!at("0.85.1"));
    assert!(at("0.86.0"));
    assert!(at("0.90.1-preview.11"));
    assert!(at("1.0"));
    assert!(!at("next"));
    assert!(!cancel_drops_unanswered(&json!({})));
  }

  #[test]
  fn outcomes_parse_and_unknown_ones_do_not() {
    assert_eq!(outcome_of(&json!({ "outcome": "injected" })), Some(Outcome::Injected));
    assert_eq!(outcome_of(&json!({ "outcome": "promptRequired", "reason": "noRunningTurn" })), Some(Outcome::PromptRequired));
    assert_eq!(outcome_of(&json!({ "outcome": "startedNewTurn" })), Some(Outcome::StartedNewTurn));
    assert_eq!(outcome_of(&json!({})), None);
    assert_eq!(params("s", &[json!({ "type": "text", "text": "hi" })])["_meta"]["steering"]["idleBehavior"], "promptRequired");
  }
}
