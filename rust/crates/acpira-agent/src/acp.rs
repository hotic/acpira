//! The ACP server side: the session table and the agent methods. Session state lives here; one turn of a session is
//! `turn::run`

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Connection, Inbound, RpcError};
use acpira_shared::model_catalog::Catalog;
use acpira_shared::providers::{Thinking, split_pick};

use crate::config::{Config, ConfigCache};
use crate::llm::Item;
use crate::modes;
use crate::permission::Rule;
use crate::store::{self, Store};

pub const PROTOCOL_VERSION: i64 = 1;
pub const AGENT_NAME: &str = "acpira";

/// What a session remembers between turns
#[derive(Default)]
pub struct SessionState {
  pub mode: String,
  /// The mode the model was last told about (`modes::Mode::overlay`)
  pub announced: String,
  /// `<provider>/<model>`; None until a model exists
  pub model: Option<String>,
  pub effort: Option<String>,
  /// The system prompt, fixed while the model's variant stays the same (prompt caches key on it)
  pub prompt: crate::prompt::Composed,
  /// The conversation as the model sees it
  pub items: Vec<Item>,
  /// Text and reasoning of the model call in flight, kept if the turn is cancelled mid-stream
  pub partial_text: String,
  pub partial_reasoning: String,
  /// The running turn's cancel signal
  pub turn: Option<Cancel>,
  /// Rules the user added from permission cards ("always allow"), for this session only
  pub allowed: Vec<Rule>,
  /// What the last model request's prefix was built from (`turn.rs` logs a `view` event when it changes)
  pub last_view: Option<Value>,
  /// The config options the client last got for this session (`Server::push_controls` sends a change)
  pub sent_options: Option<Vec<Value>>,
}

pub struct Session {
  pub id: String,
  pub cwd: PathBuf,
  pub state: parking_lot::Mutex<SessionState>,
  pub store: Store,
  tool_seq: AtomicU64,
  /// Background commands started in this session; dropping the session stops them
  pub jobs: Arc<crate::tools::jobs::Jobs>,
}

impl Session {
  /// A tool call id unique in this session
  pub fn next_tool_id(&self) -> String {
    format!("call-{}", self.tool_seq.fetch_add(1, Ordering::Relaxed) + 1)
  }

  /// Record the controls at the start and after a change, so a reopened session comes back with them (held until the
  /// session has a file: changing a control alone does not make one)
  pub fn save_state(&self, st: &SessionState) {
    self.store.prelude(json!({
      "type": "state", "mode": st.mode, "announced": st.announced, "model": st.model, "effort": st.effort,
    }));
  }

  /// Record a newly composed system prompt
  pub fn save_prompt(&self, p: &crate::prompt::Composed) {
    self.store.append(json!({
      "type": "prompt", "variant": p.variant, "version": p.version, "digest": p.digest, "text": p.text,
      "sampling": p.sampling, "thinking": p.thinking,
    }));
  }
}

pub struct Server {
  pub config: ConfigCache,
  pub catalog: crate::catalog::CatalogCache,
  pub version: String,
  pub http: ureq::Agent,
  conn: OnceLock<Connection>,
  sessions: parking_lot::Mutex<HashMap<String, Arc<Session>>>,
}

impl Server {
  pub fn new(home: PathBuf, version: impl Into<String>) -> Arc<Server> {
    Arc::new(Server {
      catalog: crate::catalog::CatalogCache::new(&home),
      config: ConfigCache::new(home),
      version: version.into(),
      http: crate::llm::default_http(),
      conn: OnceLock::new(),
      sessions: Default::default(),
    })
  }

  /// The session's own directory under the data root (spilled tool outputs, the plan)
  pub fn session_dir(&self, id: &str) -> PathBuf {
    self.config.home().join("agent").join("sessions").join(id)
  }

  /// The session's history (`store.rs`)
  pub fn session_file(&self, id: &str) -> PathBuf {
    self.config.home().join("agent").join("sessions").join(format!("{id}.jsonl"))
  }

  /// The file Plan mode writes its plan to
  pub fn plan_file(&self, id: &str) -> PathBuf {
    self.session_dir(id).join("plan.md")
  }

  pub fn attach(&self, conn: Connection) {
    let _ = self.conn.set(conn);
  }

  pub fn conn(&self) -> &Connection {
    self.conn.get().expect("the connection is attached before the first message")
  }

