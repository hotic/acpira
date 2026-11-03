//! The settings page's host-side counterpart: builds the SettingsView from acpira.*,
//! writes edits back, and scans agent inventories on demand

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use acpira_shared::appearance::AXES;
use acpira_shared::i18n::{Language, Locale, resolve_locale};
use acpira_shared::inventory::{AgentHealth, AgentInventory, AgentRuntimeInfo};
use acpira_shared::settings::{SettingsView, sanitize_setting};

use crate::acp::agents::adapter_info::read_adapter_info;
use crate::acp::agents::registry::AgentRegistry;
use crate::acp::transport::rpc::BoxFuture;
use crate::agent_ext::agent_ext;
use crate::inventory::{ScanEnv, ScanInput, scan_inventory};
use crate::shared_config::{AGENTS, Outcome, Places, SharedConfig};
use acpira_shared::providers::{ProviderAction, ProvidersView};
use acpira_shared::shared_config::{SharedAction, SharedView};

/// A setting as one window's shell sees it (None: the current window)
pub type WireRead = Arc<dyn Fn(Option<u64>, &str) -> Option<Value> + Send + Sync>;

pub struct SettingsDeps {
  pub agent_config_path: std::path::PathBuf,
  /// A setting as one window's shell sees it (its own snapshot for shell keys; None: the current window)
  pub read: WireRead,
  pub write: Arc<dyn Fn(String, Value) -> BoxFuture<Result<()>> + Send + Sync>,
  /// One window's IDE language (None: the current window's)
  pub host_language: Arc<dyn Fn(Option<u64>) -> String + Send + Sync>,
  pub registry: Arc<dyn Fn() -> Arc<AgentRegistry> + Send + Sync>,
  pub runtime_info: crate::util::Lookup<AgentRuntimeInfo>,
  pub health: crate::util::Lookup<AgentHealth>,
  pub home: Arc<dyn Fn() -> String + Send + Sync>,
  pub shared: Arc<SharedConfig>,
  /// The persona file behind the `subagents` key (`relay/roster.rs`)
  pub roster: Arc<crate::relay::roster::Roster>,
  /// The built-in agent's model sources (providers.json + their keys)
  pub providers: Arc<crate::providers::ProviderStore>,
}

pub type SettingsListener = Arc<dyn Fn(&SettingsView, Locale) + Send + Sync>;

pub struct SettingsCenter {
  deps: SettingsDeps,
  /// Each listener with the window wire whose view it hears (None: the current window's)
  listeners: parking_lot::Mutex<Vec<(u64, Option<u64>, SettingsListener)>>,
  seq: std::sync::atomic::AtomicU64,
}

impl SettingsCenter {
  pub fn agent_config_path(&self) -> &std::path::Path {
    &self.deps.agent_config_path
  }

  pub fn new(deps: SettingsDeps) -> Self {
    SettingsCenter { deps, listeners: Default::default(), seq: Default::default() }
  }

  fn read_on(&self, wire: Option<u64>, key: &str) -> Value {
    sanitize_setting(key, &(self.deps.read)(wire, key).unwrap_or(Value::Null))
  }

  pub fn view(&self) -> SettingsView {
    self.view_for(None)
  }

  /// The view one window's pages get: shared keys are everyone's, shell keys (`SHELL_SETTING_KEYS`) its own
  pub fn view_for(&self, wire: Option<u64>) -> SettingsView {
    let read = |k: &str| self.read_on(wire, k);
    let str_of = |k: &str| read(k).as_str().unwrap_or("").to_owned();
    let list = |k: &str| serde_json::from_value(read(k)).unwrap_or_default();
    SettingsView {
      language: Language::parse(&str_of("language")).unwrap_or(Language::Auto),
      locale: self.locale_for(wire),
      default_agent: str_of("defaultAgent"),
      agent_order: list("agentOrder"),
      disabled_agents: list("disabledAgents"),
      session_scope: str_of("sessionScope"),
      session_list_position: str_of("sessionListPosition"),
      auto_compact: read("autoCompact").as_bool().unwrap_or(true),
      compact_at_tokens: read("compactAtTokens").as_i64().unwrap_or(300_000),
      hidden_options: serde_json::from_value(read("hiddenOptions")).unwrap_or_default(),
      account_switch: str_of("accountSwitch"),
      theme: str_of("theme"),
      ui_font_size: read("uiFontSize").as_i64().unwrap_or(13),
      code_font_size: read("codeFontSize").as_i64().unwrap_or(12),
      diff_markers: str_of("diffMarkers"),
      font_smoothing: read("fontSmoothing").as_bool().unwrap_or(false),
      share_editor_selection: read("shareEditorSelection").as_bool().unwrap_or(true),
      steer_queued: read("steerQueued").as_bool().unwrap_or(false),
      notify_sound: read("notifySound").as_bool().unwrap_or(true),
      notify_system: read("notifySystem").as_bool().unwrap_or(false),
      plan_auto_approve: list("planAutoApprove"),
      ssh_local_agents: list("sshLocalAgents"),
      subagents: self.deps.roster.list(),
      subagent_groups: self.deps.roster.groups(),
      agent_cpu_cap: read("agentCpuCap").as_i64().unwrap_or(acpira_shared::settings::AGENT_CPU_CAP.2),
      proxy: str_of("proxy"),
      open_files_with: str_of("openFilesWith"),
      desktop_layout: str_of("desktopLayout"),
      panel_scope: str_of("panelScope"),
      panel_files: read("panelFiles").as_bool().unwrap_or(true),
      panel_terminal: read("panelTerminal").as_bool().unwrap_or(true),
      panel_browser: read("panelBrowser").as_bool().unwrap_or(true),
    }
  }

