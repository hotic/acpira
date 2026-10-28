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
use crate::acp::agents::registry::{AgentRegistry, SELF_AGENT_ID};
use crate::acp::session::CompactionPolicy;
use crate::bridge_core::{BridgeCore, Post};
use crate::external::chatgpt_store::ChatGptBridgeStore;
use crate::i18n::{set_host_locale, tp};
use crate::session_manager::{Attention, AttentionKind, ManagerDeps, SessionManager};
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
  /// The pages' terminals (the desktop tool panel's terminal tabs), shared so one survives its page's reload
  terminals: Arc<crate::terminal::TerminalHub>,
  /// SSH hosts and their helper channels (docs/dev/remote.md)
  pub connections: Arc<crate::remote::connections::Connections>,
  /// The user's plugins (docs/dev/plugins.md)
  pub plugins: Arc<crate::plugins::host::PluginHost>,
  /// Remote access: the paired-devices gateway (docs/dev/paired-remote.md)
  pub remote_access: Arc<crate::devices::RemoteAccess>,
  /// The plugins' agents the registry was last built with (`registry_of`)
  plugin_agents: parking_lot::Mutex<serde_json::Map<String, Value>>,
}

impl HostRuntime {
  /// `root`: ACPIRA_HOME / ~/.acpira; `bridge_exe`: the executable the ChatGPT connection prompt names
  pub async fn create(platform: Arc<SidecarPlatform>, root: PathBuf, bridge_exe: Option<String>) -> Result<Arc<HostRuntime>> {
    // Shared settings first: everything below reads settings through the platform
    platform.use_shared(Arc::new(crate::store::shared_settings::SharedSettings::open(&root))).await;
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
    // Plugins: what state.json enables, loaded before the registry takes their agents and any session lists Acpira's
    // MCP tools
    let plugins = crate::plugins::host::PluginHost::new(
      root.join(crate::plugins::DIR),
      crate::plugins::host::PluginHost::safe_mode_from_env(),
      log.clone(),
    );
    // Resident plugin processes run in the engine only (not in `acpira plugin …`); each engine runs its own, for the
    // sessions it owns
    plugins.runtimes().enable_before_host();
    plugins.start().await;
    let plugin_agents = plugins.agent_entries();
    let self_agent = bridge_exe.clone().map(|exe| (exe, root.clone()));
    let registry = Arc::new(registry_of(&agent_config.snapshot(), self_agent.as_ref(), &plugin_agents));

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
    // `ssh <alias>` for a host key or a locked key runs in a plain terminal: no proxy variables, ssh has its own config
    let ssh_terminal: crate::accounts::account_manager::RunInTerminal = {
      let p = platform.clone();
      Arc::new(move |title, command, args, env| p.run_in_terminal(title, command, args, env))
    };
    let connections = crate::remote::connections::Connections::new(
      &root,
      std::path::Path::new(&platform.home()),
      log.clone(),
      Some(ssh_terminal),
      Default::default(),
    );
    let remote_access = crate::devices::RemoteAccess::new(
      &root,
      std::path::Path::new(&platform.home()),
      bridge_exe.as_deref().map(PathBuf::from),
      log.clone(),
    );
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
    // The built-in agent runs with this root as its home: a session deleted for good takes the agent's log along
    store.set_agent_home(root.clone());
    // Cross-harness subagents: the persona file and the loopback hub `ask_agent` calls come back through
    let roster = Arc::new(crate::relay::roster::Roster::new(&root));
    let hub = match crate::relay::hub::RelayHub::start(roster.clone(), log.clone()).await {
      Ok(h) => {
        // `open_browser` comes back through the same hub and is carried out by the serving shell
        h.set_browser(platform.clone());
        Some(h)
      }
      Err(e) => {
        log(&format!("relay: listener failed, ask_agent and open_browser unavailable: {e}"));
        None
      }
    };
    if let Some(h) = &hub {
      h.set_plugins(Arc::downgrade(&plugins));
    }
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
    let projects_p = platform.clone();
    let mcp_home = platform.clone();
    let plugin_servers: crate::shared_config::PluginServers = {
      let p = plugins.clone();
      Arc::new(move || p.mcp_servers())
    };
    let plugin_hooks: crate::hooks::PluginHooks = {
      let p = plugins.clone();
      Arc::new(move || p.hook_configs())
    };
    let plugin_prompts: crate::hooks::PluginPrompts = {
      let p = plugins.clone();
      Arc::new(move || p.prompt_layers())
    };
    let notify_p = platform.clone();
    let scope_p = platform.clone();
    let manager = SessionManager::new(
      registry,
      ManagerDeps {
        store,
        chatgpt: Some(chatgpt),
        log: log.clone(),
        cwd: Arc::new(move || cwd_p.cwd()),
        projects: Arc::new(move || projects_p.projects()),
        default_agent: Arc::new(move || r1("defaultAgent").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| SELF_AGENT_ID.into())),
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
        // A project shell has no workspace to scope to
        scope: Arc::new(move || match r5("sessionScope").and_then(|v| v.as_str().map(str::to_owned)).as_deref() {
          _ if scope_p.projects() => "all".into(),
          Some("all") => "all".into(),
          _ => "workspace".into(),
        }),
        lease_root: root.clone(),
        bus: true,
        remote: Some(connections.clone()),
        shared_mcp: Some(crate::shared_config::mcp_provider(Arc::new(move || mcp_home.home()), Some(plugin_servers))),
        plugin_hooks: Some(plugin_hooks),
        plugin_prompts: Some(plugin_prompts),
        plugin_runtime: Some(plugins.runtimes().clone()),
        host_mcp,
        // Both switches are off by default (`notifySound` / `notifySystem`); read at each edge, so a change applies to the next one
        attention: Some(Arc::new(move |a: Attention| {
          let on = |k: &str| notify_p.read_setting(k).and_then(|v| v.as_bool()).unwrap_or(false);
          let (sound, system) = (on("notifySound"), on("notifySystem"));
          if !sound && !system {
            return;
          }
          let key = match a.kind {
            AttentionKind::Waiting => "host.attention.waiting",
            AttentionKind::Done => "host.attention.done",
            AttentionKind::Error => "host.attention.error",
          };
          notify_p.attention(&a.session_id, &a.title, &tp(key, &[("agent", &a.agent)]), sound, system);
        })),
      },
    );
    *mgr_slot.lock() = Some(Arc::downgrade(&manager));
    // What a plugin's process may ask: sessions, page actions, toasts and one-off model calls (`plugins/host_api.rs`)
    let toast_p = platform.clone();
    plugins.runtimes().enable(Some(crate::plugins::host_api::host_call(&manager, Arc::new(move |level: &str, text: &str| toast_p.toast(level, text)), root.clone())));
    // A commit here tells the other engines at once; theirs reach this one the same way (the poll below otherwise)
    let bus = Arc::downgrade(&manager);
    plugins.set_touch(Arc::new(move || {
      if let Some(m) = bus.upgrade() {
        m.touch_others("plugins");
      }
    }));
    let reload = Arc::downgrade(&plugins);
    manager.on_bus_touch(
      "plugins",
      Arc::new(move || {
        if let Some(p) = reload.upgrade() {
          p.reload();
        }
      }),
    );
    // `search_sessions` / `read_session` read through the manager; weak like the slot above, since its sessions hold grants
    if let Some(h) = &hub {
      let library: std::sync::Weak<dyn crate::session_library::SessionLibrary> = Arc::downgrade(&manager) as _;
      h.set_library(library);
    }

