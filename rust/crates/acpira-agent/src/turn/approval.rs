//! The permission layers a call is decided under (defaults, the user's approvals, the mode) and the card that asks
//! the user when they leave it open

use std::sync::Arc;

use serde_json::{Value, json};

use super::calls::Prepared;
use crate::acp::{Server, Session};
use crate::permission::{self, Decision, Guard, Rule};
use crate::tools::{self, Action};

/// The permission layers of a turn, evaluated in this order: the mode last, so its restrictions hold under any approval
/// level and over what the user allowed earlier
pub(super) struct Rules {
  pub(super) defaults: Vec<Rule>,
  pub(super) approval: Vec<Rule>,
  pub(super) mode: Vec<Rule>,
  pub(super) guard: Guard,
}

impl Rules {
  pub(super) fn decide(&self, session: &Session, action: &Action) -> Decision {
    self.decide_with(session, action, None)
  }

  /// The decision with one more session rule: an "always allow" answer is offered only when it would take effect
  fn decide_with(&self, session: &Session, action: &Action, extra: Option<&Rule>) -> Decision {
    let mut allowed = session.state.lock().allowed.clone();
    allowed.extend(extra.cloned());
    // The strictest decision over every target the call touches
    let d = action
      .permissions(&session.cwd)
      .iter()
      .map(|(key, target)| permission::evaluate(&[&self.defaults, &self.approval, &allowed, &self.mode], key, target))
      .max_by_key(|d| match d {
        Decision::Allow => 0,
        Decision::Ask => 1,
        Decision::Deny => 2,
      })
      .unwrap_or(Decision::Ask);
    // A file the host's own guard protects (the change was prepared on an SSH host) asks like one the guard here does
    let protected_there = matches!(action, Action::Write { files } if files.iter().any(|f| f.protected));
    if d == Decision::Allow && (protected_there || action.written().iter().any(|p| self.guard.protects(p))) {
      return Decision::Ask;
    }
    d
  }

  /// The (permission, target) a call the rules deny is denied on: the first of its targets that is
  pub(super) fn denied(&self, session: &Session, action: &Action) -> (&'static str, String) {
    let allowed = session.state.lock().allowed.clone();
    action
      .permissions(&session.cwd)
      .into_iter()
      .find(|(key, target)| permission::evaluate(&[&self.defaults, &self.approval, &allowed, &self.mode], key, target) == Decision::Deny)
      .unwrap_or_else(|| action.permission(&session.cwd))
  }
}

pub(super) enum Answer {
  Once,
  /// Allowed for the rest of the session: the rule pattern
  Always(String),
  Reject,
}

/// Ask the user through ACP
pub(super) async fn ask(server: &Arc<Server>, session: &Arc<Session>, rules: &Rules, p: &Prepared, action: &Action) -> Answer {
  let pres = action.describe(&session.cwd);
  let raw_input = match action {
    Action::Bash { command, .. } => json!({ "command": command }),
    _ => serde_json::from_str(&p.call.arguments).unwrap_or(Value::Null),
  };
  // Offered only when the rule would hold: a mode rule or the guard can still ask after it
  let always = action.always().filter(|(pattern, _)| {
    let rule = Rule::new(action.permission(&session.cwd).0, pattern, Decision::Allow);
    rules.decide_with(session, action, Some(&rule)) == Decision::Allow
  });
  let mut options = vec![json!({ "optionId": "allow", "name": "Allow", "kind": "allow_once" })];
  if let Some((_, label)) = &always {
    options.push(json!({ "optionId": "always", "name": label, "kind": "allow_always" }));
  }
  options.push(json!({ "optionId": "reject", "name": "Reject", "kind": "reject_once" }));
  let mut request = json!({
    "sessionId": session.id,
    "toolCall": {
      "toolCallId": p.acp, "title": pres.title, "kind": pres.kind, "status": "pending", "rawInput": raw_input,
      "locations": pres.locations.iter().map(tools::Location::json).collect::<Vec<_>>(), "content": pres.content,
    },
    "options": options,
  });
  if let Some(m) = &p.meta {
    request["toolCall"]["_meta"] = m.clone();
  }
  let Ok(r) = server.conn().request("session/request_permission", request).await else { return Answer::Reject };
  if r.pointer("/outcome/outcome").and_then(Value::as_str) != Some("selected") {
    return Answer::Reject;
  }
  match (r.pointer("/outcome/optionId").and_then(Value::as_str), always) {
    (Some("allow"), _) => Answer::Once,
    (Some("always"), Some((pattern, _))) => Answer::Always(pattern),
    _ => Answer::Reject,
  }
}
