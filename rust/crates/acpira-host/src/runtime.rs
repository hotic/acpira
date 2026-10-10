//! The one composition root: registry, vault, account layer, session manager and
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
use crate::accounts::claude::ClaudeAccountProvider;
use crate::accounts::codex::CodexAccountProvider;
use crate::accounts::devin::{BinaryFn, DevinAccountProvider};
use crate::accounts::local::LocalAccounts;
use crate::accounts::switch::SwitchStrategy;
use crate::acp::agents::registry::AgentRegistry;
use crate::acp::session::CompactionPolicy;
use crate::bridge_core::{BridgeCore, Post};
use crate::external::chatgpt_store::ChatGptBridgeStore;
use crate::i18n::{set_host_locale, tp};
use crate::session_manager::{ManagerDeps, SessionManager};
use crate::settings::{SettingsCenter, SettingsDeps};
use crate::sidecar::platform::{Affects, SidecarPlatform};
use crate::store::agent_config::AgentConfig;
use crate::store::transcript_store::{LogFn, TranscriptStore};

pub struct HostRuntime {
  pub manager: Arc<SessionManager>,
  pub settings: Arc<SettingsCenter>,
  pub sessions_dir: PathBuf,
  platform: Arc<SidecarPlatform>,
  accounts: Arc<AccountManager>,
  agent_config: Arc<AgentConfig>,
  /// The engine binary and data root the built-in agent is launched with; None when the binary path is unknown
  self_agent: Option<(String, PathBuf)>,
  config_watch: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
  bridges: parking_lot::Mutex<Vec<Arc<BridgeCore>>>,
}

impl HostRuntime {
  /// `root`: ACPIRA_HOME / ~/.acpira; `bridge_exe`: the executable the ChatGPT connection prompt names
  pub async fn create(platform: Arc<SidecarPlatform>, root: PathBuf, bridge_exe: Option<String>) -> Result<Arc<HostRuntime>> {
    let p = platform.clone();
    let log: LogFn = Arc::new(move |line: &str| p.log(line));
    let vault = Arc::new(FileVault::new(root.join("secrets.json"), log.clone()));
    let providers = Arc::new(crate::providers::ProviderStore::new(&root, vault.clone()));
    let account_store = Arc::new(AccountStore::new(root.join("accounts.json"), vault, log.clone()));
    account_store.load().await?;

    let agent_config = Arc::new(AgentConfig::new(&root));
    if let Err(e) = agent_config.initialize(platform.read_setting("agents")).await {
      platform.log(&format!("{}: {e}", agent_config.path().display()));
      platform.toast("error", &format!("{}: {e}", agent_config.path().display()));
    }
    let self_agent = bridge_exe.clone().map(|exe| (exe, root.clone()));
    let registry = Arc::new(registry_of(&agent_config.snapshot(), self_agent.as_ref()));

    // The manager is created below; the providers resolve their binaries through whatever registry is current then
    let mgr_slot: Arc<parking_lot::Mutex<Option<std::sync::Weak<SessionManager>>>> = Default::default();
    let binary_of = |agent: &'static str| -> BinaryFn {
      let (slot, fallback) = (mgr_slot.clone(), registry.clone());
      Arc::new(move || {
        let reg = slot.lock().as_ref().and_then(|w| w.upgrade()).map(|m| m.registry()).unwrap_or_else(|| fallback.clone());
        Box::pin(async move { reg.resolve_binary(agent).await })
      })
    };
    let devin = DevinAccountProvider::new(root.join("scratch"), binary_of("devin"));
    let codex = CodexAccountProvider::new(root.join("accounts").join("codex"), binary_of("codex"));
    let claude = ClaudeAccountProvider::new(root.join("accounts").join("claude"), binary_of("claude"));
    let terminal = {
      let p = platform.clone();
      // Login and install terminals take the same network route as the agents; the caller's own variables win
      Arc::new(move |title, command, args, env: Option<std::collections::BTreeMap<String, Option<String>>>| {
        let mut merged = crate::net_proxy::terminal_env();
        merged.extend(env.unwrap_or_default());
        p.run_in_terminal(title, command, args, (!merged.is_empty()).then_some(merged))
      })
    };
    let toast = {
      let p = platform.clone();
      Arc::new(move |level: &str, text: &str| p.toast(level, text))
    };
    let accounts = AccountManager::new(
      account_store,
      vec![Arc::new(devin), Arc::new(codex), Arc::new(claude)],
      log.clone(),
      terminal.clone(),
      toast.clone(),
    );
    let policy = platform.clone();
    // One strategy for every agent; the setting is read at each switch, so an edit applies to the next exhausted turn
    accounts.set_switch_policy(Arc::new(move |_agent: &str| {
      SwitchStrategy::parse(policy.read_setting("accountSwitch").as_ref().and_then(Value::as_str))
    }));