    let (m1, m2, m3) = (manager.clone(), manager.clone(), manager.clone());
    let (pr, pw, ph, phome) = (platform.clone(), platform.clone(), platform.clone(), platform.clone());
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
      plugin_agents: parking_lot::Mutex::new(plugin_agents),
      config_watch: Default::default(),
      bridges: Default::default(),
      terminals: crate::terminal::TerminalHub::new(),
      connections,
      plugins,
      remote_access,
    });
    runtime.terminals.set_remote(runtime.connections.clone());
    let ra = runtime.remote_access.clone();
    tokio::spawn(async move { ra.refresh_installed().await });
    // A host's folders are the `ssh://` entries of the project list
    let weak = Arc::downgrade(&runtime.manager);
    runtime.connections.set_projects(Arc::new(move || weak.upgrade().and_then(|m| m.categories().projects).unwrap_or_default()));
    let weak = Arc::downgrade(&runtime);
    platform.on_settings_changed(Arc::new(move |affects| {
      if let Some(rt) = weak.upgrade() {
        rt.settings_changed(affects);
      }
    }));
    let weak = Arc::downgrade(&runtime);
    platform.on_window_focus(Arc::new(move || {
      let Some(rt) = weak.upgrade() else { return };
      // Back from a terminal where a host key was confirmed, or from sleep: hosts waiting out a backoff retry now
      rt.connections.kick();
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
    // Half-written files a killed or failed write left (`<file>.<pid>.tmp`); never user content, so no review needed
    let sweep_log = log.clone();
    let temp_dirs = vec![root.clone(), root.join("sessions")];
    tokio::spawn(async move {
      let removed = crate::store::file_lock::sweep_temp_files(&temp_dirs, crate::store::file_lock::STALE_TEMP_AGE).await;
      if removed > 0 {
        sweep_log(&format!("removed {removed} temp files left by interrupted writes"));
      }
    });
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
        // Another engine (another window, the desktop app) or a hand edit changed the shared settings
        rt.platform.reload_shared();
        // Or the plugin state (an engine off the bus, a hand edit): one metadata call when nothing changed
        rt.plugins.reload();
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
    let changed = self.agent_config.reload().await?;
    // The plugins' agents follow the same path: a plugin enabled or switched off rebuilds the registry
    let plugin_agents = self.plugins.agent_entries();
    let plugins_changed = *self.plugin_agents.lock() != plugin_agents;
    if changed || plugins_changed {
      self.manager.set_registry(Arc::new(registry_of(&self.agent_config.snapshot(), self.self_agent.as_ref(), &plugin_agents)));
      *self.plugin_agents.lock() = plugin_agents;
    }
    Ok(())
  }

  /// Claude's project skill links, and user-level links once the link panel turned `auto` on (see `shared_config`)
  async fn maintain_shared(&self) {
    // A folder on an SSH host gets no links written here: only the user's own
    let cwd = self.manager.folder_for(None, None).filter(|f| !crate::workspace::is_remote(f)).unwrap_or_else(|| self.platform.home());
    if let Err(e) = self.settings.shared_maintain(&cwd).await {
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
    let core = BridgeCore::new(
      self.manager.clone(),
      self.settings.clone(),
      self.platform.clone(),
      appearance,
      post,
      host,
      blob_base,
      initial,
      self.terminals.clone(),
      self.connections.clone(),
      self.plugins.clone(),
      self.remote_access.clone(),
    );
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
    self.terminals.dispose();
    self.connections.dispose();
    self.remote_access.dispose();
    self.manager.dispose().await;
    self.plugins.runtimes().stop_all().await;
  }
}

/// The registry for an agents.json snapshot plus the plugins' agents (an agents.json entry of the same id wins), with
/// the built-in agent first when the engine binary is known
fn registry_of(custom: &Value, self_agent: Option<&(String, PathBuf)>, plugin_agents: &serde_json::Map<String, Value>) -> AgentRegistry {
  let merged = if plugin_agents.is_empty() {
    custom.clone()
  } else {
    let mut all = serde_json::Map::new();
    for (id, entry) in plugin_agents {
      match entry.get(crate::plugins::host::PROFILE_KEY).and_then(Value::as_str) {
        // A specialised agent is this binary's own loop with the plugin's profile; without the binary there is none
        Some(dir) => {
          let Some((exe, root)) = self_agent else { continue };
          all.insert(id.clone(), serde_json::json!({
            "name": entry.get("name").cloned().unwrap_or_else(|| serde_json::json!(id)),
            "command": exe,
            "args": ["agent", "--home", root.to_string_lossy(), "--profile", dir],
            "subagents": false,
            "terminalAuth": false,
          }));
        }
        None => {
          all.insert(id.clone(), entry.clone());
        }
      }
    }
    for (id, v) in custom.as_object().into_iter().flatten() {
      all.insert(id.clone(), v.clone());
    }
    Value::Object(all)
  };
  let registry = AgentRegistry::new(&merged);
  match self_agent {
    Some((exe, root)) => registry.with_self_agent(exe, root),
    None => registry,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_plugin_agent_joins_the_registry_and_agents_json_wins_its_id() {
    let plugins = serde_json::json!({ "pal": { "command": "/p/run.sh", "name": "Pal" }, "mate": { "command": "/p/mate" } });
    let plugins = plugins.as_object().cloned().unwrap_or_default();
    let custom = serde_json::json!({ "pal": { "command": "/mine/pal", "name": "My Pal" } });
    let r = registry_of(&custom, None, &plugins);
    assert_eq!(r.try_get("pal").map(|d| d.command.as_str()), Some("/mine/pal"));
    assert_eq!(r.try_get("mate").map(|d| d.command.as_str()), Some("/p/mate"));
    assert!(registry_of(&custom, None, &serde_json::Map::new()).try_get("mate").is_none());
  }

  #[test]
  fn a_specialised_agent_is_the_engine_s_own_loop_with_its_profile_and_needs_the_engine_binary() {
    let plugins = serde_json::json!({ "ledger": { "name": "Ledger", crate::plugins::host::PROFILE_KEY: "/v/ledger/abc" } });
    let plugins = plugins.as_object().cloned().unwrap_or_default();
    let engine = ("/x/acpira".to_owned(), PathBuf::from("/data"));
    let r = registry_of(&serde_json::json!({}), Some(&engine), &plugins);
    let d = r.try_get("ledger").expect("the profile agent");
    assert_eq!((d.name.as_str(), d.command.as_str()), ("Ledger", "/x/acpira"));
    assert_eq!(d.args, ["agent", "--home", "/data", "--profile", "/v/ledger/abc"]);
    assert!(!d.subagents && !d.terminal_auth);
    assert!(registry_of(&serde_json::json!({}), None, &plugins).try_get("ledger").is_none());
  }
}
