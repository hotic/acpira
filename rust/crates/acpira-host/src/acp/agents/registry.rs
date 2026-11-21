//! Agent definitions and executable discovery

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;

use acpira_shared::transcript::{AgentId, AgentInfo, AgentInstall, SessionOption, StrMap};

use crate::acp::agents::launch::{Env, ProcessEnv, resolve_executable};
use crate::platform::command::Os;
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
  /// A known CLI's label on `--version`; absent for adapters and arbitrary commands
  pub version_prefix: Option<&'static str>,
  pub env: Option<StrMap>,
  /// Modes the protocol doesn't advertise but the CLI supports; descriptions may be i18n keys
  pub modes: Option<Vec<SessionOption>>,
  /// Permission levels the CLI's modes mix with its working modes, shown as an approval select (`session/access.rs`)
  pub access: Option<Vec<AccessLevel>>,
  pub images_regardless: bool,
  pub requires: Vec<String>,
  pub ignore_modes: bool,
  /// false opts out of the subagent capability advertisement (default on)
  pub subagents: bool,
  /// false opts out of the terminal-auth capability (default on)
  pub terminal_auth: bool,
  /// Who makes the CLI and its mark (SVG path data, 24 x 24), for an agent the page knows neither of
  pub vendor: Option<String>,
  pub mark: Option<Vec<String>>,
  /// How it runs on this machine for a folder on an SSH host (`acp::local_run`); None runs it in the host's engine
  pub local_run: Option<crate::acp::local_run::LocalRun>,
}

/// One permission level of a CLI whose modes mix levels and working modes: the CLI mode it runs on, and whether the host
/// approves every request there because the CLI has no mode of its own for it
#[derive(Debug, Clone, PartialEq)]
pub struct AccessLevel {
  pub id: String,
  pub wire: String,
  pub host: bool,
}

/// Levels in order, as (id, CLI mode, host-approved). Ids are the built-in agent's (`APPROVAL_LEVELS`) plus `auto`, the
/// CLI's own judgement of what is safe
fn access(levels: &[(&str, &str, bool)]) -> Option<Vec<AccessLevel>> {
  Some(levels.iter().map(|(id, wire, host)| AccessLevel { id: s(id), wire: s(wire), host: *host }).collect())
}

/// Most paths and bytes per path a mark may carry
pub const MARK_MAX_PATHS: usize = 16;
pub const MARK_MAX_PATH_BYTES: usize = 8192;

/// A mark's path data as the page will draw it, or why it cannot be: only SVG path commands and numbers, so nothing
/// but a shape ever reaches the page
pub fn mark_paths(raw: &serde_json::Value) -> std::result::Result<Vec<String>, String> {
  let Some(items) = raw.as_array().filter(|a| !a.is_empty()) else { return Err("`mark` is a non-empty array of SVG path data".into()) };
  if items.len() > MARK_MAX_PATHS {
    return Err(format!("`mark` holds more than {MARK_MAX_PATHS} paths"));
  }
  items
    .iter()
    .map(|v| {
      let d = v.as_str().map(str::trim).filter(|d| !d.is_empty()).ok_or("`mark` paths are non-empty strings")?;
      if d.len() > MARK_MAX_PATH_BYTES {
        return Err(format!("a `mark` path is over {MARK_MAX_PATH_BYTES} bytes"));
      }
      if !d.chars().all(|c| c.is_ascii_digit() || " ,.-+eE\n\t".contains(c) || "MmLlHhVvCcSsQqTtAaZz".contains(c)) {
        return Err("a `mark` path holds characters other than SVG path commands and numbers".into());
      }
      Ok(d.to_owned())
    })
    .collect()
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
    local_run: crate::acp::local_run::builtin(id),
    ..Default::default()
  }
}

fn mode(id: &str, name: &str, description: &str) -> SessionOption {
  SessionOption { id: s(id), name: s(name), description: Some(s(description)), ..Default::default() }
}

