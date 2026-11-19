//! The agents: the registry and its probes, warm processes, remembered settings and model shapes, health, hidden
//! agents and the controls a new session paints before session/new

use std::collections::BTreeMap;
use std::sync::Arc;

use acpira_shared::agent_order::arrange_agents;
use acpira_shared::inventory::{AgentHealth, AgentHealthStage, AgentRuntimeInfo, HealthSource};
use acpira_shared::model_shapes::{ModelMemory, ModelShapes, is_model_control, learn_shape};
use acpira_shared::protocol::HostMsg;
use acpira_shared::settings::HiddenMap;
use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use super::{LOGIN_PATH_REFRESH, PROBE_INTERVAL, SessionManager};
use crate::acp::agents::login_path;
use crate::acp::agents::probe_controls::probe_agent_controls;
use crate::acp::agents::registry::{AgentRegistry, SELF_AGENT_ID};
use crate::acp::session::AcpSession;
use crate::acp::transport::rpc::BoxFuture;
use crate::util::now_iso;

impl SessionManager {
  /// Resolves once the first probe pass has run: `init` may return before it (see there)
  pub async fn agents_probed(&self) {
    let _ = self.first_probe.subscribe().wait_for(|done| *done).await;
  }

  /// Same rule as `pickDefaultAgent` (packages/shared/agentOrder.ts): the configured agent unless it is switched off or its
  /// CLI is missing, then the first enabled installed agent (the built-in one leads the registry). A missing default
  /// with nothing installed to take over stays, so the error names it; an unknown id stays (a custom agent not probed yet)
  pub(super) fn default_agent(&self) -> String {
    let preferred = (self.deps.default_agent)();
    let agents = self.agents();
    let Some(own) = agents.iter().find(|a| a.id == preferred) else { return preferred };
    let disabled = own.disabled == Some(true);
    if !disabled && own.available != Some(false) {
      return preferred;
    }
    let open: Vec<&AgentInfo> = agents.iter().filter(|a| a.disabled != Some(true)).collect();
    let installed = open.iter().find(|a| a.available != Some(false));
    installed.or(if disabled { open.first() } else { None }).map(|a| a.id.clone()).unwrap_or(preferred)
  }

  pub fn emit_agents(&self) {
    self.emit(HostMsg::Agents { agents: self.agents() });
  }

  pub(super) fn watch_registry(self: &Arc<Self>, r: &Arc<AgentRegistry>) {
    let old = {
      let st = self.state.lock();
      (st.registry.clone(), st.registry_watch)
    };
    old.0.unsubscribe(old.1);
    let me = self.me.clone();
    let id = r.subscribe(Arc::new(move || {
      if let Some(m) = me.upgrade() {
        m.emit_agents();
        m.schedule_probe();
      }
    }));
    self.state.lock().registry_watch = id;
  }

