//! The ACP server side: the session table and the agent methods. Session state lives here; one turn of a session is
//! `turn::run`

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Connection, Inbound, RpcError};
use acpira_shared::providers::split_pick;

use crate::config::{Config, ConfigCache};
use crate::llm::Item;
use crate::permission::{Approval, Rule};

pub const PROTOCOL_VERSION: i64 = 1;
pub const AGENT_NAME: &str = "acpira";

pub const MODE_AGENT: &str = "agent";

/// What a session remembers between turns
#[derive(Default)]
pub struct SessionState {
  pub mode: String,
  /// `<provider>/<model>`; None until a model exists
  pub model: Option<String>,
  pub effort: Option<String>,
  /// Fixed for the session's life (prompt caches key on it)
  pub system: String,
  /// The conversation as the model sees it
  pub items: Vec<Item>,
  /// Text and reasoning of the model call in flight, kept if the turn is cancelled mid-stream
  pub partial_text: String,
  pub partial_reasoning: String,
  /// The running turn's cancel signal
  pub turn: Option<Cancel>,
  pub approval: Approval,
  /// Rules the user added from permission cards ("always allow"), for this session only
  pub allowed: Vec<Rule>,
}

pub struct Session {
  pub id: String,
  pub cwd: PathBuf,
  pub state: parking_lot::Mutex<SessionState>,
  tool_seq: AtomicU64,
}

impl Session {
  /// A tool call id unique in this session
  pub fn next_tool_id(&self) -> String {
    format!("call-{}", self.tool_seq.fetch_add(1, Ordering::Relaxed) + 1)
  }
}

pub struct Server {
  pub config: ConfigCache,
  pub version: String,
  pub http: ureq::Agent,
  conn: OnceLock<Connection>,
  sessions: parking_lot::Mutex<HashMap<String, Arc<Session>>>,
}

impl Server {
  pub fn new(home: PathBuf, version: impl Into<String>) -> Arc<Server> {
    Arc::new(Server {
      config: ConfigCache::new(home),
      version: version.into(),
      http: crate::llm::default_http(),
      conn: OnceLock::new(),
      sessions: Default::default(),
    })
  }

  /// The session's own directory under the data root (spilled tool outputs)
  pub fn session_dir(&self, id: &str) -> PathBuf {
    self.config.home().join("agent").join("sessions").join(id)
  }

  pub fn attach(&self, conn: Connection) {
    let _ = self.conn.set(conn);
  }

  pub fn conn(&self) -> &Connection {
    self.conn.get().expect("the connection is attached before the first message")
  }

  /// One `session/update` notification
  pub fn update(&self, session_id: &str, update: Value) {
    self.conn().notify("session/update", json!({ "sessionId": session_id, "update": update }));
  }

  pub fn session(&self, id: &str) -> Result<Arc<Session>, RpcError> {
    self.sessions.lock().get(id).cloned().ok_or_else(|| RpcError::new(-32602, format!("Unknown session: {id}")))
  }

  fn initialize(&self, _params: &Value) -> Value {
    json!({
      "protocolVersion": PROTOCOL_VERSION,
      "agentInfo": { "name": AGENT_NAME, "title": "Acpira", "version": self.version },
      "agentCapabilities": {
        "loadSession": false,
        "promptCapabilities": { "image": true, "embeddedContext": true },
      },
      "authMethods": [],
    })
  }

