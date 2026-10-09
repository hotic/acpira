//! Error classification for sessions

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use acpira_shared::transcript::{PermissionKind, TurnError};

use crate::acp::transport::rpc::RpcError;
use crate::i18n::t;

/// Credential hand-off by the account layer failed: auth_required like -32000, but the reason must reach the user
#[derive(Debug)]
pub struct AccountAuthError(pub String);

impl std::fmt::Display for AccountAuthError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

impl std::error::Error for AccountAuthError {}

pub fn rpc_of(e: &anyhow::Error) -> Option<&RpcError> {
  e.downcast_ref::<RpcError>()
}

static AUTH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)auth").unwrap());
static AUTH_WHY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)required|login|unauthor").unwrap());
static AUTH_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)auth|credential|login|logged|unauthor").unwrap());
static GLOG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([IWEFD])\d{4} \d{2}:\d{2}:\d{2}\.\d+\s+\d+\s+[\w.-]+:\d+\]").unwrap());
/// A sign-in link printed while the browser opens (antigravity-acp `Open the following link to authenticate …: <url>`)
static SIGN_IN_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:open|visit|go to)\b.*\bhttps?://").unwrap());
static GONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)session not found").unwrap());
static LOCKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)already active|in use|held by|locked").unwrap());
static UNRESUMABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)not resumable|cannot be resumed").unwrap());
static RESTORE_FAILED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bcwd\b|working directory|absolute path|\bmcp\b").unwrap());

/// Which option auto-approval picks: allow_always, then allow_once, otherwise the first one
pub fn best_allow(options: &[Value]) -> anyhow::Result<String> {
  let kind = |k: &str| options.iter().find(|o| o.get("kind").and_then(Value::as_str) == Some(k));
  let o = kind("allow_always")
    .or_else(|| kind("allow_once"))
    .or(options.first())
    .ok_or_else(|| anyhow::anyhow!(t("host.noPermissionOptions")))?;
  Ok(o.get("optionId").and_then(Value::as_str).unwrap_or("").to_owned())
}

/// The option auto-approval may pick without asking; `None` when the set is a list of answers rather than an approval
/// ladder: several `allow_once` options and no `allow_always` (Antigravity's `ask_question`, where every answer is
/// `allow_once`). Codex's "this turn" / "this turn with strict auto review" pair comes with an `allow_always`, so it
/// stays a ladder
pub fn auto_allow(options: &[Value]) -> Option<String> {
  if ambiguous_allow(options) { None } else { best_allow(options).ok() }
}

/// Two or more `allow_once` options and no `allow_always`: nothing tells which one a person would have picked
pub fn ambiguous_allow(options: &[Value]) -> bool {
  let count = |k: &str| options.iter().filter(|o| o.get("kind").and_then(Value::as_str) == Some(k)).count();
  count("allow_always") == 0 && count("allow_once") > 1
}

pub fn permission_kind(v: &Value) -> PermissionKind {
  serde_json::from_value(v.clone()).unwrap_or(PermissionKind::RejectOnce)
}

pub fn is_auth(e: &anyhow::Error) -> bool {
  if e.downcast_ref::<AccountAuthError>().is_some() {
    return true;
  }
  if let Some(r) = rpc_of(e) {
    return r.code == -32000;
  }
  let m = e.to_string();
  AUTH_RE.is_match(&m) && AUTH_WHY.is_match(&m)
}

/// The reason under `data`: the ACP SDK answers a thrown plain Error with "Internal error" and the thrown text as
/// `data.details` (claude-agent-acp's missing native binary, codex's missing optional dependency), other peers use
/// message / detail / reason
fn data_text(r: &RpcError) -> Option<String> {
  let d = r.data.as_ref()?.as_object()?;
  ["message", "details", "detail", "reason"]
    .iter()
    .find_map(|k| d.get(*k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned))
}

/// An error as shown to the user: an RPC error keeps the peer's reason from `data` next to its generic message
pub fn error_text(e: &anyhow::Error) -> String {
  let text = e.to_string();
  match rpc_of(e).and_then(data_text) {
    Some(d) if !text.contains(&d) => format!("{text}: {d}"),
    _ => text,
  }
}