    let sessions_dir = root.join("sessions");
    let save_toast = toast.clone();
    let store = TranscriptStore::new(
      sessions_dir.clone(),
      log.clone(),
      Some(Arc::new(move |_id: &str, error: &str| save_toast("error", &tp("host.saveFailed", &[("error", error)])))),
    );
    // Cross-harness subagents: the persona file and the loopback hub `ask_agent` calls come back through
    let roster = Arc::new(crate::relay::roster::Roster::new(&root));
    let hub = match crate::relay::hub::RelayHub::start(roster.clone(), log.clone()).await {
      Ok(h) => Some(h),
      Err(e) => {
        log(&format!("relay: listener failed, ask_agent unavailable: {e}"));
        None
      }
    };
    let host_mcp = bridge_exe.as_deref().map(|exe| {
      let m = crate::host_mcp::HostMcp::new(exe);
      match &hub {
        Some(h) => m.with_hub(h.clone()),
        None => m,
      }
    });
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
    let (r1, r2, r3, r4, r5, r6) = (r(&platform), r(&platform), r(&platform), r(&platform), r(&platform), r(&platform));
    let cwd_p = platform.clone();
    let mcp_home = platform.clone();
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
        plan_auto_approve: Arc::new(move || id_list(&r6("planAutoApprove").unwrap_or(Value::Null))),
        hidden: Arc::new(move || {
          r4("hiddenOptions").filter(is_hidden_map).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
        }),
        scope: Arc::new(move || match r5("sessionScope").and_then(|v| v.as_str().map(str::to_owned)).as_deref() {
          Some("all") => "all".into(),
          _ => "workspace".into(),
        }),
        lease_root: root.clone(),
        shared_mcp: Some(crate::shared_config::mcp_provider(Arc::new(move || mcp_home.home()))),
        host_mcp,
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
      shared: crate::shared_config::SharedConfig::new(root.clone(), log.clone()),
      agent_config_path: agent_config.path().to_owned(),
      roster,
      providers,
    }));

    let runtime = Arc::new(HostRuntime {
      manager,
      settings,
      sessions_dir,
      platform: platform.clone(),
      accounts,
      agent_config,
      self_agent,
      config_watch: Default::default(),
      bridges: Default::default(),
    });
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
        let _ = rt.reload_agent_config().await;
        rt.manager.reprobe().await;
        rt.manager.refresh_index().await;
        rt.accounts.reload().await;
        rt.accounts.sync_local(None).await;
        rt.maintain_shared().await;
      });
    }));
    tokio::spawn(crate::model_catalog::refresh(root.clone(), log.clone()));
    runtime.manager.init().await;
    // Files may be edited from any IDE or terminal, including on volumes without reliable native watch events.
    // Keep only a weak owner between ticks and stop the task explicitly before session teardown.
    let weak = Arc::downgrade(&runtime);
    *runtime.config_watch.lock() = Some(tokio::spawn(async move {
      let mut last_error = None;
      let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
      loop {
        tick.tick().await;
        let Some(rt) = weak.upgrade() else { break };
        match rt.reload_agent_config().await {
          Ok(()) => last_error = None,
          Err(e) => {
            let error = format!("{}: {e}", rt.agent_config.path().display());
            if last_error.as_ref() != Some(&error) {
              rt.platform.log(&error);
              rt.platform.toast("error", &error);
              last_error = Some(error);
            }
          }
        }
      }
    }));
    set_host_locale(runtime.settings.locale());
    runtime.apply_cpu_cap();
    crate::net_proxy::set_setting(&runtime.settings.view().proxy);
    // Whether npm's global prefix is writable decides the copyable install line of the npm agents (installer.rs)
    let rt = runtime.clone();
    tokio::spawn(async move {
      if crate::acp::agents::installer::probe_npm_prefix(crate::platform::command::Os::current()).await == Some(true) {
        rt.manager.emit_agents();
      }
    });
    let rt = runtime.clone();
    tokio::spawn(async move { rt.maintain_shared().await });
    Ok(runtime)
  }

  /// The `agentCpuCap` setting onto the job every agent process runs in (Windows only; elsewhere the setting is inert)
  fn apply_cpu_cap(&self) {
    #[cfg(windows)]
    crate::platform::windows_process::set_cpu_cap(self.settings.view().agent_cpu_cap as u32);
  }

  async fn reload_agent_config(&self) -> Result<()> {
    if self.agent_config.reload().await? {
      self.manager.set_registry(Arc::new(registry_of(&self.agent_config.snapshot(), self.self_agent.as_ref())));
    }
    Ok(())
  }

  /// Claude's project skill links, and user-level links once the link panel turned `auto` on (see `shared_config`)
  async fn maintain_shared(&self) {
    if let Err(e) = self.settings.shared_maintain().await {
      self.platform.log(&format!("shared config: {e:#}"));
    }
  }

  /// Whether some agent's credential store (the macOS keychain) is locked for this engine right now, checked afresh
  pub async fn credentials_locked(&self) -> bool {
    self.accounts.check_locks(None).await;
    self.accounts.any_credentials_locked()
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
    // Agent definitions belong to agents.json. IDE settings changes must never replace this registry.
    if affects(Some("hiddenOptions")) {
      self.manager.emit_hidden();
    }
    if affects(Some("agentOrder")) || affects(Some("disabledAgents")) {
      self.manager.emit_agents();
    }
    if affects(Some("agentCpuCap")) {
      self.apply_cpu_cap();
    }
    if affects(Some("proxy")) {
      // Agents started from now on take the new route; running ones keep the environment they were spawned with
      crate::net_proxy::set_setting(&self.settings.view().proxy);
      self.manager.invalidate_warm_pool();
    }
    if affects(Some("disabledAgents")) {
      // A turned-off agent's links go away, a turned-on one's come back
      let rt = self.clone();
      tokio::spawn(async move { rt.maintain_shared().await });
    }
    // Checked per key: one event may carry an appearance axis and a language change together
    if SETTING_KEYS.iter().any(|k| affects(Some(k))) {
      self.settings.emit();
      set_host_locale(self.settings.locale());
    }
  }

  pub async fn dispose(&self) {
    if let Some(task) = self.config_watch.lock().take() {
      task.abort();
    }
    let bridges: Vec<_> = self.bridges.lock().drain(..).collect();
    for b in bridges {
      b.dispose();
    }
    self.manager.dispose().await;
  }
}

/// The registry for an agents.json snapshot, with the built-in agent first when the engine binary is known
fn registry_of(custom: &Value, self_agent: Option<&(String, PathBuf)>) -> AgentRegistry {
  let registry = AgentRegistry::new(custom);
  match self_agent {
    Some((exe, root)) => registry.with_self_agent(exe, root),
    None => registry,
  }
}
