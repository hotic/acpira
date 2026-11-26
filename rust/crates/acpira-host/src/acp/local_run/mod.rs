//! Local run for a folder on an SSH host (docs/dev/remote.md): the agent, its login and its loop stay on this machine
//! and only its project tools run on the host. How an agent gets there is the `LocalRun` its definition carries
//! (`AgentDef::local_run`); nothing outside this module asks which CLI it is. A CLI in a stand-in folder is one
//! `StandProfile`: where it starts (`stand::Stand`), how the remote note reaches the model (`Brief`), how its native
//! project tools are stopped (`Gate`), what replaces them (`tools`, served by `acpira mcp`) and which of its own files
//! stay here (`OwnFiles`). Under every profile the engine also refuses a permission request of a native tool that
//! writes or runs a command (`stand::blocked_native`): the last line when a CLI update slips past its gate

pub mod brief;
pub mod mcp_http;
pub mod stand;
pub mod tools;

use std::sync::Arc;

use serde_json::Value;

/// How an agent runs on this machine for a folder on an SSH host
#[derive(Clone, Debug, PartialEq)]
pub enum LocalRun {
  /// The built-in agent: its tools come to the engine as `_acpira/remoteTool` (`acp/session/remote.rs`)
  Native,
  /// Codex: its own tools run in its exec-server on the host (`remote::codex_exec`); the session's folder is the host's path
  Exec,
  /// A CLI in a stand-in folder here, its native project tools stopped and Acpira's in their place
  Stand(Arc<StandProfile>),
}

impl LocalRun {
  pub fn stand(&self) -> Option<&Arc<StandProfile>> {
    match self {
      LocalRun::Stand(p) => Some(p),
      _ => None,
    }
  }
}

/// How the remote note (where the project is, the host's instructions) reaches the model
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Brief {
  /// claude-agent-acp: `systemPrompt: { mode: "append" }` on the session request
  ClaudeSystemPrompt,
  /// `agy acp`: `_meta.antigravity.systemPrompt.append` on the session request
  AntigravityMeta,
  /// Any other CLI: written into the stand-in as the instructions file it reads for a project (`AGENTS.md`)
  StandFile(String),
}

/// How a CLI's native project tools are stopped, beside the engine's own refusal of their permission requests
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
  /// claude-agent-acp: `_meta.claudeCode.options.disallowedTools`, with `toolAliases` onto Acpira's tools and the
  /// read-only ones in `allowedTools` (the tools are gone from the model's list)
  ClaudeOptions,
  /// Launch arguments that take the native tools out of the CLI's list
  Disable(Vec<String>),
  /// A `PreToolUse` hook in the stand-in that denies them through `acpira tool-guard <agent>`
  Hook(HookFormat),
  /// Environment whose configuration takes the native tools out of the CLI's list
  Env(Vec<(String, String)>),
}

/// Where a CLI reads a `PreToolUse` hook in its project folder, and how it talks to the hook
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookFormat {
  /// `agy`'s `.agents/hooks.json`: the call in `toolCall.{name,args}`, a JSON `decision: deny` on stdout
  Agents,
  /// Claude Code's hook format in the named project file: the call in `tool_name` / `tool_input`, a deny as exit code 2
  /// with the reason on stderr and as `hookSpecificOutput.permissionDecision` on stdout
  Claude(String),
}

/// The CLI's own files on this machine, which its tools reach for and must find here
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnFiles {
  /// Folders under the home folder (`.claude`)
  pub dirs: Vec<String>,
  /// Variables naming another such folder (`CLAUDE_CONFIG_DIR`)
  pub env_dirs: Vec<String>,
  /// Prefixes of its temporary files (`/tmp/claude-`)
  pub temp_prefixes: Vec<String>,
  /// Folders under those where it writes without a permission card of its own (plans, artifacts)
  pub quiet_writes: Vec<String>,
}

/// The CLI's built-in tools that would work on this machine instead of the host
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeTools {
  /// Commands, searches, listings, moves: always refused
  pub shell: Vec<String>,
  /// File reads and writes: refused unless every path they name is one of the CLI's own files
  pub files: Vec<String>,
}