  pub fn locale(&self) -> Locale {
    self.locale_for(None)
  }

  /// The locale of one window's pages: the shared language setting over that window's IDE language
  pub fn locale_for(&self, wire: Option<u64>) -> Locale {
    let lang = Language::parse(self.read_on(wire, "language").as_str().unwrap_or("auto"));
    resolve_locale(lang, &(self.deps.host_language)(wire))
  }

  /// Write, then push; the value is checked like a hand edit of the settings file
  pub async fn set(&self, key: &str, value: &Value) -> Result<()> {
    if key == "subagents" {
      self.deps.roster.save(value).await?;
    } else {
      (self.deps.write)(key.to_owned(), sanitize_setting(key, value)).await?;
    }
    self.emit();
    Ok(())
  }

  /// An appearance axis from the page: only values the axis declares are written
  pub async fn set_appearance(&self, axis: &str, value: &str) -> Result<()> {
    let Some((_, opts)) = AXES.iter().find(|(k, _)| *k == axis) else { return Ok(()) };
    if !opts.contains(&value) {
      return Ok(());
    }
    (self.deps.write)(format!("appearance.{axis}"), Value::from(value)).await
  }

  /// Scan one agent's extension points fresh; `cwd` is the project whose files count (the view's)
  pub async fn inventory(&self, agent: &str, cwd: &str) -> AgentInventory {
    let registry = (self.deps.registry)();
    let binary = registry.resolve_binary(agent).await;
    let adapter = match (&binary, registry.try_get(agent)) {
      (Some(b), Some(def)) => read_adapter_info(b, def).await,
      _ => None,
    };
    let env = ScanEnv::new((self.deps.home)(), cwd.to_owned());
    scan_inventory(
      ScanInput {
        agent: agent.to_owned(),
        ext: agent_ext(agent),
        binary,
        runtime: (self.deps.runtime_info)(agent),
        adapter,
        health: (self.deps.health)(agent),
      },
      &env,
    )
    .await
  }

  /// Built-in agents whose CLI resolves on this machine: only they get links or show up on the Shared tab
  /// Agents turned off in the settings (`disabledAgents`): the Shared tab leaves them out and takes their links back
  fn disabled_agents(&self) -> Vec<String> {
    serde_json::from_value(self.read_on(None, "disabledAgents")).unwrap_or_default()
  }

  async fn installed_agents(&self) -> Vec<String> {
    let registry = (self.deps.registry)();
    let off = self.disabled_agents();
    let mut out = vec![];
    for a in AGENTS {
      if !off.iter().any(|o| o == a) && registry.resolve_binary(a).await.is_some() {
        out.push(a.to_owned());
      }
    }
    out
  }

  fn places(&self, cwd: &str) -> Places {
    Places::new(&(self.deps.home)(), cwd)
  }

  /// The Shared tab for the project `cwd` (keeping links up to date first when "link all" is on)
  pub async fn shared_view(&self, cwd: &str) -> SharedView {
    let runtime_info = self.deps.runtime_info.clone();
    let caps = Arc::new(move |a: &str| runtime_info(a).and_then(|r| r.mcp));
    self.deps.shared.retire(&self.disabled_agents()).await;
    self.deps.shared.view(self.places(cwd), self.installed_agents().await, caps).await
  }

  pub async fn shared_action(&self, action: SharedAction, cwd: &str) -> Result<Outcome> {
    let agents = self.installed_agents().await;
    self.deps.shared.apply(action, &self.places(cwd), &agents).await
  }

  /// Startup and window focus: new shared skills get their links without opening the page
  pub async fn shared_maintain(&self, cwd: &str) -> Result<()> {
    self.deps.shared.retire(&self.disabled_agents()).await;
    let agents = self.installed_agents().await;
    self.deps.shared.maintain(&self.places(cwd), &agents).await
  }

  pub async fn providers_view(&self) -> ProvidersView {
    self.deps.providers.view().await
  }

  pub async fn provider_action(&self, action: ProviderAction) -> Result<()> {
    self.deps.providers.apply(action).await
  }

  pub async fn provider_probe(&self, probe: acpira_shared::providers::ProviderProbe) -> acpira_shared::providers::ProbeOutcome {
    self.deps.providers.probe(probe).await
  }

  /// `wire`: the window whose view the listener hears (None: the current window's)
  pub fn subscribe(&self, wire: Option<u64>, f: SettingsListener) -> u64 {
    let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    self.listeners.lock().push((id, wire, f));
    id
  }

  pub fn unsubscribe(&self, id: u64) {
    self.listeners.lock().retain(|(i, _, _)| *i != id);
  }

  /// Re-read everything and push, each listener its own window's view
  pub fn emit(&self) {
    self.emit_where(|_| true);
  }

  /// Push to the listeners of one window only: its own shell settings or language changed while another was active
  pub fn emit_wire(&self, wire: u64) {
    self.emit_where(|w| w == Some(wire));
  }

  fn emit_where(&self, pick: impl Fn(Option<u64>) -> bool) {
    let ls: Vec<(Option<u64>, SettingsListener)> =
      self.listeners.lock().iter().filter(|(_, w, _)| pick(*w)).map(|(_, w, f)| (*w, f.clone())).collect();
    // One view per window, however many pages it has
    let mut views: std::collections::HashMap<Option<u64>, (SettingsView, Locale)> = Default::default();
    for (wire, f) in ls {
      let (view, locale) = views.entry(wire).or_insert_with(|| (self.view_for(wire), self.locale_for(wire)));
      f(view, *locale);
    }
  }
}
