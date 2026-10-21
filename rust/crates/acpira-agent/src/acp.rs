//! The ACP server side: the session table and the agent methods. Session state lives here; one turn of a session is
//! `turn::run`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
  /// The Fast switch. Kept across model switches and applied only while the model offers it (`llm::fast::offered`)
  pub fast: bool,
  /// The sidekick's `<provider>/<model>` (the `sidekick` select): the executor the engine's ask_agent lists first. Kept
  /// while its model is not configured, when the select shows none (`sidekick_of`)
  pub sidekick: Option<String>,
  /// The sidekick's reasoning effort (the `sidekick_effort` select). None, or a level its model does not offer, shows and
  /// runs as that model's default (`sidekick_effort_of`)
  pub sidekick_effort: Option<String>,
  /// The sidekick the model last learned of, from acpira's tool list or a note: one picked since is told with the next
  /// message (`turn.rs`), so the tool table stays as it was listed
  pub sidekick_told: Option<String>,
  /// The system prompt, fixed while the model's variant stays the same (prompt caches key on it)
  pub prompt: crate::prompt::Composed,
  /// The conversation as the model sees it
  pub items: Vec<Item>,
  /// Text and reasoning of the model call in flight, kept if the turn is cancelled mid-stream
  pub partial_text: String,
  pub partial_reasoning: String,
  /// (model call id, result) of a call whose change already landed while the command it carries still runs
  /// (`then_run`): what the model is told if the turn ends before the call's result is in
  pub landed: Vec<(String, String)>,
  /// The running turn's cancel signal
  pub turn: Option<Cancel>,
  /// Rules the user added from permission cards ("always allow"), for this session only
  pub allowed: Vec<Rule>,
  /// What the last model request's prefix was built from (`turn.rs` logs a `view` event when it changes)
  pub last_view: Option<Value>,
  /// The config options the client last got for this session (`Server::push_controls` sends a change)
  pub sent_options: Option<Vec<Value>>,
  /// The context (used, size) of the last `usage_update` sent: a window changed in the settings is pushed with the
  /// same `used` (`Server::push_controls`), so the client's meter does not wait for the next model call
  pub reported: Option<(u64, u64)>,
  /// Naming the session from its first prompt has started (`title.rs`); it happens once
  pub titled: bool,
  /// The first item is a compaction's summary (`compact.rs`)
  pub summarized: bool,
  /// What that summary item carries beside the summary (the user's messages, ...); the next compaction carries the
  /// messages on
  pub carry: crate::compact::Carry,
  /// The chunks the view shows (`chunk.rs`), and how many items they render to behind the summary item
  pub chunks: Vec<crate::chunk::Chunk>,
  pub folded: usize,
  /// The MCP note (servers' instructions, the ones that failed) the model was last told, in a user message like the mode
  pub mcp_note: Option<String>,
  /// Messages steered into the running turn (`_session/steering`, each one's content blocks) that no model call has
  /// carried yet. The turn takes them before its next call (`steer::take`); any left when the turn ends go into
  /// the history for the next one
  pub steers: Vec<Value>,
  /// The running turn takes steering: on from its start (except a `/compact`), off the moment the turn decides to end.
  /// Both sides switch it under this lock, so a steer either joins the turn or is handed back
  pub steerable: bool,
}

pub struct Session {
  pub id: String,
  pub cwd: PathBuf,
  pub state: parking_lot::Mutex<SessionState>,
  pub store: Store,
  tool_seq: AtomicU64,
  /// Background commands started in this session; dropping the session stops them
  pub jobs: Arc<crate::tools::jobs::Jobs>,
  /// The MCP servers the client handed this session, started when it opened
  pub mcp: Arc<crate::mcp::Hub>,
}

impl Session {
  /// A tool call id unique in this session
  pub fn next_tool_id(&self) -> String {
    format!("call-{}", self.tool_seq.fetch_add(1, Ordering::Relaxed) + 1)
  }

  /// A compaction id: unique across reopenings too, since a replay carries the earlier ones
  pub fn next_compaction_id(&self) -> String {
    format!("compaction-{}", uuid::Uuid::new_v4())
  }