/// One CLI in a stand-in folder
#[derive(Clone, Debug, PartialEq)]
pub struct StandProfile {
  /// The registry id the hook guard looks the profile up by (`builtin`)
  pub agent: String,
  /// The CLI's name in logs
  pub label: String,
  /// The stand-in's folder under `<data root>/remote/`
  pub dir: String,
  /// How the CLI names a tool of Acpira's MCP server (`mcp__acpira__Read`)
  pub prefix: String,
  /// The project instruction files the CLI reads, by preference: the first found in each folder goes into the note
  pub instructions: Vec<String>,
  pub brief: Brief,
  /// Strongest first; empty leaves only the engine's refusal of native permission requests
  pub gates: Vec<Gate>,
  /// Environment the CLI starts with in a stand-in
  pub env: Vec<(String, String)>,
  /// Files the stand-in holds for the CLI's start (a configuration its arguments name), by their path under it
  pub files: Vec<(String, String)>,
  /// The CLI asks before a remote tool writes or runs a command, so a call it did not ask about is one a rule of its own
  /// allowed. False has the engine ask for every such call the CLI did not (`AcpSession::ask_remote`): OpenCode and DSH
  /// call an MCP tool without a word
  pub own_cards: bool,
  pub own: OwnFiles,
  pub native: NativeTools,
  /// The note's last sentence: what stays on this machine and which tools to leave alone
  pub own_note: String,
}

/// How a CLI names a tool of Acpira's MCP server when its profile says nothing else: Claude Code's convention, which
/// most CLIs follow
pub const DEFAULT_PREFIX: &str = "mcp__acpira__";

impl StandProfile {
  /// Launch arguments added in a stand-in (`Gate::Disable`)
  pub fn args(&self) -> impl Iterator<Item = &str> {
    self.gates.iter().flat_map(|g| match g {
      Gate::Disable(args) => args.as_slice(),
      _ => &[],
    })
    .map(String::as_str)
  }

  /// Environment added in a stand-in: the profile's own, then its gate's (`Gate::Env`)
  pub fn envs(&self) -> impl Iterator<Item = &(String, String)> {
    self.env.iter().chain(self.gates.iter().flat_map(|g| match g {
      Gate::Env(env) => env.as_slice(),
      _ => &[],
    }))
  }

  /// The CLI starts differently in a stand-in: a warm process from elsewhere will not do
  pub fn launches(&self) -> bool {
    self.args().next().is_some() || self.envs().next().is_some()
  }

  pub fn hook(&self) -> Option<&HookFormat> {
    self.gates.iter().find_map(|g| match g {
      Gate::Hook(f) => Some(f),
      _ => None,
    })
  }

  /// Apply the stand-in's arguments and environment onto a command's args and env map.
  /// A variable holding a JSON object (OpenCode's inline configuration) merges over prior keys.
  pub fn apply_launch(&self, args: &mut Vec<String>, env_slot: &mut Option<acpira_shared::transcript::StrMap>) {
    args.extend(self.args().map(str::to_owned));
    for (key, value) in self.envs() {
      let env = env_slot.get_or_insert_with(Default::default);
      let prior = env.get(key).cloned().or_else(|| std::env::var(key).ok());
      env.insert(key.clone(), over_json(prior.as_deref(), value));
    }
  }
}

/// `value` laid over `prior` when both are JSON objects (objects merge, anything else is replaced); else `value`
fn over_json(prior: Option<&str>, value: &str) -> String {
  fn lay(base: &mut Value, over: Value) {
    match (base, over) {
      (Value::Object(b), Value::Object(o)) => {
        for (k, v) in o {
          lay(b.entry(k).or_insert(Value::Null), v);
        }
      }
      (b, o) => *b = o,
    }
  }
  let parse = |s: &str| serde_json::from_str::<Value>(s).ok().filter(Value::is_object);
  match (prior.and_then(parse), parse(value)) {
    (Some(mut base), Some(over)) => {
      lay(&mut base, over);
      base.to_string()
    }
    _ => value.to_owned(),
  }
}

fn strs(v: &[&str]) -> Vec<String> {
  v.iter().map(|s| (*s).to_owned()).collect()
}

