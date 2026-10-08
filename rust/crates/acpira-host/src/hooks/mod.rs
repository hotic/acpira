//! Workspace hooks: one `<root>/.agents/hooks.json` gates every agent the same way, whatever its CLI does with hooks of
//! its own. Acpira owns the parts of the loop every ACP agent shares — the prompt it is sent, the permission requests it
//! makes and the end of its turn — so the gate runs there instead of once per harness:
//!
//! - `context`: files sent along with a session's first prompt (the instructions the agent must have read);
//! - `beforeEdit`: asked before an edit a permission request announces (deny → the request is rejected), and again at
//!   the end of the turn for every file the turn changed without asking;
//! - `afterTurn`: run when a turn ends; a block sends its reason back to the agent as an automatic follow-up, at most
//!   `rounds` times in a row.
//!
//! Scripts speak the Claude Code hook protocol (JSON on stdin; `permissionDecision: deny` / `decision: block` on stdout,
//! or exit code 2 with the reason on stderr), so a repository's existing hook scripts run unchanged

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

const BEFORE_EDIT_TIMEOUT: Duration = Duration::from_secs(30);
const AFTER_TURN_TIMEOUT: Duration = Duration::from_secs(120);
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
  pub before_edit: Option<Hook>,
  pub after_turn: Option<Hook>,
  pub rounds: u32,
  /// Git work trees whose changes count as the turn's edits (nested repositories the root's own status cannot see)
  pub watch: Vec<PathBuf>,
}

impl HooksConfig {
  /// Whether turns need a before / after snapshot of the watched trees
  pub fn tracks_changes(&self) -> bool {
    self.before_edit.is_some() || self.after_turn.is_some()
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

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawConfig {
  #[serde(default, rename = "$schema")]
  _schema: Option<String>,
  #[serde(default)]
  context: Vec<String>,
  #[serde(default)]
  before_edit: Option<RawHook>,
  #[serde(default)]
  after_turn: Option<RawHook>,
  #[serde(default)]
  watch: Option<Vec<String>>,
}

fn hook_of(raw: Option<RawHook>, default_timeout: Duration) -> (Option<Hook>, Option<u32>) {
  match raw {
    None => (None, None),
    Some(RawHook::Command(run)) => (Some(Hook { run, timeout: default_timeout }).filter(|h| !h.run.trim().is_empty()), None),
    Some(RawHook::Full { run, timeout, rounds }) => {
      let timeout = timeout.filter(|t| t.is_finite() && *t > 0.0).map(Duration::from_secs_f64).unwrap_or(default_timeout);
      (Some(Hook { run, timeout }).filter(|h| !h.run.trim().is_empty()), rounds)
    }
  }
}

/// Parse a hooks file's text for the project at `root`
pub fn parse(root: &Path, text: &str) -> Result<HooksConfig, String> {
  let value: Value = parse_json_loose(text).ok_or_else(|| "not valid JSON".to_owned())?;
  let raw: RawConfig = serde_json::from_value(value).map_err(|e| e.to_string())?;
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
    before_edit,
    after_turn,
    rounds: rounds.unwrap_or(DEFAULT_ROUNDS),
    watch,
  })
}

/// The hooks of the project `cwd` belongs to. None: the project declares none; Err: the file is there but unusable
pub fn load(cwd: &str) -> Option<Result<HooksConfig, String>> {
  let root = project_root(cwd)?;
  let path = root.join(HOOKS_FILE);
  let text = std::fs::read_to_string(&path).ok()?;
  Some(parse(&root, &text).map_err(|e| format!("{}: {e}", path.display())))
}

/// The file format as the `workspace_hooks` MCP tool explains it to a model asked to add or change a gate. Kept next
/// to the parser so the two are edited together
pub const GUIDE: &str = r#"# Acpira workspace hooks

