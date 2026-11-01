//! The permission layers a call is decided under (defaults, the user's approvals, the mode) and the card that asks
//! the user when they leave it open

use std::path::PathBuf;
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
  pub(super) plan: PathBuf,
  pub(super) guard: Guard,
}

impl Rules {
  pub(super) fn decide(&self, session: &Session, action: &Action) -> Decision {
    self.decide_with(session, action, None)
  }

  /// The decision with one more session rule: an "always allow" answer is offered only when it would take effect
  fn decide_with(&self, session: &Session, action: &Action, extra: Option<&Rule>) -> Decision {
    let st = session.state.lock();
    let approval = st.approval.rules();
    let mode = crate::modes::get(&st.mode).rules(&self.plan, &session.cwd, st.approval);
    let mut allowed = st.allowed.clone();
    drop(st);
    allowed.extend(extra.cloned());
    // The strictest decision over every target the call touches
    let d = action
      .permissions(&session.cwd)
      .iter()
      .map(|(key, target)| permission::evaluate(&[&self.defaults, &approval, &allowed, &mode], key, target))
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
    let st = session.state.lock();
    let approval = st.approval.rules();
    let mode = crate::modes::get(&st.mode).rules(&self.plan, &session.cwd, st.approval);
    let allowed = st.allowed.clone();
    drop(st);
    action
      .permissions(&session.cwd)
      .into_iter()
      .find(|(key, target)| permission::evaluate(&[&self.defaults, &approval, &allowed, &mode], key, target) == Decision::Deny)
      .unwrap_or_else(|| action.permission(&session.cwd))
  }
}

pub(super) enum Answer {
  Once,
  /// Allowed for the rest of the session: the rule pattern
  Always(String),
  Reject,
  /// A mode switch now forbids the waiting action
  Denied,
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
  // Keep an Ask card in place; only a changed decision withdraws it. An automatic allow still goes through the host's
  // workspace and plugin gates, so switching levels cannot override a plugin that insists on a person's approval.
  let changed = session.permissions_changed.notified();
  tokio::pin!(changed);
  changed.as_mut().enable();
  let mut decision = rules.decide(session, action);
  'ask: loop {
    let r = 'request: loop {
      if decision == Decision::Deny {
        return Answer::Denied;
      }
      request["_meta"] = json!({ "acpira/autoApprove": decision == Decision::Allow });
      let pending = server.conn().request_cancellable("session/request_permission", request.clone());
      tokio::pin!(pending);
      loop {
        tokio::select! {
          r = &mut pending => break 'request r,
          _ = &mut changed => {
            changed.set(session.permissions_changed.notified());
            changed.as_mut().enable();
            let next = rules.decide(session, action);
            if next != decision {
              decision = next;
              break;
            }
          }
        }
      }
      // Dropping the old request withdraws its card before the replacement request is sent.
    };
    let Ok(r) = r else { return Answer::Reject };
    if r.pointer("/outcome/outcome").and_then(Value::as_str) != Some("selected") {
      return Answer::Reject;
    }
    // A reply racing a restrictive mode change must not authorize an action that mode now denies.
    let now = rules.decide(session, action);
    if now == Decision::Deny {
      return Answer::Denied;
    }
    // An automatic allow answered after the level went back to asking: a person decides after all
    if decision == Decision::Allow && now != Decision::Allow {
      decision = now;
      continue 'ask;
    }
    return match (r.pointer("/outcome/optionId").and_then(Value::as_str), always) {
      (Some("allow"), _) => Answer::Once,
      (Some("always"), Some((pattern, _))) => Answer::Always(pattern),
      _ => Answer::Reject,
    };
  }
}