/// Claude Code (claude-agent-acp 0.93.0 / Claude Code 2.1.300)
fn claude() -> StandProfile {
  StandProfile {
    agent: "claude".into(),
    label: "Claude Code".into(),
    // Kept empty (`docs/dev/remote.md`); the name stays, as the CLI keys its sessions by their folder
    dir: "hosts".into(),
    prefix: DEFAULT_PREFIX.into(),
    instructions: strs(&["CLAUDE.md", "AGENTS.md"]),
    brief: Brief::ClaudeSystemPrompt,
    gates: vec![Gate::ClaudeOptions],
    env: vec![],
    files: vec![],
    own_cards: true,
    own: OwnFiles {
      dirs: strs(&[".claude"]),
      env_dirs: strs(&["CLAUDE_CONFIG_DIR"]),
      temp_prefixes: strs(&["/tmp/claude-", "/private/tmp/claude-"]),
      quiet_writes: strs(&["plans"]),
    },
    native: NativeTools {
      // CLI 2.1.300: the deferred Monitor runs a command, the worktree tools a git checkout of the folder. Names an older
      // CLI lacks are ignored by it
      shell: strs(&["Bash", "BashOutput", "KillShell", "PowerShell", "Grep", "Glob", "LS", "LSP", "Monitor", "EnterWorktree", "ExitWorktree"]),
      files: strs(&["Read", "Write", "Edit", "MultiEdit", "NotebookEdit"]),
    },
    own_note: "Your own files on this machine (plans, memory, saved outputs) stay here, and the same tools reach them.".into(),
  }
}

/// Google's Antigravity CLI (`agy acp` 1.5.9): no ACP tool-disable list, so a `PreToolUse` hook guards its built-in tools
fn antigravity() -> StandProfile {
  StandProfile {
    agent: "antigravity".into(),
    label: "Antigravity".into(),
    dir: "antigravity".into(),
    prefix: "mcp_acpira_".into(),
    instructions: strs(&["GEMINI.md", "AGENTS.md"]),
    brief: Brief::AntigravityMeta,
    gates: vec![Gate::Hook(HookFormat::Agents)],
    env: vec![],
    files: vec![],
    own_cards: true,
    // A saved or plugin account keeps its own gemini dir, named to the CLI's process (and so to its hooks)
    own: OwnFiles {
      dirs: strs(&[".gemini"]),
      env_dirs: strs(&[crate::acp::vendors::antigravity::STORE_ENV]),
      temp_prefixes: vec![],
      quiet_writes: strs(&["antigravity-cli/brain", "antigravity/brain"]),
    },
    native: NativeTools {
      shell: strs(&[
        "run_command",
        "command_status",
        "send_command_input",
        "grep_search",
        "find",
        "find_by_name",
        "list_dir",
        "list_directory",
        "delete_directory",
        "move",
      ]),
      files: strs(&[
        "read_file",
        "view_file",
        "view_file_outline",
        "view_code_item",
        "write_file",
        "write_to_file",
        "edit_file",
        "replace_file_content",
        "multi_replace_file_content",
        "code_action",
      ]),
    },
    own_note: "Built-in local file, search and command tools (read_file, write_file, edit_file, grep_search, find_by_name, \
               list_dir, run_command) target this machine and are blocked for project files; always call the mcp_acpira_* \
               tools above instead. Your own artifacts, scratch files and skills under ~/.gemini stay on this machine."
      .into(),
  }
}

/// The note's last sentence for a CLI whose native project tools its gate stops
fn gated_note(prefix: &str, own: &str) -> String {
  format!(
    "Your own built-in file, search and command tools target this machine, where the project is not, and are off or \
     blocked for project files. Wherever your instructions name one of them (apply_patch included), call the {prefix}* \
     tool for that job instead: {prefix}Write and {prefix}Edit change files. Your own files under {own} stay on this \
     machine."
  )
}

/// Factory's Droid (`droid exec --output-format acp` 1.14.0): Claude Code's hook format in the project's
/// `.factory/settings.json`. `Script` stays: Droid calls an MCP tool it loaded late through it
fn droid() -> StandProfile {
  StandProfile {
    prefix: "acpira___".into(),
    instructions: strs(&["AGENTS.md", "CLAUDE.md"]),
    gates: vec![Gate::Hook(HookFormat::Claude(".factory/settings.json".into()))],
    own_cards: true,
    own: OwnFiles { dirs: strs(&[".factory"]), ..Default::default() },
    native: NativeTools { shell: strs(&["Execute", "Grep", "Glob", "LS"]), files: strs(&["Read", "Create", "Edit", "MultiEdit", "ApplyPatch"]) },
    own_note: gated_note("acpira___", "~/.factory"),
    ..generic("droid", "Droid")
  }
}