/// The built-in agents, verified on real machines (per-version notes in docs/dev/agent-quirks.md). Listed in the order
/// every agent list shows before the user saves one (arrange_agents), after the built-in Acpira agent
pub fn builtin_agents() -> Vec<AgentDef> {
  vec![
    AgentDef {
      requires: list(&["node"]),
      install: install(
        "npm install -g --include=optional @agentclientprotocol/codex-acp@2.7.0",
        "npm install -g --include=optional @agentclientprotocol/codex-acp@2.7.0",
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
        "npm install -g --include=optional @agentclientprotocol/claude-agent-acp@0.108.0",
        "npm install -g --include=optional @agentclientprotocol/claude-agent-acp@0.108.0",
        "https://github.com/agentclientprotocol/claude-agent-acp",
      ),
      access: access(&[("ask", "default", false), ("auto-edit", "acceptEdits", false), ("auto", "auto", false), ("full", "bypassPermissions", false)]),
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
        "Claude Code",
        "claude-agent-acp",
        &[],
        &["~/.local/bin/claude-agent-acp", "/opt/homebrew/bin/claude-agent-acp", "/usr/local/bin/claude-agent-acp"],
      )
    },
    AgentDef {
      login: login("grok", &["login"]),
      version_prefix: Some("grok"),
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
      version_prefix: Some("devin"),
      install: install(
        "curl -fsSL https://cli.devin.ai/install.sh | bash",
        // Keep the displayed/copied command identical to the terminal action. Strip full-line comments before joining:
        // PowerShell's line comments would otherwise swallow the rest of the one-line command.
        &include_str!("install-devin.ps1")
          .lines()
          .map(str::trim)
          .filter(|line| !line.is_empty() && !line.starts_with('#'))
          .collect::<Vec<_>>()
          .join(" "),
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
      // Kimi Code 2.2.0 `kimi login` runs the device-code flow its ACP terminal-auth method names
      login: login("kimi", &["login"]),
      version_prefix: Some("kimi"),
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
      login: login("opencode", &["auth", "login"]),
      version_prefix: Some("opencode"),
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
      adapter: Some(AdapterDef { package: s("@deepseek-ai/dsh"), engine: None }),
      ..base("dsh", "DSH", "dsh", &["--profile", "acp"], &["~/.local/bin/dsh", "/opt/homebrew/bin/dsh", "/usr/local/bin/dsh"])
    },
    antigravity(),
    AgentDef {
      requires: list(&["pi"]),
      login: login("pi", &[]),
      install: install(
        "npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp",
        "npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp",
        "https://github.com/svkozak/pi-acp",
      ),
      // pi-acp runs the global `pi` the same line installs; its models come from that package, so both are tracked
      adapter: Some(AdapterDef {
        package: s("pi-acp"),
        engine: Some(AdapterEngine {
          package: s("@earendil-works/pi-coding-agent"),
          name: s("Pi"),
          override_env: s("PI_ACP_PI_COMMAND"),
          native: None,
        }),
      }),
      // pi-acp duplicates its thinking levels as modes and as the thought_level option
      ignore_modes: true,
      ..base("pi", "Pi", "pi-acp", &[], &["~/.local/bin/pi-acp", "/opt/homebrew/bin/pi-acp", "/usr/local/bin/pi-acp"])
    },
    AgentDef {
      // No login subcommand: the interactive `droid` signs in on first run (ACP also offers a device-pairing method)
      login: login("droid", &[]),
      version_prefix: Some("droid"),
      install: install(
        "curl -fsSL https://app.factory.ai/cli | sh",
        "irm https://app.factory.ai/cli/windows | iex",
        "https://docs.factory.com/ide-integrations",
      ),
      // `--output-format acp` as Factory's docs give it; the ACP Registry's `acp-daemon` variant lingers after its parent
      // exits (droid 1.6.0). The Factory desktop app bundles its own copy
      ..base(
        "droid",
        "Droid",
        "droid",
        &["exec", "--output-format", "acp"],
        &[
          "~/.local/bin/droid",
          "/opt/homebrew/bin/droid",
          "/usr/local/bin/droid",
          "/Applications/Factory.app/Contents/Resources/bin/droid",
        ],
      )
    },
    AgentDef {
      // Provider and model setup; ACP also offers it as a terminal auth method (`hermes acp --setup`)
      login: login("hermes", &["model"]),
      version_prefix: Some("Hermes Agent"),
      install: install(
        "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash",
        "iex (irm https://hermes-agent.nousresearch.com/install.ps1)",
        "https://hermes-agent.nousresearch.com/docs/user-guide/features/acp",
      ),
      ..base("hermes", "Hermes Agent", "hermes", &["acp"], &["~/.local/bin/hermes", "/opt/homebrew/bin/hermes", "/usr/local/bin/hermes"])
    },
  ]
}

/// Google's Antigravity CLI in its ACP mode (`agy acp`, agy 1.5.9), the server the Antigravity docs give for ACP
/// clients. It offers the Claude and GPT models the separate `agy_acp_server` package only gives a client allowlist, and
/// shares the CLI's own login and conversations. Sign-in is its `terminal-login` method (`agy acp --login`)
pub fn antigravity() -> AgentDef {
  AgentDef {
    login: login("agy", &["acp", "--login"]),
    version_prefix: Some("agy"),
    // agy 1.5.9 has no full-access mode over ACP (`--dangerously-skip-permissions` is a process flag): full access runs
    // in `accept-edits` with the host approving every command. Not `auto`: that mode never asks, runs commands in agy's
    // own sandbox (a repository's `.git` is read-only, so `git pull` / `commit` fail) and refuses a sandbox bypass itself
    access: access(&[("ask", "default", false), ("auto-edit", "accept-edits", false), ("auto", "auto", false), ("full", "accept-edits", true)]),
    install: install(
      "curl -fsSL https://antigravity.google/cli/install.sh | bash",
      "irm https://antigravity.google/cli/install.ps1 | iex",
      "https://antigravity.google/docs/ide/extensions/acp",
    ),
    ..base("antigravity", "Antigravity", "agy", &["acp"], &["~/.local/bin/agy", "/opt/homebrew/bin/agy", "/usr/local/bin/agy"])
  }
}

