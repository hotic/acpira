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
use acpira_shared::shared_config::{SharedAction, SharedView};

pub struct SettingsDeps {
  pub read: Arc<dyn Fn(&str) -> Option<Value> + Send + Sync>,
  pub write: Arc<dyn Fn(String, Value) -> BoxFuture<Result<()>> + Send + Sync>,
  pub host_language: Arc<dyn Fn() -> String + Send + Sync>,
  pub registry: Arc<dyn Fn() -> Arc<AgentRegistry> + Send + Sync>,
  pub runtime_info: Arc<dyn Fn(&str) -> Option<AgentRuntimeInfo> + Send + Sync>,
  pub health: Arc<dyn Fn(&str) -> Option<AgentHealth> + Send + Sync>,
  pub home: Arc<dyn Fn() -> String + Send + Sync>,
  pub cwd: Arc<dyn Fn() -> String + Send + Sync>,
  pub shared: Arc<SharedConfig>,
}

pub type SettingsListener = Arc<dyn Fn(&SettingsView, Locale) + Send + Sync>;

pub struct SettingsCenter {
  deps: SettingsDeps,
  listeners: parking_lot::Mutex<Vec<(u64, SettingsListener)>>,
  seq: std::sync::atomic::AtomicU64,
}

impl SettingsCenter {
  pub fn new(deps: SettingsDeps) -> Self {
    SettingsCenter { deps, listeners: Default::default(), seq: Default::default() }
  }

  fn read(&self, key: &str) -> Value {
    sanitize_setting(key, &(self.deps.read)(key).unwrap_or(Value::Null))
  }

  pub fn view(&self) -> SettingsView {
    let str_of = |k: &str| self.read(k).as_str().unwrap_or("").to_owned();
    let list = |k: &str| serde_json::from_value(self.read(k)).unwrap_or_default();
    SettingsView {
      language: Language::parse(&str_of("language")).unwrap_or(Language::Auto),
      locale: self.locale(),
      default_agent: str_of("defaultAgent"),
      agent_order: list("agentOrder"),
      disabled_agents: list("disabledAgents"),
      session_scope: str_of("sessionScope"),
      session_list_position: str_of("sessionListPosition"),
      auto_compact: self.read("autoCompact").as_bool().unwrap_or(true),
      compact_at_tokens: self.read("compactAtTokens").as_i64().unwrap_or(300_000),
      hidden_options: serde_json::from_value(self.read("hiddenOptions")).unwrap_or_default(),
      account_switch: str_of("accountSwitch"),
      theme: str_of("theme"),
      ui_font_size: self.read("uiFontSize").as_i64().unwrap_or(13),
      code_font_size: self.read("codeFontSize").as_i64().unwrap_or(12),
      diff_markers: str_of("diffMarkers"),
      font_smoothing: self.read("fontSmoothing").as_bool().unwrap_or(false),
      share_editor_selection: self.read("shareEditorSelection").as_bool().unwrap_or(true),
      steer_queued: self.read("steerQueued").as_bool().unwrap_or(false),
    }
  }

  pub fn locale(&self) -> Locale {
    let lang = Language::parse(self.read("language").as_str().unwrap_or("auto"));
    resolve_locale(lang, &(self.deps.host_language)())
  }

  /// Write, then push; the value is checked like a hand edit of the settings file
  pub async fn set(&self, key: &str, value: &Value) -> Result<()> {
    (self.deps.write)(key.to_owned(), sanitize_setting(key, value)).await?;
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

  /// Scan one agent's extension points fresh
  pub async fn inventory(&self, agent: &str) -> AgentInventory {
    let registry = (self.deps.registry)();
    let binary = registry.resolve_binary(agent).await;
    let adapter = match (&binary, registry.try_get(agent)) {
      (Some(b), Some(def)) => read_adapter_info(b, def).await,
      _ => None,
    };
    let env = ScanEnv::new((self.deps.home)(), (self.deps.cwd)());
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
    serde_json::from_value(self.read("disabledAgents")).unwrap_or_default()
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

  fn places(&self) -> Places {
    Places::new(&(self.deps.home)(), &(self.deps.cwd)())
  }

  /// The Shared tab for the workspace (keeping links up to date first when "link all" is on)
  pub async fn shared_view(&self) -> SharedView {
    let runtime_info = self.deps.runtime_info.clone();
    let caps = Arc::new(move |a: &str| runtime_info(a).and_then(|r| r.mcp));
    self.deps.shared.retire(&self.disabled_agents()).await;
    self.deps.shared.view(self.places(), self.installed_agents().await, caps).await
  }

  pub async fn shared_action(&self, action: SharedAction) -> Result<Outcome> {
    let agents = self.installed_agents().await;
    self.deps.shared.apply(action, &self.places(), &agents).await
  }

  /// Startup and window focus: new shared skills get their links without opening the page
  pub async fn shared_maintain(&self) -> Result<()> {
    self.deps.shared.retire(&self.disabled_agents()).await;
    let agents = self.installed_agents().await;
    self.deps.shared.maintain(&self.places(), &agents).await
  }

  pub fn subscribe(&self, f: SettingsListener) -> u64 {
    let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    id
  }

  pub fn unsubscribe(&self, id: u64) {
    self.listeners.lock().retain(|(i, _)| *i != id);
  }

  /// Re-read everything and push
  pub fn emit(&self) {
    let view = self.view();
    let locale = self.locale();
    let ls: Vec<SettingsListener> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
    for f in ls {
      f(&view, locale);
    }
  }
}