/// Grok Build (`grok agent stdio` 1.0.77): hook files in the project's `.grok/hooks/`, which like its instructions load
/// only in a trusted folder, so folder trust is off for the stand-in (it holds nothing but Acpira's files)
fn grok() -> StandProfile {
  StandProfile {
    prefix: "acpira__".into(),
    gates: vec![Gate::Hook(HookFormat::Claude(".grok/hooks/acpira.json".into()))],
    own_cards: true,
    env: vec![("GROK_FOLDER_TRUST".into(), "0".into())],
    own: OwnFiles { dirs: strs(&[".grok"]), ..Default::default() },
    native: NativeTools {
      shell: strs(&["run_terminal_command", "run_terminal_cmd", "grep", "hashline_grep", "list_dir", "glob", "find"]),
      files: strs(&["read_file", "hashline_read", "write", "search_replace", "hashline_edit", "apply_patch"]),
    },
    own_note: gated_note("acpira__", "~/.grok"),
    ..generic("grok", "Grok Build")
  }
}

/// OpenCode (`opencode acp` 1.18.15): its inline configuration denies the native tools, which takes them off the
/// model's list; it merges over the user's own configuration
fn opencode() -> StandProfile {
  let native = NativeTools { shell: strs(&["bash", "grep", "glob", "list"]), files: strs(&["read", "write", "edit", "multiedit", "patch", "apply_patch"]) };
  // `edit` is the permission of every writing tool; a path outside the folder asks under `external_directory`
  let deny: serde_json::Map<String, Value> =
    ["bash", "grep", "glob", "list", "read", "edit", "external_directory"].iter().map(|t| ((*t).to_owned(), Value::String("deny".into()))).collect();
  StandProfile {
    prefix: "acpira_".into(),
    gates: vec![Gate::Env(vec![("OPENCODE_CONFIG_CONTENT".into(), serde_json::json!({ "permission": deny }).to_string())])],
    own: OwnFiles { dirs: strs(&[".config/opencode", ".local/share/opencode"]), ..Default::default() },
    native,
    own_note: gated_note("acpira_", "~/.config/opencode"),
    ..generic("opencode", "OpenCode")
  }
}

/// Where DSH's patch lies in its stand-in
const DSH_PATCH: &str = ".acpira/dsh.patch.yml";

/// DeepSeek's DSH (`dsh --profile acp` 0.2.2): it never asks before a tool, so a patch overlay on its profile switches
/// the plugins of its file, search and command tools off
fn dsh() -> StandProfile {
  let plugins = ["tool-bash", "tool-pwsh", "tool-jobs", "tool-fs", "tool-fs-search"];
  StandProfile {
    gates: vec![Gate::Disable(strs(&["--patch", DSH_PATCH]))],
    files: vec![(DSH_PATCH.into(), plugins.iter().map(|id| format!("- id: {id}\n  disabled: true\n")).collect())],
    own: OwnFiles { dirs: strs(&[".dsh"]), ..Default::default() },
    native: NativeTools {
      shell: strs(&["bash", "pwsh", "grep", "glob", "job_output", "job_list", "job_kill"]),
      files: strs(&["read", "read_image", "write", "edit", "apply_patch"]),
    },
    own_note: gated_note(DEFAULT_PREFIX, "~/.dsh"),
    ..generic("dsh", "DSH")
  }
}

/// Any other ACP CLI (a custom agent): the note in the stand-in's `AGENTS.md`, no gate of its own, so only the engine's
/// refusal of native writes and commands holds, and only while the CLI asks before them
pub fn generic(agent: &str, label: &str) -> StandProfile {
  StandProfile {
    agent: agent.into(),
    label: label.into(),
    dir: format!("stand/{agent}"),
    prefix: DEFAULT_PREFIX.into(),
    instructions: strs(&["AGENTS.md"]),
    brief: Brief::StandFile("AGENTS.md".into()),
    gates: vec![],
    env: vec![],
    files: vec![],
    own_cards: false,
    own: OwnFiles::default(),
    native: NativeTools::default(),
    own_note: "Your own built-in file, search and command tools work on this machine, where the project is not: use them \
               only for your own files here, and call the tools above for the project. Writes and commands of your own \
               tools are refused."
      .into(),
  }
}

