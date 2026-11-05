//! The system prompt, composed in layers: the built-in `system`, the version for the model's family, the same-name
//! files of the override folders (section by section, see `doc.rs` and `library.rs`), then AGENTS.md and the
//! environment. A family is declared by its `system@<family>` file, whose `match` patterns over `<preset>/<model>`,
//! `<source id>/<model>` and the bare model id pick the models it fits; it may carry default sampling, and a model's
//! pinned `family` names it directly. Adding a family is adding a file.
//!
//! The result stays fixed for the session, so prompt caches keep hitting: it is composed again only when a model
//! switch selects another family. What changes during a session (the mode, later the date) goes into user messages

pub mod doc;
pub mod library;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use acpira_shared::providers::{Provider, ProviderModel, Sampling, Thinking};

use crate::permission::wildcard;
use doc::Doc;
pub use library::{BASE, Library};
/// The variant name Raw mode's prompt records
pub const RAW: &str = "raw";
/// An AGENTS.md longer than this is cut, with a note
const MAX_INSTRUCTIONS: usize = 32 * 1024;
/// A prompt file on an SSH host larger than this is left out of its snapshot
const MAX_HOST_FILE: u64 = 256 * 1024;

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
  /// The prompts an override folder changed for this session, each with its text's digest (`Library::overrides`)
  pub overrides: BTreeMap<String, String>,
  /// What the text is made of, in order and covering it whole (the context report splits the prompt by these). Empty for
  /// a prompt recorded before sections were: the report then counts it as one part
  pub sections: Vec<Section>,
}

/// A part of a composed prompt, as a byte range of its text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
  pub kind: SectionKind,
  /// The instruction file a `Doc` section holds
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<String>,
  pub start: usize,
  pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionKind {
  /// The agent's own instructions (the variant, a profile's prompt, Raw mode's text), with the heading over the files
  Base,
  /// One AGENTS.md file
  Doc,
  Environment,
}

/// The instruction files and the environment after `base`, each recorded as a section; the base covers what precedes
/// the first of them, the heading over the files included
fn assemble(mut text: String, heading: &str, docs: Vec<(PathBuf, String)>, environment: String) -> (String, Vec<Section>) {
  if !docs.is_empty() {
    text.push_str(heading);
  }
  let mut sections = vec![Section { kind: SectionKind::Base, path: None, start: 0, end: text.len() }];
  for (path, block) in docs {
    let start = text.len();
    text.push_str(&block);
    sections.push(Section { kind: SectionKind::Doc, path: Some(path.display().to_string()), start, end: text.len() });
  }
  let start = text.len();
  text.push_str(&environment);
  sections.push(Section { kind: SectionKind::Environment, path: None, start, end: text.len() });
  (text, sections)
}

/// Where a session's prompt files are
pub struct Places {
  pub cwd: PathBuf,
  /// The user's home (`~/.agents` lives there)
  pub home: Option<PathBuf>,
  /// Acpira's data home (`~/.acpira`): its `prompts/` is the user's override folder, read from this disk
  pub data: Option<PathBuf>,
  /// The nearest ancestor of cwd holding `.git`, home excluded
  pub root: Option<PathBuf>,
  /// A folder on an SSH host: its files as the host read them, never this disk
  host: Option<Arc<HostFiles>>,
  /// The prompt folders of the engine's plugins in effect when the session opened (`acp.rs` `LAYERS_META`): between
  /// the user's overrides and the project's, read from this disk
  pub layers: Vec<PathBuf>,
  /// The skill folders of the engine's plugins in effect when the session opened (`acp.rs` `SKILLS_META`): between the
  /// user's skills and the project's, read from this disk
  pub skill_layers: Vec<PathBuf>,
  /// A plugin's specialised agent: its prompt in place of the built-in base and its variants (`profile.rs`)
  pub profile: Option<Arc<crate::profile::Profile>>,
  /// The session's scratch folder (`scratch.rs`), named in the environment section
  pub scratch: Option<PathBuf>,
}

