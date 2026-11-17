//! Session methods: new, load / resume (with the revive of a session another process left), list, rewind and steer

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_rpc::rpc::RpcError;

use super::options::sent_options;
use super::{
  ACPIRA_SERVER, Layers, Server, Session, SessionState, client_of, extra_of, layers_of, default_effort, reconcile,
  sessions_root, sidekick_of, str_param, summoned_of,
};
use crate::config::ConfigCache;
use crate::llm::Item;
use crate::modes;
use crate::permission::Approval;
use crate::store::{self, Store};

impl Server {
  /// `_session/rewind` (Acpira's extension): the conversation goes back to just before the turn the client named in its
  /// `session/prompt` (`_meta["acpira/turn"]`), as if that prompt and everything after it had never been sent; the
  /// client then sends the edited message as a plain prompt. The log is cut back (`Store::rewind`) and the view read
  /// again from it, so compactions and prunes made since are undone too. `dryRun` only answers whether the turn can be
  /// rewound to. Refused while a turn runs, and for a turn this session never marked (one sent before marks existed,
  /// or in another native session)
  pub(super) fn rewind(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let turn = str_param(params, "turnId")?;
    let dry = params.get("dryRun").and_then(Value::as_bool) == Some(true);
    let mut st = session.state.lock();
    if st.turn.is_some() {
      return Err(RpcError::new(-32603, "A turn is running in this session"));
    }
    let io = |e: std::io::Error| RpcError::new(-32603, format!("Cannot rewind the session log: {e}"));
    let mark = session.store.rewind(turn, dry).map_err(io)?.ok_or_else(|| RpcError::new(-32602, format!("No turn {turn} to rewind to in this session")))?;
    if dry {
      return Ok(json!({}));
    }
    let loaded = store::load(session.store.path(), false).map_err(io)?;
    session.store.set_items_logged(loaded.logged);
    st.items = loaded.items;
    st.summarized = loaded.summarized;
    st.carry = loaded.carry;
    st.chunks = loaded.chunks;
    st.folded = loaded.folded;
    st.rows = loaded.rows;
    st.reported = loaded.updates.iter().rev().find(|u| u["sessionUpdate"] == "usage_update").and_then(|u| Some((u["used"].as_u64()?, u["size"].as_u64()?)));
    // What the model was told lives in the cut messages: the next turn tells it again where it differs
    let told = |k: &str| mark.get(k).and_then(Value::as_str).map(str::to_owned);
    if let Some(a) = told("announced") {
      st.announced = a;
    }
    st.mcp_note = told("mcp_note");
    st.sidekick_told = told("sidekick_told");
    // Back to before the first message: that message names the session again, and a sidekick picked meanwhile goes into
    // acpira's tool table the way a pick before the first message does (`set_config_option`), not into a note
    if !st.items.iter().any(|i| matches!(i, Item::User(_))) {
      st.titled = false;
      st.sidekick_told = sidekick_of(&session.config(), &st);
      session.mcp.relist(ACPIRA_SERVER);
    }
    // The goal goes back with the history (`goal::rewound`); the client takes the result from the answer, since it
    // holds back the session's updates while it edits. A mark older than goals says nothing, and the goal stays
    let mut answer = json!({});
    if let Some(then) = mark.get("goal") {
      let then = serde_json::from_value(then.clone()).ok().flatten();
      let goal = crate::goal::rewound(then, st.goal.as_ref());
      if goal != st.goal {
        crate::goal::save(&session.store, goal.as_ref());
      }
      st.goal = goal;
      st.goal_note = None;
      st.goal_unsaved = false;
      st.goal_wrapping = false;
      answer["_meta"] = json!({ "goal": st.goal.as_ref().map(crate::goal::Goal::snapshot) });
    }
    Ok(answer)
  }