/// How a built-in agent runs here for a folder on an SSH host; None runs it in the host's engine. A profile is checked
/// against the real CLI (`scripts/probe/local-run-host.ts`): its own gate where the CLI offers one, else the proof that
/// it asks before a write or a command
pub fn builtin(agent: &str) -> Option<LocalRun> {
  match agent {
    acpira_shared::host_tools::SELF_AGENT_ID => Some(LocalRun::Native),
    "codex" => Some(LocalRun::Exec),
    "claude" => Some(LocalRun::Stand(Arc::new(claude()))),
    "antigravity" => Some(LocalRun::Stand(Arc::new(antigravity()))),
    "droid" => Some(LocalRun::Stand(Arc::new(droid()))),
    "grok" => Some(LocalRun::Stand(Arc::new(grok()))),
    "opencode" => Some(LocalRun::Stand(Arc::new(opencode()))),
    "dsh" => Some(LocalRun::Stand(Arc::new(dsh()))),
    // Asks before every write and command of its own, which the engine refuses (`stand::blocked_native`), and offers no
    // gate a project folder or a start could carry
    "devin" => Some(LocalRun::Stand(Arc::new(StandProfile { own_cards: true, ..generic("devin", "Devin") }))),
    _ => None,
  }
}

/// Built-in CLIs that cannot run here yet, whatever an agents.json entry of their id says: they run in the host's engine.
/// pi-acp 0.0.34 hands pi no MCP server, so the project tools never reach it. Hermes 0.22.5 asks only before a command
/// it rates dangerous and runs the rest in its local terminal; Kimi Code 2.2.0 keeps its tool switches in the user's own
/// configuration: neither offers a gate a stand-in or a start could carry
fn withheld(agent: &str) -> bool {
  matches!(agent, "pi" | "hermes" | "kimi")
}

/// A custom agent's `localRun` (agents.json): `false` runs it in the host's engine, anything else in a stand-in on the
/// generic profile, which an object tunes (`prefix`, `instructions`). Unset counts as on, so every ACP agent can run
/// here but a `withheld` one. `inherited`, the built-in of the same id's (falling back to `builtin(agent)` so `acpira`
/// keeps `LocalRun::Native` even when `agents.json` defines it before `with_self_agent`), wins over anything but
/// `false`: its gate, its brief and its stand-in folder are what the real CLI was checked against
pub fn from_setting(agent: &str, label: &str, raw: Option<&Value>, inherited: Option<LocalRun>) -> Option<LocalRun> {
  let inherited = inherited.or_else(|| builtin(agent));
  let raw = match raw {
    Some(Value::Bool(false)) => return None,
    _ if withheld(agent) => return None,
    _ if inherited.is_some() => return inherited,
    Some(v) => v,
    None => &Value::Null,
  };
  let mut p = generic(agent, label);
  let str_of = |k: &str| raw.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
  if let Some(prefix) = str_of("prefix") {
    p.prefix = prefix.to_owned();
  }
  let files: Vec<String> = raw.get("instructions").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::trim).filter(|f| is_plain_name(f)).map(str::to_owned).collect();
  if !files.is_empty() {
    p.brief = Brief::StandFile(files[0].clone());
    p.instructions = files;
  }
  Some(LocalRun::Stand(Arc::new(p)))
}