/// A session folder's prompt files as its SSH host read them (`gather`, run there by the engine's helper at session
/// start, docs/dev/remote.md): AGENTS.md, the project's prompt overrides and skills, with the host's home, project root
/// and shell
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostFiles {
  pub home: Option<PathBuf>,
  pub root: Option<PathBuf>,
  pub os: String,
  pub arch: String,
  pub shell: String,
  pub files: Vec<HostFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HostFile {
  pub path: PathBuf,
  pub text: String,
}

impl HostFiles {
  /// Read on the host: every file `Places` would read for `cwd` there
  pub fn gather(cwd: &Path, home: Option<&Path>) -> HostFiles {
    let places = Places::new(cwd, home);
    let mut paths = places.instructions();
    for dir in places.prompt_dirs() {
      paths.extend(places.md_files(&dir, library::MAX_DEPTH).into_iter().map(|(_, p)| p));
    }
    for root in crate::skills::roots(&places) {
      paths.extend(places.entries(&root, true).into_iter().map(|d| d.join("SKILL.md")));
    }
    let files = paths
      .into_iter()
      .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= MAX_HOST_FILE))
      .filter_map(|path| Some(HostFile { text: std::fs::read_to_string(&path).ok()?, path }))
      .collect();
    HostFiles {
      home: places.home,
      root: places.root,
      os: std::env::consts::OS.to_owned(),
      arch: std::env::consts::ARCH.to_owned(),
      shell: crate::tools::bash::shell().0,
      files,
    }
  }
}

impl Places {
  pub fn new(cwd: &Path, home: Option<&Path>) -> Places {
    Places { cwd: cwd.to_path_buf(), home: home.map(Path::to_path_buf), data: None, root: project_root(cwd, home), host: None, layers: vec![], skill_layers: vec![], profile: None, scratch: None }
  }

  /// A folder on an SSH host, read from what the host sent
  pub fn on_host(cwd: &Path, files: Arc<HostFiles>) -> Places {
    Places { cwd: cwd.to_path_buf(), home: files.home.clone(), data: None, root: files.root.clone(), host: Some(files), layers: vec![], skill_layers: vec![], profile: None, scratch: None }
  }

  /// With Acpira's data home, whose `prompts/` is the user's override folder
  pub fn with_data(mut self, data: Option<&Path>) -> Places {
    self.data = data.map(Path::to_path_buf);
    self
  }

  /// With the session's scratch folder
  pub fn with_scratch(mut self, scratch: Option<&Path>) -> Places {
    self.scratch = scratch.map(Path::to_path_buf);
    self
  }

  /// With the plugins' prompt folders
  pub fn with_profile(mut self, profile: Option<Arc<crate::profile::Profile>>) -> Places {
    self.profile = profile;
    self
  }

  pub fn with_layers(mut self, layers: Vec<PathBuf>) -> Places {
    self.layers = layers;
    self
  }

  /// With the plugins' skill folders
  pub fn with_skill_layers(mut self, skills: Vec<PathBuf>) -> Places {
    self.skill_layers = skills;
    self
  }

  /// Whether `path` is in the user's prompt folder or a plugin's prompt or skill folder, which are always on this disk
  fn in_layer(&self, path: &Path) -> bool {
    self.user_prompts().iter().chain(&self.layers).chain(&self.skill_layers).any(|l| path.starts_with(l))
  }

  /// The user's override folder: Acpira's own settings, so a folder on an SSH host takes this machine's
  fn user_prompts(&self) -> Option<PathBuf> {
    self.data.as_ref().map(|d| d.join("prompts"))
  }

  /// A file's text
  pub(crate) fn read(&self, path: &Path) -> Option<String> {
    match &self.host {
      Some(h) if !self.in_layer(path) => h.files.iter().find(|f| f.path == path).map(|f| f.text.clone()),
      _ => std::fs::read_to_string(path).ok(),
    }
  }

  /// The folders (`dirs`) or files directly inside `dir`, sorted
  pub(crate) fn entries(&self, dir: &Path, dirs: bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match &self.host {
      Some(h) if !self.in_layer(dir) => h
        .files
        .iter()
        .filter_map(|f| {
          let rest = f.path.strip_prefix(dir).ok()?;
          let first = rest.components().next()?;
          let nested = rest.components().nth(1).is_some();
          (nested == dirs).then(|| dir.join(first))
        })
        .collect(),
      _ => match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() == dirs).collect(),
        Err(_) => vec![],
      },
    };
    out.sort();
    out.dedup();
    out
  }

  /// Override directories, the later winning: the user's (`~/.acpira/prompts`), the plugins', the project's
  /// (`.acpira/prompts` at its root, beside the scratch folders)
  pub(crate) fn prompt_dirs(&self) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = self.user_prompts().into_iter().collect();
    out.extend(self.layers.iter().cloned());
    if let Some(r) = self.root.as_ref().or(Some(&self.cwd)) {
      out.push(project_prompts(r));
    }
    out.dedup();
    out
  }

  /// The Markdown files under `dir` at most `depth` folders down, by prompt name
  fn md_files(&self, dir: &Path, depth: usize) -> Vec<(String, PathBuf)> {
    let files: Vec<PathBuf> = match &self.host {
      Some(h) if !self.in_layer(dir) => h.files.iter().filter(|f| f.path.starts_with(dir)).map(|f| f.path.clone()).collect(),
      _ => {
        let mut out = vec![];
        let mut stack = vec![(dir.to_path_buf(), 1)];
        while let Some((d, level)) = stack.pop() {
          // Links followed: a prompt file may be a link into a dotfiles repo
          for p in std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()) {
            if p.is_dir() {
              if level < depth {
                stack.push((p, level + 1));
              }
            } else if p.is_file() {
              out.push(p);
            }
          }
        }
        out
      }
    };
    library::names_in(dir, files, depth)
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

