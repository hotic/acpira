//! Workspace hooks: one `<root>/.agents/hooks.json` gates every agent the same way, whatever its CLI does with hooks of
//! its own. Acpira owns the parts of the loop every ACP agent shares — the prompt it is sent, the permission requests it
//! makes and the end of its turn — so the gate runs there instead of once per harness:
//!
//! - `context`: files sent along with a session's first prompt (the instructions the agent must have read);
//! - `beforePrompt`: asked before every prompt the user sends; its output rides along as extra context, a block keeps
//!   the prompt from being sent;
//! - `beforeEdit`: asked before an edit a permission request announces (deny → the request is rejected), and again at
//!   the end of the turn for every file the turn changed without asking;
//! - `afterTurn`: run when a turn ends; a block sends its reason back to the agent as an automatic follow-up, at most
//!   `rounds` times in a row.
//!
//! Scripts speak the Claude Code hook protocol (JSON on stdin; `permissionDecision: deny` / `decision: block` on stdout,
//! or exit code 2 with the reason on stderr), so a repository's existing hook scripts run unchanged

pub mod host;
pub mod run;
pub mod snapshot;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::inventory::parse_json_loose;
use crate::platform::paths::wire_path;
use crate::shared_config::project_root;

/// Where a project declares its hooks, relative to its root
pub const HOOKS_FILE: &str = ".agents/hooks.json";

/// Short: every send waits for it
const BEFORE_PROMPT_TIMEOUT: Duration = Duration::from_secs(10);
const BEFORE_EDIT_TIMEOUT: Duration = Duration::from_secs(30);
const AFTER_TURN_TIMEOUT: Duration = Duration::from_secs(120);
/// The longest timeout a hooks file may ask for; a larger one is cut to this (`Duration::from_secs_f64` panics past its
/// range, and a gate that waits longer holds the queue for good anyway)
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3600);
/// Automatic follow-ups one user prompt may trigger before the gate gives up and leaves the rest to the user
const DEFAULT_ROUNDS: u32 = 2;
/// A context file larger than this is cut (the agent can still read the rest itself)
pub const CONTEXT_MAX_BYTES: usize = 256 * 1024;

/// One hook command, run through the platform shell in the project root
#[derive(Debug, Clone, PartialEq)]
pub struct Hook {
  pub run: String,
  pub timeout: Duration,
}

/// A project's parsed hooks file
#[derive(Debug, Clone, PartialEq)]
pub struct HooksConfig {
  pub root: PathBuf,
  pub context: Vec<PathBuf>,
  pub before_prompt: Option<Hook>,
  pub before_edit: Option<Hook>,
  pub after_turn: Option<Hook>,
  pub rounds: u32,
  /// Git work trees whose changes count as the turn's edits (nested repositories the root's own status cannot see)
  pub watch: Vec<PathBuf>,
  /// Keys this version does not know (a newer Acpira's, or a typo): reported, while the known ones still apply
  pub unknown: Vec<String>,
  /// Set on a plugin's hooks: where they run and what their environment adds. None for the project's own
  pub plugin: Option<PluginSource>,
  /// The hooks of the plugins in effect, run after the project's own in this order (`sources`); only the project's
  /// `watch` and `rounds` count
  pub chained: Vec<HooksConfig>,
}

/// Where a plugin's hooks run: its frozen copy, with its folders in the environment
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSource {
  pub id: String,
  pub dir: PathBuf,
  pub env: Vec<(String, String)>,
}

/// The hooks of the plugins in effect, for a session on this machine (`plugins::host`)
pub type PluginHooks = std::sync::Arc<dyn Fn() -> Vec<HooksConfig> + Send + Sync>;

/// The prompt folders of the plugins in effect, for the built-in agent's sessions (`plugins::host`)
pub type PluginPrompts = std::sync::Arc<dyn Fn() -> Vec<PathBuf> + Send + Sync>;

impl HooksConfig {
  /// Whether turns need a before / after snapshot of the watched trees
  pub fn tracks_changes(&self) -> bool {
    self.sources().any(|s| s.before_edit.is_some() || s.after_turn.is_some())
  }