/// What a failed session/prompt leaves on the turn
pub fn turn_error_of(e: &anyhow::Error) -> TurnError {
  let Some(r) = rpc_of(e) else { return TurnError { message: e.to_string(), ..Default::default() } };
  let detail = data_text(r);
  let data = r.data.as_ref().and_then(Value::as_object);
  TurnError {
    message: match detail {
      Some(d) if !r.message.contains(&d) => format!("{}: {d}", r.message),
      _ => r.message.clone(),
    },
    code: Some(r.code),
    kind: data.and_then(|d| d.get("cognition.ai/errorKind")).and_then(Value::as_str).map(str::to_owned),
    retryable: data.and_then(|d| d.get("cognition.ai/retryable")).and_then(Value::as_bool),
    ..Default::default()
  }
}

/// A human-readable reason out of one stderr line when it is about authentication
pub fn auth_hint_of(line: &str) -> Option<String> {
  let mut text = line.trim();
  // absl / glog (antigravity-acp): info and debug lines are progress, not a reason; warnings and errors lose the prefix
  if let Some(m) = GLOG.captures(text) {
    if matches!(&m[1], "I" | "D") {
      return None;
    }
    text = text[m.get(0).unwrap().end()..].trim();
  }
  if text.is_empty()
    || SIGN_IN_LINK.is_match(text)
    || text.contains("jsonrpc::outgoing_actor")
    || text.contains("ACP: Creating session without credentials - agent may not work")
  {
    return None;
  }
  if text.starts_with('{')
    && let Ok(Value::Object(j)) = serde_json::from_str::<Value>(text)
  {
    let m = j.get("msg").and_then(Value::as_str).or_else(|| j.get("message").and_then(Value::as_str)).unwrap_or("");
    let err = j.get("error").and_then(Value::as_str).or_else(|| j.get("err").and_then(Value::as_str));
    if !AUTH_WORDS.is_match(&format!("{m} {}", err.unwrap_or(""))) {
      return None;
    }
    return err.map(str::to_owned).or_else(|| (!m.is_empty()).then(|| m.to_owned()));
  }
  AUTH_WORDS.is_match(text).then(|| text.to_owned())
}

static QUOTA_TEXT: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?i)usage (?:quota|limit) has been (?:exhausted|reached)|usage limit reached|^You've (?:hit|reached) your|out of usage").unwrap());

/// The account behind the session ran out of allowance, so another account can carry on: Devin's typed
/// `resource_exhausted` (-32011), the AIR `quota_exhausted` failure codex-acp / claude-agent-acp send (category limit with no
/// remedy action — rate limits offer retry, context and budget limits offer new_session), or the plain texts both CLIs use
/// when AIR is not negotiated
pub fn is_quota_exhausted(e: &TurnError) -> bool {
  if e.kind.as_deref() == Some("resource_exhausted") {
    return true;
  }
  if e.failure_id.is_some() {
    return e.kind.as_deref() == Some("limit") && e.actions.as_ref().is_some_and(Vec::is_empty);
  }
  QUOTA_TEXT.is_match(&e.message)
}

fn error_kind(e: &anyhow::Error) -> Option<String> {
  rpc_of(e)?.data.as_ref()?.get("cognition.ai/errorKind")?.as_str().map(str::to_owned)
}

pub fn is_session_gone(e: &anyhow::Error) -> bool {
  error_kind(e).as_deref() == Some("session_not_found") || GONE.is_match(&e.to_string())
}

pub fn is_session_locked(e: &anyhow::Error) -> bool {
  error_kind(e).as_deref() == Some("session_locked")
}

