//! Everything the Shared tab shows, derived from the files alone: which shared skills / servers / prompts exist and
//! how each installed agent reaches them. The wiring points (`wires`) are shared with the actions, so what the page
//! calls "missing" is exactly what "link all" creates

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use acpira_shared::inventory::McpCaps;
use acpira_shared::shared_config::*;

use super::ledger::Ledger;
use super::links::{Spot, has_import, inspect, same_content};
use super::mcp::{all_servers, native_names};
use super::{AGENTS, Places, pi_trust, preview, scope_of_template};
use crate::agent_ext::{RuleWire, agent_ext};
use crate::inventory::parse_frontmatter;
use crate::platform::paths::wire_path;

/// The import line Claude's global CLAUDE.md gets
pub const CLAUDE_GLOBAL_IMPORT: &str = "@~/.agents/AGENTS.md";
/// The import line a project CLAUDE.md gets
pub const CLAUDE_PROJECT_IMPORT: &str = "@AGENTS.md";

#[derive(Debug, Clone, PartialEq)]
pub enum WireKind {
  Link,
  Import,
}

/// One place where a shared resource has to show up for one agent
#[derive(Debug, Clone)]
pub struct Wire {
  pub agent: &'static str,
  pub scope: SharedScope,
  /// The shared skill directory or prompt file
  pub target: PathBuf,
  /// Where the link / import goes
  pub at: PathBuf,
  pub kind: WireKind,
  /// The skill name; None for the prompt
  pub skill: Option<String>,
}

impl Wire {
  /// Project links are relative and kept out of git status
  pub fn relative(&self) -> bool {
    self.scope == SharedScope::Project
  }

  /// An import file that carries the shared prompt's import line and instructions of its own besides
  pub fn mixed_import(&self) -> bool {
    if self.kind != WireKind::Import {
      return false;
    }
    let Ok(text) = std::fs::read_to_string(&self.at) else { return false };
    let forms = import_forms(&self.target);
    has_import(&text, &forms) && text.lines().map(str::trim).any(|l| !l.is_empty() && !forms.iter().any(|f| f == l))
  }

  /// The state overwrite goes by: an import file with instructions of its own besides the import line is a conflict,
  /// since that agent would read more than the shared prompt
  pub fn overwrite_state(&self) -> ReachState {
    if self.mixed_import() { ReachState::Conflict } else { self.state() }
  }

  pub fn state(&self) -> ReachState {
    match self.kind {
      WireKind::Link => match inspect(&self.at, &self.target) {
        Spot::Absent | Spot::Same => ReachState::Missing,
        Spot::Linked => ReachState::Linked,
        Spot::Elsewhere | Spot::Differs => ReachState::Conflict,
      },
      WireKind::Import => {
        let Ok(text) = std::fs::read_to_string(&self.at) else { return ReachState::Missing };
        if has_import(&text, &import_forms(&self.target)) {
          ReachState::Linked
        } else if text.trim().is_empty() || std::fs::read_to_string(&self.target).is_ok_and(|t| t.trim() == text.trim()) {
          ReachState::Missing
        } else {
          ReachState::Conflict
        }
      }
    }
  }
}

/// The import lines that count for the shared global prompt: the `~/` form Acpira writes, or its absolute path
pub fn import_forms(target: &Path) -> Vec<String> {
  vec![CLAUDE_GLOBAL_IMPORT.to_owned(), format!("@{}", target.display())]
}

/// Skill directories (with a SKILL.md, dot-names skipped) of one skills folder: (dir name, dir, description)
pub fn skill_dirs(dir: &Path) -> Vec<(String, PathBuf, Option<String>)> {
  let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
  let mut out: Vec<_> = rd
    .flatten()
    .filter_map(|e| {
      let name = e.file_name().to_string_lossy().into_owned();
      if name.starts_with('.') {
        return None;
      }
      let path = e.path();
      let text = std::fs::read_to_string(path.join("SKILL.md")).ok()?;
      Some((name, path, parse_frontmatter(&text).1))
    })
    .collect();
  out.sort_by(|a, b| crate::inventory::locale_compare(&a.0, &b.0));
  out
}

fn ext_of(agent: &str) -> Option<(&'static str, &'static crate::agent_ext::AgentExt)> {
  let id = AGENTS.iter().find(|a| **a == agent)?;
  Some((id, agent_ext(id)?))
}

