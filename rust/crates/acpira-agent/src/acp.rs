//! The ACP server side: the session table and the agent methods. Session state lives here; one turn of a session is
//! `turn::run`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Connection, Inbound, RpcError};
use acpira_shared::providers::split_pick;

use crate::config::{Config, ConfigCache};
use crate::llm::Item;
use crate::permission::{Approval, Rule};
use crate::store::Store;

mod options;
mod sessions;

pub use self::options::{ACPIRA_SERVER, APPROVAL, NO_SIDEKICK, Offers, SIDEKICK, SIDEKICK_EFFORT, SIDEKICK_FAST, config_options, sidekick_of};
pub(crate) use self::options::{default_effort, reconcile};

pub const PROTOCOL_VERSION: i64 = 1;
pub const AGENT_NAME: &str = "acpira";

/// What a session remembers between turns
#[derive(Default)]
pub struct SessionState {
  /// The engine's plugin hooks into this loop declared at the start of the turn (`plugin_hooks::refresh`)
  pub plugin_hooks: Vec<String>,
  pub mode: String,
  /// Opened for a summoned agent (`SUMMONED_META`): Read-only is listed. Not recorded, since every reopen says it again
  pub summoned: bool,
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
  /// The sidekick's Fast switch (the `sidekick_fast` option). Kept across sidekick switches and applied only while the
  /// sidekick's model offers it (`sidekick_fast_of`)
  pub sidekick_fast: bool,
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
  /// Each model tool call's id beside the id of the row the client shows for it, in the order the rows opened (the
  /// log's `call` events): the context report names a call's row by it (`context.rs`)
  pub rows: Vec<(String, String)>,
  /// The running turn's cancel signal
  pub turn: Option<Cancel>,
  /// How much runs without a card (the `approval` select), read before each tool runs
  pub approval: Approval,
  /// Rules the user added from permission cards ("always allow"), for this session only
  pub allowed: Vec<Rule>,
  /// What the last model request's prefix was built from (`turn/request.rs` logs a `view` event when it changes)
  pub last_view: Option<Value>,
  /// The config options the client last got for this session (`Server::push_controls` sends a change)
  pub sent_options: Option<Vec<Value>>,
  /// The mode list last sent to the client; settings changes update it on already open sessions
  pub sent_modes: Vec<Value>,
  /// The context (used, size) of the last `usage_update` sent: a window changed in the settings is pushed with the
  /// same `used` (`Server::push_controls`), so the client's meter does not wait for the next model call
  pub reported: Option<(u64, u64)>,
  /// What the last counted model call sent and got back, or the history a compaction left: the context report's source
  /// (`context.rs`). None until the session's first call since it opened
  pub context: Option<Arc<crate::context::Snapshot>>,
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
  /// The session's goal (`goal.rs`), recorded in the log at every change
  pub goal: Option<crate::goal::Goal>,
  /// While a turn runs: when its time last counted toward the goal (`goal::tick`)
  pub goal_clock: Option<std::time::Instant>,
  /// The budget note waiting for the running turn's next call (`goal::take_note`)
  pub goal_note: Option<String>,
  /// The goal's counters moved since the log last recorded it (`goal::change`)
  pub goal_unsaved: bool,
  /// The goal ran out of budget in the running turn, which still counts toward it while it wraps up (`goal::account`)
  pub goal_wrapping: bool,
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
  /// The SSH host the session's folder is on (`remote.rs`); its project tools run there
  pub remote: Option<Arc<crate::remote::Remote>>,
  /// The plugins' prompt and skill folders as the engine sent them when the session opened (`LAYERS_META`, `SKILLS_META`)
  pub layers: Layers,
  /// Every prompt text this session's model gets beside the system prompt (mode notes, compaction, goals, ...), with
  /// the override folders read when it opened (`prompt::library`)
  pub prompts: Arc<crate::prompt::Library>,
  /// Wakes open permission requests when the mode or approval level changes
  pub permissions_changed: tokio::sync::Notify,
  /// Where its commands keep temp files (`scratch.rs`)
  pub scratch: crate::scratch::Scratch,
  /// The settings it reads: the data root's own, or those of the machine that sent its last turn (`CLIENT_SETTINGS_META`)
  pub settings: parking_lot::Mutex<Arc<ConfigCache>>,
}

/// The plugins' folders a session reads besides the user's and the project's
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layers {
  /// Prompt sections, one more override layer of the system prompt
  pub prompts: Vec<PathBuf>,
  /// Skill folders (`<name>/SKILL.md`), between the user's skills and the project's
  pub skills: Vec<PathBuf>,
}

