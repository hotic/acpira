//! Top-level ACP JSON-RPC request and notification dispatch.
//! Sits above `acp`, `goal`, `context`, and `turn` so the session state machine stays acyclic.

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Inbound, RpcError};

use crate::acp::{SETTINGS_CHANGED, Server, Session};

pub struct Handler(pub Arc<Server>);

impl Inbound for Handler {
  fn notification(&self, method: &str, params: Value) {
    if method == "session/cancel" {
      self.0.cancel(&params);
    } else if method == SETTINGS_CHANGED {
      self.0.push_controls();
    } else if method == crate::remote::PROGRESS
      && let Some(id) = params.get("sessionId").and_then(Value::as_str)
      && let Ok(session) = self.0.session(id)
      && let Some(remote) = session.remote.clone()
    {
      remote.progress(&params);
    }
  }

  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let server = self.0.clone();
    Box::pin(async move {
      match method.as_str() {
        "initialize" => Ok(server.initialize(&params)),
        "session/new" => server.new_session(&params),
        "session/load" => server.restore(&params, true),
        "session/resume" => server.restore(&params, false),
        "session/list" => Ok(server.list(&params)),
        "session/set_mode" => server.set_mode(&params),
        "session/set_config_option" => server.set_config_option(&params),
        "session/prompt" => crate::turn::run(server, params, cancel).await,
        "_session/steering" => server.steer(&params),
        "_session/rewind" => server.rewind(&params),
        crate::goal::METHOD => crate::goal::control(&server, &params),
        crate::context::METHOD_REPORT => report_of(&server, &params).await,
        crate::context::METHOD_ITEM => item_of(&server, &params),
        crate::context::METHOD_RAW => raw_of(&server, &params),
        _ => Err(RpcError::method_not_found(&method)),
      }
    })
  }
}

fn session_of(server: &Server, params: &Value) -> Result<Arc<Session>, RpcError> {
  let id = params.get("sessionId").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "sessionId is required"))?;
  server.session(id)
}

/// `_acpira/context`: `{ report }`; a session opened without a call since gets its restored history, estimated
/// (`turn::opened_context`), none before the first call of a new one
pub async fn report_of(server: &Server, params: &Value) -> Result<Value, RpcError> {
  let session = session_of(server, params)?;
  let detail = params.get("detail").and_then(Value::as_bool) == Some(true);
  let (snap, size) = {
    let st = session.state.lock();
    (st.context.clone(), st.reported.map(|r| r.1))
  };
  let snap = match snap {
    Some(s) => Some(s),
    None => crate::turn::opened_context(server, &session).await,
  };
  let Some(snap) = snap else { return Ok(json!({})) };
  let rows = session.state.lock().rows.clone();
  let report = snap.report(size.unwrap_or(0), detail, &rows);
  Ok(json!({ "report": report }))
}

/// `_acpira/contextItem`: `{ content }` of a leaf of the report named by `rev`
pub fn item_of(server: &Server, params: &Value) -> Result<Value, RpcError> {
  let session = session_of(server, params)?;
  let (rev, id) = (params.get("rev").and_then(Value::as_str).unwrap_or_default(), params.get("id").and_then(Value::as_str).unwrap_or_default());
  let snap = session.state.lock().context.clone();
  Ok(match snap.and_then(|s| s.content(rev, id)) {
    Some(c) => json!({ "content": c }),
    None => json!({}),
  })
}

/// `_acpira/contextRaw`: `{ raw }`, the last request as it went out
pub fn raw_of(server: &Server, params: &Value) -> Result<Value, RpcError> {
  let session = session_of(server, params)?;
  let snap = session.state.lock().context.clone();
  Ok(match snap.and_then(|s| s.raw()) {
    Some(r) => json!({ "raw": r }),
    None => json!({}),
  })
}