pub const SELF_AGENT_ID: &str = "acpira";

/// The built-in agent: no install, no login, launched as `<engine> agent --home <root>`
pub fn self_agent(exe: &str, root: &std::path::Path) -> AgentDef {
  AgentDef {
    // The agent has no subagents and no login of its own
    subagents: false,
    terminal_auth: false,
    ..base(SELF_AGENT_ID, "Acpira", exe, &["agent", "--home", &root.to_string_lossy()], &[])
  }
}

/// The error for an agent whose binary cannot be found. When that binary is the engine's own (the built-in agent, a
/// plugin's specialised agent), an IDE replaced the install under this running engine, and the cure is reloading the
/// window onto the new engine, not installing a CLI
pub fn not_found(def: &AgentDef) -> anyhow::Error {
  let own = std::env::current_exe().ok().is_some_and(|exe| is_own_binary(&def.command, &exe.to_string_lossy()));
  if own {
    anyhow!(crate::i18n::t("host.engineReplaced"))
  } else {
    anyhow!(tp("host.notFound", &[("command", &def.command), ("agent", &def.name)]))
  }
}

/// Linux reports a deleted executable as `<path> (deleted)`
fn is_own_binary(command: &str, exe: &str) -> bool {
  command == exe || exe.strip_suffix(" (deleted)") == Some(command)
}

/// Custom agents from the machine-local agents.json (id → definition fragment)
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
  pub vendor: Option<String>,
  /// Checked by `mark_paths`; an invalid mark is dropped, the agent kept
  pub mark: Option<Value>,
  /// Whether and how it runs on this machine for a folder on an SSH host (`local_run::from_setting`)
  pub local_run: Option<Value>,
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
  /// Agents whose installed adapter is older than the version their install line pins: that version
  updates: parking_lot::Mutex<HashMap<AgentId, String>>,
  probed: AtomicBool,
  listeners: parking_lot::Mutex<Vec<(u64, Listener)>>,
  next_listener: std::sync::atomic::AtomicU64,
}

/// `command` starts the built-in's `builtin` launcher: the same name, or a path to it (an npm shim's absolute path, its
/// `.cmd` on Windows). A path read as a different command dropped the adapter and with it the update offer
fn same_launcher(builtin: &str, command: &str) -> bool {
  let file = command.rsplit(['/', '\\']).next().unwrap_or(command);
  let stem = match file.rsplit_once('.') {
    Some((stem, ext)) if ["cmd", "exe", "bat", "ps1"].iter().any(|e| ext.eq_ignore_ascii_case(e)) => stem,
    _ => file,
  };
  stem == builtin
}

/// A built-in definition with an agents.json entry for the same launcher laid over it: fields the entry sets win, env
/// merges key by key. An install line naming the built-in's own adapter package is a stale copy of the pin (IDE settings
/// seeded agents.json with it, then the pin moved on), so the built-in line wins over it
fn overlay(mut b: AgentDef, c: CustomAgentSetting, login: Vec<String>, install: Option<String>, docs: Option<String>) -> AgentDef {
  // A path to the launcher runs that very file, as the entry asked
  b.command = c.command.trim().to_owned();
  if let Some(name) = c.name {
    b.name = name;
  }
  if let Some(args) = c.args {
    b.args = args;
  }
  if let Some((cmd, rest)) = login.split_first() {
    b.login = Some(LoginDef { command: cmd.clone(), args: rest.to_vec() });
  }
  let own_package = |line: &str| b.adapter.as_ref().is_some_and(|a| line.contains(a.package.as_str()));
  if let Some(line) = install.filter(|l| !own_package(l)) {
    let docs = docs.or_else(|| b.install.as_ref().and_then(|i| i.docs.clone()));
    b.install = Some(InstallDef { posix: Some(line.clone()), windows: Some(line), docs });
  }
  if let Some(env) = c.env {
    b.env.get_or_insert_with(Default::default).extend(env);
  }
  if c.modes.is_some() {
    b.modes = c.modes;
  }
  if let Some(v) = c.prompt.and_then(|p| p.images_regardless) {
    b.images_regardless = v;
  }
  if let Some(r) = c.requires {
    b.requires = r;
  }
  if let Some(v) = c.ignore_modes {
    b.ignore_modes = v;
  }
  if let Some(v) = c.subagents {
    b.subagents = v;
  }
  if let Some(v) = c.terminal_auth {
    b.terminal_auth = v;
  }
  if let Some(v) = c.vendor.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
    b.vendor = Some(v);
  }
  if let Some(m) = c.mark.as_ref().and_then(|m| mark_paths(m).ok()) {
    b.mark = Some(m);
  }
  if c.local_run.is_some() {
    b.local_run = crate::acp::local_run::from_setting(&b.id, &b.name, c.local_run.as_ref(), b.local_run.take());
  }
  b
}