  pub fn reprobe(self: &Arc<Self>) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move {
      // Back from a terminal with something still missing: the rc files may have just gained its directory
      if me.registry().missing() && login_path::refresh(LOGIN_PATH_REFRESH).await {
        me.log_login_path();
      }
      me.registry().probe_all().await;
      me.schedule_probe();
    })
  }

  /// Poll only while something is missing
  pub(super) fn schedule_probe(self: &Arc<Self>) {
    let mut st = self.state.lock();
    if let Some(h) = st.probe_timer.take() {
      h.abort();
    }
    if st.disposed || !st.registry.missing() {
      return;
    }
    let weak = self.me.clone();
    let h = tokio::spawn(async move {
      tokio::time::sleep(PROBE_INTERVAL).await;
      if let Some(m) = weak.upgrade() {
        m.state.lock().probe_timer = None;
        m.reprobe().await;
      }
    });
    st.probe_timer = Some(h.abort_handle());
  }

  pub(super) fn warm(&self, agent: &str, account: Option<String>) {
    let Some(cwd) = self.folder_for(None, None) else { return };
    // A session in a folder on an SSH host starts its agent in this machine's home (`AcpSession::process_cwd`)
    let cwd = crate::workspace::process_folder(cwd);
    let acc = account.or_else(|| self.default_account(agent));
    self.pool.ensure(agent, &cwd, acc.as_deref());
  }

  pub(super) fn default_account(&self, agent: &str) -> Option<String> {
    self.deps.accounts.as_ref().filter(|a| a.supports(agent)).and_then(|a| a.default_for(agent)).map(|a| a.id)
  }

  pub fn last_settings(&self, agent: &str) -> Option<TurnSettings> {
    self.state.lock().prefs.last_settings.get(agent).cloned()
  }

  pub(super) fn remember(&self, s: &AcpSession) {
    let config = capture_turn_settings(&s.agent_controls()).config;
    // Once a split-access agent remembers its approval select, drop any stale pre-split level mode (`auto`, `bypassPermissions`, …)
    let levels = config
      .contains_key(crate::acp::session::access::APPROVAL)
      .then(|| self.registry().get(&s.agent).ok().and_then(|d| d.access.clone()))
      .flatten();
    {
      let mut st = self.state.lock();
      let cur = st.prefs.last_settings.entry(s.agent.clone()).or_default();
      cur.config = config;
      if let Some(levels) = levels.as_deref()
        && cur.mode_id.as_deref().is_some_and(|m| crate::acp::session::access::is_level_mode(levels, m))
      {
        cur.mode_id = None;
      }
    }
    self.remember_model(s);
    self.save_prefs(&s.agent);
  }

  /// The model a pick left the session on, with its own parameters as they stand, becomes the agent's newest (the
  /// picker shows each model with what it last ran with, and a switch back restores it). Every open session of the
  /// agent shows the memory, so each is pushed again
  fn remember_model(&self, s: &AcpSession) {
    let controls = s.agent_controls();
    {
      let mut st = self.state.lock();
      let mut memory = st.prefs.model_memory.get(&s.agent).cloned().unwrap_or_default();
      if !memory.note(&controls) {
        return;
      }
      st.prefs.model_memory.insert(s.agent.clone(), memory);
    }
    for other in self.live_sessions() {
      if other.agent == s.agent {
        other.republish();
      }
    }
  }

  /// A user's pick on a live session, then the parameters that belong with it: `also` when the page sent them (a model
  /// picked together with its level), else what a newly picked model last ran with. The model's own switch has ended
  /// by then, carried-over effort included (`set_config_wire`), so nothing here races it. A parameter is set only when
  /// the session offers that value and holds another; one that fails is logged and the rest still apply
  pub(super) async fn pick_config(&self, s: &Arc<AcpSession>, config_id: String, value: String, also: Option<BTreeMap<String, String>>) -> anyhow::Result<()> {
    let model = s.agent_controls().options.iter().any(|c| c.id == config_id && is_model_control(c));
    s.select_config(config_id.clone(), value.clone()).await?;
    let controls = s.agent_controls();
    let landed = controls.options.iter().any(|c| c.id == config_id && c.value.as_deref() == Some(value.as_str()));
    let params = match also {
      Some(also) => Some(also),
      None if model && landed => self.state.lock().prefs.model_memory.get(&s.agent).and_then(|m| m.picks.get(&value).cloned()),
      None => None,
    };
    for (id, wanted) in params.into_iter().flatten() {
      // A session still starting defers its picks (`defer_pick`): the parameter joins them, checked when they replay
      let apply = s.status() != SessionStatus::Ready
        || s.agent_controls().options.iter().any(|c| c.id == id && c.value.as_deref() != Some(wanted.as_str()) && c.options.iter().any(|o| o.id == wanted));
      if apply && let Err(e) = s.select_config(id.clone(), wanted.clone()).await {
        self.log(&format!("setConfig {id}={wanted} after {config_id} failed: {e}"));
      }
    }
    self.remember(s);
    Ok(())
  }

  /// What a session took over its remembered values while it started (`adopt_controls`) is remembered like a pick
  pub(super) fn remember_picked(&self, s: &AcpSession, picked: &TurnSettings) {
    if !picked.config.is_empty() {
      self.remember(s);
    }
    if let Some(mode) = picked.mode_id.as_deref().filter(|m| s.agent_controls().mode_id.as_deref() == Some(*m)) {
      self.remember_mode(&s.agent, mode);
    }
  }

  pub(super) fn remember_mode(&self, agent: &str, mode: &str) {
    {
      let mut st = self.state.lock();
      let cur = st.prefs.last_settings.entry(agent.to_owned()).or_default();
      if cur.mode_id.as_deref() == Some(mode) {
        return;
      }
      cur.mode_id = Some(mode.to_owned());
    }
    self.save_prefs(agent);
  }

  /// A ready session's agent truth tells which parameters its current model comes with
  pub(super) fn learn_shape_of(&self, s: &AcpSession) {
    if s.status() != SessionStatus::Ready {
      return;
    }
    let mut controls = s.agent_controls();
    // The host's sidekick selects list other CLIs' models: the session's, not a parameter this model came with
    controls.options.retain(|o| !crate::acp::session::host_sidekick::ours(o));
    let next = {
      let st = self.state.lock();
      learn_shape(st.prefs.model_shapes.as_ref().and_then(|m| m.get(&s.agent)), &controls)
    };
    let Some(next) = next else { return };
    self.state.lock().prefs.model_shapes.get_or_insert_with(Default::default).insert(s.agent.clone(), next);
    self.save_prefs(&s.agent);
  }

  fn save_prefs(&self, agent: &str) {
    let Some(me) = self.me.upgrade() else { return };
    let agent = agent.to_owned();
    let task = tokio::spawn(async move {
      // Each write takes the newest prefs when its turn comes, so a late one can never put back an older snapshot
      let _turn = me.prefs_writer.lock().await;
      let prefs = me.state.lock().prefs.clone();
      if let Err(e) = me.deps.store.save_prefs(&prefs, &[agent]).await {
        me.log(&format!("prefs save failed: {e}"));
      }
    });
    let mut saves = self.prefs_saves.lock();
    saves.retain(|h| !h.is_finished());
    saves.push(task);
  }

  /// Model shapes another window learned reach this one's history editor, and the models only another engine ran
  /// join this one's memory
  pub(super) async fn sync_shapes(&self) {
    let prefs = self.deps.store.load_prefs().await;
    let disk = prefs.model_shapes.unwrap_or_default();
    let changed: Vec<String> = {
      let mut st = self.state.lock();
      let mine = st.prefs.model_shapes.clone().unwrap_or_default();
      let mut changed: Vec<String> = disk
        .iter()
        .filter(|(agent, models)| models.iter().any(|(model, shape)| mine.get(*agent).and_then(|m| m.get(model)) != Some(shape)))
        .map(|(a, _)| a.clone())
        .collect();
      if !changed.is_empty() {
        let shapes = st.prefs.model_shapes.get_or_insert_with(Default::default);
        for agent in &changed {
          let entry = shapes.entry(agent.clone()).or_default();
          for (k, v) in &disk[agent] {
            entry.insert(k.clone(), v.clone());
          }
        }
      }
      for (agent, theirs) in &prefs.model_memory {
        let mut memory = st.prefs.model_memory.get(agent).cloned().unwrap_or_default();
        memory.absorb(theirs);
        if st.prefs.model_memory.get(agent) != Some(&memory) && !memory.recent.is_empty() {
          st.prefs.model_memory.insert(agent.clone(), memory);
          if !changed.contains(agent) {
            changed.push(agent.clone());
          }
        }
      }
      if changed.is_empty() {
        return;
      }
      changed
    };
    for s in self.live_sessions() {
      if changed.contains(&s.agent) {
        s.republish();
      }
    }
  }

  /// Swap the registry (acpira.agents changed): drop warm processes started with the old command, re-probe, re-push
  pub fn set_registry(self: &Arc<Self>, r: Arc<AgentRegistry>) {
    self.watch_registry(&r);
    self.state.lock().registry = r;
    self.pool.invalidate(None);
    let me = self.clone();
    tokio::spawn(async move {
      me.reprobe().await;
      me.emit_agents();
    });
  }

  /// Agents started from now on get a fresh process (the network route changed); running sessions keep theirs
  pub fn invalidate_warm_pool(&self) {
    self.pool.invalidate(None);
  }

  /// Every agent's latest in-app install, for the init state
  pub fn agent_installs(&self) -> Vec<acpira_shared::protocol::AgentInstallProgress> {
    self.installs.list()
  }

  pub fn agents(&self) -> Vec<AgentInfo> {
    let registry = self.registry();
    let all: Vec<AgentInfo> = registry
      .list()
      .into_iter()
      .map(|mut a| {
        if let Some(m) = self.deps.accounts.as_ref().filter(|m| m.supports(&a.id)) {
          a.accounts = Some(true);
          a.account_keys = m.accepts_keys(&a.id).then_some(true);
          a.credentials_locked = m.credentials_locked(&a.id).then_some(true);
        } else {
          a.local_account = self.deps.local_accounts.as_ref().and_then(|l| l.get(&a.id));
        }
        a
      })
      .collect();
    arrange_agents(all, &(self.deps.agent_prefs)())
  }

  pub fn runtime_info(&self, agent: &str) -> Option<AgentRuntimeInfo> {
    for s in self.live_sessions() {
      if s.agent == agent
        && let Some(info) = s.runtime_info()
      {
        return Some(info);
      }
    }
    self.state.lock().probed.get(agent).map(|p| p.runtime.clone())
  }

  pub fn agent_health(&self, agent: &str) -> Option<AgentHealth> {
    self.state.lock().health.get(agent).cloned()
  }

  pub fn hidden(&self) -> HiddenMap {
    (self.deps.hidden)()
  }

  pub fn emit_hidden(&self) {
    self.emit(HostMsg::Hidden { hidden: self.hidden() });
  }

  /// The built-in agent's settings were saved here: its processes push their sessions' changed controls at once
  /// (`acpira_agent::acp::SETTINGS_CHANGED`), each process told once, and every page holding its model list (the
  /// persona page, the agent page, the summon menu) gets the new one
  pub fn builtin_settings_saved(&self) {
    let live: Vec<_> = self.state.lock().live.values().filter(|s| s.agent == SELF_AGENT_ID).cloned().collect();
    let mut told: Vec<Arc<crate::acp::transport::process::AgentProcess>> = vec![];
    for s in live {
      let proc = s.core.lock().proc.clone();
      if let Some(p) = proc.filter(|p| !told.iter().any(|t| Arc::ptr_eq(t, p))) {
        p.notify(acpira_agent::acp::SETTINGS_CHANGED, serde_json::json!({}));
        told.push(p);
      }
    }
    let Some(me) = self.me.upgrade() else { return };
    tokio::spawn(async move {
      let agent = SELF_AGENT_ID.to_owned();
      let commands = me.known_commands(&agent).await;
      me.emit(HostMsg::Controls { controls: me.builtin_controls(), shapes: me.model_shapes(&agent), commands, agent, cwd: None });
    });
  }

  /// The built-in agent's configOptions as a new session of it gets them. They follow from its settings
  /// (`providers.json`), so they are worked out from the file rather than remembered from a session or probe that read
  /// an older one: a model added or switched off shows everywhere at once
  fn builtin_controls(&self) -> Vec<ConfigControl> {
    use acpira_agent::acp::{Offers, SessionState, config_options};
    let config = acpira_agent::config::ConfigCache::new(self.data_root()).get();
    let st = SessionState { approval: config.approval(), ..Default::default() };
    // The capabilities the engine's initialize declares (`transport/process.rs`), so the preview has the live shape
    let options = config_options(&config, &crate::model_catalog::current(), &st, Offers { booleans: true, sidekick: true });
    let mut controls = SessionControls::default();
    crate::acp::transcript::normalize::apply_config_options(&mut controls, &options);
    controls.options
  }

  /// The configOptions an agent offered most recently
  pub async fn known_controls(&self, agent: &str) -> Vec<ConfigControl> {
    if agent == SELF_AGENT_ID {
      return self.builtin_controls();
    }
    if let Some(p) = self.state.lock().probed.get(agent).filter(|p| !p.options.is_empty()) {
      return p.options.clone();
    }
    let index = self.state.lock().index.clone();
    for s in index.iter().filter(|s| s.agent == agent) {
      let options = match self.live(&s.id) {
        Some(l) => l.agent_controls().options,
        None => self.deps.store.load_controls(&s.id).await.map(|c| c.options).unwrap_or_default(),
      };
      if !options.is_empty() {
        return options;
      }
    }
    vec![]
  }

  /// The CLIs a host-owned sidekick can run in (`acp/session/host_sidekick.rs`): every agent that is on and found, with
  /// the model select its probe, a live session or one of its newest records showed
  pub async fn sidekick_sources(&self) -> Vec<crate::acp::session::host_sidekick::SidekickSource> {
    // Records to open per agent before giving up on it: older ones are not worth the reads
    const RECORDS: usize = 3;
    let index = self.state.lock().index.clone();
    let mut out = vec![];
    for a in self.agents() {
      if a.disabled == Some(true) || a.available == Some(false) {
        continue;
      }
      // The built-in agent's list comes from its settings, never from an older record
      let builtin = a.id == SELF_AGENT_ID;
      let mut options = if builtin {
        self.builtin_controls()
      } else {
        self.state.lock().probed.get(&a.id).map(|p| p.options.clone()).unwrap_or_default()
      };
      for s in index.iter().filter(|s| s.agent == a.id && !builtin).take(RECORDS) {
        if options.iter().any(is_model_control) {
          break;
        }
        options = match self.live(&s.id) {
          Some(l) => l.agent_controls().options,
          None => self.deps.store.load_controls(&s.id).await.map(|c| c.options).unwrap_or_default(),
        };
      }
      let Some(model) = options.into_iter().find(|o| is_model_control(o) && !o.options.is_empty()) else { continue };
      out.push(crate::acp::session::host_sidekick::SidekickSource { shapes: self.model_shapes(&a.id), agent: a.id, name: a.name, model });
    }
    out
  }

  /// The slash commands this agent's newest session advertised, for a page with no session of its own yet
  pub async fn known_commands(&self, agent: &str) -> Vec<SlashCommand> {
    let index = self.state.lock().index.clone();
    for s in index.iter().filter(|s| s.agent == agent) {
      let commands = match self.live(&s.id) {
        Some(l) => l.agent_commands(),
        None => self.deps.store.load_commands(&s.id).await.unwrap_or_default(),
      };
      if !commands.is_empty() {
        return commands;
      }
    }
    vec![]
  }

  /// The parameters each of this agent's models came with, as remembered from its sessions (`model_shapes.rs`)
  pub fn model_shapes(&self, agent: &str) -> Option<ModelShapes> {
    self.state.lock().prefs.model_shapes.as_ref().and_then(|s| s.get(agent).cloned())
  }

  /// What the user last ran each of this agent's models with
  pub fn model_memory(&self, agent: &str) -> Option<ModelMemory> {
    self.state.lock().prefs.model_memory.get(agent).cloned()
  }

  /// What a new session of this agent paints before session/new: the probe's or a stored session's options (the built-in
  /// agent's from its settings), and the modes a stored session showed (protocol modes are never probed)
  pub(super) async fn known_start_controls(&self, agent: &str) -> SessionControls {
    let probed = if agent == SELF_AGENT_ID {
      Some(self.builtin_controls())
    } else {
      self.state.lock().probed.get(agent).filter(|p| !p.options.is_empty()).map(|p| p.options.clone())
    };
    let index = self.state.lock().index.clone();
    let mut out = SessionControls::default();
    for s in index.iter().filter(|s| s.agent == agent) {
      let controls = match self.live(&s.id) {
        Some(l) => l.agent_controls(),
        None => self.deps.store.load_controls(&s.id).await.unwrap_or_default(),
      };
      if out.modes.is_empty() && !controls.modes.is_empty() {
        out.modes = controls.modes;
        out.mode_config_id = controls.mode_config_id;
      }
      if !controls.options.is_empty() && probed.is_none() {
        out.options = controls.options;
        break;
      }
      // The probe has the options: the newest session's modes are enough, older records are not worth loading
      if probed.is_some() {
        break;
      }
    }
    if let Some(options) = probed {
      out.options = options;
    }
    out
  }

  /// The settings page's refresh button: a throwaway spawn reads the CLI's current configOptions
  pub async fn probe_controls(self: &Arc<Self>, agent: &str) -> Vec<ConfigControl> {
    self.pool.invalidate(Some(agent));
    let registry = self.registry();
    let Ok(def) = registry.get(agent).cloned() else { return self.known_controls(agent).await };
    let Some(bin) = registry.resolve_binary(agent).await else {
      self.log(&format!("probe {agent}: no binary"));
      return self.known_controls(agent).await;
    };
    let acc = self.default_account(agent);
    let env = match (&acc, &self.deps.accounts) {
      (Some(a), Some(m)) => m.spawn_env_for(agent, a).await,
      _ => None,
    };
    // A project shell without any project probes in the home directory, like a shell without a workspace always did
    let cwd = crate::workspace::process_folder(self.folder_for(None, None).unwrap_or_else(|| (self.deps.cwd)()));
    match probe_agent_controls(&def, &bin, &cwd, env.as_ref(), self.deps.log.clone(), None).await {
      Ok(r) => {
        let options = r.options.clone();
        {
          let mut st = self.state.lock();
          st.probed.insert(agent.to_owned(), r);
          st.health.insert(
            agent.to_owned(),
            AgentHealth { stage: AgentHealthStage::Ready, at: now_iso(), error: None, source: HealthSource::Probe },
          );
        }
        self.warm(agent, acc);
        options
      }
      Err(e) => {
        self.state.lock().health.insert(
          agent.to_owned(),
          AgentHealth { stage: e.stage, at: now_iso(), error: Some(e.message.clone()), source: HealthSource::Probe },
        );
        self.log(&format!("probe {agent} failed: {}", e.message));
        self.known_controls(agent).await
      }
    }
  }
}