  /// Record the controls at the start and after a change, so a reopened session comes back with them (held until the
  /// session has a file: changing a control alone does not make one)
  pub fn save_state(&self, st: &SessionState) {
    self.store.prelude(json!({
      "type": "state", "mode": st.mode, "announced": st.announced, "model": st.model, "effort": st.effort, "fast": st.fast,
      "sidekick": st.sidekick, "sidekick_effort": st.sidekick_effort,
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
  /// The client takes boolean config options (`clientCapabilities.session.configOptions.boolean`); without it the Fast
  /// switch goes out as a Standard / Fast select
  booleans: AtomicBool,
  /// The client shows elicitation forms (`clientCapabilities.elicitation.form`): the `question` tool's card
  forms: AtomicBool,
  /// The client runs a sidekick (`clientCapabilities._meta["acpira/sidekick"]`, the engine): the `sidekick` select is offered
  sidekicks: AtomicBool,
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
      booleans: AtomicBool::new(false),
      forms: AtomicBool::new(false),
      sidekicks: AtomicBool::new(false),
      conn: OnceLock::new(),
      sessions: Default::default(),
    })
  }

  /// The session's own directory under the data root (spilled tool outputs, the plan)
  pub fn session_dir(&self, id: &str) -> PathBuf {
    sessions_root(self.config.home()).join(id)
  }

  /// The session's history (`store.rs`)
  pub fn session_file(&self, id: &str) -> PathBuf {
    sessions_root(self.config.home()).join(format!("{id}.jsonl"))
  }

  /// The file Plan mode writes its plan to
  pub fn plan_file(&self, id: &str) -> PathBuf {
    self.session_dir(id).join("plan.md")
  }

  pub fn attach(&self, conn: Connection) {
    let _ = self.conn.set(conn);
  }

  pub fn forms(&self) -> bool {
    self.forms.load(Ordering::Relaxed)
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

  fn initialize(&self, params: &Value) -> Value {
    let booleans = params.pointer("/clientCapabilities/session/configOptions/boolean").is_some_and(|b| !b.is_null());
    self.booleans.store(booleans, Ordering::Relaxed);
    let forms = params.pointer("/clientCapabilities/elicitation/form").is_some_and(|f| !f.is_null());
    self.forms.store(forms, Ordering::Relaxed);
    let sidekicks = params.pointer("/clientCapabilities/_meta/acpira~1sidekick").and_then(Value::as_bool) == Some(true);
    self.sidekicks.store(sidekicks, Ordering::Relaxed);
    json!({
      "protocolVersion": PROTOCOL_VERSION,
      "agentInfo": { "name": AGENT_NAME, "title": "Acpira", "version": self.version },
      "agentCapabilities": {
        "loadSession": true,
        "promptCapabilities": { "image": true, "embeddedContext": true },
        "sessionCapabilities": { "list": {}, "resume": {} },
        // stdio is always spoken; the legacy SSE transport is not (`mcp/mod.rs`)
        "mcpCapabilities": { "http": true, "sse": false },
      },
      "authMethods": [],
      // The steering extension claude-agent-acp and codex-acp implement, advertised at the top level as they do
      "_meta": { "steering": { "supported": true } },
    })
  }

  /// `_session/steering`: a message for the running turn. It is answered at once and reaches the model with the turn's
  /// next call, after the tool round in flight, or right after a reply that would otherwise have ended the turn
  /// (`steer::take`). With no turn running, or one that has already decided to end, the message is handed back
  /// for a `session/prompt` when the client opted into that (`idleBehavior: promptRequired`, as the engine always
  /// does) and refused otherwise: this agent never starts a turn no `session/prompt` owns
  fn steer(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let prompt = params.get("prompt").and_then(Value::as_array).filter(|p| !p.is_empty()).ok_or_else(|| RpcError::new(-32602, "prompt is required"))?;
    {
      let mut st = session.state.lock();
      if st.turn.is_some() && st.steerable {
        st.steers.push(Value::Array(prompt.clone()));
        return Ok(json!({ "outcome": "injected" }));
      }
    }
    if params.pointer("/_meta/steering/idleBehavior").and_then(Value::as_str) == Some("promptRequired") {
      return Ok(json!({ "outcome": "promptRequired", "reason": "noRunningTurn" }));
    }
    Err(RpcError::new(-32603, "No turn is taking messages in this session; send it as a prompt"))
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
      "configOptions": sent_options(&config, &catalog, &mut state, self.offers()),
    });
    let store = Store::new(self.session_file(&id), &id, std::path::Path::new(cwd));
    // The file starts with the first event; the prompt goes first so a reopen keeps the very same text
    store.prelude(json!({ "type": "prompt", "variant": state.prompt.variant, "version": state.prompt.version, "digest": state.prompt.digest,
      "text": state.prompt.text, "sampling": state.prompt.sampling, "thinking": state.prompt.thinking }));
    let mcp = crate::mcp::Hub::start(params.get("mcpServers").unwrap_or(&Value::Null), std::path::Path::new(cwd));
    let session = Session { id: id.clone(), cwd: PathBuf::from(cwd), state: parking_lot::Mutex::new(state), store, tool_seq: AtomicU64::new(0), jobs: Default::default(), mcp };
    // The starting controls too: a reopen must not take a default that changed in the meantime
    session.save_state(&session.state.lock());
    self.sessions.lock().insert(id.clone(), Arc::new(session));
    self.announce_commands(&id, std::path::Path::new(cwd));
    Ok(response)
  }

