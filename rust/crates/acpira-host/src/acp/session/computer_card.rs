//! Computer use's question to the user (`computer.rs`): may this session's agent see and control one app. A question
//! card like a plugin's, answered for the session, for good, or with a refusal

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use acpira_shared::transcript::{Question, QuestionKind, QuestionOption};

use crate::acp::session::AcpSession;
use crate::i18n::{t, tp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerAccess {
  /// Until the session ends
  Session,
  /// For every session from now on
  Always,
  Denied,
}

const QUESTION: &str = "access";

impl AcpSession {
  /// Asks the user whether the agent may drive the app called `app`. None when nobody answered within `wait`
  pub async fn computer_ask(self: &Arc<Self>, app: &str, wait: Duration) -> Option<ComputerAccess> {
    let option = |id: &str, key: &str| QuestionOption { id: id.into(), label: t(key), description: None };
    let question = Question {
      id: QUESTION.into(),
      title: None,
      text: tp("host.computer.ask", &[("app", app)]),
      kind: QuestionKind::Single,
      options: vec![option("session", "host.computer.allowSession"), option("always", "host.computer.allowAlways"), option("deny", "host.computer.deny")],
      other: Some(false),
      numeric: None,
      required: None,
    };
    let answer = self.host_ask(vec![question], None, wait).await?;
    Some(access_of(&answer))
  }
}

/// A dismissed or skipped card is a refusal: nothing is driven without a clear yes
fn access_of(answer: &Value) -> ComputerAccess {
  if answer.get("skipped") == Some(&Value::Bool(true)) || answer.get("cancelled") == Some(&Value::Bool(true)) {
    return ComputerAccess::Denied;
  }
  match answer.pointer(&format!("/answers/{QUESTION}")).and_then(Value::as_str) {
    Some("session") => ComputerAccess::Session,
    Some("always") => ComputerAccess::Always,
    _ => ComputerAccess::Denied,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn only_a_clear_yes_gives_access() {
    assert_eq!(access_of(&json!({ "skipped": false, "answers": { "access": "session" } })), ComputerAccess::Session);
    assert_eq!(access_of(&json!({ "skipped": false, "answers": { "access": "always" } })), ComputerAccess::Always);
    assert_eq!(access_of(&json!({ "skipped": false, "answers": { "access": "deny" } })), ComputerAccess::Denied);
    assert_eq!(access_of(&json!({ "skipped": true, "answers": { "access": "always" } })), ComputerAccess::Denied);
    assert_eq!(access_of(&json!({ "cancelled": true })), ComputerAccess::Denied);
    assert_eq!(access_of(&json!({ "skipped": false, "answers": {} })), ComputerAccess::Denied);
  }
}