/// The project's override folder, at its root
pub fn project_prompts(root: &Path) -> PathBuf {
  root.join(".acpira").join("prompts")
}

/// The keys a family's `match` patterns are tried against, lowercase
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

/// The family a model gets, with its system prompt document
fn select<'a>(lib: &'a Library, target: Option<(&Provider, &ProviderModel)>) -> Option<(&'a str, &'a Doc)> {
  let (provider, model) = target?;
  let families = lib.families();
  // A pinned family that names a declared one decides; one that names only a request family falls through to matching
  if let Some(f) = model.family.as_deref().and_then(|pin| families.iter().find(|(name, _)| *name == pin)) {
    return Some(*f);
  }
  let keys = keys(provider, model);
  families.into_iter().find(|(_, d)| d.list("match").iter().any(|p| keys.iter().any(|k| wildcard(&p.to_lowercase(), k))))
}

/// The family a model's prompts are taken for, whatever a profile replaces (a compaction's writer, a title's model)
pub fn family(lib: &Library, target: Option<(&Provider, &ProviderModel)>) -> String {
  select(lib, target).map_or_else(|| BASE.to_owned(), |(f, _)| f.to_owned())
}

/// The family a model gets, without composing (to see whether a model switch changes the prompt)
pub fn variant_name(places: &Places, lib: &Library, target: Option<(&Provider, &ProviderModel)>) -> String {
  if let Some(p) = &places.profile {
    return p.variant();
  }
  family(lib, target)
}

/// Compose the system prompt for a model (None before any model is configured)
pub fn compose(places: &Places, lib: &Library, target: Option<(&Provider, &ProviderModel)>) -> Composed {
  if let Some(p) = &places.profile {
    return Composed { overrides: lib.overrides(), ..profiled(places, p) };
  }
  let family = select(lib, target).map_or(BASE, |(f, _)| f);
  let doc = lib.doc(library::SYSTEM, family);
  // The family's own version, else the plain prompt's (an override's wins over the built-in one either way)
  let own = (family != BASE).then(|| lib.doc(&format!("{}@{family}", library::SYSTEM), BASE));
  let version = own.as_ref().unwrap_or(&doc).text("version").unwrap_or("0").to_owned();
  let heading = "\n\n## Instructions from AGENTS.md\nThe user's own instructions. Where they differ from the guidance above, they win.\n";
  let (text, sections) = assemble(doc.render(), heading, instructions(places), environment(places));
  let num = |k: &str| doc.text(k).and_then(|v| v.parse::<f64>().ok());
  let sampling = Sampling { temperature: num("temperature"), top_p: num("top_p"), top_k: num("top_k").map(|k| k as u64) };
  let thinking = match doc.text("thinking") {
    Some("on") => Some(Thinking::On),
    Some("off") => Some(Thinking::Off),
    Some("auto") => Some(Thinking::Auto),
    _ => None,
  };
  let digest = digest(&text);
  Composed { text, variant: family.to_owned(), version, digest, sampling, thinking, overrides: lib.overrides(), sections }
}

/// A specialised agent's prompt: the profile's text, AGENTS.md when it asks for it, the environment; no variant defaults
fn profiled(places: &Places, p: &crate::profile::Profile) -> Composed {
  let docs = if p.instructions { instructions(places) } else { vec![] };
  let heading = "\n\n## Instructions from AGENTS.md\nThe user's own instructions for this folder.\n";
  let (text, sections) = assemble(p.prompt.trim_end().to_owned(), heading, docs, environment(places));
  Composed { digest: digest(&text), text, variant: p.variant(), version: "0".into(), sections, ..Default::default() }
}

/// Raw mode's prompt: the settings page's text as it is, with nothing composed around it and no variant defaults
pub fn raw(text: &str) -> Composed {
  let sections = vec![Section { kind: SectionKind::Base, path: None, start: 0, end: text.len() }];
  Composed { text: text.to_owned(), variant: RAW.to_owned(), digest: digest(text), sections, ..Default::default() }
}