/// Every wiring point for the given installed agents; `skill_filter` narrows skills to one name
pub fn wires(places: &Places, agents: &[String]) -> Vec<Wire> {
  let mut out = vec![];
  for scope in [SharedScope::Project, SharedScope::Global] {
    let Some(dir) = places.skills_dir(scope) else { continue };
    for (name, path, _) in skill_dirs(&dir) {
      for a in agents {
        let Some((id, ext)) = ext_of(a) else { continue };
        if ext.reads_shared_skills(scope == SharedScope::Global) {
          continue;
        }
        let Some((user, project)) = ext.shared.skill_links else { continue };
        let Some(base) = places.expand(if scope == SharedScope::Global { user } else { project }) else { continue };
        out.push(Wire { agent: id, scope, target: path.clone(), at: base.join(&name), kind: WireKind::Link, skill: Some(name.clone()) });
      }
    }
  }
  if let Some(target) = places.prompt_file(SharedScope::Global) {
    for a in agents {
      let Some((id, ext)) = ext_of(a) else { continue };
      let Some((tpl, wire)) = ext.shared.global_rules else { continue };
      let Some(at) = places.expand(tpl) else { continue };
      let kind = if wire == RuleWire::Import { WireKind::Import } else { WireKind::Link };
      out.push(Wire { agent: id, scope: SharedScope::Global, target: target.clone(), at, kind, skill: None });
    }
  }
  out
}

fn reach_of(w: &Wire) -> Reach {
  Reach { agent: w.agent.to_owned(), state: w.state(), path: Some(wire_path(&w.at)) }
}

/// Native readers of a skill scope, as Reach entries; Pi reads project skills only in a project it trusts
fn native_reach(agents: &[String], global: bool, pi_untrusted: bool) -> Vec<Reach> {
  agents
    .iter()
    .filter_map(|a| ext_of(a))
    .filter(|(_, ext)| ext.reads_shared_skills(global))
    .map(|(id, _)| {
      let state = if !global && id == "pi" && pi_untrusted { ReachState::Untrusted } else { ReachState::Native };
      Reach { agent: id.to_owned(), state, path: None }
    })
    .collect()
}

/// Pi is installed, the project has shared skills, and Pi would skip them
pub fn pi_untrusted(places: &Places, agents: &[String]) -> bool {
  let Some(root) = &places.root else { return false };
  agents.iter().any(|a| a == "pi")
    && places.skills_dir(SharedScope::Project).is_some_and(|d| !skill_dirs(&d).is_empty())
    && !pi_trust::trusted(places, root)
}

