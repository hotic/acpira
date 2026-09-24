//! Error classification for sessions (mirror of src/host/acp/sessionErrors.ts)

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use acpira_shared::transcript::{PermissionKind, TurnError};

use super::rpc::RpcError;
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

fn data_text(r: &RpcError) -> Option<String> {
  let d = r.data.as_ref()?.as_object()?;
  ["message", "detail", "reason"].iter().find_map(|k| d.get(*k).and_then(Value::as_str).filter(|v| !v.trim().is_empty()).map(str::to_owned))
}

/// What a failed session/prompt leaves on the turn
pub fn turn_error_of(e: &anyhow::Error) -> TurnError {
  let Some(r) = rpc_of(e) else { return TurnError { message: e.to_string(), ..Default::default() } };
  let detail = data_text(r);
  let data = r.data.as_ref().and_then(Value::as_object);
  TurnError {
    message: match detail {
      Some(d) if d != r.message => format!("{}: {d}", r.message),
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
  let text = line.trim();
  if text.is_empty()
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