  /// The project's own hooks, then each plugin's: every event runs them in this order, and the first block decides
  pub fn sources(&self) -> impl Iterator<Item = &HooksConfig> {
    std::iter::once(self).chain(self.chained.iter())
  }

  /// No hooks of its own, for the project at `root`: what plugin hooks chain onto where the project declares none
  pub fn empty(root: &Path) -> HooksConfig {
    HooksConfig {
      root: root.to_path_buf(),
      context: vec![],
      before_prompt: None,
      before_edit: None,
      after_turn: None,
      rounds: DEFAULT_ROUNDS,
      watch: vec![root.to_path_buf()],
      unknown: vec![],
      plugin: None,
      chained: vec![],
    }
  }
}

/// A hook written as a bare command or as `{ run, timeout?, rounds? }` (timeout in seconds)
#[derive(Deserialize)]
#[serde(untagged)]
enum RawHook {
  Command(String),
  Full {
    run: String,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    rounds: Option<u32>,
  },
}

// Unknown keys are collected rather than rejected: a file written for a newer Acpira must not switch every gate off
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
  #[serde(default, rename = "$schema")]
  _schema: Option<String>,
  #[serde(default)]
  context: Vec<String>,
  #[serde(default)]
  before_prompt: Option<RawHook>,
  #[serde(default)]
  before_edit: Option<RawHook>,
  #[serde(default)]
  after_turn: Option<RawHook>,
  #[serde(default)]
  watch: Option<Vec<String>>,
  #[serde(flatten)]
  unknown: std::collections::BTreeMap<String, Value>,
}

fn hook_of(raw: Option<RawHook>, default_timeout: Duration) -> (Option<Hook>, Option<u32>) {
  match raw {
    None => (None, None),
    Some(RawHook::Command(run)) => (Some(Hook { run, timeout: default_timeout }).filter(|h| !h.run.trim().is_empty()), None),
    Some(RawHook::Full { run, timeout, rounds }) => {
      let timeout = timeout
        .filter(|t| t.is_finite() && *t > 0.0)
        .map(|t| Duration::from_secs_f64(t.min(MAX_TIMEOUT.as_secs_f64())))
        .unwrap_or(default_timeout);
      (Some(Hook { run, timeout }).filter(|h| !h.run.trim().is_empty()), rounds)
    }
  }
}

/// Parse a hooks file's text for the project at `root`
pub fn parse(root: &Path, text: &str) -> Result<HooksConfig, String> {
  let value: Value = parse_json_loose(text).ok_or_else(|| "not valid JSON".to_owned())?;
  let raw: RawConfig = serde_json::from_value(value).map_err(|e| e.to_string())?;
  let (before_prompt, _) = hook_of(raw.before_prompt, BEFORE_PROMPT_TIMEOUT);
  let (before_edit, _) = hook_of(raw.before_edit, BEFORE_EDIT_TIMEOUT);
  let (after_turn, rounds) = hook_of(raw.after_turn, AFTER_TURN_TIMEOUT);
  let join = |p: &String| root.join(p.trim_start_matches("./"));
  let watch = match raw.watch {
    Some(list) if !list.is_empty() => list.iter().map(join).collect(),
    _ => vec![root.to_path_buf()],
  };
  Ok(HooksConfig {
    root: root.to_path_buf(),
    context: raw.context.iter().filter(|p| !p.trim().is_empty()).map(join).collect(),
    before_prompt,
    before_edit,
    after_turn,
    rounds: rounds.unwrap_or(DEFAULT_ROUNDS),
    watch,
    unknown: raw.unknown.into_keys().collect(),
    plugin: None,
    chained: vec![],
  })
}

/// The hooks of the project `cwd` belongs to. None: the project declares none; Err: the file is there but unusable
/// (unreadable or not UTF-8 included: a gate the project asked for is never dropped without a word)
pub fn load(cwd: &str) -> Option<Result<HooksConfig, String>> {
  let root = project_root(cwd)?;
  let path = root.join(HOOKS_FILE);
  let text = match std::fs::read_to_string(&path) {
    Ok(text) => text,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
    Err(e) => return Some(Err(format!("{}: {e}", path.display()))),
  };
  Some(parse(&root, &text).map_err(|e| format!("{}: {e}", path.display())))
}

