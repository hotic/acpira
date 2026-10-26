//! Permission rules, after OpenCode's semantics: a rule is (permission, pattern, decision), rules come in layers
//! (defaults, the approval level, what the user allowed during the session, then the mode, whose restrictions hold
//! over everything before it) and the last rule that matches wins. The mode layer is built with the approval level in
//! hand (`modes.rs`), which leaves a mode's `ask` rules out under full access; its `deny` rules always stay. A path pattern is matched against the workspace-relative path (absolute outside the workspace), a command
//! pattern against the whole command line. Edits to the agent's own configuration always ask, whatever the layers say

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
  Allow,
  Ask,
  Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
  /// `read`, `edit`, `bash`, `todo`, `job`, `web`, `web-local`, `mcp`, `mcp-read`, or `*`
  pub permission: String,
  pub pattern: String,
  pub decision: Decision,
}

impl Rule {
  pub fn new(permission: &str, pattern: &str, decision: Decision) -> Rule {
    Rule { permission: permission.to_owned(), pattern: pattern.to_owned(), decision }
  }
}

/// Permission keys of the tools
pub const READ: &str = "read";
pub const EDIT: &str = "edit";
pub const BASH: &str = "bash";
pub const TODO: &str = "todo";
/// The history tools of chunk compaction, which read only the session's own log
pub const HISTORY: &str = "history";
pub const JOB: &str = "job";
/// `webfetch` of a public URL and `websearch`
pub const WEB: &str = "web";
/// `webfetch` of this machine or a private network
pub const WEB_LOCAL: &str = "web-local";
/// A tool of an MCP server, target `<server>/<tool>`
pub const MCP: &str = "mcp";
/// The same for a tool its server marks `readOnlyHint`
pub const MCP_READ: &str = "mcp-read";
/// Stands for the engine's own MCP tools in a rule pattern (`acpira mcp`: show_image, workspace_hooks, ask_agent …),
/// which run like a read. `engine_rules` and `Mode::rules` expand it to one `acpira/<tool>` rule per tool, so a tool a
/// plugin adds to the same server is not covered and asks like any other MCP tool
pub const ENGINE_MCP: &str = "{engine}";

/// One rule per tool of the engine's own MCP server
pub fn engine_rules(decision: Decision) -> impl Iterator<Item = Rule> {
  acpira_shared::host_tools::ALL.iter().map(move |tool| Rule::new(MCP, &format!("{}/{tool}", acpira_shared::host_tools::SERVER), decision))
}

/// The approval level: how much runs without a card. A setting of the agent (`ProvidersFile::approval`, ids as in
/// `APPROVAL_LEVELS`), read at the start of each turn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Approval {
  /// Edits and commands ask
  #[default]
  Ask,
  /// Edits run, commands ask
  AutoEdit,
  /// Everything runs, except edits to the agent's own configuration
  Full,
}

impl Approval {
  pub const ALL: [Approval; 3] = [Approval::Ask, Approval::AutoEdit, Approval::Full];

  pub fn id(self) -> &'static str {
    match self {
      Approval::Ask => "ask",
      Approval::AutoEdit => "auto-edit",
      Approval::Full => "full",
    }
  }

  pub fn parse(s: &str) -> Option<Approval> {
    Approval::ALL.into_iter().find(|a| a.id() == s)
  }

  pub fn rules(self) -> Vec<Rule> {
    match self {
      Approval::Ask => vec![],
      Approval::AutoEdit => vec![Rule::new(EDIT, "*", Decision::Allow)],
      Approval::Full => vec![Rule::new("*", "*", Decision::Allow)],
    }
  }
}

/// What every session starts from: reads, the to-do list, the public web, MCP tools their server marks read-only and the
/// engine's own MCP tools run, the rest asks. The session's spilled outputs are always readable
pub fn defaults(outputs: &Path) -> Vec<Rule> {
  let mut rules = vec![
    Rule::new("*", "*", Decision::Ask),
    Rule::new(READ, "*", Decision::Allow),
    Rule::new(TODO, "*", Decision::Allow),
    Rule::new(HISTORY, "*", Decision::Allow),
    Rule::new(JOB, "*", Decision::Allow),
    Rule::new(WEB, "*", Decision::Allow),
    Rule::new(MCP_READ, "*", Decision::Allow),
  ];
  rules.extend(engine_rules(Decision::Allow));
  rules.push(Rule::new(READ, &format!("{}/*", slashed(outputs)), Decision::Allow));
  rules
}

