//! The system prompt, composed in layers: the built-in base, a variant for the model's family, the user's same-name
//! files under `~/.agents/acpira/prompts/` and the project's `.agents/acpira/prompts/` (section by section, see
//! `doc.rs`), then AGENTS.md and the environment. A variant declares the models it fits (`match` patterns over
//! `<preset>/<model>`, `<source id>/<model>` and the bare model id) and may carry default sampling; a model's pinned
//! `family` names its variant directly. Adding a variant is adding a file.
//!
//! The result stays fixed for the session, so prompt caches keep hitting: it is composed again only when a model
//! switch selects another variant. What changes during a session (the mode, later the date) goes into user messages

pub mod doc;

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use acpira_shared::providers::{Provider, ProviderModel, Sampling, Thinking};

use crate::permission::wildcard;
use doc::Doc;

/// Built-in variants: (file name, text). `base` first
const BUILTIN: &[(&str, &str)] = &[
  ("base", include_str!("builtin/base.md")),
  ("deepseek", include_str!("builtin/deepseek.md")),
  ("claude", include_str!("builtin/claude.md")),
];

pub const BASE: &str = "base";
/// An AGENTS.md longer than this is cut, with a note
const MAX_INSTRUCTIONS: usize = 32 * 1024;

/// A composed system prompt and what it was made from
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Composed {
  pub text: String,
  pub variant: String,
  /// The variant file's `version` (the user's file wins when it sets one)
  pub version: String,
  /// SHA-256 of the text, first 12 hex digits: tells user overrides and AGENTS.md edits apart within one version
  pub digest: String,
  /// The variant's default sampling; a model's own values win
  pub sampling: Sampling,
  pub thinking: Option<Thinking>,
}

/// Where a session's prompt files are
pub struct Places {
  pub cwd: PathBuf,
  /// The user's home (`~/.agents` lives there)
  pub home: Option<PathBuf>,
  /// The nearest ancestor of cwd holding `.git`, home excluded
  pub root: Option<PathBuf>,
}

impl Places {
  pub fn new(cwd: &Path, home: Option<&Path>) -> Places {
    Places { cwd: cwd.to_path_buf(), home: home.map(Path::to_path_buf), root: project_root(cwd, home) }
  }

  /// Override directories, the later winning
  fn prompt_dirs(&self) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(h) = &self.home {
      out.push(h.join(".agents").join("acpira").join("prompts"));
    }
    if let Some(r) = self.root.as_ref().or(Some(&self.cwd)) {
      out.push(r.join(".agents").join("acpira").join("prompts"));
    }
    out.dedup();
    out
  }

  /// AGENTS.md files in reading order: the user's, the project's, then the session folder's own when it is deeper
  fn instructions(&self) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(h) = &self.home {
      out.push(h.join(".agents").join("AGENTS.md"));
    }
    if let Some(r) = &self.root {
      out.push(r.join("AGENTS.md"));
    }
    out.push(self.cwd.join("AGENTS.md"));
    out.dedup();
    out
  }
}

/// The project a folder belongs to: the nearest ancestor with `.git`; the home folder is never one
pub fn project_root(cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
  cwd.ancestors().find(|d| d.join(".git").exists()).filter(|r| Some(*r) != home).map(Path::to_path_buf)
}

/// One candidate variant: the built-in text (if any) with the user's files over it
struct Variant {
  name: String,
  doc: Doc,
}

