//! The one composition root (mirror of src/host/runtime.ts): registry, vault, account layer, session manager and
//! settings center built from a platform, reacting to its settings / focus events. Views attach as BridgeCores

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use acpira_shared::agent_order::AgentPrefs;
use acpira_shared::appearance::{Appearance, appearance_from_settings};
use acpira_shared::protocol::WebviewHost;
use acpira_shared::settings::{SETTING_KEYS, id_list, is_hidden_map};
use acpira_shared::sidecar::InitialView;

use crate::accounts::account_manager::AccountManager;
use crate::accounts::account_store::{AccountStore, FileVault};
use crate::accounts::devin::DevinAccountProvider;
use crate::accounts::local::LocalAccounts;
use crate::acp::agent_registry::AgentRegistry;
use crate::acp::session::CompactionPolicy;
use crate::bridge_core::{BridgeCore, Post};
use crate::external::chatgpt_store::ChatGptBridgeStore;
use crate::i18n::{set_host_locale, tp};
use crate::session_manager::{ManagerDeps, SessionManager};
use crate::settings::{SettingsCenter, SettingsDeps};
use crate::sidecar::platform::{Affects, SidecarPlatform};
use crate::store::transcript_store::{LogFn, TranscriptStore};

pub struct HostRuntime {
  pub manager: Arc<SessionManager>,
  pub settings: Arc<SettingsCenter>,
  pub sessions_dir: PathBuf,
  platform: Arc<SidecarPlatform>,
  accounts: Arc<AccountManager>,
  bridges: parking_lot::Mutex<Vec<Arc<BridgeCore>>>,
}

impl HostRuntime {
  /// `root`: ACPIRA_HOME / ~/.acpira; `bridge_exe`: the executable the ChatGPT connection prompt names
  pub async fn create(platform: Arc<SidecarPlatform>, root: PathBuf, bridge_exe: Option<String>) -> Result<Arc<HostRuntime>> {
    let p = platform.clone();
    let log: LogFn = Arc::new(move |line: &str| p.log(line));
    let vault = Arc::new(FileVault::new(root.join("secrets.json"), log.clone()));
    let account_store = Arc::new(AccountStore::new(root.join("accounts.json"), vault, log.clone()));
    account_store.load().await?;

    let read = {
      let p = platform.clone();
      move |key: &str| p.read_setting(key)
    };
    let registry = Arc::new(AgentRegistry::new(&read("agents").unwrap_or(Value::Null)));

    // The manager is created below; the Devin provider resolves its binary through whatever registry is current then
    let mgr_slot: Arc<parking_lot::Mutex<Option<std::sync::Weak<SessionManager>>>> = Default::default();
    let slot = mgr_slot.clone();
    let fallback = registry.clone();
    let devin = DevinAccountProvider::new(
      root.join("scratch"),
      Arc::new(move || {
        let reg = slot.lock().as_ref().and_then(|w| w.upgrade()).map(|m| m.registry()).unwrap_or_else(|| fallback.clone());
        Box::pin(async move { reg.resolve_binary("devin").await })
      }),
    );
    let terminal = {
      let p = platform.clone();
      Arc::new(move |title, command, args, env| p.run_in_terminal(title, command, args, env))
    };
    let toast = {
      let p = platform.clone();
      Arc::new(move |level: &str, text: &str| p.toast(level, text))
    };
    let accounts = AccountManager::new(account_store, vec![Arc::new(devin)], log.clone(), terminal.clone(), toast.clone());

    let sessions_dir = root.join("sessions");
    let save_toast = toast.clone();
    let store = TranscriptStore::new(
      sessions_dir.clone(),
      log.clone(),
      Some(Arc::new(move |_id: &str, error: &str| save_toast("error", &tp("host.saveFailed", &[("error", error)])))),
    );
    let chatgpt = ChatGptBridgeStore::new(root.join("bridges").join("chatgpt"), log.clone(), bridge_exe);

    let local_env_slot = mgr_slot.clone();
    let local_accounts = LocalAccounts::new(Arc::new(move |agent: &str| {
      let mut env: HashMap<String, String> = std::env::vars().collect();
      if let Some(m) = local_env_slot.lock().as_ref().and_then(|w| w.upgrade())
        && let Some(def) = m.registry().try_get(agent)
      {
        env.extend(def.env.clone().unwrap_or_default());
      }
      env
    }));

    let r = |p: &Arc<SidecarPlatform>| {
      let p = p.clone();
      move |k: &str| p.read_setting(k)
    };
    let (r1, r2, r3, r4, r5) = (r(&platform), r(&platform), r(&platform), r(&platform), r(&platform));
    let cwd_p = platform.clone();
    let manager = SessionManager::new(
      registry,
      ManagerDeps {
        store,
        chatgpt: Some(chatgpt),
        log: log.clone(),
        cwd: Arc::new(move || cwd_p.cwd()),
        default_agent: Arc::new(move || r1("defaultAgent").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "grok".into())),
        agent_prefs: Arc::new(move || AgentPrefs {
          order: id_list(&r2("agentOrder").unwrap_or(Value::Null)),
          disabled: id_list(&r2("disabledAgents").unwrap_or(Value::Null)),
        }),
        run_in_terminal: terminal,
        toast,
        accounts: Some(accounts.clone()),
        local_accounts: Some(local_accounts),
        compaction: Arc::new(move || CompactionPolicy {
          at_tokens: r3("compactAtTokens").and_then(|v| v.as_f64()).unwrap_or(300_000.0),
          auto: r3("autoCompact").and_then(|v| v.as_bool()).unwrap_or(true),
        }),
        hidden: Arc::new(move || {
          r4("hiddenOptions").filter(is_hidden_map).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
        }),
        scope: Arc::new(move || match r5("sessionScope").and_then(|v| v.as_str().map(str::to_owned)).as_deref() {
          Some("all") => "all".into(),
          _ => "workspace".into(),
        }),
      },
    );
    *mgr_slot.lock() = Some(Arc::downgrade(&manager));