/// The last matching rule over the layers in order; ask when none matches. An allow rule narrower than `*` names one
/// command (`echo review *`), so it never matches a command line that chains, pipes, redirects or substitutes another
/// one: the text match alone would let `echo review && rm -rf x` through
pub fn evaluate(layers: &[&[Rule]], permission: &str, target: &str) -> Decision {
  let compound = permission == BASH && compound_command(target);
  let mut out = Decision::Ask;
  for layer in layers {
    for r in layer.iter() {
      if compound && r.decision == Decision::Allow && r.pattern != "*" {
        continue;
      }
      if wildcard(&r.permission, permission) && wildcard(&r.pattern, target) {
        out = r.decision;
      }
    }
  }
  out
}

/// Whether a command line holds shell structure beyond one command and its arguments: a list or pipeline (`;`, `&`,
/// `|`, a line break), a redirection or process substitution (`<`, `>`), or command substitution (`` ` ``, `$(`).
/// Quoting is not parsed, so a quoted `;` counts too; that only means one more card
pub fn compound_command(command: &str) -> bool {
  command.contains([';', '&', '|', '<', '>', '`', '\n', '\r']) || command.contains("$(")
}

/// `*` matches any run of characters (`/` included), `?` one character, the rest is literal. A pattern ending in ` *`
/// also matches the command without arguments (`git status *` takes `git status`)
pub fn wildcard(pattern: &str, text: &str) -> bool {
  if let Some(bare) = pattern.strip_suffix(" *")
    && bare == text
  {
    return true;
  }
  let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
  // Greedy match with backtracking to the last star
  let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
  while ti < t.len() {
    if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
      pi += 1;
      ti += 1;
    } else if pi < p.len() && p[pi] == '*' {
      star = Some(pi);
      mark = ti;
      pi += 1;
    } else if let Some(s) = star {
      pi = s + 1;
      mark += 1;
      ti = mark;
    } else {
      return false;
    }
  }
  p[pi..].iter().all(|c| *c == '*')
}

/// A path as rules see it: relative to the workspace with `/`, absolute outside it
pub fn path_target(path: &Path, cwd: &Path) -> String {
  match path.strip_prefix(cwd) {
    Ok(rel) if !rel.as_os_str().is_empty() => slashed(rel),
    Ok(_) => ".".to_owned(),
    Err(_) => slashed(path),
  }
}

fn slashed(p: &Path) -> String {
  p.to_string_lossy().replace('\\', "/")
}

/// The pattern an "always allow" answer adds for a command: its program and subcommand (`cargo test *`), or the program
/// alone when the second word is a flag or a path
pub fn command_pattern(command: &str) -> String {
  let mut words = command.split_whitespace();
  let Some(first) = words.next() else { return command.to_owned() };
  match words.next() {
    Some(second) if second.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && !second.contains(['/', '\\', '.', '=']) => {
      format!("{first} {second} *")
    }
    _ => format!("{first} *"),
  }
}

/// Files that configure the agent itself: its model sources and keys, prompt overrides and the workspace hooks (in the
/// session folder and at its project root). Edits
/// to them ask even under full access, so a model cannot widen its own permissions or reroute its own calls
pub struct Guard {
  files: Vec<PathBuf>,
  dirs: Vec<PathBuf>,
}

impl Guard {
  pub fn new(home: &Path, cwd: &Path, user_home: Option<&Path>) -> Guard {
    let mut files = vec![
      home.join(acpira_shared::providers::PROVIDERS_FILE),
      home.join("secrets.json"),
      cwd.join(".agents").join("hooks.json"),
    ];
    let mut dirs = vec![home.join("agent").join("config"), cwd.join(".agents").join("acpira")];
    // The project's own files when the session folder is below its root (`prompt::project_root`)
    if let Some(root) = crate::prompt::project_root(cwd, user_home).filter(|r| r != cwd) {
      dirs.push(root.join(".agents").join("acpira"));
      files.push(root.join(".agents").join("hooks.json"));
    }
    if let Some(u) = user_home {
      dirs.push(u.join(".agents").join("acpira"));
      files.push(u.join(".agents").join("hooks.json"));
    }
    // Each protected path in both spellings: as written and with its symlinks resolved (a temp or home folder is often
    // reached through one, `/var` → `/private/var` on macOS)
    let both = |ps: Vec<PathBuf>| ps.iter().flat_map(|p| [normal(p), resolve(p)]).collect();
    Guard { files: both(files), dirs: both(dirs) }
  }