/// A file name of the project folder itself: the brief is written there, so no folder or parent may sneak in
fn is_plain_name(f: &str) -> bool {
  !f.is_empty() && f != "." && f != ".." && !f.contains(['/', '\\']) && !f.contains('\0')
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn every_builtin_stand_in_has_a_gate_of_its_own() {
    for id in ["claude", "antigravity", "droid", "grok", "opencode", "dsh"] {
      let Some(LocalRun::Stand(p)) = builtin(id) else { panic!("{id} runs in a stand-in") };
      assert!(!p.gates.is_empty(), "{id} needs more than the permission refusal");
      assert_eq!(p.agent, id);
      assert!(!p.native.shell.is_empty() && !p.native.files.is_empty(), "{id} names its native tools");
    }
    assert_eq!(builtin("acpira"), Some(LocalRun::Native));
    assert_eq!(builtin("codex"), Some(LocalRun::Exec));
    // No gate of its own: the engine's refusal, with full access kept on the engine's side
    assert!(builtin("devin").and_then(|l| l.stand().cloned()).is_some_and(|p| p.gates.is_empty() && p.agent == "devin"));
    // No way to stop their native tools yet: not even an entry that turns local run on gets them here
    for id in ["pi", "hermes", "kimi"] {
      assert_eq!(builtin(id), None, "{id}");
      assert_eq!(from_setting(id, id, Some(&json!(true)), None), None, "{id}");
      assert_eq!(from_setting(id, id, None, None), None, "{id}");
    }
    // A gate in the environment or the arguments means a start of its own
    let launched = |id: &str| {
      let mut def = crate::acp::agents::registry::AgentDef::default();
      builtin(id).and_then(|l| l.stand().cloned()).unwrap().apply_launch(&mut def.args, &mut def.env);
      def
    };
    assert!(launched("opencode").env.unwrap()["OPENCODE_CONFIG_CONTENT"].contains(r#""bash":"deny""#));
    assert_eq!(launched("grok").env.unwrap()["GROK_FOLDER_TRUST"], "0");
    assert_eq!(launched("dsh").args, ["--patch", ".acpira/dsh.patch.yml"]);
    assert!(launched("droid").args.is_empty());
  }

  #[test]
  fn a_custom_agent_declares_local_run_or_inherits_it() {
    let inherited = builtin("claude");
    assert_eq!(from_setting("claude", "Claude", None, inherited.clone()), inherited);
    assert_eq!(from_setting("claude", "Claude", Some(&json!(false)), inherited.clone()), None);
    // Turning it on, or tuning it, keeps a built-in's own profile (Codex's exec-server too)
    assert_eq!(from_setting("claude", "Claude", Some(&json!(true)), inherited.clone()), inherited);
    assert_eq!(from_setting("claude", "Claude", Some(&json!({ "prefix": "x_" })), inherited.clone()), inherited);
    assert_eq!(from_setting("codex", "Codex", Some(&json!(true)), builtin("codex")), Some(LocalRun::Exec));
    // An agents.json entry for the built-in agent keeps LocalRun::Native unless localRun is false
    assert_eq!(from_setting("acpira", "Acpira", None, None), Some(LocalRun::Native));
    assert_eq!(from_setting("acpira", "Acpira", Some(&json!(true)), None), Some(LocalRun::Native));
    assert_eq!(from_setting("acpira", "Acpira", Some(&json!(false)), None), None);
    let Some(LocalRun::Stand(p)) = from_setting("mine", "Mine", Some(&json!(true)), None) else { panic!() };
    assert_eq!((p.dir.as_str(), p.prefix.as_str(), &p.brief), ("stand/mine", DEFAULT_PREFIX, &Brief::StandFile("AGENTS.md".into())));
    assert!(p.gates.is_empty());
    // Unset is on: any ACP agent can run here unless its entry says no
    assert_eq!(from_setting("mine", "Mine", None, None), Some(LocalRun::Stand(p)));
    assert_eq!(from_setting("mine", "Mine", Some(&json!(false)), None), None);
    let raw = json!({ "prefix": "acpira_", "instructions": ["../x", "MINE.md", "AGENTS.md"] });
    let Some(LocalRun::Stand(p)) = from_setting("mine", "Mine", Some(&raw), None) else { panic!() };
    assert_eq!(p.prefix, "acpira_");
    assert_eq!(p.instructions, strs(&["MINE.md", "AGENTS.md"]));
    assert_eq!(p.brief, Brief::StandFile("MINE.md".into()));
  }

  #[test]
  fn launch_arguments_and_environment_join_the_definition() {
    let mut p = generic("mine", "Mine");
    assert!(!p.launches());
    p.gates.push(Gate::Disable(strs(&["--remove-tools", "Execute"])));
    p.env.push(("X".into(), "1".into()));
    let mut out = crate::acp::agents::registry::AgentDef { args: strs(&["acp"]), ..Default::default() };
    p.apply_launch(&mut out.args, &mut out.env);
    assert!(p.launches());
    assert_eq!(out.args, strs(&["acp", "--remove-tools", "Execute"]));
    assert_eq!(out.env.unwrap().get("X").map(String::as_str), Some("1"));
  }

  #[test]
  fn the_gates_inline_configuration_keeps_the_users_own() {
    let own = json!({ "model": "mine/m1", "provider": { "mine": { "npm": "x" } }, "permission": { "bash": { "git *": "allow" }, "webfetch": "ask" } });
    let mut out = crate::acp::agents::registry::AgentDef {
      env: Some([("OPENCODE_CONFIG_CONTENT".to_owned(), own.to_string())].into_iter().collect()),
      ..Default::default()
    };
    builtin("opencode").and_then(|l| l.stand().cloned()).unwrap().apply_launch(&mut out.args, &mut out.env);
    let v: Value = serde_json::from_str(&out.env.unwrap()["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!((&v["model"], &v["provider"]["mine"]["npm"]), (&json!("mine/m1"), &json!("x")));
    assert_eq!((&v["permission"]["bash"], &v["permission"]["edit"], &v["permission"]["webfetch"]), (&json!("deny"), &json!("deny"), &json!("ask")));
  }
}
