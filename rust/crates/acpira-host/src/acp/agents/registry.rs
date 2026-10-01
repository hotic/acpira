//! Agent definitions and executable discovery

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;

use acpira_shared::transcript::{AgentId, AgentInfo, AgentInstall, SessionOption, StrMap};

use crate::acp::agents::launch::{Env, Os, ProcessEnv, resolve_executable};
use crate::acp::agents::native_release::{self, NativeRelease};
use crate::i18n::tp;
use crate::store::data_dir::home_dir;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstallDef {
  pub posix: Option<String>,
  pub windows: Option<String>,
  pub docs: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdapterEngine {
  pub package: String,
  pub name: String,
  pub override_env: String,
  /// Where the engine keeps its per-platform native binary, when it ships one as an npm optional dependency
  pub native: Option<NativeLayout>,
}

/// Per-platform native binaries delivered as npm optional dependencies: an install run with `omit=optional` in its npm
/// config, or one whose optional download failed quietly (a proxy), leaves the engine without it and the agent fails
/// its first session with "Internal error"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeLayout {
  /// `@anthropic-ai/claude-agent-sdk-<platform>-<arch>[-musl]/claude[.exe]`, resolved from the SDK (claude-agent-acp 0.83)
  ClaudeSdk,
  /// `@openai/codex-<platform>-<arch>/vendor/<triple>/bin/codex[.exe]`, else the engine's own `vendor/` (codex 0.155 `bin/codex.js`)
  CodexVendor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdapterDef {
  pub package: String,
  pub engine: Option<AdapterEngine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoginDef {
  pub command: String,
  pub args: Vec<String>,
}

/// How an ACP agent is launched and which protocol quirks the host papers over
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentDef {
  pub id: AgentId,
  pub name: String,
  pub command: String,
  pub args: Vec<String>,
  pub candidates: Vec<String>,
  pub login: Option<LoginDef>,
  pub install: Option<InstallDef>,
  pub adapter: Option<AdapterDef>,
  pub env: Option<StrMap>,
  /// Modes the protocol doesn't advertise but the CLI supports; descriptions may be i18n keys
  pub modes: Option<Vec<SessionOption>>,
  pub images_regardless: bool,
  pub requires: Vec<String>,
  pub ignore_modes: bool,
  /// false opts out of the subagent capability advertisement (default on)
  pub subagents: bool,
  /// false opts out of the terminal-auth capability (default on)
  pub terminal_auth: bool,
  /// A native release archive installed under `$ACPIRA_HOME/agents/<id>/` (`acpira install-agent <id>`): looked up there
  /// first, spawned by its real path (the launcher finds its helpers next to it) in a process group of its own, so its
  /// helper processes end with it
  pub release: Option<&'static NativeRelease>,
}

fn s(v: &str) -> String {
  v.to_owned()
}

fn list(v: &[&str]) -> Vec<String> {
  v.iter().map(|x| s(x)).collect()
}

fn install(posix: &str, windows: &str, docs: &str) -> Option<InstallDef> {
  Some(InstallDef { posix: Some(s(posix)), windows: Some(s(windows)), docs: Some(s(docs)) })
}

fn login(command: &str, args: &[&str]) -> Option<LoginDef> {
  Some(LoginDef { command: s(command), args: list(args) })
}

fn base(id: &str, name: &str, command: &str, args: &[&str], candidates: &[&str]) -> AgentDef {
  AgentDef {
    id: s(id),
    name: s(name),
    command: s(command),
    args: list(args),
    candidates: list(candidates),
    subagents: true,
    terminal_auth: true,
    ..Default::default()
  }
}

fn mode(id: &str, name: &str, description: &str) -> SessionOption {
  SessionOption { id: s(id), name: s(name), description: Some(s(description)), ..Default::default() }
}

/// The built-in agents, verified on real machines (per-version notes in docs/dev/agent-quirks.md)
pub fn builtin_agents() -> Vec<AgentDef> {
  vec![
    AgentDef {
      login: login("grok", &["login"]),
      install: install(
        "curl -fsSL https://x.ai/cli/install.sh | bash",
        "irm https://x.ai/cli/install.ps1 | iex",
        "https://docs.x.ai/build/overview",
      ),
      // Grok advertises promptCapabilities.image: false yet the model sees inline images
      images_regardless: true,
      // Grok gives no modes in session/new but accepts session/set_mode; yolo is host-side auto-approval
      modes: Some(vec![
        mode("default", "Agent", "mode.grok.default"),
        mode("plan", "Plan", "mode.grok.plan"),
        mode("yolo", "Auto accept", "mode.grok.yolo"),
      ]),
      ..base(
        "grok",
        "Grok Build",
        "grok",
        &["agent", "stdio"],
        &["~/.grok/bin/grok", "~/.local/bin/grok", "/opt/homebrew/bin/grok", "/usr/local/bin/grok"],
      )
    },
    AgentDef {
      login: login("devin", &["auth", "login"]),
      install: install(
        "curl -fsSL https://cli.devin.ai/install.sh | bash",
        "irm https://cli.devin.ai/install.ps1 | iex",
        "https://docs.devin.ai/cli",
      ),
      // ACP_BACKEND makes the account layer the sole source of credentials
      env: Some([(s("ACP_BACKEND"), s("windsurf"))].into_iter().collect()),
      // ...so a terminal login method must never be offered
      terminal_auth: false,
      ..base(
        "devin",
        "Devin",
        "devin",
        &["acp"],
        &[
          "~/.local/bin/devin",
          "/opt/homebrew/bin/devin",
          "/usr/local/bin/devin",
          "/Applications/Devin.app/Contents/Resources/app/extensions/windsurf/devin/bin/devin",
        ],
      )
    },
    AgentDef {
      login: login("kimi", &[]),
      install: install(
        "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash",
        "irm https://code.kimi.com/kimi-code/install.ps1 | iex",
        "https://www.kimi.com/code/docs/en/kimi-code-cli/guides/getting-started.html",
      ),
      ..base(
        "kimi",
        "Kimi Code",
        "kimi",
        &["acp"],
        &["~/.local/bin/kimi", "~/.kimi-code/bin/kimi", "/opt/homebrew/bin/kimi", "/usr/local/bin/kimi"],
      )
    },
    AgentDef {
      requires: list(&["node"]),
      install: install(
        "npm install -g --include=optional @agentclientprotocol/codex-acp@1.13.0",
        "npm install -g --include=optional @agentclientprotocol/codex-acp@1.13.0",
        "https://github.com/agentclientprotocol/codex-acp",
      ),
      login: login("codex-acp", &["cli", "login"]),
      adapter: Some(AdapterDef {
        package: s("@agentclientprotocol/codex-acp"),
        engine: Some(AdapterEngine {
          package: s("@openai/codex"),
          name: s("Codex"),
          override_env: s("CODEX_PATH"),
          native: Some(NativeLayout::CodexVendor),
        }),
      }),
      ..base("codex", "Codex", "codex-acp", &[], &["~/.local/bin/codex-acp", "/opt/homebrew/bin/codex-acp", "/usr/local/bin/codex-acp"])
    },
    AgentDef {
      requires: list(&["node"]),
      login: login("claude-agent-acp", &["--cli", "auth", "login"]),
      install: install(
        "npm install -g --include=optional @agentclientprotocol/claude-agent-acp@0.84.0",
        "npm install -g --include=optional @agentclientprotocol/claude-agent-acp@0.84.0",
        "https://github.com/agentclientprotocol/claude-agent-acp",
      ),
      adapter: Some(AdapterDef {
        package: s("@agentclientprotocol/claude-agent-acp"),
        engine: Some(AdapterEngine {
          package: s("@anthropic-ai/claude-agent-sdk"),
          name: s("Claude Agent SDK"),
          override_env: s("CLAUDE_CODE_EXECUTABLE"),
          native: Some(NativeLayout::ClaudeSdk),
        }),
      }),
      ..base(
        "claude",
        "Claude",
        "claude-agent-acp",
        &[],
        &["~/.local/bin/claude-agent-acp", "/opt/homebrew/bin/claude-agent-acp", "/usr/local/bin/claude-agent-acp"],
      )
    },
    AgentDef {
      login: login("opencode", &["auth", "login"]),
      install: install("curl -fsSL https://opencode.ai/install | bash", "npm install -g --include=optional opencode-ai", "https://opencode.ai/docs/acp/"),
      ..base(
        "opencode",
        "OpenCode",
        "opencode",
        &["acp"],
        &["/opt/homebrew/bin/opencode", "/usr/local/bin/opencode", "~/.opencode/bin/opencode", "~/.local/bin/opencode"],
      )
    },
    AgentDef {
      // No login command: credentials are saved through the Web UI
      login: login("dsh", &["web"]),
      install: install(
        "npm install -g @deepseek-ai/dsh",
        "npm install -g @deepseek-ai/dsh",
        "https://deepseekdocs.com/en/docs/guides/acp-automation-server",
      ),
      ..base("dsh", "DSH", "dsh", &["--profile", "acp"], &["~/.local/bin/dsh", "/opt/homebrew/bin/dsh", "/usr/local/bin/dsh"])
    },
    antigravity(native_release::current_platform().as_deref().unwrap_or("")),
    AgentDef {
      requires: list(&["pi"]),
      login: login("pi", &[]),
      install: install(
        "npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp",
        "npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp",
        "https://github.com/svkozak/pi-acp",
      ),
      // pi-acp duplicates its thinking levels as modes and as the thought_level option
      ignore_modes: true,
      ..base("pi", "Pi", "pi-acp", &[], &["~/.local/bin/pi-acp", "/opt/homebrew/bin/pi-acp", "/usr/local/bin/pi-acp"])
    },
  ]
}

/// Google's official Antigravity ACP server (agy_acp_server 1.2.1): a native archive per platform, its launcher and
/// arguments as the ACP Registry entry gives them; sign-in goes through ACP `authenticate` (a browser flow on the host)
pub fn antigravity(platform: &str) -> AgentDef {
  let r = &native_release::ANTIGRAVITY;
  let (cmd, args) = r.launch(platform);
  let args: Vec<&str> = args.iter().map(String::as_str).collect();
  AgentDef {
    release: Some(r),
    install: Some(InstallDef { posix: None, windows: None, docs: Some(s(r.docs)) }),
    ..base("antigravity", "Google Antigravity", cmd, &args, &[])
  }
}

/// Custom agents from the acpira.agents setting (id → definition fragment)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAgentSetting {
  pub name: Option<String>,
  #[serde(default)]
  pub command: String,
  pub args: Option<Vec<String>>,
  pub login: Option<String>,
  pub install: Option<CustomInstall>,
  pub env: Option<StrMap>,
  pub modes: Option<Vec<SessionOption>>,
  pub prompt: Option<CustomPrompt>,
  pub requires: Option<Vec<String>>,
  pub ignore_modes: Option<bool>,
  pub subagents: Option<bool>,
  pub terminal_auth: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CustomInstall {
  pub command: Option<String>,
  pub docs: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPrompt {
  pub images_regardless: Option<bool>,
}

pub type Listener = Arc<dyn Fn() + Send + Sync>;

pub struct AgentRegistry {
  order: Vec<AgentId>,
  defs: HashMap<AgentId, AgentDef>,
  os: Os,
  resolved: parking_lot::Mutex<HashMap<AgentId, String>>,
  missing_cmds: parking_lot::Mutex<HashMap<AgentId, Vec<String>>>,
  searched: parking_lot::Mutex<HashMap<AgentId, Vec<String>>>,
  probed: AtomicBool,
  listeners: parking_lot::Mutex<Vec<(u64, Listener)>>,
  next_listener: std::sync::atomic::AtomicU64,
}

impl AgentRegistry {
  /// `custom` is the raw acpira.agents setting; entries without a command are skipped
  pub fn new(custom: &Value) -> Self {
    Self::with_os(custom, Os::current())
  }

  pub fn with_os(custom: &Value, os: Os) -> Self {
    let mut order = vec![];
    let mut defs = HashMap::new();
    for d in builtin_agents() {
      order.push(d.id.clone());
      defs.insert(d.id.clone(), d);
    }
    for (id, raw) in custom.as_object().into_iter().flatten() {
      let Ok(c) = serde_json::from_value::<CustomAgentSetting>(raw.clone()) else { continue };
      if c.command.is_empty() {
        continue;
      }
      let login: Vec<String> = c.login.as_deref().map(|l| l.split_whitespace().map(str::to_owned).collect()).unwrap_or_default();
      let command = c.install.as_ref().and_then(|i| i.command.as_deref()).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
      let docs = c.install.as_ref().and_then(|i| i.docs.as_deref()).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
      let def = AgentDef {
        id: id.clone(),
        name: c.name.unwrap_or_else(|| id.clone()),
        command: c.command,
        args: c.args.unwrap_or_default(),
        candidates: vec![],
        env: c.env,
        modes: c.modes,
        images_regardless: c.prompt.and_then(|p| p.images_regardless).unwrap_or(false),
        requires: c.requires.unwrap_or_default(),
        subagents: c.subagents != Some(false),
        terminal_auth: c.terminal_auth != Some(false),
        ignore_modes: c.ignore_modes == Some(true),
        release: None,
        login: login.split_first().map(|(cmd, rest)| LoginDef { command: cmd.clone(), args: rest.to_vec() }),
        install: (command.is_some() || docs.is_some()).then(|| InstallDef { posix: command.clone(), windows: command, docs }),
        adapter: None,
      };
      if !defs.contains_key(id) {
        order.push(id.clone());
      }
      defs.insert(id.clone(), def);
    }
    AgentRegistry {
      order,
      defs,
      os,
      resolved: Default::default(),
      missing_cmds: Default::default(),
      searched: Default::default(),
      probed: AtomicBool::new(false),
      listeners: Default::default(),
      next_listener: Default::default(),
    }
  }

  pub fn ids(&self) -> &[AgentId] {
    &self.order
  }

  /// Only after a probe pass can `available` be claimed
  pub fn list(&self) -> Vec<AgentInfo> {
    let probed = self.probed.load(Ordering::Acquire);
    let resolved = self.resolved.lock();
    let missing = self.missing_cmds.lock();
    let searched = self.searched.lock();
    self
      .order
      .iter()
      .map(|id| {
        let d = &self.defs[id];
        let found = resolved.contains_key(id);
        AgentInfo {
          id: d.id.clone(),
          name: d.name.clone(),
          available: probed.then_some(found),
          missing: if probed && !found { missing.get(id).filter(|m| !m.is_empty()).cloned() } else { None },
          searched: if probed && !found { searched.get(id).filter(|m| !m.is_empty()).cloned() } else { None },
          install: self.install(id),
          ..Default::default()
        }
      })
      .collect()
  }

  /// The install line for this platform (plus docs); None when the definition offers nothing usable here. A native
  /// release installs through this executable's own `install-agent`
  pub fn install(&self, id: &str) -> Option<AgentInstall> {
    let agent = self.defs.get(id)?;
    let def = agent.install.as_ref()?;
    let command = match agent.release {
      Some(r) => native_release::current_platform()
        .filter(|p| r.asset(p).is_some())
        .and_then(|_| native_release::install_command(&agent.id, self.os == Os::Windows)),
      None if self.os == Os::Windows => def.windows.clone(),
      None => def.posix.clone(),
    };
    if command.is_none() && def.docs.is_none() {
      return None;
    }
    Some(AgentInstall { command, docs: def.docs.clone() })
  }

  pub fn subscribe(&self, f: Listener) -> u64 {
    let id = self.next_listener.fetch_add(1, Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    id
  }

  pub fn unsubscribe(&self, id: u64) {
    self.listeners.lock().retain(|(i, _)| *i != id);
  }

  /// Some agent still has no executable
  pub fn missing(&self) -> bool {
    let r = self.resolved.lock();
    self.order.iter().any(|id| !r.contains_key(id))
  }

  /// Locate every agent's executable; resolves to whether the available set changed since the previous pass
  pub async fn probe_all(&self) -> bool {
    let first = !self.probed.load(Ordering::Acquire);
    let before = self.snapshot();
    let futs: Vec<_> = self.order.iter().map(|id| self.locate(id)).collect();
    for f in futs {
      f.await;
    }
    self.probed.store(true, Ordering::Release);
    if first { false } else { self.settle(&before) }
  }

  pub fn get(&self, id: &str) -> Result<&AgentDef> {
    self.defs.get(id).ok_or_else(|| anyhow!(tp("host.unknownAgent", &[("id", id)])))
  }

  pub fn try_get(&self, id: &str) -> Option<&AgentDef> {
    self.defs.get(id)
  }

  /// Find the executable; a lookup that flips availability notifies subscribers like a full probe would
  pub async fn resolve_binary(&self, id: &str) -> Option<String> {
    let before = self.snapshot();
    let found = self.locate(id).await;
    if self.probed.load(Ordering::Acquire) {
      self.settle(&before);
    }
    found
  }

  async fn locate(&self, id: &str) -> Option<String> {
    let def = self.defs.get(id)?;
    // A managed install is read fresh on every pass: `current` moves when a new version lands
    let mut candidates = def.candidates.clone();
    if let Some(r) = def.release {
      candidates.insert(0, r.dir(&crate::store::data_dir::acpira_home()).join("<version>").join(&def.command).to_string_lossy().into_owned());
      if let Some(managed) = r.managed_binary() {
        candidates[0] = managed;
      }
    }
    let cached = self.resolved.lock().get(id).cloned();
    let found = match cached {
      Some(c) if def.release.is_none() && resolve_executable(&c, self.os, &ProcessEnv).await.is_some() => Some(c),
      _ => resolve_command(&def.command, &candidates, self.os, &ProcessEnv).await,
    };
    let mut missing = vec![];
    if found.is_none() {
      missing.push(def.command.clone());
      let dirs = search_dirs(&def.command, &candidates, self.os, &ProcessEnv);
      self.searched.lock().insert(id.to_owned(), dirs);
    }
    for req in &def.requires {
      if resolve_command(req, &[], self.os, &ProcessEnv).await.is_none() {
        missing.push(req.clone());
      }
    }
    match found {
      Some(f) if missing.is_empty() => {
        self.resolved.lock().insert(id.to_owned(), f.clone());
        self.missing_cmds.lock().remove(id);
        Some(f)
      }
      _ => {
        self.resolved.lock().remove(id);
        self.missing_cmds.lock().insert(id.to_owned(), missing);
        None
      }
    }
  }

  fn snapshot(&self) -> String {
    let r = self.resolved.lock();
    self.order.iter().filter(|id| r.contains_key(*id)).cloned().collect::<Vec<_>>().join("\0")
  }

  fn settle(&self, before: &str) -> bool {
    let changed = before != self.snapshot();
    if changed {
      let ls: Vec<Listener> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
      for f in ls {
        f();
      }
    }
    changed
  }
}

/// Global bin directories package managers use outside the usual PATH entries, tried last: an npm prefix set up to avoid
/// sudo, pnpm / bun / volta / yarn homes. They cover a CLI installed where no rc file (or an unread one) adds the directory
const FALLBACK_DIRS: &[&str] =
  &["~/.npm-global/bin", "~/.local/share/pnpm", "~/Library/pnpm", "~/.bun/bin", "~/.volta/bin", "~/.yarn/bin"];

/// Every path tried for `command`, in order: the definition's candidates, PATH (which includes what the login shell
/// adds), then `FALLBACK_DIRS` on POSIX; duplicates dropped
pub fn search_paths(command: &str, candidates: &[String], os: Os, env: &dyn Env) -> Vec<String> {
  if std::path::Path::new(command).is_absolute() {
    return vec![command.to_owned()];
  }
  let mut out: Vec<String> = candidates.iter().map(|c| expand_home(c)).collect();
  let sep = if os == Os::Windows { ';' } else { ':' };
  let path = env.get("PATH").unwrap_or_default();
  let mut dirs: Vec<String> = path.split(sep).filter(|d| !d.is_empty()).map(str::to_owned).collect();
  if os == Os::Posix {
    dirs.extend(FALLBACK_DIRS.iter().map(|d| expand_home(d)));
  }
  out.extend(dirs.iter().map(|d| std::path::Path::new(d).join(command).to_string_lossy().into_owned()));
  let mut seen = std::collections::HashSet::new();
  out.retain(|p| seen.insert(p.clone()));
  out
}

/// The directories `search_paths` covers, for the not-found hint
pub fn search_dirs(command: &str, candidates: &[String], os: Os, env: &dyn Env) -> Vec<String> {
  let mut seen = std::collections::HashSet::new();
  search_paths(command, candidates, os, env)
    .iter()
    .filter_map(|p| std::path::Path::new(p).parent().map(|d| d.to_string_lossy().into_owned()))
    .filter(|d| seen.insert(d.clone()))
    .collect()
}

pub async fn resolve_command(command: &str, candidates: &[String], os: Os, env: &dyn Env) -> Option<String> {
  for p in search_paths(command, candidates, os, env) {
    if let Some(hit) = resolve_executable(&p, os, env).await {
      return Some(hit);
    }
  }
  None
}

pub fn expand_home(p: &str) -> String {
  match p.strip_prefix("~/") {
    Some(rest) => home_dir().join(rest).to_string_lossy().into_owned(),
    None => p.to_owned(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn custom_agents_merge_after_builtins() {
    let r = AgentRegistry::with_os(
      &json!({ "mine": { "command": "my-acp", "login": "my-acp auth login", "install": { "docs": "https://x" }, "terminalAuth": false }, "bad": {} }),
      Os::Posix,
    );
    let ids: Vec<_> = r.list().into_iter().map(|a| a.id).collect();
    assert_eq!(ids.last().map(String::as_str), Some("mine"));
    assert!(!ids.contains(&"bad".to_owned()));
    let d = r.get("mine").unwrap();
    assert_eq!(d.login, Some(LoginDef { command: "my-acp".into(), args: vec!["auth".into(), "login".into()] }));
    assert!(!d.terminal_auth);
    assert_eq!(r.install("mine"), Some(AgentInstall { command: None, docs: Some("https://x".into()) }));
    assert_eq!(r.install("grok").unwrap().command.unwrap(), "curl -fsSL https://x.ai/cli/install.sh | bash");
    assert!(r.list()[0].available.is_none(), "not probed yet");
  }
}