/// `_meta` key of session/new, load and resume: the prompt folders of the engine's plugins in effect, one more override
/// layer of the system prompt (`prompt::Places::layers`). Taken when the session opens, so a plugin enabled during the
/// session changes nothing before the next one
pub const LAYERS_META: &str = "acpira/promptLayers";
/// `_meta` key of the same requests: the skill folders of the engine's plugins in effect, taken the same way
pub const SKILLS_META: &str = "acpira/skillLayers";
/// `_meta` of `session/new` / `load` / `resume`: the engine opens this session for a summoned agent, so its mode list
/// carries Read-only (`modes::READ_ONLY`), the mode the engine picks for a consult
pub const SUMMONED_META: &str = "acpira/summoned";
/// `_meta` of `session/new` / `load` / `resume` and every `session/prompt`: the id of the machine that drives the
/// session over SSH, whose built-in agent settings it reads (`ConfigCache::of_client`); absent, the data root's own.
/// Taken when a turn starts, so a running turn never switches (`Server::follow_settings`)
pub const CLIENT_SETTINGS_META: &str = "acpira/clientSettings";

/// Notification from the engine: the settings were saved, so open sessions get their changed controls now rather than
/// at the next once-a-second look (`Server::watch_controls`), which stays for edits the engine does not make
pub const SETTINGS_CHANGED: &str = "_acpira/settingsChanged";

/// The machine a request names (`CLIENT_SETTINGS_META`)
fn client_of(params: &Value) -> Option<&str> {
  params.pointer("/_meta/acpira~1clientSettings").and_then(Value::as_str).filter(|m| !m.is_empty())
}

/// The request's ACP `additionalDirectories`: absolute paths only
fn extra_of(params: &Value) -> Vec<PathBuf> {
  let dirs = params.get("additionalDirectories").and_then(Value::as_array);
  dirs.into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).filter(|p| p.is_absolute()).collect()
}

/// Whether the request opens a summoned agent's session (`SUMMONED_META`)
fn summoned_of(params: &Value) -> bool {
  params.get("_meta").and_then(|m| m.get(SUMMONED_META)).and_then(Value::as_bool) == Some(true)
}

/// The plugin folders a session request carries
fn layers_of(params: &Value) -> Layers {
  let list = |meta: &str| {
    let key = format!("/_meta/{}", meta.replace('/', "~1"));
    params.pointer(&key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect()).unwrap_or_default()
  };
  Layers { prompts: list(LAYERS_META), skills: list(SKILLS_META) }
}

impl Session {
  /// The settings as the session reads them now
  pub fn config(&self) -> Arc<Config> {
    self.settings.lock().get()
  }

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
      "sidekick": st.sidekick, "sidekick_effort": st.sidekick_effort, "sidekick_fast": st.sidekick_fast, "approval": st.approval.id(),
    }));
  }

  /// Record a newly composed system prompt
  pub fn save_prompt(&self, p: &crate::prompt::Composed) {
    self.store.append(json!({
      "type": "prompt", "cwd": self.cwd, "variant": p.variant, "version": p.version, "digest": p.digest, "text": p.text,
      "sampling": p.sampling, "thinking": p.thinking, "overrides": p.overrides, "sections": p.sections,
    }));
  }
}