  /// Whether a write to `path` changes a protected file, also when the path reaches it through a symlinked file or
  /// folder
  pub fn protects(&self, path: &Path) -> bool {
    [normal(path), resolve(path)].iter().any(|p| self.files.contains(p) || self.dirs.iter().any(|d| p.starts_with(d)))
  }
}

/// The path with every symlink resolved, for a target that may not exist yet: the nearest existing ancestor is
/// canonicalized and the rest appended; a dangling symlink is followed to where it points
fn resolve(p: &Path) -> PathBuf {
  resolve_in(&normal(p), 0)
}

fn resolve_in(p: &Path, depth: u32) -> PathBuf {
  let mut head = p.to_path_buf();
  let mut tail = vec![];
  loop {
    if let Ok(real) = std::fs::canonicalize(&head) {
      return tail.iter().rev().fold(real, |acc, c| acc.join(c));
    }
    // A link whose target does not exist (yet): continue from the target, a bounded number of hops
    if depth < 40
      && let Ok(target) = std::fs::read_link(&head)
    {
      let base = head.parent().map(|d| d.join(&target)).unwrap_or(target);
      return resolve_in(&normal(&tail.iter().rev().fold(base, |acc, c| acc.join(c))), depth + 1);
    }
    match (head.file_name().map(|n| n.to_owned()), head.parent()) {
      (Some(name), Some(parent)) => {
        tail.push(name);
        head = parent.to_path_buf();
      }
      _ => return p.to_path_buf(),
    }
  }
}