  /// One `session/update` notification, recorded for a later replay
  pub fn update(&self, session_id: &str, update: Value) {
    if let Some(s) = self.sessions.lock().get(session_id).cloned() {
      s.store.record_update(&update);
    }
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
        "loadSession": true,
        "promptCapabilities": { "image": true, "embeddedContext": true },
        "sessionCapabilities": { "list": {}, "resume": {} },
      },
      "authMethods": [],
    })
  }

  fn new_session(&self, params: &Value) -> Result<Value, RpcError> {
    let cwd = params.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).ok_or_else(|| RpcError::new(-32602, "cwd is required"))?;
    let config = self.config.get();
    let catalog = self.catalog.get();
    let id = uuid::Uuid::new_v4().to_string();
    let mut state = SessionState { mode: modes::AGENT.to_owned(), announced: modes::AGENT.to_owned(), ..Default::default() };
    state.model = config.default_pick();
    let places = crate::prompt::Places::new(std::path::Path::new(cwd), crate::turn::user_home().as_deref());
    state.prompt = crate::prompt::compose(&places, state.model.as_deref().and_then(|p| config.find(p)));
    state.effort = default_effort(&config, &catalog, state.model.as_deref());
    let response = json!({
      "sessionId": id,
      "modes": modes_of(&state),
      "configOptions": sent_options(&config, &catalog, &mut state),
    });
    let store = Store::new(self.session_file(&id), &id, std::path::Path::new(cwd));
    // The file starts with the first event; the prompt goes first so a reopen keeps the very same text
    store.prelude(json!({ "type": "prompt", "variant": state.prompt.variant, "version": state.prompt.version, "digest": state.prompt.digest,
      "text": state.prompt.text, "sampling": state.prompt.sampling, "thinking": state.prompt.thinking }));
    let session = Session { id: id.clone(), cwd: PathBuf::from(cwd), state: parking_lot::Mutex::new(state), store, tool_seq: AtomicU64::new(0), jobs: Default::default() };
    // The starting controls too: a reopen must not take a default that changed in the meantime
    session.save_state(&session.state.lock());
    self.sessions.lock().insert(id, Arc::new(session));
    Ok(response)
  }

  /// `session/load` (replay = true) and `session/resume`: a session open in this process continues as it is, another
  /// one is read back from its file. The replay sends the recorded updates before the answer
  fn restore(&self, params: &Value, replay: bool) -> Result<Value, RpcError> {
    let id = str_param(params, "sessionId")?;
    let gone = || RpcError::new(-32002, format!("Session not found: {id}"));
    let path = self.session_file(id);
    let open = self.sessions.lock().get(id).cloned();
    let (session, updates) = match open {
      Some(s) => {
        s.store.flush();
        let updates = if replay { store::load(&path, false).map(|l| l.updates).unwrap_or_default() } else { vec![] };
        (s, updates)
      }
      None => {
        let loaded = store::load(&path, false).map_err(|_| gone())?;
        if loaded.id != id {
          return Err(gone());
        }
        let cwd = params.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).map(PathBuf::from).unwrap_or(loaded.cwd.clone());
        let session = Arc::new(self.revive(id, cwd, &loaded, path));
        self.sessions.lock().insert(id.to_owned(), session.clone());
        (session, loaded.updates)
      }
    };
    if replay {
      // Straight to the client: a replay is not recorded again
      for u in updates {
        self.conn().notify("session/update", json!({ "sessionId": id, "update": u }));
      }
    }
    let config = self.config.get();
    let catalog = self.catalog.get();
    let mut st = session.state.lock();
    if reconcile(&config, &catalog, &mut st) {
      session.save_state(&st);
    }
    Ok(json!({ "modes": modes_of(&st), "configOptions": sent_options(&config, &catalog, &mut st) }))
  }

  /// A session object from its file: the history, the controls and the system prompt as they were
  fn revive(&self, id: &str, cwd: PathBuf, loaded: &store::Loaded, path: PathBuf) -> Session {
    let config = self.config.get();
    let catalog = self.catalog.get();
    let s = |k: &str| loaded.state.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut st = SessionState {
      mode: s("mode").filter(|m| modes::find(m).is_some()).unwrap_or_else(|| modes::AGENT.to_owned()),
      announced: s("announced").unwrap_or_else(|| modes::AGENT.to_owned()),
      model: s("model").or_else(|| config.default_pick()),
      items: loaded.items.clone(),
      ..Default::default()
    };
    st.effort = s("effort").or_else(|| default_effort(&config, &catalog, st.model.as_deref()));
    if let Some(p) = &loaded.prompt {
      let ps = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
      st.prompt = crate::prompt::Composed {
        text: ps("text"),
        variant: ps("variant"),
        version: ps("version"),
        digest: ps("digest"),
        sampling: p.get("sampling").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        thinking: p.get("thinking").cloned().and_then(|v| serde_json::from_value(v).ok()),
      };
    }
    // A turn cut off by a crash leaves calls without results; the next request must be well formed
    let before = st.items.len();
    crate::turn::settle(&mut st);
    let store = Store::new(path, id, &cwd);
    for item in &st.items[before..] {
      store.item(item);
    }
    // Tool call ids continue after the ones the replay holds
    let seq = loaded
      .updates
      .iter()
      .filter_map(|u| u.get("toolCallId").and_then(Value::as_str)?.strip_prefix("call-")?.parse::<u64>().ok())
      .max()
      .unwrap_or(0);
    Session { id: id.to_owned(), cwd, state: parking_lot::Mutex::new(st), store, tool_seq: AtomicU64::new(seq), jobs: Default::default() }
  }

  /// `session/list`: this data root's sessions with at least one message, newest first, filtered by `cwd` when given
  fn list(&self, params: &Value) -> Value {
    const PAGE: usize = 50;
    let dir = self.config.home().join("agent").join("sessions");
    let want = params.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).map(PathBuf::from);
    let mut found: Vec<(u64, Value)> = std::fs::read_dir(&dir)
      .into_iter()
      .flatten()
      .flatten()
      .map(|e| e.path())
      .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
      .filter_map(|p| store::load(&p, true).ok())
      .filter(|l| l.title.is_some() && want.as_ref().is_none_or(|w| same_dir(w, &l.cwd)))
      .map(|l| {
        let info = json!({ "sessionId": l.id, "cwd": l.cwd, "title": l.title, "updatedAt": acpira_shared::time::iso_of_ms(l.updated_ms as i64) });
        (l.updated_ms, info)
      })
      .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    let start = params.get("cursor").and_then(Value::as_str).and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
    let page: Vec<Value> = found.iter().skip(start).take(PAGE).map(|(_, v)| v.clone()).collect();
    let mut out = json!({ "sessions": page });
    if start + PAGE < found.len() {
      out["nextCursor"] = Value::String((start + PAGE).to_string());
    }
    out
  }

  fn set_mode(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let mode = str_param(params, "modeId")?;
    if modes::find(mode).is_none() {
      return Err(RpcError::new(-32602, format!("Unknown mode: {mode}")));
    }
    let mut st = session.state.lock();
    st.mode = mode.to_owned();
    session.save_state(&st);
    Ok(json!({}))
  }

  fn set_config_option(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let config_id = str_param(params, "configId")?;
    let value = params.get("value").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "value must be a string"))?;
    let config = self.config.get();
    let catalog = self.catalog.get();
    let mut st = session.state.lock();
    // The effort check below must see the model the picker shows
    reconcile(&config, &catalog, &mut st);
    match config_id {
      "model" => {
        if config.find(value).is_none() {
          return Err(RpcError::new(-32602, format!("Unknown model: {value}")));
        }
        st.model = Some(value.to_owned());
        // An effort the new model does not offer falls back to its default
        let efforts = efforts_of(&config, &catalog, st.model.as_deref());
        if !st.effort.as_ref().is_some_and(|e| efforts.contains(e)) {
          st.effort = default_effort(&config, &catalog, st.model.as_deref());
        }
      }
      "effort" => {
        if !efforts_of(&config, &catalog, st.model.as_deref()).iter().any(|e| e == value) {
          return Err(RpcError::new(-32602, format!("Unknown effort: {value}")));
        }
        st.effort = Some(value.to_owned());
      }
      other => return Err(RpcError::new(-32602, format!("Unknown config option: {other}"))),
    }
    session.save_state(&st);
    Ok(json!({ "configOptions": sent_options(&config, &catalog, &mut st) }))
  }

  /// Send `config_option_update` to every open session whose options changed since the client last got them: a model
  /// source saved in settings (or edited elsewhere) adds or removes models, and the composer's picker reads the
  /// session's own options, not the settings page's probe
  pub fn push_controls(&self) {
    let config = self.config.get();
    let catalog = self.catalog.get();
    let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
    for s in sessions {
      let options = {
        let mut st = s.state.lock();
        if reconcile(&config, &catalog, &mut st) {
          s.save_state(&st);
        }
        let options = config_options(&config, &catalog, &st);
        if st.sent_options.as_ref().is_none_or(|sent| *sent == options) {
          continue;
        }
        st.sent_options = Some(options.clone());
        options
      };
      self.conn().notify("session/update", json!({ "sessionId": s.id, "update": { "sessionUpdate": "config_option_update", "configOptions": options } }));
    }
  }

  /// `push_controls` once a second while the connection lives. Nothing on the wire announces a changed
  /// `providers.json`, and the check is two `stat` calls while nothing changed (`ConfigCache`, `CatalogCache`)
  pub async fn watch_controls(self: Arc<Self>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
      tick.tick().await;
      self.push_controls();
    }
  }

  /// The client went away or the process was told to exit: cancel every running turn (dropping a turn kills its
  /// foreground command's process group) and stop every background job, then wait briefly for the turns to unwind.
  /// Done before `run` returns, since an exit by signal would skip the `Drop` impls that otherwise clean up
  pub async fn close(&self) {
    let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
    for s in &sessions {
      if let Some(c) = s.state.lock().turn.clone() {
        c.cancel();
      }
      s.jobs.stop_all();
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while sessions.iter().any(|s| s.state.lock().turn.is_some()) && tokio::time::Instant::now() < deadline {
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
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

fn modes_of(st: &SessionState) -> Value {
  json!({
    "currentModeId": st.mode,
    "availableModes": modes::MODES.iter().map(|m| json!({ "id": m.id, "name": m.name, "description": m.description })).collect::<Vec<_>>(),
  })
}

/// The configured levels; a model entered without any (added by id rather than fetched) gets the catalogue's, as model
/// discovery would have given it, unless its thinking is switched off
fn efforts_of(config: &Config, catalog: &Catalog, pick: Option<&str>) -> Vec<String> {
  let Some((_, m)) = pick.and_then(|p| config.find(p)) else { return vec![] };
  if !m.efforts.is_empty() || m.thinking == Thinking::Off {
    return m.efforts.clone();
  }
  crate::catalog::lookup(catalog, &m.id).filter(|c| c.reasoning).map(|c| c.efforts.clone()).unwrap_or_default()
}

/// The model family's preferred level when offered, else the last (strongest) level
fn default_effort(config: &Config, catalog: &Catalog, pick: Option<&str>) -> Option<String> {
  let efforts = efforts_of(config, catalog, pick);
  let preferred = pick.and_then(|p| config.find(p)).map(|(p, m)| crate::llm::family::resolve(p, m).effort);
  efforts.iter().find(|e| Some(e.as_str()) == preferred).or(efforts.last()).cloned()
}

/// Bring the session's model and effort in line with what `config_options` shows: a session opened before any model
/// existed takes the default, and an effort the current model does not offer (or none, where it now offers levels)
/// becomes the model's default. Without this the picker showed a fallback the session did not hold, so setting the
/// shown effort was refused and a turn sent no effort until the model was switched once. True when something changed
pub(crate) fn reconcile(config: &Config, catalog: &Catalog, st: &mut SessionState) -> bool {
  let before = (st.model.clone(), st.effort.clone());
  if st.model.is_none() {
    st.model = config.default_pick();
  }
  // A model that is no longer configured keeps its effort: the turn refuses to run it anyway
  if st.model.as_deref().is_some_and(|p| config.find(p).is_some()) {
    let efforts = efforts_of(config, catalog, st.model.as_deref());
    if !st.effort.as_ref().is_some_and(|e| efforts.contains(e)) {
      st.effort = default_effort(config, catalog, st.model.as_deref());
    }
  }
  (st.model.clone(), st.effort.clone()) != before
}

/// `config_options`, remembered as what the client has
fn sent_options(config: &Config, catalog: &Catalog, st: &mut SessionState) -> Vec<Value> {
  let options = config_options(config, catalog, st);
  st.sent_options = Some(options.clone());
  options
}

/// The full configOptions set: the model select (every usable model, labelled with its source) and the effort select
/// when the current model offers levels. The approval level is not a session control: it is the agent's setting in
/// `providers.json` (`Config::approval`), so the composer shows one mode chip and nothing beside it
pub fn config_options(config: &Config, catalog: &Catalog, st: &SessionState) -> Vec<Value> {
  let mut out = vec![];
  let current = st.model.as_deref().filter(|p| config.find(p).is_some()).map(str::to_owned).or_else(|| config.default_pick());
  if let Some(current) = current {
    let options: Vec<Value> = config
      .providers
      .usable()
      .map(|(p, m)| json!({ "value": acpira_shared::providers::model_pick(&p.id, &m.id), "name": m.display_name(), "description": p.display_name() }))
      .collect();
    out.push(json!({ "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": current, "options": options }));
    let efforts = efforts_of(config, catalog, Some(&current));
    if !efforts.is_empty() {
      let value = st.effort.clone().filter(|e| efforts.contains(e)).or_else(|| default_effort(config, catalog, Some(&current))).unwrap_or_default();
      out.push(json!({
        "id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": value,
        "options": efforts.iter().map(|e| json!({ "value": e, "name": title_case(e) })).collect::<Vec<_>>(),
      }));
    }
  }
  out
}

fn title_case(s: &str) -> String {
  let mut c = s.chars();
  c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The same folder, also when one spelling goes through a symlink (macOS `/var` and `/private/var`)
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
  a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
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
        "session/load" => server.restore(&params, true),
        "session/resume" => server.restore(&params, false),
        "session/list" => Ok(server.list(&params)),
        "session/set_mode" => server.set_mode(&params),
        "session/set_config_option" => server.set_config_option(&params),
        "session/prompt" => crate::turn::run(server, params, cancel).await,
        _ => Err(RpcError::method_not_found(&method)),
      }
    })
  }
}