pub struct Server {
  /// The data root's own settings; a session reads its own (`Session::config`)
  pub config: Arc<ConfigCache>,
  /// The settings other machines brought for the sessions they drive here, one reader per machine (`settings_for`)
  clients: parking_lot::Mutex<HashMap<String, Arc<ConfigCache>>>,
  /// A plugin's specialised agent: every session takes its prompt, tools, modes and model (`profile.rs`)
  pub profile: Option<Arc<crate::profile::Profile>>,
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
    Server::with_profile(home, version, None)
  }

  pub fn with_profile(home: PathBuf, version: impl Into<String>, profile: Option<crate::profile::Profile>) -> Arc<Server> {
    Arc::new(Server {
      profile: profile.map(Arc::new),
      catalog: crate::catalog::CatalogCache::new(&home),
      config: Arc::new(ConfigCache::new(home)),
      clients: Default::default(),
      version: version.into(),
      http: crate::llm::default_http(),
      booleans: AtomicBool::new(false),
      forms: AtomicBool::new(false),
      sidekicks: AtomicBool::new(false),
      conn: OnceLock::new(),
      sessions: Default::default(),
    })
  }

  /// The settings a session reads: those `client` brought for the sessions it drives here, else the data root's own
  pub fn settings_for(&self, client: Option<&str>) -> Arc<ConfigCache> {
    let Some(machine) = client else { return self.config.clone() };
    let mut clients = self.clients.lock();
    if let Some(c) = clients.get(machine) {
      return c.clone();
    }
    match ConfigCache::of_client(self.config.home().to_path_buf(), machine) {
      Some(c) => clients.entry(machine.to_owned()).or_insert(Arc::new(c)).clone(),
      None => self.config.clone(),
    }
  }

  /// A turn's start is where a session takes the settings of the machine that sent it: from then on its pickers and
  /// its turns read those (a model they lack gives way to their default, `reconcile`), and a running turn keeps the
  /// settings it started with
  pub fn follow_settings(&self, session: &Arc<Session>, params: &Value) {
    let next = self.settings_for(client_of(params));
    {
      let mut now = session.settings.lock();
      if Arc::ptr_eq(&now, &next) {
        return;
      }
      *now = next;
    }
    self.push_controls_of(session);
  }

  /// The session's own directory under the data root (spilled tool outputs, the plan)
  pub fn session_dir(&self, id: &str) -> PathBuf {
    sessions_root(self.config.home()).join(id)
  }

  /// The session's history (`store.rs`)
  pub fn session_file(&self, id: &str) -> PathBuf {
    sessions_root(self.config.home()).join(format!("{id}.jsonl"))
  }

  /// Where the session's instructions, prompt overrides and skills are read. A folder on an SSH host is not this
  /// machine's: the same path here may hold another project, so nothing of it is read from this disk, only what the
  /// host sent when the session opened
  pub fn places(&self, cwd: &std::path::Path, remote: Option<&crate::remote::Remote>) -> crate::prompt::Places {
    self.places_of(cwd, remote, Layers::default())
  }

  /// `places` with the session's plugin prompt folders (`LAYERS_META`)
  pub fn places_of(&self, cwd: &std::path::Path, remote: Option<&crate::remote::Remote>, layers: Layers) -> crate::prompt::Places {
    let places = match remote {
      Some(r) => match &r.files {
        Some(files) => crate::prompt::Places::on_host(cwd, files.clone()),
        None => crate::prompt::Places::on_host(cwd, Default::default()),
      },
      None => crate::prompt::Places::new(cwd, crate::turn::user_home().as_deref()),
    };
    places.with_data(Some(self.config.home())).with_layers(layers.prompts).with_skill_layers(layers.skills).with_profile(self.profile.clone())
  }

  /// The folder processes of the session start in here (MCP servers): the session folder, or for a folder on an SSH
  /// host the data root
  pub fn process_dir(&self, cwd: &std::path::Path, remote: bool) -> PathBuf {
    if remote { self.config.home().to_path_buf() } else { cwd.to_path_buf() }
  }

  /// A session's scratch folder: none on an SSH host or with the setting off (`scratch.rs`)
  fn scratch_of(&self, cwd: &std::path::Path, id: &str, remote: bool, config: &Config) -> crate::scratch::Scratch {
    let place = (!remote && config.scratch_folder()).then(|| crate::scratch::place(cwd, id, crate::turn::user_home().as_deref())).flatten();
    crate::scratch::Scratch::new(place, id, self.config.home())
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
    // `run_until` attaches it right after `Connection::start`; a message dispatched in that instant waits for it
    // instead of panicking
    self.conn.wait()
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
        // Further project folders are named in a new session's environment section (`prompt::environment`)
        "sessionCapabilities": { "list": {}, "resume": {}, "additionalDirectories": {} },
        // stdio is always spoken; the legacy SSE transport is not (`mcp/mod.rs`)
        "mcpCapabilities": { "http": true, "sse": false },
      },
      "authMethods": [],
      // The steering extension claude-agent-acp and codex-acp implement, advertised at the top level as they do,
      // codex-acp's goal extension (`goal.rs`), Acpira's own history rewind (`_session/rewind`) and context report
      // (`context.rs`)
      "_meta": {
        "steering": { "supported": true }, "goal": crate::goal::capability(), "acpira/rewind": { "supported": true },
        "acpira/context": crate::context::capability(),
      },
    })
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

  fn cancel(&self, params: &Value) {
    let Some(id) = params.get("sessionId").and_then(Value::as_str) else { return };
    if let Ok(session) = self.session(id)
      && let Some(c) = session.state.lock().turn.clone()
    {
      c.cancel();
    }
  }
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
    } else if method == SETTINGS_CHANGED {
      self.0.push_controls();
    } else if method == crate::remote::PROGRESS {
      let session = params.get("sessionId").and_then(Value::as_str).and_then(|id| self.0.sessions.lock().get(id).cloned());
      if let Some(remote) = session.and_then(|s| s.remote.clone()) {
        remote.progress(&params);
      }
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
        crate::context::METHOD_REPORT => crate::context::report_of(&server, &params).await,
        crate::context::METHOD_ITEM => crate::context::item_of(&server, &params),
        crate::context::METHOD_RAW => crate::context::raw_of(&server, &params),
        _ => Err(RpcError::method_not_found(&method)),
      }
    })
  }
}