  fn new_session(&self, params: &Value) -> Result<Value, RpcError> {
    let cwd = params.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).ok_or_else(|| RpcError::new(-32602, "cwd is required"))?;
    let config = self.config.get();
    let id = uuid::Uuid::new_v4().to_string();
    let mut state = SessionState { mode: MODE_AGENT.to_owned(), system: crate::prompt::system_prompt(std::path::Path::new(cwd)), ..Default::default() };
    state.model = config.default_pick();
    state.effort = default_effort(&config, state.model.as_deref());
    let response = json!({
      "sessionId": id,
      "modes": modes_of(&state),
      "configOptions": config_options(&config, &state),
    });
    self.sessions.lock().insert(id.clone(), Arc::new(Session { id, cwd: PathBuf::from(cwd), state: parking_lot::Mutex::new(state), tool_seq: AtomicU64::new(0) }));
    Ok(response)
  }

  fn set_mode(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let mode = str_param(params, "modeId")?;
    if !MODES.iter().any(|(id, _, _)| *id == mode) {
      return Err(RpcError::new(-32602, format!("Unknown mode: {mode}")));
    }
    session.state.lock().mode = mode.to_owned();
    Ok(json!({}))
  }

  fn set_config_option(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let config_id = str_param(params, "configId")?;
    let value = params.get("value").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "value must be a string"))?;
    let config = self.config.get();
    let mut st = session.state.lock();
    match config_id {
      "model" => {
        if config.find(value).is_none() {
          return Err(RpcError::new(-32602, format!("Unknown model: {value}")));
        }
        st.model = Some(value.to_owned());
        // An effort the new model does not offer falls back to its default
        let efforts = efforts_of(&config, st.model.as_deref());
        if !st.effort.as_ref().is_some_and(|e| efforts.contains(e)) {
          st.effort = default_effort(&config, st.model.as_deref());
        }
      }
      "approval" => {
        st.approval = Approval::parse(value).ok_or_else(|| RpcError::new(-32602, format!("Unknown approval level: {value}")))?;
      }
      "effort" => {
        if !efforts_of(&config, st.model.as_deref()).iter().any(|e| e == value) {
          return Err(RpcError::new(-32602, format!("Unknown effort: {value}")));
        }
        st.effort = Some(value.to_owned());
      }
      other => return Err(RpcError::new(-32602, format!("Unknown config option: {other}"))),
    }
    Ok(json!({ "configOptions": config_options(&config, &st) }))
  }

  fn cancel(&self, params: &Value) {
    let Some(id) = params.get("sessionId").and_then(Value::as_str) else { return };
    if let Ok(session) = self.session(id)
      && let Some(c) = session.state.lock().turn.clone()
    {
      c.cancel();
    }
  }

}

/// (id, name, description)
pub const MODES: &[(&str, &str, &str)] = &[(MODE_AGENT, "Agent", "Reads, edits and runs commands, asking before each change")];

fn modes_of(st: &SessionState) -> Value {
  json!({
    "currentModeId": st.mode,
    "availableModes": MODES.iter().map(|(id, name, description)| json!({ "id": id, "name": name, "description": description })).collect::<Vec<_>>(),
  })
}

fn efforts_of(config: &Config, pick: Option<&str>) -> Vec<String> {
  pick.and_then(|p| config.find(p)).map(|(_, m)| m.efforts.clone()).unwrap_or_default()
}

/// `high` when offered, else the last (strongest) level
fn default_effort(config: &Config, pick: Option<&str>) -> Option<String> {
  let efforts = efforts_of(config, pick);
  efforts.iter().find(|e| *e == "high").or(efforts.last()).cloned()
}

/// The full configOptions set: the model select (every usable model, labelled with its source), the effort select when
/// the current model offers levels, and the approval level
pub fn config_options(config: &Config, st: &SessionState) -> Vec<Value> {
  let mut out = vec![];
  let current = st.model.as_deref().filter(|p| config.find(p).is_some()).map(str::to_owned).or_else(|| config.default_pick());
  if let Some(current) = current {
    let options: Vec<Value> = config
      .providers
      .usable()
      .map(|(p, m)| json!({ "value": acpira_shared::providers::model_pick(&p.id, &m.id), "name": m.display_name(), "description": p.display_name() }))
      .collect();
    out.push(json!({ "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": current, "options": options }));
    let efforts = efforts_of(config, Some(&current));
    if !efforts.is_empty() {
      let value = st.effort.clone().filter(|e| efforts.contains(e)).or_else(|| default_effort(config, Some(&current))).unwrap_or_default();
      out.push(json!({
        "id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": value,
        "options": efforts.iter().map(|e| json!({ "value": e, "name": title_case(e) })).collect::<Vec<_>>(),
      }));
    }
  }
  // How much runs without a permission card; applies from the next turn
  out.push(json!({
    "id": "approval", "name": "Approval", "type": "select", "currentValue": st.approval.id(),
    "options": Approval::ALL.iter().map(|a| json!({ "value": a.id(), "name": a.name(), "description": a.description() })).collect::<Vec<_>>(),
  }));
  out
}

fn title_case(s: &str) -> String {
  let mut c = s.chars();
  c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, RpcError> {
  params.get(key).and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, format!("{key} is required")))
}

/// The provider part of a pick, for messages
pub fn provider_of(pick: &str) -> &str {
  split_pick(pick).map(|(p, _)| p).unwrap_or(pick)
}

pub struct Handler(pub Arc<Server>);

impl Inbound for Handler {
  fn notification(&self, method: &str, params: Value) {
    if method == "session/cancel" {
      self.0.cancel(&params);
    }
  }

  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let server = self.0.clone();
    Box::pin(async move {
      match method.as_str() {
        "initialize" => Ok(server.initialize(&params)),
        "session/new" => server.new_session(&params),
        "session/set_mode" => server.set_mode(&params),
        "session/set_config_option" => server.set_config_option(&params),
        "session/prompt" => crate::turn::run(server, params, cancel).await,
        _ => Err(RpcError::method_not_found(&method)),
      }
    })
  }
}
