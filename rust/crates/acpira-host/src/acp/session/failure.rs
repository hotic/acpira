//! JetBrains AIR sessionFailure: structured warnings / errors at
//! `_meta.jetbrains.air.sessionFailure`; same id + higher revision updates in place

use serde_json::Value;

use acpira_shared::transcript::{FailureAction, FailureCategory, Severity, TurnError};

#[derive(Debug, Clone, PartialEq)]
pub struct SessionFailure {
  pub id: String,
  pub revision: f64,
  pub category: FailureCategory,
  pub severity: Severity,
  pub title: String,
  pub details: Option<String>,
  pub actions: Vec<FailureAction>,
}

fn text(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn js_string(v: Option<&Value>) -> String {
  match v {
    None => "undefined".into(),
    Some(Value::String(s)) => s.clone(),
    Some(other) => other.to_string(),
  }
}

/// Strict read of one payload; None when absent or malformed (malformed is logged)
pub fn failure_of(meta: Option<&Value>, log: Option<&dyn Fn(&str)>) -> Option<SessionFailure> {
  let raw = meta?.get("jetbrains")?.get("air")?.get("sessionFailure")?;
  if raw.is_null() {
    return None;
  }
  let fail = |why: String| -> Option<SessionFailure> {
    if let Some(l) = log {
      l(&format!("sessionFailure ignored ({why}): {raw}"));
    }
    None
  };
  let Some(p) = raw.as_object() else { return fail("not an object".into()) };
  let id = text(p.get("id"));
  let title = text(p.get("title"));
  let revision = p.get("revision").and_then(Value::as_f64).filter(|r| r.fract() == 0.0 && *r > 0.0);
  let severity = match p.get("severity").and_then(Value::as_str) {
    Some("warning") => Some(Severity::Warning),
    Some("error") => Some(Severity::Error),
    _ => None,
  };
  let Some(id) = id else { return fail("id missing".into()) };
  let Some(revision) = revision else { return fail(format!("revision not a positive integer: {}", js_string(p.get("revision")))) };
  let Some(severity) = severity else { return fail(format!("severity invalid: {}", js_string(p.get("severity")))) };
  let Some(title) = title else { return fail("title missing".into()) };
  let Some(actions_raw) = p.get("actions").and_then(Value::as_array) else { return fail("actions not an array".into()) };
  let category =
    p.get("category").and_then(|c| serde_json::from_value::<FailureCategory>(c.clone()).ok()).unwrap_or(FailureCategory::Unknown);
  let mut actions: Vec<FailureAction> = vec![];
  for a in actions_raw {
    if let Ok(a) = serde_json::from_value::<FailureAction>(a.clone())
      && !actions.contains(&a)
    {
      actions.push(a);
    }
  }
  let details = text(p.get("details")).or_else(|| text(p.get("reason")));
  Some(SessionFailure { id, revision, category, severity, title, details, actions })
}

fn category_name(c: FailureCategory) -> String {
  serde_json::to_value(c).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// The TurnError a turn-ending failure settles with; actions always present, even empty
pub fn failure_turn_error(f: &SessionFailure) -> TurnError {
  TurnError {
    message: match &f.details {
      Some(d) => format!("{}\n{d}", f.title),
      None => f.title.clone(),
    },
    code: None,
    kind: Some(category_name(f.category)),
    retryable: Some(f.actions.contains(&FailureAction::Retry)),
    failure_id: Some(f.id.clone()),
    actions: Some(f.actions.clone()),
  }
}