/// Every variant, the user's own first (they take over a match), then the built-in ones in table order. The user's
/// `base.md` overlays the base and is not a variant of its own
fn variants(places: &Places) -> (Doc, Vec<Variant>) {
  let mut base = Doc::parse(BUILTIN[0].1);
  let mut list: Vec<Variant> = BUILTIN[1..].iter().map(|(name, text)| Variant { name: (*name).to_owned(), doc: Doc::parse(text) }).collect();
  let mut user_names: Vec<String> = vec![];
  for dir in places.prompt_dirs() {
    let Ok(rd) = std::fs::read_dir(&dir) else { continue };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
    files.sort();
    for path in files {
      let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
      let Ok(text) = std::fs::read_to_string(&path) else { continue };
      let doc = Doc::parse(&text);
      if stem == BASE {
        base.overlay(&doc);
        continue;
      }
      match list.iter_mut().find(|v| v.name == stem) {
        Some(v) => v.doc.overlay(&doc),
        None => list.push(Variant { name: stem.clone(), doc }),
      }
      if !user_names.contains(&stem) {
        user_names.push(stem);
      }
    }
  }
  // User variants match before the built-in ones
  list.sort_by_key(|v| !user_names.contains(&v.name));
  (base, list)
}

/// The keys a variant's `match` patterns are tried against, lowercase
fn keys(provider: &Provider, model: &ProviderModel) -> Vec<String> {
  let id = model.id.to_lowercase();
  let mut out = vec![];
  if !provider.preset.is_empty() {
    out.push(format!("{}/{id}", provider.preset.to_lowercase()));
  }
  out.push(format!("{}/{id}", provider.id.to_lowercase()));
  out.push(id);
  out
}

fn select<'a>(list: &'a [Variant], target: Option<(&Provider, &ProviderModel)>) -> Option<&'a Variant> {
  let (provider, model) = target?;
  // A pinned family that names a variant decides; one that names only a request family falls through to matching
  if let Some(v) = model.family.as_deref().and_then(|pin| list.iter().find(|v| v.name == pin)) {
    return Some(v);
  }
  let keys = keys(provider, model);
  list.iter().find(|v| v.doc.list("match").iter().any(|p| keys.iter().any(|k| wildcard(&p.to_lowercase(), k))))
}

/// The variant a model gets, without composing (to see whether a model switch changes the prompt)
pub fn variant_name(places: &Places, target: Option<(&Provider, &ProviderModel)>) -> String {
  let (_, list) = variants(places);
  select(&list, target).map(|v| v.name.clone()).unwrap_or_else(|| BASE.to_owned())
}