    let (m1, m2, m3) = (manager.clone(), manager.clone(), manager.clone());
    let (pr, pw, ph, phome, pcwd) = (platform.clone(), platform.clone(), platform.clone(), platform.clone(), platform.clone());
    let settings = Arc::new(SettingsCenter::new(SettingsDeps {
      read: Arc::new(move |k| pr.read_setting(k)),
      write: Arc::new(move |k, v| {
        let p = pw.clone();
        Box::pin(async move { p.write_setting(&k, v).await })
      }),
      host_language: Arc::new(move || ph.host_language()),
      registry: Arc::new(move || m1.registry()),
      runtime_info: Arc::new(move |a| m2.runtime_info(a)),
      health: Arc::new(move |a| m3.agent_health(a)),
      home: Arc::new(move || phome.home()),
      cwd: Arc::new(move || pcwd.cwd()),
    }));

    let runtime =
      Arc::new(HostRuntime { manager, settings, sessions_dir, platform: platform.clone(), accounts, bridges: Default::default() });
    let weak = Arc::downgrade(&runtime);
    platform.on_settings_changed(Arc::new(move |affects| {
      if let Some(rt) = weak.upgrade() {
        rt.settings_changed(affects);
      }
    }));
    let weak = Arc::downgrade(&runtime);
    platform.on_window_focus(Arc::new(move || {
      let Some(rt) = weak.upgrade() else { return };
      // Back from a terminal where a CLI was installed, or from another window sharing ~/.acpira
      tokio::spawn(async move {
        rt.manager.reprobe().await;
        rt.manager.refresh_index().await;
        rt.accounts.reload().await;
      });
    }));
    runtime.manager.init().await;
    set_host_locale(runtime.settings.locale());
    Ok(runtime)
  }

  pub fn appearance(&self) -> Appearance {
    appearance_from_settings(|k| self.platform.read_setting(&format!("appearance.{k}")))
  }

  pub fn attach_view(
    self: &Arc<Self>,
    host: WebviewHost,
    initial: Option<InitialView>,
    blob_base: Option<String>,
    post: Post,
  ) -> Arc<BridgeCore> {
    let rt = Arc::downgrade(self);
    let appearance = Arc::new(move || rt.upgrade().map(|r| r.appearance()).unwrap_or_default());
    let core =
      BridgeCore::new(self.manager.clone(), self.settings.clone(), self.platform.clone(), appearance, post, host, blob_base, initial);
    self.bridges.lock().push(core.clone());
    core
  }

  pub fn detach_view(&self, core: &Arc<BridgeCore>) {
    let mut bridges = self.bridges.lock();
    if let Some(i) = bridges.iter().position(|b| Arc::ptr_eq(b, core)) {
      bridges.remove(i);
      drop(bridges);
      core.dispose();
    }
  }

  fn settings_changed(self: &Arc<Self>, affects: Affects) {
    if affects(Some("appearance")) {
      for b in self.bridges.lock().clone() {
        b.push_appearance();
      }
    }
    if affects(Some("agents")) {
      let custom = self.platform.read_setting("agents").unwrap_or(Value::Null);
      self.manager.set_registry(Arc::new(AgentRegistry::new(&custom)));
    }
    if affects(Some("hiddenOptions")) {
      self.manager.emit_hidden();
    }
    if affects(Some("agentOrder")) || affects(Some("disabledAgents")) {
      self.manager.emit_agents();
    }
    // Checked per key: one event may carry an appearance axis and a language change together
    if SETTING_KEYS.iter().any(|k| affects(Some(k))) {
      self.settings.emit();
      set_host_locale(self.settings.locale());
    }
  }

  pub async fn dispose(&self) {
    let bridges: Vec<_> = self.bridges.lock().drain(..).collect();
    for b in bridges {
      b.dispose();
    }
    self.manager.dispose().await;
  }
}