/// User-level link points not in place yet: what the link panel offers. A prompt link point counts even before
/// `~/.agents/AGENTS.md` exists, since picking an agent's own file there creates it
fn plan(all_wires: &[Wire], ledger: &Ledger) -> Vec<PlanItem> {
  all_wires
    .iter()
    .filter(|w| w.scope == SharedScope::Global)
    .filter_map(|w| {
      let state = w.state();
      if !matches!(state, ReachState::Missing | ReachState::Conflict) {
        return None;
      }
      let (kind, name) = match &w.skill {
        Some(name) => (PlanKind::Skill, name.clone()),
        None => (PlanKind::Prompt, w.at.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
      };
      Some(PlanItem {
        at: wire_path(&w.at),
        agent: w.agent.to_owned(),
        kind,
        name,
        state,
        skipped: ledger.is_skipped(&w.at),
      })
    })
    .collect()
}

/// The lines of `text` that `base` lacks (compared trimmed, blank lines and the shared prompt's import lines ignored),
/// in file order; runs that were apart in `text` stay apart by one blank line. Empty when nothing is new.
/// Code fences (with or without a language tag) and lines without a letter or digit (rules, table separators) never
/// count as known, or a merged block would lose the fences around its code. They only travel with new text, though:
/// a paragraph (a run of non-blank lines, a fenced block counted whole) without one new content line adds nothing,
/// so a table whose rows are all known does not leave its separator row behind on its own
pub fn unique_lines(text: &str, base: &str, imports: &[String]) -> String {
  let fence = |l: &str| l.starts_with("```") || l.starts_with("~~~");
  let structural = |l: &str| fence(l) || !l.chars().any(char::is_alphanumeric);
  let known: HashSet<&str> = base.lines().map(str::trim).filter(|l| !structural(l)).collect();
  let fresh = |t: &str| !t.is_empty() && !known.contains(t) && !imports.iter().any(|f| f == t);
  let lines: Vec<&str> = text.lines().collect();
  // keep[i]: line i is new and its paragraph holds at least one new line that is not structural
  let mut keep = vec![false; lines.len()];
  let (mut start, mut in_fence) = (0, false);
  for i in 0..=lines.len() {
    let t = lines.get(i).map(|l| l.trim());
    if let Some(t) = t.filter(|t| !t.is_empty() || in_fence) {
      in_fence ^= fence(t);
      continue;
    }
    let adds = (start..i).any(|j| fresh(lines[j].trim()) && !structural(lines[j].trim()));
    (start..i).for_each(|j| keep[j] = adds && fresh(lines[j].trim()));
    start = i + 1;
  }
  let mut out: Vec<&str> = vec![];
  let mut gap = false;
  for (line, keep) in lines.iter().zip(keep) {
    if !keep {
      gap = true;
      continue;
    }
    if gap && !out.is_empty() {
      out.push("");
    }
    gap = false;
    out.push(line.trim_end());
  }
  out.join("\n")
}

/// The agents' own global prompts that hold lines the shared one lacks, for the overwrite panel
fn takeover(all_wires: &[Wire]) -> Vec<Takeover> {
  all_wires
    .iter()
    .filter(|w| w.skill.is_none() && w.overwrite_state() == ReachState::Conflict)
    .filter_map(|w| {
      let text = std::fs::read_to_string(&w.at).ok()?;
      let base = std::fs::read_to_string(&w.target).unwrap_or_default();
      let unique = unique_lines(&text, &base, &import_forms(&w.target));
      (!unique.is_empty()).then(|| Takeover { agent: w.agent.to_owned(), path: wire_path(&w.at), unique })
    })
    .collect()
}

/// Whether the project CLAUDE.md hides the project AGENTS.md from Claude (Claude reads AGENTS.md only without it)
pub fn claude_blocked(root: &Path) -> Option<PathBuf> {
  let file = root.join("CLAUDE.md");
  let text = std::fs::read_to_string(&file).ok()?;
  (!has_import(&text, &[CLAUDE_PROJECT_IMPORT.to_owned()])).then_some(file)
}

/// Skills in agents' own directories; a link into a shared skills folder is not one
fn private_skills(places: &Places, agents: &[String], shared: &[(SharedScope, String, PathBuf)], planned: &HashSet<PathBuf>) -> Vec<PrivateSkill> {
  let shared_real: HashSet<PathBuf> = shared.iter().filter_map(|(_, _, p)| std::fs::canonicalize(p).ok()).collect();
  let mut seen = HashSet::new();
  let mut out = vec![];
  for a in agents {
    let Some((id, ext)) = ext_of(a) else { continue };
    for tpl in ext.shared.own_skills {
      let scope = scope_of_template(tpl);
      let Some(dir) = places.expand(tpl) else { continue };
      for (name, path, description) in skill_dirs(&dir) {
        let Ok(real) = std::fs::canonicalize(&path) else { continue };
        // A user-level link point with content of its own is decided in the link panel instead
        if planned.contains(&path) || shared_real.contains(&real) || !seen.insert(real) {
          continue;
        }
        let twin = shared.iter().find(|(s, n, _)| *s == scope && *n == name);
        let matches = match twin {
          None => PrivateMatch::Unique,
          Some((_, _, p)) if same_content(&path, p) => PrivateMatch::Same,
          Some(_) => PrivateMatch::Differs,
        };
        // An identical copy where a link belongs is fixed by "link all"; listing it here too would only be noise
        if matches == PrivateMatch::Same && ext.shared.skill_links.is_some() {
          continue;
        }
        out.push(PrivateSkill { name, description, path: wire_path(&path), agent: id.to_owned(), scope, matches });
      }
    }
  }
  out
}

pub fn build(places: &Places, agents: &[String], caps: &dyn Fn(&str) -> Option<McpCaps>, ledger: &Ledger) -> SharedView {
  let all_wires = wires(places, agents);
  let pi_untrusted = pi_untrusted(places, agents);
  let mut shared_dirs = vec![];
  let mut skills = vec![];
  for scope in [SharedScope::Project, SharedScope::Global] {
    let Some(dir) = places.skills_dir(scope) else { continue };
    for (name, path, description) in skill_dirs(&dir) {
      let mut reach = native_reach(agents, scope == SharedScope::Global, pi_untrusted);
      reach.extend(all_wires.iter().filter(|w| w.scope == scope && w.skill.as_deref() == Some(&name)).map(reach_of));
      shared_dirs.push((scope, name.clone(), path.clone()));
      skills.push(SharedSkill { name, description, path: wire_path(&path), scope, reach });
    }
  }

  let mcp = all_servers(places)
    .into_iter()
    .map(|(scope, s, shadowed)| {
      let mut unsupported = vec![];
      let mut native = vec![];
      for a in agents {
        let Some((id, ext)) = ext_of(a) else { continue };
        let c = caps(id);
        // Transport support is only known once the agent ran; unknown is not shown as unsupported
        let transport_ok = match s.transport {
          acpira_shared::inventory::McpTransport::Stdio => true,
          acpira_shared::inventory::McpTransport::Http => c.is_none_or(|c| c.http),
          acpira_shared::inventory::McpTransport::Sse => c.is_none_or(|c| c.sse),
        };
        if !ext.shared.mcp {
          continue;
        }
        if !transport_ok {
          unsupported.push(id.to_owned());
        } else if native_names(id, places).contains(&s.name) {
          native.push(id.to_owned());
        }
      }
      SharedMcp { name: s.name.clone(), scope, transport: s.transport, target: s.target(), enabled: s.enabled, shadowed, unsupported, native }
    })
    .collect();

  let mut prompts = vec![];
  if let Some(root) = &places.root {
    let path = root.join("AGENTS.md");
    let text = std::fs::read_to_string(&path).ok();
    let mut reach = vec![];
    if text.is_some() {
      for a in agents {
        let Some((id, _)) = ext_of(a) else { continue };
        reach.push(match (id, claude_blocked(root)) {
          ("claude", Some(file)) => Reach { agent: id.into(), state: ReachState::Conflict, path: Some(wire_path(&file)) },
          _ => Reach { agent: id.into(), state: ReachState::Native, path: None },
        });
      }
    }
    prompts.push(SharedPrompt {
      scope: SharedScope::Project,
      path: wire_path(&path),
      exists: text.is_some(),
      preview: text.as_deref().map(|t| preview(t, 6)).unwrap_or_default(),
      text: text.unwrap_or_default(),
      reach,
    });
  }
  if let Some(path) = places.prompt_file(SharedScope::Global) {
    let text = std::fs::read_to_string(&path).ok();
    let mut reach = vec![];
    for a in agents {
      let Some((id, ext)) = ext_of(a) else { continue };
      match all_wires.iter().find(|w| w.skill.is_none() && w.agent == id) {
        Some(w) => {
          if text.is_some() {
            reach.push(Reach { agent: id.into(), state: w.state(), path: Some(wire_path(&w.at)) });
          }
        }
        None if ext.shared.global_rules.is_none() && text.is_some() => {
          reach.push(Reach { agent: id.into(), state: ReachState::Unsupported, path: None });
        }
        None => {}
      }
    }
    prompts.push(SharedPrompt {
      scope: SharedScope::Global,
      path: wire_path(&path),
      exists: text.is_some(),
      preview: text.as_deref().map(|t| preview(t, 6)).unwrap_or_default(),
      text: text.unwrap_or_default(),
      reach,
    });
  }

  let planned: HashSet<PathBuf> = all_wires.iter().filter(|w| w.scope == SharedScope::Global && w.skill.is_some()).map(|w| w.at.clone()).collect();
  let root_str = places.root.as_ref().map(|r| wire_path(r));
  SharedView {
    root: root_str.clone(),
    home: wire_path(&places.home),
    auto: ledger.auto,
    user_linked: ledger.entries.iter().any(|e| e.project_root().is_none()),
    project_auto: !ledger.project_manual,
    project_shared: places.root.as_ref().is_some_and(|r| ledger.is_shared_project(r)),
    project_linked: root_str.as_deref().is_some_and(|r| ledger.entries.iter().any(|e| e.project_root() == Some(r))),
    pi_untrusted,
    shared_prompt: places.prompt_file(SharedScope::Global).is_some_and(|p| p.exists()),
    overwrite: ledger.overwrite,
    takeover: if ledger.overwrite { vec![] } else { takeover(&all_wires) },
    private_skills: private_skills(places, agents, &shared_dirs, &planned),
    plan: plan(&all_wires, ledger),
    skills,
    mcp,
    no_mcp: agents.iter().filter_map(|a| ext_of(a)).filter(|(_, ext)| !ext.shared.mcp).map(|(id, _)| id.to_owned()).collect(),
    prompts,
  }
}