  /// The session's slash commands: `/compact` and the skills found now (`skills.rs`). Sent inside `session/new` /
  /// `load` / `resume` like Devin does, straight to the client: a replay would only repeat a stale list
  fn announce_commands(&self, id: &str, cwd: &std::path::Path) {
    let places = crate::prompt::Places::new(cwd, crate::turn::user_home().as_deref());
    let commands = crate::skills::commands(&crate::skills::scan(&places));
    self.conn().notify("session/update", json!({ "sessionId": id, "update": { "sessionUpdate": "available_commands_update", "availableCommands": commands } }));
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
        let session = Arc::new(self.revive(id, cwd, &loaded, path, params.get("mcpServers").unwrap_or(&Value::Null)));
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
    self.announce_commands(id, &session.cwd);
    let config = self.config.get();
    let catalog = self.catalog.get();
    let mut st = session.state.lock();
    if reconcile(&config, &catalog, &mut st) {
      session.save_state(&st);
    }
    Ok(json!({ "modes": modes_of(&st), "configOptions": sent_options(&config, &catalog, &mut st, self.offers()) }))
  }

  /// A session object from its file: the history, the controls and the system prompt as they were
  fn revive(&self, id: &str, cwd: PathBuf, loaded: &store::Loaded, path: PathBuf, mcp_servers: &Value) -> Session {
    let config = self.config.get();
    let catalog = self.catalog.get();
    let s = |k: &str| loaded.state.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut st = SessionState {
      mode: s("mode").filter(|m| modes::find(m).is_some()).unwrap_or_else(|| modes::AGENT.to_owned()),
      announced: s("announced").unwrap_or_else(|| modes::AGENT.to_owned()),
      model: s("model").or_else(|| config.default_pick()),
      items: loaded.items.clone(),
      summarized: loaded.summarized,
      carry: loaded.carry.clone(),
      chunks: loaded.chunks.clone(),
      folded: loaded.folded,
      // The replay ends on this context; a window edited since is pushed by the next `push_controls`
      reported: loaded.updates.iter().rev().find(|u| u["sessionUpdate"] == "usage_update").and_then(|u| Some((u["used"].as_u64()?, u["size"].as_u64()?))),
      ..Default::default()
    };
    st.effort = s("effort").or_else(|| default_effort(&config, &catalog, st.model.as_deref()));
    st.fast = loaded.state.get("fast").and_then(Value::as_bool).unwrap_or(false);
    st.sidekick = s("sidekick");
    st.sidekick_effort = s("sidekick_effort");
    // The servers list their tools as the session reopens, with the sidekick the engine holds for it: this one
    st.sidekick_told = sidekick_of(&config, &st);
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
    store.set_items_logged(loaded.logged);
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
    let mcp = crate::mcp::Hub::start(mcp_servers, &cwd);
    Session { id: id.to_owned(), cwd, state: parking_lot::Mutex::new(st), store, tool_seq: AtomicU64::new(seq), jobs: Default::default(), mcp }
  }

