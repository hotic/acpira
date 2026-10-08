//! The client side of one agent process generation: updates, permission / question requests, stderr and exit routed
//! into the session, with a replaced process's late callbacks ignored

use std::sync::{Arc, Weak};

use serde_json::{Value, json};

use acpira_shared::transcript::*;

use crate::acp::session::AcpSession;
use crate::acp::session::errors::auth_hint_of;
use crate::acp::session::queue::PeerTurn;
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::process::ClientHandlers;
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::acp::vendors::Vendor;
use crate::i18n::{t, tp};

/// The client handlers of one process generation: a replaced process's late callbacks are ignored
pub(crate) struct SessionHandlers {
  pub session: Weak<AcpSession>,
  pub gen_id: u64,
}

impl SessionHandlers {
  fn live(&self) -> Option<Arc<AcpSession>> {
    let s = self.session.upgrade()?;
    let current = s.core.lock().proc_gen;
    (current == self.gen_id).then_some(s)
  }
}

impl ClientHandlers for SessionHandlers {
  fn on_auth_retry(&self, params: Value) {
    let Some(s) = self.live().filter(|s| s.vendor == Vendor::Claude) else { return };
    let closing = {
      let mut c = s.core.lock();
      // A child's retries, replay and late frames must never terminate the root's current turn.
      if c.proc_gen != self.gen_id || c.replaying || !c.phase.running
        || params.get("sessionId").and_then(Value::as_str) != c.acp_session_id.as_deref() {
        return;
      }
      let message = t("host.claudeAuthRejected");
      s.log("Claude credentials rejected repeatedly (HTTP 401); ending the turn");
      s.settle(&mut c, TurnStop::Cancelled, Some(TurnError {
        message: message.clone(), kind: Some("authentication_failed".into()), retryable: Some(false),
        actions: Some(vec![FailureAction::Login]), ..Default::default()
      }));
      c.status = SessionStatus::AuthRequired;
      c.error = Some(message);
      // The CLI is still retrying. Detach its generation before killing it so a late response cannot erase the error.
      let closing = s.drop_process(&mut c);
      s.touch(&mut c);
      closing
    };
    if let Some(closing) = closing {
      tokio::spawn(closing);
    }
  }

  fn on_update(&self, params: Value) {
    if let Some(s) = self.live() {
      s.on_update(params);
    }
  }

  fn on_permission(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    Box::pin(async move {
      match s {
        Some(s) => s.on_permission(req, cancel).await,
        None => Ok(json!({ "outcome": { "outcome": "cancelled" } })),
      }
    })
  }

  fn on_elicitation(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_elicitation(req, cancel).await),
        None => Ok(json!({ "action": "cancel" })),
      }
    })
  }

  fn on_grok_question(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_grok_question(req, cancel).await),
        None => Ok(json!({ "outcome": "skip_interview" })),
      }
    })
  }

  fn on_stderr(&self, line: &str) {
    let Some(s) = self.live() else { return };
    s.log(&format!("stderr: {line}"));
    if let Some(hint) = auth_hint_of(line) {
      let mut c = s.core.lock();
      // stderr and the -32000 on stdout are separate pipes: a reason read after the failure still reaches the view
      if c.status == SessionStatus::AuthRequired && c.error.is_none() {
        c.error = Some(hint.clone());
        s.touch(&mut c);
      }
      c.auth_hint = Some(hint);
    }
  }

  fn on_exit(&self, code: Option<i32>, signal: Option<String>) {
    let Some(s) = self.session.upgrade() else { return };
    s.log(&format!(
      "exit code={} signal={}",
      code.map(|c| c.to_string()).unwrap_or_else(|| "null".into()),
      signal.as_deref().unwrap_or("null")
    ));
    let mut c = s.core.lock();
    if c.proc_gen != self.gen_id || c.status == SessionStatus::Closed {
      return;
    }
    let def = s.def();
    c.status = SessionStatus::Error;
    if c.error.is_none() {
      let why = code.map(|x| x.to_string()).or(signal).unwrap_or_else(|| "?".into());
      c.error = Some(tp("host.exited", &[("agent", &def.name), ("code", &why)]));
    }
    c.tree.settle("connection-lost");
    s.drain_terminal(&mut c);
    s.disconnect_tasks(&mut c);
    c.peer = PeerTurn::default();
    s.settle(&mut c, TurnStop::Cancelled, None);
    s.touch(&mut c);
  }
}