  /// `_session/steering`: a message for the running turn. It is answered at once and reaches the model with the turn's
  /// next call, after the tool round in flight, or right after a reply that would otherwise have ended the turn
  /// (`steer::take`). With no turn running, or one that has already decided to end, the message is handed back
  /// for a `session/prompt` when the client opted into that (`idleBehavior: promptRequired`, as the engine always
  /// does) and refused otherwise: this agent never starts a turn no `session/prompt` owns
  pub(super) fn steer(&self, params: &Value) -> Result<Value, RpcError> {
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

  pub(super) fn new_session(&self, params: &Value) -> Result<Value, RpcError> {
    let cwd = params.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).ok_or_else(|| RpcError::new(-32602, "cwd is required"))?;
    let settings = self.settings_for(client_of(params));
    let config = settings.get();
    let catalog = self.catalog.get();
    let id = uuid::Uuid::new_v4().to_string();
    let mut state = SessionState { mode: modes::AGENT.to_owned(), announced: modes::AGENT.to_owned(), summoned: summoned_of(params), ..Default::default() };
    state.model = self.default_pick(&config);
    // The settings file's level until the host replays the user's last pick
    state.approval = config.approval();
    let remote = crate::remote::Remote::of(params);
    let layers = layers_of(params);
    let scratch = self.scratch_of(std::path::Path::new(cwd), &id, remote.is_some(), &config);
    let places = self.places_of(std::path::Path::new(cwd), remote.as_deref(), layers.clone()).with_scratch(scratch.path()).with_extra(extra_of(params));
    let prompts = Arc::new(crate::prompt::Library::load(&places));
    state.prompt = crate::prompt::compose(&places, &prompts, state.model.as_deref().and_then(|p| config.find(p)));
    state.effort = default_effort(&config, &catalog, state.model.as_deref());
    let response = json!({
      "sessionId": id,
      "modes": self.modes_of(&config, &mut state),
      "configOptions": sent_options(&config, &catalog, &mut state, self.offers()),
    });
    let store = Store::new(self.session_file(&id), &id, std::path::Path::new(cwd));
    // The file starts with the first event; the prompt goes first so a reopen keeps the very same text
    store.prelude(json!({ "type": "prompt", "variant": state.prompt.variant, "version": state.prompt.version, "digest": state.prompt.digest,
      "text": state.prompt.text, "sampling": state.prompt.sampling, "thinking": state.prompt.thinking, "overrides": state.prompt.overrides }));
    let mcp = crate::mcp::Hub::start(params.get("mcpServers").unwrap_or(&Value::Null), &self.process_dir(std::path::Path::new(cwd), remote.is_some()));
    let session = Session {
      id: id.clone(),
      cwd: PathBuf::from(cwd),
      state: parking_lot::Mutex::new(state),
      store,
      tool_seq: AtomicU64::new(0),
      jobs: Default::default(),
      mcp,
      remote,
      layers,
      prompts,
      permissions_changed: Default::default(),
      scratch,
      settings: parking_lot::Mutex::new(settings),
    };
    // The starting controls too: a reopen must not take a default that changed in the meantime
    session.save_state(&session.state.lock());
    let (remote, layers) = (session.remote.clone(), session.layers.clone());
    self.sessions.lock().insert(id.clone(), Arc::new(session));
    self.announce_commands(&id, std::path::Path::new(cwd), remote.as_deref(), layers);
    Ok(response)
  }

  /// The session's slash commands: `/compact` and the skills found now (`skills.rs`). Sent inside `session/new` /
  /// `load` / `resume` like Devin does, straight to the client: a replay would only repeat a stale list
  fn announce_commands(&self, id: &str, cwd: &std::path::Path, remote: Option<&crate::remote::Remote>, layers: Layers) {
    // The plugins' skills too (`SKILLS_META`): they are the session's like the user's
    let places = self.places_of(cwd, remote, layers);
    let commands = crate::skills::commands(&crate::skills::scan(&places));
    self.conn().notify("session/update", json!({ "sessionId": id, "update": { "sessionUpdate": "available_commands_update", "availableCommands": commands } }));
  }