/// SHA-256 of a prompt, first 12 hex digits
fn digest(text: &str) -> String {
  Sha256::digest(text.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// Each instruction file with the block the prompt carries it in
fn instructions(places: &Places) -> Vec<(PathBuf, String)> {
  let mut out = vec![];
  for path in places.instructions() {
    let Some(text) = places.read(&path) else { continue };
    let text = text.trim();
    if text.is_empty() {
      continue;
    }
    let body = if text.len() > MAX_INSTRUCTIONS {
      format!("{}\n[Cut at {} KB; read the file for the rest.]", crate::budget::cut(text, MAX_INSTRUCTIONS), MAX_INSTRUCTIONS / 1024)
    } else {
      text.to_owned()
    };
    let block = format!("\n<instructions path=\"{}\">\n{body}\n</instructions>\n", path.display());
    out.push((path, block));
  }
  out
}

fn environment(places: &Places) -> String {
  // A folder on an SSH host: the host's platform and shell, where the tools run
  let (os, arch, shell) = match &places.host {
    Some(h) => (h.os.clone(), h.arch.clone(), h.shell.clone()),
    None => (std::env::consts::OS.to_owned(), std::env::consts::ARCH.to_owned(), crate::tools::bash::shell().0),
  };
  let shell = Path::new(&shell).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(shell);
  let mut text = format!(
    "\n\n## Environment\n- Session folder: {}\n- Platform: {os} ({arch})\n- Shell for the bash tool: {shell}\n- Git repository: {}\n",
    places.cwd.display(),
    match &places.root {
      Some(r) if r != &places.cwd => format!("yes, rooted at {}", r.display()),
      Some(_) => "yes".to_owned(),
      None => "no".to_owned(),
    },
  );
  if let Some(s) = &places.scratch {
    text.push_str(&format!(
      "- Scratch folder: {}; TMPDIR, TEMP and TMP point at it. It is git-ignored and deleted with the session: put temp files and throwaway scripts there, not in /tmp or elsewhere in the project\n",
      s.display()
    ));
  }
  text
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn target(preset: &str, model: &str) -> (Provider, ProviderModel) {
    (serde_json::from_value(json!({ "id": "src", "preset": preset, "baseUrl": "https://x" })).unwrap(), ProviderModel::new(model))
  }

  fn places(dir: &Path) -> Places {
    Places::new(&dir.join("work"), Some(&dir.join("home"))).with_data(Some(&dir.join("data")))
  }

  fn composed(pl: &Places, target: Option<(&Provider, &ProviderModel)>) -> Composed {
    compose(pl, &Library::load(pl), target)
  }

  #[test]
  fn variants_match_by_pattern_and_pin_and_record_their_version() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let pl = places(dir.path());
    let lib = Library::load(&pl);
    let (p, m) = target("deepseek", "deepseek-chat");
    let c = compose(&pl, &lib, Some((&p, &m)));
    assert_eq!((c.variant.as_str(), c.version.as_str(), c.digest.len()), ("deepseek", "1", 12));
    assert!(c.text.starts_with("You are Acpira") && c.text.contains("## Tool calls\n- A tool runs only") && c.text.contains("## Environment"));
    assert!(c.overrides.is_empty());
    let (p, m) = target("custom", "my-model");
    assert_eq!(compose(&pl, &lib, Some((&p, &m))).variant, "base");
    let (p, mut m) = target("custom", "gw/Claude-Opus-5-5");
    assert_eq!(variant_name(&pl, &lib, Some((&p, &m))), "claude");
    m.family = Some("deepseek".into());
    assert_eq!(variant_name(&pl, &lib, Some((&p, &m))), "deepseek");
    // A pin that names only a request family falls back to matching
    m.family = Some("glm".into());
    assert_eq!(variant_name(&pl, &lib, Some((&p, &m))), "claude");
    assert_eq!(compose(&pl, &lib, None).variant, "base");
  }

  #[test]
  fn user_files_override_sections_add_variants_and_agents_md_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let (home, work, data) = (dir.path().join("home"), dir.path().join("work"), dir.path().join("data"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    let global = data.join("prompts");
    let project = project_prompts(&work);
    std::fs::create_dir_all(&global).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(home.join(".agents")).unwrap();
    std::fs::write(global.join("system.md"), "## Answering\n- Answer in French.\n").unwrap();
    std::fs::write(project.join("system@deepseek.md"), "---\nversion: 7\ntemperature: 0.3\n---\n## Tool calls\n\n## Style\n- Terse.\n").unwrap();
    std::fs::write(project.join("system@mine.md"), "---\nmatch: [\"*-mine\"]\nthinking: off\n---\nYou are Mine.\n").unwrap();
    std::fs::write(home.join(".agents/AGENTS.md"), "Use tabs.").unwrap();
    std::fs::write(work.join("AGENTS.md"), "Run make check.").unwrap();
    let pl = places(dir.path());
    let (p, m) = target("deepseek", "deepseek-chat");
    let c = composed(&pl, Some((&p, &m)));
    assert_eq!((c.version.as_str(), c.sampling.temperature), ("7", Some(0.3)));
    assert!(c.text.contains("## Answering\n- Answer in French.") && !c.text.contains("## Tool calls") && c.text.contains("## Style\n- Terse."));
    assert_eq!(c.overrides.keys().collect::<Vec<_>>(), ["system", "system@deepseek", "system@mine"]);
    let global_md = c.text.find("Use tabs.").unwrap();
    assert!(global_md < c.text.find("Run make check.").unwrap(), "the user's file first, then the project's");
    let (p, m) = target("custom", "big-mine");
    let c = composed(&pl, Some((&p, &m)));
    assert_eq!((c.variant.as_str(), c.thinking), ("mine", Some(Thinking::Off)));
    assert!(c.text.starts_with("You are Mine.") && c.text.contains("## Working"));

    // A folder on an SSH host composes the same prompt from what the host read (`HostFiles::gather` there), with the
    // user's folder from this disk
    let sent = Arc::new(HostFiles::gather(&work, Some(&home)));
    std::fs::remove_dir_all(&work).unwrap();
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(composed(&Places::on_host(&work, sent).with_data(Some(&data)), Some((&p, &m))), c);
  }

  #[test]
  fn sections_cover_the_prompt_one_per_instruction_file() {
    let dir = tempfile::tempdir().unwrap();
    let (home, work) = (dir.path().join("home"), dir.path().join("work"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    std::fs::create_dir_all(home.join(".agents")).unwrap();
    std::fs::write(home.join(".agents/AGENTS.md"), "Use tabs.").unwrap();
    std::fs::write(work.join("AGENTS.md"), "## Rules\nRun make check.").unwrap();
    let c = composed(&places(dir.path()), None);
    let kinds: Vec<SectionKind> = c.sections.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [SectionKind::Base, SectionKind::Doc, SectionKind::Doc, SectionKind::Environment]);
    assert!(c.sections.windows(2).all(|w| w[0].end == w[1].start) && c.sections[0].start == 0 && c.sections[3].end == c.text.len());
    let doc = c.text.get(c.sections[2].start..c.sections[2].end).unwrap();
    assert!(doc.contains("## Rules\nRun make check.") && c.sections[2].path.as_deref().is_some_and(|p| p.ends_with("AGENTS.md")), "{doc}");
    assert!(c.text.get(c.sections[3].start..).unwrap().contains("## Environment"));
    assert_eq!(raw("x").sections.len(), 1);
  }

  #[test]
  fn a_plugin_s_prompt_folder_sits_between_the_user_s_and_the_project_s() {
    let dir = tempfile::tempdir().unwrap();
    let (home, work, data, layer) = (dir.path().join("home"), dir.path().join("work"), dir.path().join("data"), dir.path().join("plugin/prompts"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    for d in [data.join("prompts"), project_prompts(&work), layer.clone()] {
      std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(data.join("prompts/system.md"), "## Answering\n- From the user.\n## Style\n- User style.\n").unwrap();
    std::fs::write(layer.join("system.md"), "## Answering\n- From the plugin.\n## Style\n- Plugin style.\n").unwrap();
    std::fs::write(project_prompts(&work).join("system.md"), "## Style\n- Project style.\n").unwrap();
    let pl = Places::new(&work, Some(&home)).with_data(Some(&data)).with_layers(vec![layer.clone()]);
    let c = composed(&pl, None);
    assert!(c.text.contains("- From the plugin.") && !c.text.contains("- From the user."), "{}", c.text);
    assert!(c.text.contains("- Project style.") && !c.text.contains("- Plugin style."), "{}", c.text);
    // A folder on an SSH host still reads the plugin's folder from this disk
    let sent = Arc::new(HostFiles::gather(&work, Some(&home)));
    assert!(composed(&Places::on_host(&work, sent).with_layers(vec![layer]), None).text.contains("- From the plugin."));
  }
}
