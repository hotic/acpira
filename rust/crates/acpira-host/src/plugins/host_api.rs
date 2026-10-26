//! What a plugin's resident process may ask of the engine beyond its store (docs/dev/plugins.md, Session API): read
//! sessions, act on them the way a page does, show a toast, and make one model call from the built-in agent's model
//! sources. A process acts through a viewer of its own, so its actions take the path a click takes; answering a
//! permission or question card, plugin, account and setting changes are not on its list, so a plugin cannot approve
//! on the user's behalf

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use acpira_rpc::rpc::{BoxFuture, RpcError};
use serde_json::{Value, json};

use acpira_shared::protocol::WebviewMsg;

use crate::plugins::runtime::HostCall;
use crate::session_manager::{SessionManager, Viewer};

/// The page messages a process may send through `session/act`
pub const ACTIONS: &[&str] = &[
  "send",
  "stop",
  "setMode",
  "setConfig",
  "selectSession",
  "newSession",
  "renameSession",
  "deleteSession",
  "pinSession",
  "setSessionCategory",
  "forkSession",
  "compact",
  "retry",
  "retryTurn",
  "goal",
  "dequeue",
  "sendQueued",
  "steerQueued",
  "editQueued",
  "cancelSubagent",
  "stopAsyncTask",
  "reconnect",
];

/// Turns a `session/get` keeps when the caller names none
const TURNS_SHOWN: usize = 20;

pub type Toast = Arc<dyn Fn(&str, &str) + Send + Sync>;

struct Api {
  manager: Weak<SessionManager>,
  /// One viewer per plugin, attached on its first action
  viewers: parking_lot::Mutex<HashMap<String, Arc<Viewer>>>,
  toast: Toast,
  /// The built-in agent's home, where its model sources live
  agent_home: PathBuf,
}

/// The engine's answer to everything a process asks beyond its store
pub fn host_call(manager: &Arc<SessionManager>, toast: Toast, agent_home: PathBuf) -> HostCall {
  let api = Arc::new(Api { manager: Arc::downgrade(manager), viewers: Default::default(), toast, agent_home });
  Arc::new(move |plugin: String, method: String, params: Value| -> BoxFuture<Result<Value, RpcError>> {
    let api = api.clone();
    Box::pin(async move { api.call(&plugin, &method, params).await })
  })
}

fn bad(message: impl Into<String>) -> RpcError {
  RpcError::new(-32602, message)
}

impl Api {
  fn manager(&self) -> Result<Arc<SessionManager>, RpcError> {
    self.manager.upgrade().ok_or_else(|| RpcError::new(-32000, "the engine is closing"))
  }

  fn viewer(&self, m: &SessionManager, plugin: &str) -> Arc<Viewer> {
    self.viewers.lock().entry(plugin.to_owned()).or_insert_with(|| m.attach(None)).clone()
  }

  async fn call(&self, plugin: &str, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
      "session/list" => {
        let m = self.manager()?;
        let cwd = params.get("cwd").and_then(Value::as_str);
        let sessions: Vec<Value> = m
          .sessions()
          .into_iter()
          .filter_map(|s| serde_json::to_value(s).ok())
          .filter(|s| cwd.is_none_or(|c| s.get("cwd").and_then(Value::as_str) == Some(c)))
          .collect();
        Ok(json!({ "sessions": sessions }))
      }
      "session/get" => {
        let m = self.manager()?;
        let id = params.get("session").and_then(Value::as_str).ok_or_else(|| bad("`session` is needed"))?;
        let Some(s) = m.live_session(id) else { return Ok(json!({ "session": null })) };
        let mut view = serde_json::to_value(s.view()).map_err(|e| RpcError::internal(e.to_string()))?;
        // The newest turns only: a long session would flood the process
        let keep = params.get("turns").and_then(Value::as_u64).map_or(TURNS_SHOWN, |n| n as usize);
        if let Some(turns) = view.get_mut("turns").and_then(Value::as_array_mut)
          && turns.len() > keep
        {
          let cut = turns.len() - keep;
          turns.drain(..cut);
          view["turnsLeftOut"] = json!(cut);
        }
        Ok(json!({ "session": view }))
      }
      "session/act" => {
        let m = self.manager()?;
        let message = params.get("message").cloned().ok_or_else(|| bad("`message` is needed"))?;
        let kind = message.get("type").and_then(Value::as_str).unwrap_or_default();
        if !ACTIONS.contains(&kind) {
          return Err(bad(format!("`{kind}` is not an action a plugin may take; it may: {}", ACTIONS.join(", "))));
        }
        let msg: WebviewMsg = serde_json::from_value(message).map_err(|e| bad(format!("message: {e}")))?;
        let v = self.viewer(&m, plugin);
        m.handle_for(&v, msg).await;
        Ok(json!({ "active": v.active_id() }))
      }
      "ui/toast" => {
        let text = params.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).ok_or_else(|| bad("`text` is needed"))?;
        let level = params.get("level").and_then(Value::as_str).filter(|l| matches!(*l, "info" | "warning" | "error")).unwrap_or("info");
        let text: String = text.chars().take(500).collect();
        (self.toast)(level, &text);
        Ok(json!({}))
      }
      "ui/ask" => {
        let m = self.manager()?;
        let id = params.get("session").and_then(Value::as_str).ok_or_else(|| bad("`session` is needed"))?;
        let s = m.live_session(id).ok_or_else(|| bad(format!("session {id} is not open in this engine")))?;
        let questions: Vec<acpira_shared::transcript::Question> =
          serde_json::from_value(params.get("questions").cloned().unwrap_or(Value::Null)).map_err(|e| bad(format!("questions: {e}")))?;
        if questions.is_empty() {
          return Err(bad("`questions` is empty"));
        }
        let message = params.get("message").and_then(Value::as_str).map(str::to_owned);
        let wait = std::time::Duration::from_secs(params.get("waitSecs").and_then(Value::as_u64).unwrap_or(300));
        Ok(match s.plugin_ask(plugin, questions, message, wait).await {
          Some(answer) => answer,
          None => json!({ "timedOut": true }),
        })
      }
      "model/generate" => {
        let prompt = params.get("prompt").and_then(Value::as_str).ok_or_else(|| bad("`prompt` is needed"))?;
        let system = params.get("system").and_then(Value::as_str).unwrap_or_default();
        let model = params.get("model").and_then(Value::as_str);
        let max = params.get("maxTokens").and_then(Value::as_u64);
        let g = acpira_agent::generate::generate(&self.agent_home, model, system, prompt, max).await.map_err(|e| RpcError::new(-32000, e))?;
        Ok(json!({ "text": g.text, "model": g.model, "usage": { "input": g.input_tokens, "output": g.output_tokens } }))
      }
      _ => Err(RpcError::method_not_found(method)),
    }
  }
}