/// Compose the system prompt for a model (None before any model is configured)
pub fn compose(places: &Places, target: Option<(&Provider, &ProviderModel)>) -> Composed {
  let (mut doc, list) = variants(places);
  let chosen = select(&list, target);
  let mut version = doc.text("version").unwrap_or("0").to_owned();
  if let Some(v) = chosen {
    doc.overlay(&v.doc);
    version = v.doc.text("version").unwrap_or("0").to_owned();
  }
  let mut text = doc.render();
  let instructions = instructions(places);
  if !instructions.is_empty() {
    text.push_str("\n\n## Instructions from AGENTS.md\nThe user's own instructions. Where they differ from the guidance above, they win.\n");
    text.push_str(&instructions);
  }
  text.push_str(&environment(places));
  let num = |k: &str| doc.text(k).and_then(|v| v.parse::<f64>().ok());
  let sampling = Sampling { temperature: num("temperature"), top_p: num("top_p"), top_k: num("top_k").map(|k| k as u64) };
  let thinking = match doc.text("thinking") {
    Some("on") => Some(Thinking::On),
    Some("off") => Some(Thinking::Off),
    Some("auto") => Some(Thinking::Auto),
    _ => None,
  };
  let digest = Sha256::digest(text.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect();
  Composed { text, variant: chosen.map(|v| v.name.clone()).unwrap_or_else(|| BASE.to_owned()), version, digest, sampling, thinking }
}

fn instructions(places: &Places) -> String {
  let mut out = String::new();
  for path in places.instructions() {
    let Ok(text) = std::fs::read_to_string(&path) else { continue };
    let text = text.trim();
    if text.is_empty() {
      continue;
    }
    let body = if text.len() > MAX_INSTRUCTIONS {
      format!("{}\n[Cut at {} KB; read the file for the rest.]", crate::budget::cut(text, MAX_INSTRUCTIONS), MAX_INSTRUCTIONS / 1024)
    } else {
      text.to_owned()
    };
    out.push_str(&format!("\n<instructions path=\"{}\">\n{body}\n</instructions>\n", path.display()));
  }
  out
}

fn environment(places: &Places) -> String {
  let (shell, _) = crate::tools::bash::shell();
  let shell = Path::new(&shell).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(shell);
  format!(
    "\n\n## Environment\n- Session folder: {}\n- Platform: {} ({})\n- Shell for the bash tool: {shell}\n- Git repository: {}\n",
    places.cwd.display(),
    std::env::consts::OS,
    std::env::consts::ARCH,
    match &places.root {
      Some(r) if r != &places.cwd => format!("yes, rooted at {}", r.display()),
      Some(_) => "yes".to_owned(),
      None => "no".to_owned(),
    },
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn target(preset: &str, model: &str) -> (Provider, ProviderModel) {
    (serde_json::from_value(json!({ "id": "src", "preset": preset, "baseUrl": "https://x" })).unwrap(), ProviderModel::new(model))
  }

  fn places(dir: &Path) -> Places {
    Places::new(&dir.join("work"), Some(&dir.join("home")))
  }

  #[test]
  fn variants_match_by_pattern_and_pin_and_record_their_version() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let pl = places(dir.path());
    let (p, m) = target("deepseek", "deepseek-chat");
    let c = compose(&pl, Some((&p, &m)));
    assert_eq!((c.variant.as_str(), c.version.as_str(), c.digest.len()), ("deepseek", "1", 12));
    assert!(c.text.starts_with("You are Acpira") && c.text.contains("## Tool calls\n- A tool runs only") && c.text.contains("## Environment"));
    let (p, m) = target("custom", "my-model");
    assert_eq!(compose(&pl, Some((&p, &m))).variant, "base");
    let (p, mut m) = target("custom", "gw/Claude-Opus-5-5");
    assert_eq!(variant_name(&pl, Some((&p, &m))), "claude");
    m.family = Some("deepseek".into());
    assert_eq!(variant_name(&pl, Some((&p, &m))), "deepseek");
    // A pin that names only a request family falls back to matching
    m.family = Some("glm".into());
    assert_eq!(variant_name(&pl, Some((&p, &m))), "claude");
    assert_eq!(compose(&pl, None).variant, "base");
  }

  #[test]
  fn user_files_override_sections_add_variants_and_agents_md_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let (home, work) = (dir.path().join("home"), dir.path().join("work"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    let global = home.join(".agents/acpira/prompts");
    let project = work.join(".agents/acpira/prompts");
    std::fs::create_dir_all(&global).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(global.join("base.md"), "## Answering\n- Answer in French.\n").unwrap();
    std::fs::write(project.join("deepseek.md"), "---\nversion: 7\ntemperature: 0.3\n---\n## Tool calls\n\n## Style\n- Terse.\n").unwrap();
    std::fs::write(project.join("mine.md"), "---\nmatch: [\"*-mine\"]\nthinking: off\n---\nYou are Mine.\n").unwrap();
    std::fs::write(home.join(".agents/AGENTS.md"), "Use tabs.").unwrap();
    std::fs::write(work.join("AGENTS.md"), "Run make check.").unwrap();
    let pl = places(dir.path());
    let (p, m) = target("deepseek", "deepseek-chat");
    let c = compose(&pl, Some((&p, &m)));
    assert_eq!((c.version.as_str(), c.sampling.temperature), ("7", Some(0.3)));
    assert!(c.text.contains("## Answering\n- Answer in French.") && !c.text.contains("## Tool calls") && c.text.contains("## Style\n- Terse."));
    let global_md = c.text.find("Use tabs.").unwrap();
    assert!(global_md < c.text.find("Run make check.").unwrap(), "the user's file first, then the project's");
    let (p, m) = target("custom", "big-mine");
    let c = compose(&pl, Some((&p, &m)));
    assert_eq!((c.variant.as_str(), c.thinking), ("mine", Some(Thinking::Off)));
    assert!(c.text.starts_with("You are Mine.") && c.text.contains("## Working"));
  }
}