`<project root>/.agents/hooks.json` (the root is the nearest ancestor holding `.git`) gates every agent the user runs
through Acpira the same way, whatever CLI it is. Acpira runs the scripts itself; the agents' own hook settings are not
involved. The file is JSON with comments and trailing commas allowed, and is read again before every prompt, so an edit
takes effect from the next turn. An unknown key makes the whole file invalid (reported to the user), never ignored.

```jsonc
{
  // Files sent with the first prompt of every session (paths relative to the root)
  "context": ["AGENTS.md"],
  // Asked before an edit the agent requests permission for; deny = the request is rejected and the reason goes back
  // to the agent. Asked again when the turn ends for every file the turn changed without asking (phase "audit")
  "beforeEdit": "python3 tools/hooks/check_edit.py",
  // Run when a turn ends; a block sends the reason back to the agent as an automatic follow-up, at most `rounds`
  // times per user prompt (default 2), then the user is told the gate gave up
  "afterTurn": { "run": "python3 tools/hooks/stop_gate.py", "timeout": 90, "rounds": 1 },
  // Git work trees whose changes count as the turn's edits (default: the root). List nested repositories the
  // root's own `git status` cannot see
  "watch": [".", "vendor/server"]
}
```

A hook is a bare command string or `{ "run": "...", "timeout": seconds }` (`rounds` only on afterTurn). Defaults:
beforeEdit 30 s, afterTurn 120 s. Commands run through `/bin/sh -c` (`cmd /c` on Windows) in the project root, with
`ACPIRA_PROJECT_DIR` and `CLAUDE_PROJECT_DIR` set to the root,
`ACPIRA_SESSION_ID` and `ACPIRA_AGENT` to the session's. A hook that times out, crashes or prints something
unreadable lets the edit / turn through and shows the user a warning.

## Input (JSON on stdin)

beforeEdit: `hook_event_name: "PreToolUse"`, `session_id`, `agent` (Acpira's agent id), `cwd` (the root),
`tool_name` (`Edit` | `Delete` | `Move`), `tool_input` (the tool's raw input plus `file_path`), `files` (absolute paths
of every target), `read_files` (absolute paths the session's tools have read), `phase` (`"permission"` | `"audit"`).

afterTurn: `hook_event_name: "Stop"`, `session_id`, `agent`, `cwd`, `stop_hook_active` (true on a follow-up round),
`round`, `turn_files` (changed by this turn), `changed_files` (changed by the session so far), `read_files`.

## Answer (the Claude Code hook protocol, so existing Claude Code hook scripts work unchanged)

- exit 0 with no output, or `{}`: pass;
- exit 2: block, stderr is the reason;
- exit 0 with JSON: `{"hookSpecificOutput": {"permissionDecision": "deny", "permissionDecisionReason": "..."}}`
  (beforeEdit) or `{"decision": "block", "reason": "..."}` (afterTurn) blocks with that reason.

Write reasons for the agent: they are sent to it as they are, so say what is wrong and how to fix it. After writing
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
        "`{}` is valid. context: {} file(s); beforeEdit: {}; afterTurn: {}, rounds {}; watch: {}.",
        wire_path(&path),
        c.context.len(),
        hook(&c.before_edit),
        hook(&c.after_turn),
        c.rounds,
        c.watch.iter().map(|p| wire_path(p)).collect::<Vec<_>>().join(", "),
      );
      if !missing.is_empty() {
        text.push_str(&format!(" Context files that do not exist: {}.", missing.join(", ")));
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
  fn a_misspelled_key_is_an_error_not_a_silently_open_gate() {
    let e = parse(Path::new("/w"), r#"{ "afterturn": "x" }"#).unwrap_err();
    assert!(e.contains("afterturn"), "{e}");
    assert!(parse(Path::new("/w"), "{").is_err());
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
  fn context_alone_tracks_nothing() {
    let c = parse(Path::new("/w"), r#"{ "context": ["AGENTS.md"] }"#).unwrap();
    assert!(!c.tracks_changes());
  }
}
