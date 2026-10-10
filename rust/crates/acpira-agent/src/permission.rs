//! Permission rules, after OpenCode's semantics: a rule is (permission, pattern, decision), rules come in layers
//! (defaults, the approval level, what the user allowed during the session, then the mode, whose restrictions hold
//! over everything before it) and the last rule that matches wins. A path pattern is matched against the workspace-relative path (absolute outside the workspace), a command
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
  /// `read`, `edit`, `bash`, `todo`, or `*`
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

/// The approval level: how much runs without a card
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

  pub fn name(self) -> &'static str {
    match self {
      Approval::Ask => "Ask",
      Approval::AutoEdit => "Auto-edit",
      Approval::Full => "Full access",
    }
  }

  pub fn description(self) -> &'static str {
    match self {
      Approval::Ask => "Ask before every edit and command",
      Approval::AutoEdit => "Edit files without asking; ask before commands",
      Approval::Full => "Run everything without asking, except changes to Acpira's own configuration",
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

/// What every session starts from: reads and the to-do list run, the rest asks. The session's spilled outputs are
/// always readable
pub fn defaults(outputs: &Path) -> Vec<Rule> {
  vec![
    Rule::new("*", "*", Decision::Ask),
    Rule::new(READ, "*", Decision::Allow),
    Rule::new(TODO, "*", Decision::Allow),
    Rule::new(READ, &format!("{}/*", slashed(outputs)), Decision::Allow),
  ]
}

/// The last matching rule over the layers in order; ask when none matches
pub fn evaluate(layers: &[&[Rule]], permission: &str, target: &str) -> Decision {
  let mut out = Decision::Ask;
  for layer in layers {
    for r in layer.iter() {
      if wildcard(&r.permission, permission) && wildcard(&r.pattern, target) {
        out = r.decision;
      }
    }
  }
  out
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
    Guard { files: files.iter().map(|p| normal(p)).collect(), dirs: dirs.iter().map(|p| normal(p)).collect() }
  }

  pub fn protects(&self, path: &Path) -> bool {
    let p = normal(path);
    self.files.contains(&p) || self.dirs.iter().any(|d| p.starts_with(d))
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
}