  /// `session/load` (replay = true) and `session/resume`: a session open in this process continues as it is, another
  /// one is read back from its file. The replay sends the recorded updates before the answer
  pub(super) fn restore(&self, params: &Value, replay: bool) -> Result<Value, RpcError> {
    let id = str_param(params, "sessionId")?;
    let gone = || RpcError::new(-32002, format!("Session not found: {id}"));
    let path = self.session_file(id);
    let want_cwd = params.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).map(PathBuf::from);
    let open = {
      let mut sessions = self.sessions.lock();
      match sessions.get(id).cloned() {
        Some(s) if want_cwd.as_ref().is_some_and(|c| c != &s.cwd) => {
          if s.state.lock().turn.is_some() {
            return Err(RpcError::new(-32603, "A turn is already running in this session"));
          }
          s.store.flush();
          sessions.remove(id);
          None
        }
        other => other,
      }
    };
    let settings = self.settings_for(client_of(params));
    let (session, updates) = match open {
      Some(s) => {
        s.store.flush();
        // Reopened as it runs, it keeps the settings its turn started with
        if s.state.lock().turn.is_none() {
          *s.settings.lock() = settings;
        }
        let updates = if replay { store::load(&path, false).map(|l| l.updates).unwrap_or_default() } else { vec![] };
        (s, updates)
      }
      None => {
        let loaded = store::load(&path, false).map_err(|_| gone())?;
        if loaded.id != id {
          return Err(gone());
        }
        let cwd = want_cwd.unwrap_or_else(|| loaded.cwd.clone());
        let mut revived = self.revive(id, cwd, &loaded, path, params.get("mcpServers").unwrap_or(&Value::Null), crate::remote::Remote::of(params), settings);
        // A session reopened in the same folder keeps its recorded prompt; moving it to another folder drops it so the next turn recomposes it
        revived.layers = layers_of(params);
        revived.prompts = Arc::new(crate::prompt::Library::load(&self.places_of(&revived.cwd, revived.remote.as_deref(), revived.layers.clone())));
        let session = Arc::new(revived);
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
    self.announce_commands(id, &session.cwd, session.remote.as_deref(), session.layers.clone());
    // An unfinished goal, straight to the client like the commands: the replay carries none (`goal::publish`)
    let goal = session.state.lock().goal.clone();
    if let Some(g) = crate::goal::published(goal.as_ref()) {
      crate::goal::publish(self, id, Some(g));
    }
    let config = session.config();
    let catalog = self.catalog.get();
    let mut st = session.state.lock();
    st.summoned |= summoned_of(params);
    if reconcile(&config, &catalog, &mut st) {
      session.save_state(&st);
    }
    Ok(json!({ "modes": self.modes_of(&config, &mut st), "configOptions": sent_options(&config, &catalog, &mut st, self.offers()) }))
  }

  /// A session object from its file: the history, the controls and the system prompt as they were
  #[expect(clippy::too_many_arguments, reason = "one reopening's parts, each taken once")]
  fn revive(
    &self,
    id: &str,
    cwd: PathBuf,
    loaded: &store::Loaded,
    path: PathBuf,
    mcp_servers: &Value,
    remote: Option<Arc<crate::remote::Remote>>,
    settings: Arc<ConfigCache>,
  ) -> Session {
    let config = settings.get();
    let catalog = self.catalog.get();
    let s = |k: &str| loaded.state.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut st = SessionState {
      mode: s("mode").filter(|m| self.offers_mode(m)).unwrap_or_else(|| modes::AGENT.to_owned()),
      announced: s("announced").unwrap_or_else(|| modes::AGENT.to_owned()),
      model: s("model").or_else(|| self.default_pick(&config)),
      items: loaded.items.clone(),
      summarized: loaded.summarized,
      carry: loaded.carry.clone(),
      chunks: loaded.chunks.clone(),
      folded: loaded.folded,
      rows: loaded.rows.clone(),
      // The replay ends on this context; a window edited since is pushed by the next `push_controls`
      reported: loaded.updates.iter().rev().find(|u| u["sessionUpdate"] == "usage_update").and_then(|u| Some((u["used"].as_u64()?, u["size"].as_u64()?))),
      ..Default::default()
    };
    st.effort = s("effort").or_else(|| default_effort(&config, &catalog, st.model.as_deref()));
    st.fast = loaded.state.get("fast").and_then(Value::as_bool).unwrap_or(false);
    st.sidekick = s("sidekick");
    st.sidekick_effort = s("sidekick_effort");
    st.sidekick_fast = loaded.state.get("sidekick_fast").and_then(Value::as_bool).unwrap_or(false);
    // A record from before the level was the session's starts on the settings file's
    st.approval = s("approval").and_then(|a| Approval::parse(&a)).unwrap_or_else(|| config.approval());
    // An active goal comes back paused: its run ended with the process that ran it (`goal::revive`)
    st.goal = loaded.goal.clone();
    let paused = crate::goal::revive(&mut st.goal);
    // The servers list their tools as the session reopens, with the sidekick the engine holds for it: this one
    st.sidekick_told = sidekick_of(&config, &st);
    if let Some(p) = &loaded.prompt {
      let prompt_cwd = p.get("cwd").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(|| loaded.cwd.clone());
      if prompt_cwd == cwd {
        let ps = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        st.prompt = crate::prompt::Composed {
          text: ps("text"),
          variant: ps("variant"),
          version: ps("version"),
          digest: ps("digest"),
          sampling: p.get("sampling").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
          thinking: p.get("thinking").cloned().and_then(|v| serde_json::from_value(v).ok()),
          overrides: p.get("overrides").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
          sections: p.get("sections").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        };
      }
    }
    // A turn cut off by a crash leaves calls without results; the next request must be well formed
    let before = st.items.len();
    crate::turn::settle(&mut st);
    let store = Store::new(path, id, &cwd);
    store.set_items_logged(loaded.logged);
    if loaded.cwd != cwd {
      store.prelude(json!({ "type": "session", "id": id, "cwd": cwd, "format": store::FORMAT }));
    }
    for item in &st.items[before..] {
      store.item(item);
    }
    if paused {
      crate::goal::save(&store, st.goal.as_ref());
    }
    // Tool call ids continue after the ones the replay holds
    let seq = loaded
      .updates
      .iter()
      .filter_map(|u| u.get("toolCallId").and_then(Value::as_str)?.strip_prefix("call-")?.parse::<u64>().ok())
      .max()
      .unwrap_or(0);
    let mcp = crate::mcp::Hub::start(mcp_servers, &self.process_dir(&cwd, remote.is_some()));
    // Reopened in another folder (`moveSession`), both commands and the next turn's recomposed prompt use the new folder
    let scratch = self.scratch_of(&cwd, id, remote.is_some(), &config);
    Session {
      id: id.to_owned(),
      cwd,
      state: parking_lot::Mutex::new(st),
      store,
      tool_seq: AtomicU64::new(seq),
      jobs: Default::default(),
      mcp,
      remote,
      layers: Layers::default(),
      prompts: Default::default(),
      permissions_changed: Default::default(),
      scratch,
      settings: parking_lot::Mutex::new(settings),
    }
  }

  /// `session/list`: this data root's sessions with at least one message, newest first, filtered by `cwd` when given
  pub(super) fn list(&self, params: &Value) -> Value {
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
}

/// The same folder, also when one spelling goes through a symlink (macOS `/var` and `/private/var`)
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
  a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}