/// Resolve `.` / `..` without touching the disk (the target may not exist yet); symlinks are not followed
fn normal(p: &Path) -> PathBuf {
  let mut out = PathBuf::new();
  for c in p.components() {
    match c {
      std::path::Component::ParentDir => {
        out.pop();
      }
      std::path::Component::CurDir => {}
      other => out.push(other),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn wildcards_match_paths_and_commands() {
    assert!(wildcard("*", "src/a.rs"));
    assert!(wildcard("src/*.rs", "src/deep/a.rs"));
    assert!(!wildcard("src/*.rs", "lib/a.rs"));
    assert!(wildcard("a?c", "abc") && !wildcard("a?c", "ac"));
    assert!(wildcard("git status *", "git status") && wildcard("git status *", "git status -s"));
    assert!(!wildcard("git status *", "git statuses"));
    assert!(wildcard("*.env", ".env") && wildcard("**", ""));
  }

  #[test]
  fn the_levels_match_the_settings_contract() {
    assert_eq!(Approval::ALL.map(Approval::id), acpira_shared::providers::APPROVAL_LEVELS);
    assert_eq!(Approval::default().id(), acpira_shared::providers::APPROVAL_LEVELS[0]);
  }

  #[test]
  fn a_command_prefix_rule_allows_only_a_single_command() {
    let allowed = [Rule::new(BASH, "echo review *", Decision::Allow)];
    let ev = |layers: &[&[Rule]], cmd: &str| evaluate(layers, BASH, cmd);
    assert_eq!(ev(&[&allowed], "echo review -n"), Decision::Allow);
    for cmd in ["echo review && touch x", "echo review; touch x", "echo review | sh", "echo review > x", "echo review $(touch x)", "echo review `touch x`", "echo review\ntouch x", "echo review & touch x"] {
      assert_eq!(ev(&[&allowed], cmd), Decision::Ask, "{cmd}");
    }
    // Full access and the mode's own rules still cover a compound command
    assert_eq!(ev(&[&Approval::Full.rules()], "a && b"), Decision::Allow);
    assert_eq!(ev(&[&Approval::Full.rules(), &[Rule::new(BASH, "*", Decision::Ask)]], "a && b"), Decision::Ask);
    // A deny still matches
    assert_eq!(ev(&[&Approval::Full.rules(), &[Rule::new(BASH, "rm *", Decision::Deny)]], "rm -rf x && ls"), Decision::Deny);
  }

  #[test]
  fn the_last_matching_rule_wins_across_layers() {
    let defaults = defaults(Path::new("/h/agent/sessions/s/outputs"));
    let mode = [Rule::new(EDIT, "*", Decision::Deny), Rule::new(EDIT, ".acpira/plans/*", Decision::Allow)];
    let full = Approval::Full.rules();
    let session = [Rule::new(BASH, "rm *", Decision::Deny)];
    let ev = |layers: &[&[Rule]], p: &str, t: &str| evaluate(layers, p, t);
    assert_eq!(ev(&[&defaults], EDIT, "a.rs"), Decision::Ask);
    assert_eq!(ev(&[&defaults], READ, "/anywhere"), Decision::Allow);
    assert_eq!(ev(&[&defaults, &mode], EDIT, "a.rs"), Decision::Deny);
    assert_eq!(ev(&[&defaults, &mode], EDIT, ".acpira/plans/p.md"), Decision::Allow);
    // A later layer overrides an earlier one, also a deny
    assert_eq!(ev(&[&defaults, &mode, &full], EDIT, "a.rs"), Decision::Allow);
    assert_eq!(ev(&[&defaults, &full, &session], BASH, "rm -rf x"), Decision::Deny);
    assert_eq!(ev(&[&defaults, &Approval::AutoEdit.rules()], BASH, "ls"), Decision::Ask);
    assert_eq!(ev(&[], "anything", "x"), Decision::Ask);
  }

  #[test]
  fn only_the_engine_s_own_tools_run_without_a_card() {
    let defaults = defaults(Path::new("/h/agent/sessions/s/outputs"));
    for tool in acpira_shared::host_tools::ALL {
      assert_eq!(evaluate(&[&defaults], MCP, &format!("acpira/{tool}")), Decision::Allow, "{tool}");
    }
    // A plugin's tool on the same server asks, unless the server marks it read-only
    assert_eq!(evaluate(&[&defaults], MCP, "acpira/currency_convert"), Decision::Ask);
    assert_eq!(evaluate(&[&defaults], MCP_READ, "acpira/currency_convert"), Decision::Allow);
    assert_eq!(evaluate(&[&defaults], MCP, "acpira/show_image_too"), Decision::Ask);
  }

  #[test]
  fn always_patterns_and_targets() {
    assert_eq!(command_pattern("cargo test -p x"), "cargo test *");
    assert_eq!(command_pattern("ls -la"), "ls *");
    assert_eq!(command_pattern("node ./x.js"), "node *");
    assert_eq!(path_target(Path::new("/w/src/a.rs"), Path::new("/w")), "src/a.rs");
    assert_eq!(path_target(Path::new("/etc/hosts"), Path::new("/w")), "/etc/hosts");
  }

  #[test]
  fn the_guard_covers_config_files_and_prompt_dirs() {
    let g = Guard::new(Path::new("/h/.acpira"), Path::new("/w"), Some(Path::new("/h")));
    assert!(g.protects(Path::new("/h/.acpira/providers.json")));
    assert!(g.protects(Path::new("/w/sub/../.agents/hooks.json")));
    assert!(g.protects(Path::new("/w/.agents/acpira/prompts/base.md")));
    assert!(g.protects(Path::new("/h/.agents/acpira/prompts/x.md")));
    assert!(!g.protects(Path::new("/w/.agents/skills/x/SKILL.md")));
    assert!(!g.protects(Path::new("/w/src/providers.json")));
  }

  #[cfg(unix)]
  #[test]
  fn the_guard_follows_symlinked_files_and_folders() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("secrets.json"), "{}").unwrap();
    let link = |target: &Path, name: &str| std::os::unix::fs::symlink(target, cwd.path().join(name)).unwrap();
    link(home.path(), "alias");
    link(&home.path().join("secrets.json"), "keys.json");
    // Points at a protected file that does not exist yet
    link(&home.path().join("providers.json"), "sources.json");
    let g = Guard::new(home.path(), cwd.path(), None);
    assert!(g.protects(&cwd.path().join("alias/secrets.json")));
    assert!(g.protects(&cwd.path().join("alias/providers.json")));
    assert!(g.protects(&cwd.path().join("alias/agent/config/new/x.md")));
    assert!(g.protects(&cwd.path().join("keys.json")));
    assert!(g.protects(&cwd.path().join("sources.json")));
    assert!(!g.protects(&cwd.path().join("alias/other.json")));
    assert!(!g.protects(&cwd.path().join("src/a.rs")));
  }
}
