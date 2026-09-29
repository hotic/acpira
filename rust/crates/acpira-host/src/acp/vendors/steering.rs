//! `_session/steering`: the ACP steering extension both JetBrains-maintained adapters implement. The request injects a prompt
//! into the running turn; its output streams as part of that turn and the pending `session/prompt` settles once, after it
//! (verified 2026-09-29 with `scripts/probe-steering.ts`: claude-agent-acp 0.83.0 on Sonnet, codex-acp 1.13.0 on GPT-6 Astra).
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
  fn outcomes_parse_and_unknown_ones_do_not() {
    assert_eq!(outcome_of(&json!({ "outcome": "injected" })), Some(Outcome::Injected));
    assert_eq!(outcome_of(&json!({ "outcome": "promptRequired", "reason": "noRunningTurn" })), Some(Outcome::PromptRequired));
    assert_eq!(outcome_of(&json!({ "outcome": "startedNewTurn" })), Some(Outcome::StartedNewTurn));
    assert_eq!(outcome_of(&json!({})), None);
    assert_eq!(params("s", &[json!({ "type": "text", "text": "hi" })])["_meta"]["steering"]["idleBehavior"], "promptRequired");
  }
}