impl AgentRegistry {
  /// `custom` is the agents.json object; entries without a command are skipped
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
      // Same id and launcher as a built-in: the entry only tunes it (env, args, flags), so the built-in's adapter,
      // lookup paths and pinned install line survive and an update stays offered
      if let Some(b) = defs.get(id).filter(|b| same_launcher(&b.command, c.command.trim())).cloned() {
        let def = overlay(b, c, login, command, docs);
        defs.insert(id.clone(), def);
        continue;
      }
      let name = c.name.unwrap_or_else(|| id.clone());
      // A custom entry under a built-in id runs that CLI: its way to an SSH host stays unless the entry says otherwise
      let local_run = crate::acp::local_run::from_setting(id, &name, c.local_run.as_ref(), defs.get(id).and_then(|b| b.local_run.clone()));
      let def = AgentDef {
        id: id.clone(),
        name,
        command: c.command,
        args: c.args.unwrap_or_default(),
        candidates: vec![],
        env: c.env,
        modes: c.modes,
        // A custom entry under a built-in id runs that CLI: its mode split stays
        access: defs.get(id).and_then(|b| b.access.clone()),
        images_regardless: c.prompt.and_then(|p| p.images_regardless).unwrap_or(false),
        requires: c.requires.unwrap_or_default(),
        subagents: c.subagents != Some(false),
        terminal_auth: c.terminal_auth != Some(false),
        ignore_modes: c.ignore_modes == Some(true),
        login: login.split_first().map(|(cmd, rest)| LoginDef { command: cmd.clone(), args: rest.to_vec() }),
        install: (command.is_some() || docs.is_some()).then(|| InstallDef { posix: command.clone(), windows: command, docs }),
        adapter: None,
        version_prefix: None,
        vendor: c.vendor.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()),
        mark: c.mark.as_ref().and_then(|m| mark_paths(m).ok()),
        local_run,
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
      updates: Default::default(),
      probed: AtomicBool::new(false),
      listeners: Default::default(),
      next_listener: Default::default(),
    }
  }

  /// Put the built-in agent (`acpira agent`, served by this very executable) first, unless agents.json defines that id
  /// itself. `exe` is the engine binary, `root` the data root it serves (passed as `--home`, so every launch path —
  /// warm pool, cold start, controls probe — reads the same definition)
  pub fn with_self_agent(mut self, exe: &str, root: &std::path::Path) -> Self {
    if self.defs.contains_key(SELF_AGENT_ID) {
      return self;
    }
    let def = self_agent(exe, root);
    self.order.insert(0, def.id.clone());
    self.defs.insert(def.id.clone(), def);
    self
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
    let updates = self.updates.lock();
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
          update: if found { updates.get(id).cloned() } else { None },
          vendor: d.vendor.clone(),
          mark: d.mark.clone(),
          ssh_local: Some(d.local_run.is_some()),
          ..Default::default()
        }
      })
      .collect()
  }

  /// The install line for this platform (plus docs); None when the definition offers nothing usable here
  pub fn install(&self, id: &str) -> Option<AgentInstall> {
    let agent = self.defs.get(id)?;
    let def = agent.install.as_ref()?;
    let command = if self.os == Os::Windows {
      def.windows.clone()
    } else if super::installer::npm_prefix_blocked() {
      // npm's global prefix is not writable here (a system Node on a shared server): the copyable line goes to ~/.local,
      // where the in-app install puts it too
      def.posix.as_deref().map(|l| if super::installer::is_npm_global(l) { super::installer::with_user_prefix(l) } else { l.to_owned() })
    } else {
      def.posix.clone()
    };
    // A newer adapter on npm than the pin: install (and update to) that one
    let targets: Vec<(&str, semver::Version)> = tracked(agent).into_iter().filter_map(|p| Some((p, target_for(agent, p)?))).collect();
    let command = command.map(|l| retarget(&l, &targets));
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
    // Ask npm in the background what each adapter's latest is; this pass offers what earlier lookups found
    for package in self.order.iter().filter_map(|id| self.defs.get(id)).flat_map(tracked) {
      super::npm_latest::kick(package, self.os);
    }
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

  /// Whether `id`'s executable was found on the last probe (`None` before the first probe finishes)
  pub fn available(&self, id: &str) -> Option<bool> {
    self.probed.load(Ordering::Acquire).then(|| self.resolved.lock().contains_key(id))
  }

  /// Find the executable; a lookup that flips availability notifies subscribers like a full probe would
  /// Drop the path `id` was last found at, so the next lookup searches anew: an install may have put a newer copy
  /// earlier in the search order while the old one stays where it was
  pub fn forget_resolved(&self, id: &str) {
    self.resolved.lock().remove(id);
  }

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
    // CLIs installed through rc files (an npm prefix, nvm, mise …) are only on the login shell's PATH: every lookup
    // waits for its first run (bounded), so the engine's startup does not have to
    super::login_path::ready_within_first_wait().await;
    let candidates = def.candidates.clone();
    let cached = self.resolved.lock().get(id).cloned();
    let found = match cached {
      Some(c) if resolve_executable(&c, self.os, &ProcessEnv).await.is_some() => Some(c),
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
        let update = update_for(def, &f).await;
        match update {
          Some(v) => self.updates.lock().insert(id.to_owned(), v),
          None => self.updates.lock().remove(id),
        };
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

  /// The available set plus pending updates: either changing notifies subscribers
  fn snapshot(&self) -> String {
    let r = self.resolved.lock();
    let u = self.updates.lock();
    let mut out = self.order.iter().filter(|id| r.contains_key(*id)).cloned().collect::<Vec<_>>().join("\0");
    for id in &self.order {
      if let Some(v) = u.get(id) {
        out.push_str(&format!("\n{id}@{v}"));
      }
    }
    out
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

/// How `line` names `package`: None when it does not install it, Some(None) unpinned (`pkg` installs npm's latest),
/// Some(Some(v)) pinned at `pkg@v`
fn line_version(line: &str, package: &str) -> Option<Option<semver::Version>> {
  let prefix = format!("{package}@");
  line.split_whitespace().find_map(|w| {
    if w == package {
      Some(None)
    } else {
      w.strip_prefix(&prefix).map(|v| semver::Version::parse(v).ok())
    }
  })
}

/// The npm packages an update covers: the adapter, plus its engine when the install line installs that too (Pi)
fn tracked(def: &AgentDef) -> Vec<&str> {
  let Some(a) = def.adapter.as_ref() else { return vec![] };
  let line = def.install.as_ref().and_then(|i| i.posix.as_deref()).unwrap_or_default();
  let mut out = vec![a.package.as_str()];
  out.extend(a.engine.as_ref().map(|e| e.package.as_str()).filter(|p| line_version(line, p).is_some()));
  out
}

/// The version an install or update of `package` goes to: npm's latest when newer than the pin, else the pin. An
/// unpinned line has only npm's latest; None until it is known, or when the line does not install `package`
fn target_for(def: &AgentDef, package: &str) -> Option<semver::Version> {
  let pinned = line_version(def.install.as_ref()?.posix.as_deref()?, package)?;
  let latest = super::npm_latest::cached(package);
  match (pinned, latest) {
    (Some(p), Some(l)) => Some(if l > p { l } else { p }),
    (p, l) => p.or(l),
  }
}

/// `line` with each of `targets` (package, version) pinned at that version, whether it named a version before or not
fn retarget(line: &str, targets: &[(&str, semver::Version)]) -> String {
  let word = |w: &str| {
    for (package, version) in targets {
      if w == *package || w.strip_prefix(*package).is_some_and(|r| r.starts_with('@')) {
        return format!("{package}@{version}");
      }
    }
    w.to_owned()
  };
  line.split(' ').map(word).collect::<Vec<_>>().join(" ")
}

/// The version to offer when a tracked package found next to `binary` is older than its target (the adapter's first);
/// a version that cannot be read offers nothing
async fn update_for(def: &AgentDef, binary: &str) -> Option<String> {
  let adapter = def.adapter.as_ref()?;
  for package in tracked(def) {
    let Some(target) = target_for(def, package) else { continue };
    let installed = if package == adapter.package {
      super::adapter_info::installed_version(binary, package).await
    } else {
      super::adapter_info::read_adapter_info(binary, def).await.and_then(|i| i.engine?.version)
    };
    let Some(installed) = installed.and_then(|v| semver::Version::parse(&v).ok()) else { continue };
    if installed < target {
      return Some(target.to_string());
    }
  }
  None
}

/// Global bin directories package managers use outside the usual PATH entries, tried last: an npm prefix set up to avoid
/// sudo, pnpm / bun / volta / yarn homes. They cover a CLI installed where no rc file (or an unread one) adds the directory
const FALLBACK_DIRS: &[&str] = &[
  "~/.npm-global/bin",
  // npm's user prefix of the in-app install when the global one is not writable (`installer::USER_PREFIX`)
  "~/.local/bin",
  "~/.local/share/pnpm",
  "~/Library/pnpm",
  "~/.bun/bin",
  "~/.volta/bin",
  "~/.yarn/bin",
];

fn is_absolute_for(path: &str, os: Os) -> bool {
  if os == Os::Windows {
    let b = path.as_bytes();
    let drive = b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    drive || path.starts_with(r"\\")
  } else {
    path.starts_with('/')
  }
}

fn uses_win_sep(dir: &str, os: Os) -> bool {
  os == Os::Windows && (cfg!(windows) || dir.contains('\\') || dir.as_bytes().get(1) == Some(&b':'))
}

fn join_for(dir: &str, leaf: &str, os: Os) -> String {
  if uses_win_sep(dir, os) {
    let base = dir.replace('/', "\\").trim_end_matches('\\').to_owned();
    let tail = leaf.replace('/', "\\").trim_start_matches('\\').to_owned();
    if base.is_empty() { tail } else { format!("{base}\\{tail}") }
  } else {
    let tail = if os == Os::Windows { leaf.replace('\\', "/") } else { leaf.to_owned() };
    std::path::Path::new(dir).join(tail).to_string_lossy().into_owned()
  }
}

fn parent_for(path: &str, os: Os) -> Option<String> {
  if uses_win_sep(path, os) {
    let norm = path.replace('/', "\\");
    let (dir, _) = norm.rsplit_once('\\')?;
    (!dir.is_empty()).then(|| dir.to_owned())
  } else {
    std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned())
  }
}

fn dedup_key(path: &str, os: Os) -> String {
  if os == Os::Windows {
    path.trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
  } else {
    path.trim_end_matches('/').to_owned()
  }
}

fn expand_home_for(p: &str, os: Os, env: &dyn Env) -> String {
  let Some(rest) = p.strip_prefix("~/") else { return p.to_owned() };
  if os == Os::Windows {
    let home = env.get("USERPROFILE").filter(|d| !d.is_empty()).unwrap_or_else(|| home_dir().to_string_lossy().into_owned());
    join_for(&home, rest, Os::Windows)
  } else {
    home_dir().join(rest).to_string_lossy().into_owned()
  }
}

/// Every path tried for `command`, in order: the definition's candidates, PATH (which includes what the login shell
/// adds), then platform-specific install directories; duplicates dropped
pub fn search_paths(command: &str, candidates: &[String], os: Os, env: &dyn Env) -> Vec<String> {
  if is_absolute_for(command, os) || std::path::Path::new(command).is_absolute() {
    return vec![command.to_owned()];
  }
  let mut out: Vec<String> = candidates
    .iter()
    // POSIX-only absolute candidates (/opt/homebrew/..., /usr/local/..., /Applications/...) never apply on Windows
    .filter(|c| os != Os::Windows || !c.starts_with('/'))
    .map(|c| expand_home_for(c, os, env))
    .collect();
  let sep = if os == Os::Windows { ';' } else { ':' };
  let path = env.get("PATH").unwrap_or_default();
  let mut dirs: Vec<String> = path.split(sep).filter(|d| !d.is_empty()).map(str::to_owned).collect();
  if os == Os::Posix {
    dirs.extend(FALLBACK_DIRS.iter().map(|d| expand_home(d)));
  } else {
    // Windows installers update User PATH and their child shell, not the running IDE's environment. Probe the official
    // install directories directly so a refresh finds a new install without restarting the IDE (Devin 3000.11.3;
    // Antigravity's install.ps1 uses %LOCALAPPDATA%\agy\bin; Hermes's directory is from its install docs)
    let cmd_lower = command.to_ascii_lowercase();
    let local = env.get("LOCALAPPDATA").filter(|d| !d.is_empty());
    let appdata = env.get("APPDATA").filter(|d| !d.is_empty());
    let profile = env.get("USERPROFILE").filter(|d| !d.is_empty());
    let sub: &[&str] = match cmd_lower.as_str() {
      "devin" => &["devin", "cli", "bin"],
      "hermes" => &["hermes", "bin"],
      "agy" => &["agy", "bin"],
      _ => &[],
    };
    if let Some(local) = local.as_deref()
      && !sub.is_empty()
    {
      dirs.push(sub.iter().fold(local.to_owned(), |p, s| join_for(&p, s, Os::Windows)));
    }
    if matches!(cmd_lower.as_str(), "node" | "npm") {
      if let Some(pf) = env.get("ProgramFiles").filter(|d| !d.is_empty()) {
        dirs.push(join_for(&pf, "nodejs", Os::Windows));
      }
      if let Some(local) = local.as_deref() {
        dirs.push(join_for(local, r"Programs\nodejs", Os::Windows));
      }
    }
    // Common Windows global bin folders for npm (including Acpira's private Node.js global prefix), pnpm, scoop, bun, volta, winget
    if let Some(appdata) = appdata.as_deref() {
      dirs.push(join_for(appdata, "npm", Os::Windows));
    }
    if let Some(local) = local.as_deref() {
      dirs.push(join_for(local, "pnpm", Os::Windows));
      dirs.push(join_for(local, r"Microsoft\WinGet\Links", Os::Windows));
    }
    if let Some(profile) = profile.as_deref() {
      dirs.push(join_for(profile, r"scoop\shims", Os::Windows));
      dirs.push(join_for(profile, r".bun\bin", Os::Windows));
      dirs.push(join_for(profile, r".volta\bin", Os::Windows));
    }
  }
  out.extend(dirs.iter().map(|d| join_for(d, command, os)));
  let mut seen = std::collections::HashSet::new();
  out.retain(|p| seen.insert(dedup_key(p, os)));
  out
}

/// The directories `search_paths` covers, for the not-found hint
pub fn search_dirs(command: &str, candidates: &[String], os: Os, env: &dyn Env) -> Vec<String> {
  let mut seen = std::collections::HashSet::new();
  search_paths(command, candidates, os, env)
    .iter()
    .filter_map(|p| parent_for(p, os))
    .filter(|d| seen.insert(dedup_key(d, os)))
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

  #[tokio::test]
  async fn a_newer_adapter_on_npm_is_offered_and_installed_over_the_pin() {
    let mut def = builtin_agents().into_iter().find(|d| d.id == "claude").unwrap();
    let package = "@acpira-test/npm-latest-adapter";
    def.adapter.as_mut().unwrap().package = package.into();
    let line = format!("npm install -g --include=optional {package}@1.2.0");
    def.install = Some(InstallDef { posix: Some(line.clone()), windows: Some(line), docs: None });
    assert_eq!(target_for(&def, package).unwrap().to_string(), "1.2.0", "nothing looked up: the pin");
    super::super::npm_latest::seed(package, "1.1.0");
    assert_eq!(target_for(&def, package).unwrap().to_string(), "1.2.0", "an older latest never undercuts the pin");
    super::super::npm_latest::seed(package, "1.5.0");
    let target = target_for(&def, package).unwrap();
    assert_eq!(target.to_string(), "1.5.0");
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("node_modules").join(package);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), format!(r#"{{"name":"{package}","version":"1.2.0"}}"#)).unwrap();
    let shim = t.path().join("claude-agent-acp.cmd");
    std::fs::write(&shim, "").unwrap();
    assert_eq!(update_for(&def, shim.to_str().unwrap()).await.as_deref(), Some("1.5.0"));
    assert_eq!(
      retarget("npm install -g --include=optional @acpira-test/npm-latest-adapter@1.2.0", &[(package, target)]),
      "npm install -g --include=optional @acpira-test/npm-latest-adapter@1.5.0"
    );
  }

  #[test]
  fn an_unpinned_line_follows_npm_and_pi_tracks_its_engine_too() {
    let r = AgentRegistry::with_os(&json!({}), Os::Posix);
    let pi = r.get("pi").unwrap();
    assert_eq!(tracked(pi), vec!["pi-acp", "@earendil-works/pi-coding-agent"]);
    assert_eq!(tracked(r.get("claude").unwrap()), vec!["@agentclientprotocol/claude-agent-acp"], "the SDK is not on claude's line");
    assert_eq!(target_for(pi, "pi-acp"), None, "unpinned and nothing looked up yet");
    let v = |s: &str| semver::Version::parse(s).unwrap();
    assert_eq!(
      retarget("npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp", &[("pi-acp", v("0.0.35")), ("@earendil-works/pi-coding-agent", v("1.5.1"))]),
      "npm install -g --ignore-scripts @earendil-works/pi-coding-agent@1.5.1 pi-acp@0.0.35"
    );
    assert!(v("0.2.0-rc.2") < v("0.2.2-rc.1"), "DSH ships release candidates as latest");
  }

  #[test]
  fn an_entry_for_a_built_in_launcher_tunes_it_and_keeps_the_pin() {
    let stale = "npm install -g --include=optional @agentclientprotocol/claude-agent-acp@0.1.0";
    let r = AgentRegistry::with_os(
      &json!({
        "claude": { "command": "claude-agent-acp", "args": [], "install": { "command": stale }, "env": { "TZ": "Asia/Tokyo" } },
        "codex": { "command": "my-codex", "install": { "command": "make install" } },
      }),
      Os::Posix,
    );
    let builtin = builtin_agents().into_iter().find(|d| d.id == "claude").unwrap();
    let claude = r.get("claude").unwrap();
    assert_eq!(claude.install, builtin.install, "a stale copy of the pinned line loses to the pin");
    assert!(claude.adapter.is_some() && claude.candidates == builtin.candidates && claude.login == builtin.login);
    assert_eq!(claude.env.as_ref().and_then(|e| e.get("TZ")).map(String::as_str), Some("Asia/Tokyo"));
    // Another launcher under a built-in id replaces the definition outright
    let codex = r.get("codex").unwrap();
    assert!(codex.adapter.is_none() && codex.candidates.is_empty());
    assert_eq!(r.install("codex").unwrap().command.as_deref(), Some("make install"));
  }

  #[test]
  fn a_path_to_a_built_in_launcher_still_overlays_it() {
    let shim = r"C:\Users\u\AppData\Roaming\npm\claude-agent-acp.cmd";
    let r = AgentRegistry::with_os(
      &json!({ "claude": { "command": shim, "login": format!("{shim} --cli auth login") }, "codex": { "command": "/opt/codex-acp-fork" } }),
      Os::Windows,
    );
    let claude = r.get("claude").unwrap();
    assert!(claude.adapter.is_some() && claude.install.is_some(), "adapter and install line survive, so the update stays offered");
    assert_eq!(claude.command, shim, "the entry's path is still what runs");
    assert!(r.get("codex").unwrap().adapter.is_none(), "a different file name is another launcher");
    assert!(same_launcher("codex-acp", "/usr/local/bin/codex-acp") && same_launcher("devin", r"D:\bin\devin.EXE"));
    assert!(!same_launcher("devin", r"D:\bin\devin-old.exe"));
  }

  #[test]
  fn a_custom_agent_carries_its_vendor_and_a_valid_mark_only() {
    let r = AgentRegistry::with_os(
      &json!({ "a": { "command": "a", "vendor": " Alibaba ", "mark": ["M3 3h18v18H3z"] }, "b": { "command": "b", "mark": ["url(x)"] } }),
      Os::Posix,
    );
    let list = r.list();
    let (a, b) = (list.iter().find(|x| x.id == "a").unwrap(), list.iter().find(|x| x.id == "b").unwrap());
    assert_eq!((a.vendor.as_deref(), a.mark.clone()), (Some("Alibaba"), Some(vec!["M3 3h18v18H3z".to_owned()])));
    assert_eq!(b.mark, None, "an invalid mark is dropped, the agent kept");
  }

  #[tokio::test]
  async fn an_adapter_older_than_the_pinned_install_offers_that_version() {
    let defs = builtin_agents();
    let claude = defs.iter().find(|d| d.id == "claude").unwrap();
    let package = &claude.adapter.as_ref().unwrap().package;
    let pinned = line_version(claude.install.as_ref().unwrap().posix.as_deref().unwrap(), package).flatten().expect("claude pins its adapter");
    assert_eq!(line_version("npm install -g @deepseek-ai/dsh", "@deepseek-ai/dsh"), Some(None), "an unpinned line");
    // A global npm layout read through a .cmd shim (the lookup branches on the extension, not the platform)
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("node_modules").join(package);
    std::fs::create_dir_all(&dir).unwrap();
    let shim = t.path().join("claude-agent-acp.cmd");
    std::fs::write(&shim, "").unwrap();
    let bin = shim.to_str().unwrap();
    for (installed, offered) in [("0.1.0", Some(pinned.to_string())), (pinned.to_string().as_str(), None), ("99.0.0", None)] {
      std::fs::write(dir.join("package.json"), format!(r#"{{"name":"{package}","version":"{installed}"}}"#)).unwrap();
      assert_eq!(update_for(claude, bin).await, offered, "installed {installed}");
    }
  }

  #[test]
  fn the_built_in_agent_comes_first_unless_agents_json_defines_it() {
    let root = std::path::Path::new("/data/acpira");
    let r = AgentRegistry::with_os(&json!({}), Os::Posix).with_self_agent("/x/acpira", root);
    assert_eq!(r.ids()[0], SELF_AGENT_ID);
    let d = r.get(SELF_AGENT_ID).unwrap();
    assert_eq!((d.command.as_str(), d.args.clone()), ("/x/acpira", vec!["agent".to_owned(), "--home".into(), "/data/acpira".into()]));
    let custom = AgentRegistry::with_os(&json!({ "acpira": { "command": "/y/other" } }), Os::Posix).with_self_agent("/x/acpira", root);
    assert_eq!(custom.get(SELF_AGENT_ID).unwrap().command, "/y/other");
    assert_ne!(custom.ids()[0], SELF_AGENT_ID);
  }

  #[test]
  fn windows_search_paths_drop_posix_candidates_and_cover_windows_bin_dirs() {
    struct Map(HashMap<String, String>);
    impl Env for Map {
      fn get(&self, k: &str) -> Option<String> {
        self.0.get(k).cloned()
      }
    }
    let env = Map(
      [
        ("PATH", r"C:\Windows\System32;c:\users\u\.local\bin"),
        ("USERPROFILE", r"C:\Users\u"),
        ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
        ("APPDATA", r"C:\Users\u\AppData\Roaming"),
        ("ProgramFiles", r"C:\Program Files"),
      ]
      .into_iter()
      .map(|(k, v)| (k.to_owned(), v.to_owned()))
      .collect(),
    );
    let agy = antigravity();
    let dirs = search_dirs(&agy.command, &agy.candidates, Os::Windows, &env);
    assert!(!dirs.iter().any(|d| d.starts_with('/')), "POSIX candidates must not leak on Windows: {dirs:?}");
    assert_eq!(dirs.iter().filter(|d| d.eq_ignore_ascii_case(r"C:\Users\u\.local\bin")).count(), 1, "case-insensitive dedup: {dirs:?}");
    for expected in [r"C:\Users\u\AppData\Local\agy\bin", r"C:\Users\u\AppData\Roaming\npm", r"C:\Users\u\scoop\shims"] {
      assert!(dirs.iter().any(|d| d.eq_ignore_ascii_case(expected)), "missing {expected} in {dirs:?}");
    }
    let npm_dirs = search_dirs("npm", &[], Os::Windows, &env);
    assert!(npm_dirs.iter().any(|d| d.eq_ignore_ascii_case(r"C:\Program Files\nodejs")), "{npm_dirs:?}");
    assert_eq!(search_paths(r"D:\tools\agy.exe", &agy.candidates, Os::Windows, &env), vec![r"D:\tools\agy.exe"]);
  }
}
