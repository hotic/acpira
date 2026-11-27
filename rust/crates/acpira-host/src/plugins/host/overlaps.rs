//! Which loaded plugins overlap, where the user's tile order decides which one wins

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use acpira_shared::plugins::PluginOverlap;

use super::{Active, restyled_hooks};
use crate::plugins::manifest;

/// Pairwise overlaps among loaded plugins where tile order decides precedence (first in order loads / wins first)
pub(super) fn compute_overlaps(active: &[Arc<Active>]) -> BTreeMap<String, Vec<PluginOverlap>> {
  struct Sig {
    commands: BTreeSet<String>,
    providers: BTreeSet<String>,
    mcp: BTreeSet<String>,
    hooks: BTreeSet<String>,
    has_style: bool,
    style_hooks: BTreeSet<String>,
    prompts: BTreeSet<String>,
    skills: BTreeSet<String>,
  }

  let sigs: Vec<Sig> = active
    .iter()
    .map(|a| {
      let c = &a.manifest.contributes;
      let commands = c.commands.iter().map(|cmd| format!("/{}", cmd.name)).collect();
      let providers = c.providers.iter().filter_map(|raw| manifest::provider_of(raw, &a.id).ok().map(|p| p.id)).collect();
      let mcp = c.mcp.as_ref().map(|m| m.keys().cloned().collect()).unwrap_or_default();
      let mut hooks = BTreeSet::new();
      if let Some(rt) = &c.runtime
        && rt.scope.as_deref() != Some(manifest::SCOPE_AGENT)
      {
        for h in &rt.hooks {
          // `command` and `menu` hooks only serve this plugin's own slash commands and buttons
          if h != "command" && h != "menu" {
            hooks.insert(h.clone());
          }
        }
      }
      if let Some(obj) = c.hooks.as_ref().and_then(Value::as_object) {
        for (k, v) in obj {
          if !matches!(k.as_str(), "watch" | "rounds") && !v.is_null() {
            hooks.insert(k.clone());
          }
        }
      }
      let (has_style, style_hooks) = match c.style.as_deref() {
        Some(rel) => {
          let css = manifest::inside(&a.dir, rel).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
          (true, restyled_hooks(&css).into_iter().collect())
        }
        None => (false, BTreeSet::new()),
      };
      let prompts = c
        .prompts
        .as_deref()
        .and_then(|rel| manifest::inside(&a.dir, rel))
        .filter(|d| d.is_dir())
        .map(|d| collect_relative_md(&d))
        .unwrap_or_default();
      let skills = c
        .skills
        .as_deref()
        .and_then(|rel| manifest::inside(&a.dir, rel))
        .filter(|d| d.is_dir())
        .map(|d| collect_skill_names(&d))
        .unwrap_or_default();
      Sig { commands, providers, mcp, hooks, has_style, style_hooks, prompts, skills }
    })
    .collect();

  let mut out: BTreeMap<String, Vec<PluginOverlap>> = BTreeMap::new();
  let mut push = |a: &str, b: &str, kind: &str, items: Vec<String>| {
    out.entry(a.to_owned()).or_default().push(PluginOverlap { plugin: b.to_owned(), kind: kind.to_owned(), items: items.clone() });
    out.entry(b.to_owned()).or_default().push(PluginOverlap { plugin: a.to_owned(), kind: kind.to_owned(), items });
  };
  let shared = |x: &BTreeSet<String>, y: &BTreeSet<String>| -> Vec<String> { x.intersection(y).cloned().collect() };

  for i in 0..active.len() {
    for j in (i + 1)..active.len() {
      let (ai, aj) = (&active[i].id, &active[j].id);
      let (si, sj) = (&sigs[i], &sigs[j]);
      for (kind, items) in [
        ("command", shared(&si.commands, &sj.commands)),
        ("provider", shared(&si.providers, &sj.providers)),
        ("mcp", shared(&si.mcp, &sj.mcp)),
        ("hook", shared(&si.hooks, &sj.hooks)),
        ("prompt", shared(&si.prompts, &sj.prompts)),
        ("skill", shared(&si.skills, &sj.skills)),
      ] {
        if !items.is_empty() {
          push(ai, aj, kind, items);
        }
      }
      if si.has_style && sj.has_style {
        let items = shared(&si.style_hooks, &sj.style_hooks);
        // Two stylesheets targeting disjoint known hooks do not conflict; otherwise CSS cascade order applies
        if !items.is_empty() || si.style_hooks.is_empty() || sj.style_hooks.is_empty() {
          push(ai, aj, "style", items);
        }
      }
    }
  }
  out
}

/// Relative `.md` file paths inside a plugin's `prompts` folder
fn collect_relative_md(root: &Path) -> BTreeSet<String> {
  fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
      let path = e.path();
      if path.is_dir() {
        walk(root, &path, out);
      } else if path.extension().and_then(|s| s.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        && let Ok(rel) = path.strip_prefix(root)
      {
        out.insert(rel.to_string_lossy().replace('\\', "/"));
      }
    }
  }
  let mut out = BTreeSet::new();
  walk(root, root, &mut out);
  out
}

/// Skill folder names (`<name>/SKILL.md`) inside a plugin's `skills` folder
fn collect_skill_names(root: &Path) -> BTreeSet<String> {
  let Ok(entries) = std::fs::read_dir(root) else { return BTreeSet::new() };
  entries
    .flatten()
    .filter_map(|e| {
      let path = e.path();
      (path.is_dir() && path.join("SKILL.md").is_file()).then(|| e.file_name().to_string_lossy().into_owned())
    })
    .collect()
}
