//! Shared config: one set of skills, MCP servers and prompts in open files, reaching every agent.
//!
//! The content never lives in Acpira. Sources: `~/.agents/{skills/, mcp.json, AGENTS.md}` (global) and
//! `<project>/{.agents/skills/, .agents/mcp.json, AGENTS.md}`. Agents that read `.agents` natively are left alone;
//! the rest get per-skill symlinks or a link / import line in their own global instruction file (`agent_ext` `Shared`).
//! MCP servers go over ACP in `mcpServers` and never touch a CLI's config. `~/.acpira/shared-links.json` records the links
//! Acpira made, so they can be repaired or removed; it holds no content

pub mod actions;
pub mod ledger;
pub mod links;
pub mod mcp;
pub mod pi_trust;
pub mod view;

pub use actions::{Outcome, SharedConfig};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use acpira_shared::inventory::McpCaps;
use acpira_shared::shared_config::SharedScope;

use crate::acp::transport::rpc::BoxFuture;
use crate::inventory::ScanEnv;

/// What a session asks for when it builds session/new, load or resume: (agent, cwd, the agent's MCP caps) →
/// (ACP `mcpServers`, a log line naming what was sent and skipped)
pub type McpProvider = Arc<dyn Fn(String, String, Option<McpCaps>) -> BoxFuture<(Vec<Value>, Option<String>)> + Send + Sync>;

/// The provider over the shared files, for the home the shell reports
pub fn mcp_provider(home: Arc<dyn Fn() -> String + Send + Sync>) -> McpProvider {
  Arc::new(move |agent, cwd, caps| {
    let places = Places::new(&home(), &cwd);
    Box::pin(async move { mcp::session_servers(&places, &agent, caps).await })
  })
}

/// Built-in agents with shared wiring, in registry order
pub const AGENTS: [&str; 8] = ["grok", "devin", "kimi", "codex", "claude", "opencode", "dsh", "pi"];

/// Where the shared sources are, for one home and (optionally) one project
#[derive(Clone, Debug)]
pub struct Places {
  pub home: PathBuf,
  /// `$CONFIG/` of the path templates
  pub config: PathBuf,
  /// The project root; None without a workspace
  pub root: Option<PathBuf>,
}

impl Places {
  /// `cwd` is the workspace (or a session's cwd); the project root is the nearest ancestor holding `.git`
  pub fn new(home: &str, cwd: &str) -> Self {
    let env = ScanEnv::new(home.to_owned(), cwd.to_owned());
    // Without a workspace the shells report home as the cwd; home is never a project (its `.agents` is the user level)
    let root = project_root(cwd).filter(|r| r.as_path() != Path::new(home));
    Places { home: PathBuf::from(home), config: env.config, root }
  }

  /// The directory a scope's `.agents` lives in
  pub fn base(&self, scope: SharedScope) -> Option<PathBuf> {
    match scope {
      SharedScope::Global => Some(self.home.join(".agents")),
      SharedScope::Project => self.root.as_ref().map(|r| r.join(".agents")),
    }
  }

  pub fn skills_dir(&self, scope: SharedScope) -> Option<PathBuf> {
    self.base(scope).map(|b| b.join("skills"))
  }

  /// Global: `~/.agents/mcp.json` (no common place exists); project: the root's `.mcp.json`, the common project file
  /// Claude and Devin read themselves (verified over ACP 2026-10-01), so a terminal session gets it too
  pub fn mcp_file(&self, scope: SharedScope) -> Option<PathBuf> {
    match scope {
      SharedScope::Global => Some(self.home.join(".agents").join("mcp.json")),
      SharedScope::Project => self.root.as_ref().map(|r| r.join(".mcp.json")),
    }
  }

  /// The project file used before `.mcp.json` (until 2026-10-01): still read, and edited for the servers it holds
  pub fn legacy_mcp_file(&self) -> Option<PathBuf> {
    self.root.as_ref().map(|r| r.join(".agents").join("mcp.json"))
  }

  /// Global: `~/.agents/AGENTS.md`; project: the root's own `AGENTS.md`, which every agent reads natively
  pub fn prompt_file(&self, scope: SharedScope) -> Option<PathBuf> {
    match scope {
      SharedScope::Global => Some(self.home.join(".agents").join("AGENTS.md")),
      SharedScope::Project => self.root.as_ref().map(|r| r.join("AGENTS.md")),
    }
  }

  /// A path template of `agent_ext` (`~/`, `$CONFIG/`, or project-relative); None for a project path without a project
  pub fn expand(&self, template: &str) -> Option<PathBuf> {
    if let Some(rest) = template.strip_prefix("~/") {
      Some(self.home.join(rest))
    } else if let Some(rest) = template.strip_prefix("$CONFIG/") {
      Some(self.config.join(rest))
    } else {
      self.root.as_ref().map(|r| r.join(template))
    }
  }

  /// The scan environment of the inventory, rooted at the project
  pub fn scan_env(&self) -> ScanEnv {
    ScanEnv {
      home: self.home.to_string_lossy().into_owned(),
      cwd: self.root.as_ref().map(|r| r.to_string_lossy().into_owned()).unwrap_or_default(),
      config: self.config.clone(),
    }
  }
}

pub fn scope_of_template(template: &str) -> SharedScope {
  if template.starts_with("~/") || template.starts_with("$CONFIG/") { SharedScope::Global } else { SharedScope::Project }
}

/// The nearest ancestor of `cwd` holding `.git` (a directory, or a worktree's file), else `cwd` itself
pub fn project_root(cwd: &str) -> Option<PathBuf> {
  if cwd.trim().is_empty() {
    return None;
  }
  let start = Path::new(cwd);
  let mut dir = Some(start);
  while let Some(d) = dir {
    if d.join(".git").exists() {
      return Some(d.to_path_buf());
    }
    dir = d.parent();
  }
  Some(start.to_path_buf())
}

/// The first `n` non-empty lines of a prompt file, frontmatter skipped
pub fn preview(text: &str, n: usize) -> String {
  let body = match text.strip_prefix("---\n").and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..])) {
    Some(after) => after,
    None => text,
  };
  body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).take(n).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn finds_the_git_root_and_skips_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    assert_eq!(project_root(&root.join("a/b").to_string_lossy()), Some(root.clone()));
    assert_eq!(project_root(""), None);
    assert_eq!(preview("---\nx: 1\n---\n\n# T\n\nline\nmore\n", 2), "# T\nline");
    // Home as the cwd (no workspace) is the user level, not a project
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let h = home.to_string_lossy();
    assert_eq!(Places::new(&h, &h).root, None);
    assert_eq!(Places::new(&h, &root.to_string_lossy()).root, Some(root));
  }
}