/// The file format as the `workspace_hooks` MCP tool explains it to a model asked to add or change a gate. Kept next
/// to the parser so the two are edited together
pub const GUIDE: &str = r#"# Acpira workspace hooks

`<project root>/.agents/hooks.json` (the root is the nearest ancestor holding `.git`) gates every agent the user runs
through Acpira the same way, whatever CLI it is. Acpira runs the scripts itself; the agents' own hook settings are not
involved. The file is JSON with comments and trailing commas allowed, and is read again before every prompt, so an edit
takes effect from the next turn. An unknown key (a typo, or a key of a newer Acpira) is reported to the user and
skipped; the keys this version knows still apply. Malformed JSON or a known key of the wrong type makes the whole file
invalid (also reported).

```jsonc
{
  // Files sent with the first prompt of every session (paths relative to the root)
  "context": ["AGENTS.md"],
  // Asked before every prompt the user sends (slash commands included). Its output is sent along with the prompt as
  // extra context; a block keeps the prompt from being sent and shows the user the reason
  "beforePrompt": "python3 tools/hooks/route_prompt.py",
  // Asked before an edit the agent requests permission for; deny = the request is rejected and the reason goes back
  // to the agent. Asked again when the turn ends for every file the turn changed without asking (phase "audit")
  "beforeEdit": "python3 tools/hooks/check_edit.py",
  // Run when a turn ends; a block sends the reason back to the agent as an automatic follow-up, at most `rounds`
  // times per user prompt (default 2), then the user is told the gate gave up
  "afterTurn": { "run": "python3 tools/hooks/stop_gate.py", "timeout": 90, "rounds": 1 },
  // Git work trees whose changes count as the turn's edits when it ran a shell command, a subagent or another tool
  // that can write anywhere (default: the root). List nested repositories the root's own `git status` cannot see
  "watch": [".", "vendor/server"]
}
```

A hook is a bare command string or `{ "run": "...", "timeout": seconds }` (`rounds` only on afterTurn). Defaults:
beforePrompt 10 s, beforeEdit 30 s, afterTurn 120 s; at most 3600 s. Commands run through `/bin/sh -c` (`cmd /c` on Windows) in the project root, with
`ACPIRA_PROJECT_DIR` and `CLAUDE_PROJECT_DIR` set to the root,
`ACPIRA_SESSION_ID` and `ACPIRA_AGENT` to the session's. A hook that times out, crashes or prints something
unreadable lets the prompt / edit / turn through and shows the user a warning.

## Input (JSON on stdin)