  /// `session/list`: this data root's sessions with at least one message, newest first, filtered by `cwd` when given
  fn list(&self, params: &Value) -> Value {
    const PAGE: usize = 50;
    let dir = sessions_root(self.config.home());
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
    // Raw mode sends no tools, and Anthropic refuses a request whose history holds tool calls without them (other
    // formats vary), so a session that has used tools stays out of it; Raw → Agent / Plan is always fine
    if mode == modes::RAW && st.mode != modes::RAW && st.items.iter().any(uses_tools) {
      return Err(RpcError::new(-32602, "Raw mode needs a session without tool calls; start a new session for it"));
    }
    st.mode = mode.to_owned();
    session.save_state(&st);
    Ok(json!({}))
  }

  fn set_config_option(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let config_id = str_param(params, "configId")?;
    let raw = params.get("value").unwrap_or(&Value::Null);
    let value = raw.as_str().unwrap_or_default();
    if config_id != FAST && !raw.is_string() {
      return Err(RpcError::new(-32602, "value must be a string"));
    }
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
      FAST => {
        if !st.model.as_deref().and_then(|p| config.find(p)).is_some_and(|(p, m)| crate::llm::fast::offered(p, m)) {
          return Err(RpcError::new(-32602, "Fast is not offered for this model"));
        }
        // A real boolean from a client that takes boolean options, else the select's value
        st.fast = match (raw.as_bool(), value) {
          (Some(on), _) => on,
          (None, "true" | FAST) => true,
          (None, "false" | STANDARD) => false,
          _ => return Err(RpcError::new(-32602, format!("Unknown fast value: {raw}"))),
        };
      }
      SIDEKICK => {
        let next = match value {
          NO_SIDEKICK => None,
          pick if config.find(pick).is_some() => Some(pick.to_owned()),
          _ => return Err(RpcError::new(-32602, format!("Unknown model: {value}"))),
        };
        st.sidekick = next;
        // A level the new sidekick does not offer falls back to its default
        let efforts = efforts_of(&config, &catalog, st.sidekick.as_deref());
        if !st.sidekick_effort.as_ref().is_some_and(|e| efforts.contains(e)) {
          st.sidekick_effort = None;
        }
        // Before the first message nothing went out with the tool table yet: acpira lists its tools again, so the first
        // request names the sidekick in ask_agent. Later the next message carries a note instead (`turn.rs`)
        if st.items.is_empty() {
          st.sidekick_told = sidekick_of(&config, &st);
          session.mcp.relist(ACPIRA_SERVER);
        }
      }
      SIDEKICK_EFFORT => {
        // The sidekick runs on another model, so its levels are that model's; the lead is never told of them
        let pick = sidekick_of(&config, &st);
        if !efforts_of(&config, &catalog, pick.as_deref()).iter().any(|e| e == value) {
          return Err(RpcError::new(-32602, format!("Unknown sidekick effort: {value}")));
        }
        st.sidekick_effort = Some(value.to_owned());
      }
      other => return Err(RpcError::new(-32602, format!("Unknown config option: {other}"))),
    }
    session.save_state(&st);
    Ok(json!({ "configOptions": sent_options(&config, &catalog, &mut st, self.offers()) }))
  }

  /// Send `config_option_update` to every open session whose options changed since the client last got them: a model
  /// source saved in settings (or edited elsewhere) adds or removes models, and the composer's picker reads the
  /// session's own options, not the settings page's probe
  pub fn push_controls(&self) {
    let config = self.config.get();
    let catalog = self.catalog.get();
    let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
    for s in sessions {
      let (options, usage) = {
        let mut st = s.state.lock();
        if reconcile(&config, &catalog, &mut st) {
          s.save_state(&st);
        }
        let options = config_options(&config, &catalog, &st, self.offers());
        let options = if st.sent_options.as_ref().is_none_or(|sent| *sent == options) {
          None
        } else {
          st.sent_options = Some(options.clone());
          Some(options)
        };
        // The window of the session's model as configured now (edited in the settings, or another model picked)
        let window = st.model.as_deref().and_then(|p| crate::turn::limits(&config, &catalog, p)).map(|m| m.context.unwrap_or(crate::turn::DEFAULT_CONTEXT));
        let usage = match (st.reported, window) {
          (Some((used, size)), Some(w)) if w != size => {
            st.reported = Some((used, w));
            Some((used, w))
          }
          _ => None,
        };
        (options, usage)
      };
      if let Some(options) = options {
        self.conn().notify("session/update", json!({ "sessionId": s.id, "update": { "sessionUpdate": "config_option_update", "configOptions": options } }));
      }
      if let Some((used, size)) = usage {
        // Recorded like the turn's own, so a replay ends on the current window
        self.update(&s.id, json!({ "sessionUpdate": "usage_update", "used": used, "size": size }));
      }
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
  /// foreground command's process group), stop every background job and MCP server, then wait briefly for the turns to
  /// unwind.
  /// Done before `run` returns, since an exit by signal would skip the `Drop` impls that otherwise clean up
  pub async fn close(&self) {
    let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
    for s in &sessions {
      if let Some(c) = s.state.lock().turn.clone() {
        c.cancel();
      }
      s.jobs.stop_all();
      s.mcp.close();
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while sessions.iter().any(|s| s.state.lock().turn.is_some()) && tokio::time::Instant::now() < deadline {
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  }

  /// What the client takes beside plain selects, for `config_options`
  fn offers(&self) -> Offers {
    Offers { booleans: self.booleans.load(Ordering::Relaxed), sidekick: self.sidekicks.load(Ordering::Relaxed) }
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
pub(crate) fn default_effort(config: &Config, catalog: &Catalog, pick: Option<&str>) -> Option<String> {
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
fn sent_options(config: &Config, catalog: &Catalog, st: &mut SessionState, offers: Offers) -> Vec<Value> {
  let options = config_options(config, catalog, st, offers);
  st.sent_options = Some(options.clone());
  options
}

/// The Fast switch's id, and its values as a select for a client without boolean options (the shape Devin uses)
const FAST: &str = "fast";
const STANDARD: &str = "standard";

/// The sidekick select: its id, the value for none, and its category (custom, so a client that does not know it shows it
/// as a plain select rather than as a model or a model parameter)
pub const SIDEKICK: &str = "sidekick";
pub const NO_SIDEKICK: &str = "none";
const SIDEKICK_CATEGORY: &str = "_sidekick";
/// The sidekick's effort select, offered while the sidekick's model has levels. Its own custom category, so a client
/// does not take it for the lead's effort
pub const SIDEKICK_EFFORT: &str = "sidekick_effort";
const SIDEKICK_EFFORT_CATEGORY: &str = "_sidekick_effort";

/// The engine's own MCP server (`acpira mcp`), whose ask_agent lists the sidekick
pub const ACPIRA_SERVER: &str = "acpira";

/// What the client takes beside plain selects
#[derive(Debug, Clone, Copy, Default)]
pub struct Offers {
  /// Boolean config options (`clientCapabilities.session.configOptions.boolean`): Fast goes out as a switch
  pub booleans: bool,
  /// A sidekick (`clientCapabilities._meta["acpira/sidekick"]`): the `sidekick` select is offered
  pub sidekick: bool,
}

/// The session's sidekick while its model is configured: what the select shows and the model is told of
pub fn sidekick_of(config: &Config, st: &SessionState) -> Option<String> {
  st.sidekick.clone().filter(|p| config.find(p).is_some())
}

/// The sidekick's effort as the select shows it: the level picked while its model offers it, else the model's default.
/// None while there is no sidekick or its model has no levels
fn sidekick_effort_of(config: &Config, catalog: &Catalog, st: &SessionState) -> Option<String> {
  let pick = sidekick_of(config, st)?;
  let efforts = efforts_of(config, catalog, Some(&pick));
  st.sidekick_effort.clone().filter(|e| efforts.contains(e)).or_else(|| default_effort(config, catalog, Some(&pick)))
}

/// Where a data root keeps this agent's sessions: one `<id>.jsonl` log each, with an `<id>/` folder beside it (spilled
/// tool output, the plan). The engine reads it to delete both together with the Acpira session that drove them
pub fn sessions_root(home: &Path) -> PathBuf {
  home.join("agent").join("sessions")
}

/// The files one session owns under `sessions_root`: its log and its folder
pub fn session_paths(home: &Path, id: &str) -> [PathBuf; 2] {
  let root = sessions_root(home);
  [root.join(format!("{id}.jsonl")), root.join(id)]
}

/// The full configOptions set: the model select (every usable model, labelled with its source), the effort select when
/// the current model offers levels, the Fast switch when it has a fast tier (a boolean, or a Standard / Fast select
/// for a client that does not take booleans; the composer shows either as its Fast toggle), and for a client that runs
/// one the sidekick select (none, or any usable model) and, while the sidekick's model has levels, its effort select. The approval level is not a session control: it is the agent's
/// setting in `providers.json` (`Config::approval`), so the composer shows one mode chip and nothing beside it
pub fn config_options(config: &Config, catalog: &Catalog, st: &SessionState, offers: Offers) -> Vec<Value> {
  let mut out = vec![];
  let current = st.model.as_deref().filter(|p| config.find(p).is_some()).map(str::to_owned).or_else(|| config.default_pick());
  if let Some(current) = current {
    let options: Vec<Value> = config
      .providers
      .usable()
      .map(|(p, m)| json!({ "value": acpira_shared::providers::model_pick(&p.id, &m.id), "name": m.display_name(), "description": p.display_name() }))
      .collect();
    let sidekicks = offers.sidekick.then(|| {
      let mut list = vec![json!({ "value": NO_SIDEKICK, "name": "None" })];
      list.extend(options.iter().cloned());
      list
    });
    out.push(json!({ "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": current, "options": options }));
    let efforts = efforts_of(config, catalog, Some(&current));
    if !efforts.is_empty() {
      let value = st.effort.clone().filter(|e| efforts.contains(e)).or_else(|| default_effort(config, catalog, Some(&current))).unwrap_or_default();
      out.push(json!({
        "id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": value,
        "options": efforts.iter().map(|e| json!({ "value": e, "name": title_case(e) })).collect::<Vec<_>>(),
      }));
    }
    if config.find(&current).is_some_and(|(p, m)| crate::llm::fast::offered(p, m)) {
      let description = "The provider's faster tier, billed at a higher rate";
      out.push(if offers.booleans {
        json!({ "id": FAST, "name": "Fast", "description": description, "category": "model_config", "type": "boolean", "currentValue": st.fast })
      } else {
        json!({
          "id": FAST, "name": "Fast", "description": description, "category": "model_config", "type": "select",
          "currentValue": if st.fast { FAST } else { STANDARD },
          "options": [{ "value": STANDARD, "name": "Standard" }, { "value": FAST, "name": "Fast" }],
        })
      });
    }
    if let Some(list) = sidekicks {
      let value = sidekick_of(config, st).unwrap_or_else(|| NO_SIDEKICK.to_owned());
      out.push(json!({
        "id": SIDEKICK, "name": "Sidekick", "category": SIDEKICK_CATEGORY, "type": "select", "currentValue": value, "options": list,
        "description": "A second model this agent can hand well-specified work to, in a conversation of its own",
      }));
      if let Some(value) = sidekick_effort_of(config, catalog, st) {
        let efforts = efforts_of(config, catalog, sidekick_of(config, st).as_deref());
        out.push(json!({
          "id": SIDEKICK_EFFORT, "name": "Sidekick effort", "category": SIDEKICK_EFFORT_CATEGORY, "type": "select", "currentValue": value,
          "options": efforts.iter().map(|e| json!({ "value": e, "name": title_case(e) })).collect::<Vec<_>>(),
        }));
      }
    }
  }
  out
}

/// A history item a request without tools cannot carry: a reply with tool calls, or a tool's result
fn uses_tools(item: &Item) -> bool {
  match item {
    Item::Assistant { tool_calls, .. } => !tool_calls.is_empty(),
    Item::ToolResult { .. } => true,
    Item::User(_) => false,
  }
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
        "_session/steering" => server.steer(&params),
        _ => Err(RpcError::method_not_found(&method)),
      }
    })
  }
}