pub fn is_method_missing(e: &anyhow::Error) -> bool {
  rpc_of(e).is_some_and(|r| r.code == -32601)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreFailure {
  Gone,
  Locked,
  Unresumable,
  Failed,
}

/// How a failed session/resume or session/load ended; None = the method isn't there at all
pub fn classify_restore_error(e: &anyhow::Error) -> Option<RestoreFailure> {
  if is_session_gone(e) {
    return Some(RestoreFailure::Gone);
  }
  // ACP's resourceNotFound (-32002, "Resource not found: <sessionId>"): claude-agent-acp answers a resume this way when
  // Claude Code never wrote the conversation (a session/new that was never prompted) or no longer has it
  if rpc_of(e).is_some_and(|r| r.code == -32002) {
    return Some(RestoreFailure::Gone);
  }
  if is_session_locked(e) {
    return Some(RestoreFailure::Locked);
  }
  if is_method_missing(e) {
    return None;
  }
  if let Some(r) = rpc_of(e)
    && r.code == -32602
  {
    let extra: Vec<&str> = r
      .data
      .as_ref()
      .and_then(Value::as_object)
      .map(|d| ["message", "detail", "reason"].iter().filter_map(|k| d.get(*k).and_then(Value::as_str)).collect())
      .unwrap_or_default();
    let text = format!("{} {}", r.message, extra.join(" "));
    if LOCKED.is_match(&text) {
      return Some(RestoreFailure::Locked);
    }
    if UNRESUMABLE.is_match(&text) {
      return Some(RestoreFailure::Unresumable);
    }
    if RESTORE_FAILED.is_match(&text) {
      return Some(RestoreFailure::Failed);
    }
    return Some(RestoreFailure::Gone);
  }
  Some(RestoreFailure::Failed)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn rpc(message: &str, data: Option<Value>) -> anyhow::Error {
    anyhow::Error::new(RpcError { code: -32603, message: message.into(), data })
  }

  #[test]
  fn the_sdk_internal_error_keeps_its_details() {
    // claude-agent-acp 0.83 without its platform package, as the ACP SDK's errorToResult sends it
    let why = "Claude native binary not found for linux-x64. Reinstall @anthropic-ai/claude-agent-sdk without --omit=optional, or set CLAUDE_CODE_EXECUTABLE.";
    let e = rpc("Internal error", Some(json!({ "details": why })));
    assert_eq!(error_text(&e), format!("Internal error: {why}"));
    assert_eq!(turn_error_of(&e).message, format!("Internal error: {why}"));
  }

  #[test]
  fn antigravity_progress_logs_are_not_login_reasons() {
    // antigravity-acp 1.1.1 stderr around a browser login
    for line in [
      "I1001 10:07:20.330541 8325766784 server.py:2390] Authenticate called with method_id='oauth-personal', kwargs={}",
      "I1001 10:07:20.536166 6149386240 credential_manager.py:553] Credentials missing or invalid. Launching browser login flow...",
      "I1001 10:08:55.140615 8325766784 settings.py:302] settings: path=/Users/x/.gemini/antigravity-acp/settings.json status=ok auth.type=oauth-personal",
      "Open the following link to authenticate the ACP server: https://accounts.google.com/o/oauth2/v2/auth?response_type=code",
    ] {
      assert_eq!(auth_hint_of(line), None, "{line}");
    }
    assert_eq!(
      auth_hint_of("E1001 10:09:01.701020 8325766784 oauth_manager.py:90] Credential refresh failed: invalid_grant").as_deref(),
      Some("Credential refresh failed: invalid_grant")
    );
    assert_eq!(auth_hint_of("Error: not logged in").as_deref(), Some("Error: not logged in"));
  }

  #[test]
  fn auto_approval_skips_a_list_of_answers() {
    let o = |id: &str, kind: &str| json!({ "optionId": id, "name": id, "kind": kind });
    // antigravity-acp 1.2.1: Allow Always / Allow / Deny
    assert_eq!(auto_allow(&[o("allow_always", "allow_always"), o("allow", "allow_once"), o("deny", "reject_once")]).as_deref(), Some("allow_always"));
    assert_eq!(auto_allow(&[o("allow", "allow_once"), o("deny", "reject_once")]).as_deref(), Some("allow"));
    // ask_question answers
    assert_eq!(auto_allow(&[o("blue", "allow_once"), o("green", "allow_once"), o("deny", "reject_once")]), None);
    // Codex permission profile: two allow_once under an allow_always
    assert_eq!(
      auto_allow(&[o("turn", "allow_once"), o("strict", "allow_once"), o("session", "allow_always"), o("no", "reject_once")]).as_deref(),
      Some("session")
    );
  }

  #[test]
  fn nothing_is_repeated_or_invented() {
    assert_eq!(error_text(&rpc("Internal error", None)), "Internal error");
    assert_eq!(error_text(&rpc("Internal error", Some(json!({})))), "Internal error");
    assert_eq!(error_text(&rpc("Boom: disk full", Some(json!({ "details": "disk full" })))), "Boom: disk full");
    assert_eq!(error_text(&anyhow::anyhow!("plain")), "plain");
  }
}