beforePrompt: `hook_event_name: "UserPromptSubmit"`, `session_id`, `agent`, `cwd`, `prompt` (the text the user sent),
`origin` (`"user"`, or `"goal"` for the prompt a session goal sends), `attachments` (what the prompt carries besides
its text: images, files, selections, quotes), `first` (true on the session's first prompt). Automatic prompts Acpira
sends itself (gate follow-ups, the continue after a Retry or an account switch, /compact before an over-budget prompt)
and messages steered into a running turn are not asked.

beforeEdit: `hook_event_name: "PreToolUse"`, `session_id`, `agent` (Acpira's agent id), `cwd` (the root),
`tool_name` (`Edit` | `Delete` | `Move`), `tool_input` (the tool's raw input plus `file_path`), `files` (absolute paths
of every target), `read_files` (absolute paths the session's tools have read), `phase` (`"permission"` | `"audit"`).
The audit leaves out files whose every edit already passed the hook in a permission request, and adds `turn_tools` and
`file_sources` (see afterTurn). A turn that ends without finishing (stopped, failed, out of quota) is not gated; its
changes are audited with the next turn that finishes.

afterTurn: `hook_event_name: "Stop"`, `session_id`, `agent`, `cwd`, `stop_hook_active` (true on a follow-up round),
`round`, `turn_files` (changed by this turn), `changed_files` (changed by the session so far), `read_files`,
`turn_tools` (the turn's tool calls: `kind`, `status`, `locations`), `file_sources` (each changed file → `"tool"`, named
by an edit tool call, or `"snapshot"`, only seen changing in a watched work tree). Other people and agents may share the
work tree, so a turn that only read takes no files, and a turn whose only writes were edits naming their files takes
just those; the work trees' changes count only after a shell command, a subagent, an MCP or unknown tool, or an edit
naming no file. A script that wants to be stricter about `"snapshot"` files can look at `turn_tools` itself.

## Answer (the Claude Code hook protocol, so existing Claude Code hook scripts work unchanged)

- exit 0 with no output, or `{}`: pass;
- beforePrompt only: exit 0 with plain text, or with `{"hookSpecificOutput": {"additionalContext": "..."}}`, passes
  and sends that text to the agent after the prompt (not shown in the user's message; left out of a slash command,
  which must stay the prompt's only text);
- exit 2: block, stderr is the reason;
- exit 0 with JSON: `{"hookSpecificOutput": {"permissionDecision": "deny", "permissionDecisionReason": "..."}}`
  (beforeEdit) or `{"decision": "block", "reason": "..."}` (beforePrompt, afterTurn) blocks with that reason.

Write beforeEdit / afterTurn reasons for the agent: they are sent to it as they are, so say what is wrong and how to
fix it. A beforePrompt reason is shown to the user instead (the prompt never reaches the agent) next to the prompt's
text, so it can be changed and sent again. After writing
or changing the file, call `workspace_hooks` again to check that it parses."#;

/// What `workspace_hooks` reports about the project `cwd` belongs to, after the guide
pub fn describe(cwd: &str) -> String {
  let Some(root) = project_root(cwd) else {
    return format!("{cwd} is not inside a git repository, so it has no project root for `{HOOKS_FILE}`.");
  };
  let path = root.join(HOOKS_FILE);
  match load(cwd) {
    None => format!("This project ({}) has no `{HOOKS_FILE}` yet; create it at {}.", wire_path(&root), wire_path(&path)),
    Some(Err(e)) => format!("`{}` is invalid, so no gate runs until it is fixed: {e}", wire_path(&path)),
    Some(Ok(c)) => {
      let hook = |h: &Option<Hook>| h.as_ref().map_or("none".to_owned(), |h| format!("`{}` ({} s)", h.run, h.timeout.as_secs_f64()));
      let missing: Vec<String> = c.context.iter().filter(|p| !p.is_file()).map(|p| wire_path(p)).collect();
      let mut text = format!(
        "`{}` is valid. context: {} file(s); beforePrompt: {}; beforeEdit: {}; afterTurn: {}, rounds {}; watch: {}.",
        wire_path(&path),
        c.context.len(),
        hook(&c.before_prompt),
        hook(&c.before_edit),
        hook(&c.after_turn),
        c.rounds,
        c.watch.iter().map(|p| wire_path(p)).collect::<Vec<_>>().join(", "),
      );
      if !missing.is_empty() {
        text.push_str(&format!(" Context files that do not exist: {}.", missing.join(", ")));
      }
      if !c.unknown.is_empty() {
        text.push_str(&format!(" Unknown keys, skipped: {}.", c.unknown.join(", ")));
      }
      text
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bare_commands_take_the_default_timeouts_and_rounds() {
    let c = parse(Path::new("/w"), r#"{ "beforeEdit": "python3 a.py", "afterTurn": "python3 b.py" }"#).unwrap();
    assert_eq!(c.before_edit, Some(Hook { run: "python3 a.py".into(), timeout: BEFORE_EDIT_TIMEOUT }));
    assert_eq!(c.after_turn, Some(Hook { run: "python3 b.py".into(), timeout: AFTER_TURN_TIMEOUT }));
    assert_eq!(c.rounds, DEFAULT_ROUNDS);
    assert_eq!(c.watch, [PathBuf::from("/w")]);
    assert!(c.tracks_changes());
  }

  #[test]
  fn the_full_form_and_comments_are_read() {
    let text = r#"{
      // injected once per session
      "context": ["Sources/Server/AGENTS.md", ""],
      "afterTurn": { "run": "python3 gate.py", "timeout": 5, "rounds": 3 },
      "watch": [".", "./Sources/Server"],
    }"#;
    let c = parse(Path::new("/w"), text).unwrap();
    assert_eq!(c.context, [Path::new("/w").join("Sources/Server/AGENTS.md")]);
    assert_eq!(c.after_turn.as_ref().unwrap().timeout, Duration::from_secs(5));
    assert_eq!(c.rounds, 3);
    assert_eq!(c.watch, [Path::new("/w").join("."), Path::new("/w").join("Sources/Server")]);
    assert!(c.before_edit.is_none());
  }

  #[test]
  fn an_unknown_key_is_reported_while_the_known_ones_still_apply() {
    let c = parse(Path::new("/w"), r#"{ "afterturn": "x", "beforeEdit": "y", "$schema": "s" }"#).unwrap();
    assert_eq!(c.unknown, ["afterturn"]);
    assert_eq!(c.before_edit.unwrap().run, "y");
    assert!(c.after_turn.is_none());
    assert!(describe_unknown_mentions_it());
    // Broken JSON and a known key of the wrong type still invalidate the file
    assert!(parse(Path::new("/w"), "{").is_err());
    assert!(parse(Path::new("/w"), r#"{ "beforeEdit": 3 }"#).is_err());
  }

  fn describe_unknown_mentions_it() -> bool {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
    std::fs::write(dir.path().join(HOOKS_FILE), r#"{ "afterturn": "x" }"#).unwrap();
    describe(&dir.path().to_string_lossy()).contains("Unknown keys, skipped: afterturn")
  }

  #[test]
  fn before_prompt_takes_its_short_default_and_tracks_nothing() {
    let c = parse(Path::new("/w"), r#"{ "beforePrompt": "python3 p.py" }"#).unwrap();
    assert_eq!(c.before_prompt, Some(Hook { run: "python3 p.py".into(), timeout: BEFORE_PROMPT_TIMEOUT }));
    assert!(!c.tracks_changes());
    assert!(c.unknown.is_empty());
  }

  // The shape a real project writes (Dev-Workspace, 2026-10-08): comments in another language, a bare beforeEdit, nested
  // repositories listed for the snapshot
  #[test]
  fn a_commented_project_file_parses() {
    let text = r#"{
      // 会话第一条消息附带 Server 规范全文
      "context": ["Sources/Server/AGENTS.md"],
      "beforeEdit": "python3 tools/hooks/route_docs.py",
      "afterTurn": { "run": "python3 tools/hooks/stop_gate.py", "timeout": 90, "rounds": 1 },
      "watch": [".", "Sources/Server", "Runtime/Dev/plugins"]
    }"#;
    let c = parse(Path::new("/w"), text).unwrap();
    assert_eq!(c.rounds, 1);
    assert_eq!(c.after_turn.unwrap().timeout, Duration::from_secs(90));
    assert_eq!(c.watch.len(), 3);
  }

  #[test]
  fn a_huge_timeout_is_capped_rather_than_a_panic() {
    for t in ["1e20", "1e308", "4000"] {
      let c = parse(Path::new("/w"), &format!(r#"{{ "afterTurn": {{ "run": "x", "timeout": {t} }} }}"#)).unwrap();
      assert_eq!(c.after_turn.unwrap().timeout, MAX_TIMEOUT, "{t}");
    }
    let c = parse(Path::new("/w"), r#"{ "beforeEdit": { "run": "x", "timeout": 0.25 } }"#).unwrap();
    assert_eq!(c.before_edit.unwrap().timeout, Duration::from_millis(250));
  }

  #[test]
  fn an_unreadable_hooks_file_is_an_error_not_an_open_gate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let cwd = root.to_string_lossy().into_owned();
    assert!(load(&cwd).is_none(), "no file: no hooks");
    std::fs::create_dir_all(root.join(".agents")).unwrap();
    std::fs::write(root.join(HOOKS_FILE), [&b"{ \"afterTurn\": \""[..], &[0xff], &b"\" }"[..]].concat()).unwrap();
    assert!(matches!(load(&cwd), Some(Err(e)) if e.contains("hooks.json")), "not UTF-8");
    // A directory where the file should be cannot be read either
    std::fs::remove_file(root.join(HOOKS_FILE)).unwrap();
    std::fs::create_dir(root.join(HOOKS_FILE)).unwrap();
    assert!(matches!(load(&cwd), Some(Err(_))));
  }

  #[test]
  fn context_alone_tracks_nothing() {
    let c = parse(Path::new("/w"), r#"{ "context": ["AGENTS.md"] }"#).unwrap();
    assert!(!c.tracks_changes());
  }
}
