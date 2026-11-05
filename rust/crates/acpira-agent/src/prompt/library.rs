//! Every text Acpira writes for a model, by name: the system prompt, mode notes, compaction, goals, titles, steering,
//! the sidekick, the MCP server's instructions and the edit / fork / relay leads. Each is a Markdown file under
//! `builtin/` (its path without `.md` is its name), compiled in, with the same-name files of the override folders
//! laid over it (`Places::prompt_dirs`: Acpira's data home, the plugins', the project's).
//!
//! `<name>@<family>.md` is the version for one model family, laid over `<name>`. A family is declared by its system
//! prompt, `system@<family>.md`, whose `match` patterns pick the models it fits (`super::select`); every other prompt
//! may then have a version for it. The engine's own texts (MCP, leads) go to external agents and take no family.
//!
//! A prompt's `vars` frontmatter names the `{placeholders}` the code fills in. A text without all of them (an
//! override that dropped one) falls back to the built-in one, since the caller depends on them; braces that name no
//! var stay as written

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::Places;
use super::doc::Doc;

/// The built-in prompts: (name, text)
const BUILTIN: &[(&str, &str)] = &[
  ("system", include_str!("builtin/system.md")),
  ("system@claude", include_str!("builtin/system@claude.md")),
  ("system@deepseek", include_str!("builtin/system@deepseek.md")),
  ("mode/agent", include_str!("builtin/mode/agent.md")),
  ("mode/plan", include_str!("builtin/mode/plan.md")),
  ("mode/team", include_str!("builtin/mode/team.md")),
  ("mode/read-only", include_str!("builtin/mode/read-only.md")),
  ("plan/approved", include_str!("builtin/plan/approved.md")),
  ("plan/rejected", include_str!("builtin/plan/rejected.md")),
  ("compact/system", include_str!("builtin/compact/system.md")),
  ("compact/instructions", include_str!("builtin/compact/instructions.md")),
  ("compact/previous", include_str!("builtin/compact/previous.md")),
  ("compact/omitted", include_str!("builtin/compact/omitted.md")),
  ("compact/length", include_str!("builtin/compact/length.md")),
  ("goal/set", include_str!("builtin/goal/set.md")),
  ("goal/resume", include_str!("builtin/goal/resume.md")),
  ("goal/continue", include_str!("builtin/goal/continue.md")),
  ("goal/no-progress", include_str!("builtin/goal/no-progress.md")),
  ("goal/rules", include_str!("builtin/goal/rules.md")),
  ("title/system", include_str!("builtin/title/system.md")),
  ("steer/note", include_str!("builtin/steer/note.md")),
  ("sidekick/lead", include_str!("builtin/sidekick/lead.md")),
  ("sidekick/brief", include_str!("builtin/sidekick/brief.md")),
  ("sidekick/added", include_str!("builtin/sidekick/added.md")),
  ("sidekick/changed", include_str!("builtin/sidekick/changed.md")),
  ("sidekick/removed", include_str!("builtin/sidekick/removed.md")),
  ("mcp/base", include_str!("builtin/mcp/base.md")),
  ("mcp/relay", include_str!("builtin/mcp/relay.md")),
  ("mcp/sessions", include_str!("builtin/mcp/sessions.md")),
  ("mcp/plugins", include_str!("builtin/mcp/plugins.md")),
  ("mcp/caller", include_str!("builtin/mcp/caller.md")),
  ("mcp/browser", include_str!("builtin/mcp/browser.md")),
  ("edit/history", include_str!("builtin/edit/history.md")),
  ("edit/continue", include_str!("builtin/edit/continue.md")),
  ("fork/history", include_str!("builtin/fork/history.md")),
  ("relay/pack", include_str!("builtin/relay/pack.md")),
  ("relay/consult", include_str!("builtin/relay/consult.md")),
  ("relay/frame", include_str!("builtin/relay/frame.md")),
  ("relay/closing-review", include_str!("builtin/relay/closing-review.md")),
  ("relay/closing-consult", include_str!("builtin/relay/closing-consult.md")),
  ("relay/closing-work", include_str!("builtin/relay/closing-work.md")),
];

/// The system prompt's name; `system@<family>` declares a family
pub const SYSTEM: &str = "system";
/// The family of a model no family's `match` takes: the plain prompts alone
pub const BASE: &str = "base";
/// Folders an override folder is searched to: `mode/plan.md` sits one below the top
pub(super) const MAX_DEPTH: usize = 3;

/// The prompts of one session (or one engine request): built-in, with the override folders' files over them. Read
/// once and kept, so a file edited meanwhile changes nothing until the next session and prompt caches keep hitting
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
  /// Each name's built-in document (when it has one) with every layer's file laid over it in order
  docs: BTreeMap<String, Doc>,
  /// The built-in documents alone: the fallback for a text that lost a var
  builtin: BTreeMap<String, Doc>,
  /// Names a layer's file changed, each with the digest of the text it gives (`overrides`)
  changed: Vec<String>,
  /// Families declared or changed by a layer, in the order first seen: they match before the built-in ones
  own_families: Vec<String>,
}

impl Default for Library {
  fn default() -> Self {
    Library::builtin()
  }
}

impl Library {
  /// The built-in prompts alone
  pub fn builtin() -> Library {
    let builtin: BTreeMap<String, Doc> = BUILTIN.iter().map(|(name, text)| ((*name).to_owned(), Doc::parse(text))).collect();
    Library { docs: builtin.clone(), builtin, changed: vec![], own_families: vec![] }
  }

  /// The built-in prompts with the override folders of `places` over them
  pub fn load(places: &Places) -> Library {
    let mut lib = Library::builtin();
    for dir in places.prompt_dirs() {
      for (name, path) in places.md_files(&dir, MAX_DEPTH) {
        let Some(text) = places.read(&path) else { continue };
        lib.lay(&name, &Doc::parse(&text));
      }
    }
    lib
  }

  /// Lay one override file over its name
  fn lay(&mut self, name: &str, doc: &Doc) {
    let (base, family) = split(name);
    if !self.builtin.contains_key(base) {
      // A family version of a prompt Acpira does not have, or a misspelt name: nothing would ever read it
      eprintln!("[prompts] {name}.md names no built-in prompt and is left out");
      return;
    }
    match self.docs.get_mut(name) {
      Some(d) => d.overlay(doc),
      None => {
        self.docs.insert(name.to_owned(), doc.clone());
      }
    }
    if !self.changed.iter().any(|c| c == name) {
      self.changed.push(name.to_owned());
    }
    if base == SYSTEM
      && let Some(f) = family
      && !self.own_families.iter().any(|o| o == f)
    {
      self.own_families.push(f.to_owned());
    }
  }

  /// The declared families with their system prompt documents: the ones a layer declared or changed first (they take
  /// over a match), then the built-in ones in table order
  pub fn families(&self) -> Vec<(&str, &Doc)> {
    let built_in = BUILTIN.iter().filter_map(|(name, _)| name.strip_prefix("system@")).filter(|f| !self.own_families.iter().any(|o| o == f));
    self
      .own_families
      .iter()
      .map(String::as_str)
      .chain(built_in)
      .filter_map(|f| self.docs.get(&format!("{SYSTEM}@{f}")).map(|d| (f, d)))
      .collect()
  }

  /// A prompt's document for a family: the plain one with the family's version over it
  pub fn doc(&self, name: &str, family: &str) -> Doc {
    compose(&self.docs, name, family)
  }

  /// A prompt's text for a family, with no vars to fill
  pub fn text(&self, name: &str, family: &str) -> String {
    self.render(name, family, &[])
  }

  /// A prompt's text for a family with its `{vars}` filled in, in one pass (a value is never searched again). A text
  /// missing one of the vars its built-in declares is replaced by the built-in text
  pub fn render(&self, name: &str, family: &str, vars: &[(&str, &str)]) -> String {
    debug_assert!(self.builtin.contains_key(name), "unknown prompt {name}");
    let text = self.doc(name, family).render();
    let declared = self.builtin.get(name).map(|d| d.list("vars")).unwrap_or_default();
    let missing: Vec<&String> = declared.iter().filter(|v| !text.contains(&format!("{{{v}}}"))).collect();
    let text = if missing.is_empty() {
      text
    } else {
      eprintln!("[prompts] {name} for {family} leaves out {missing:?}: the built-in text is used instead");
      compose(&self.builtin, name, family).render()
    };
    fill(&text, vars)
  }

  /// The prompts an override folder changed, each with the first 12 hex digits of the SHA-256 of what it gives now
  /// (a family's version as written): recorded with the session so evals tell overridden runs apart
  pub fn overrides(&self) -> BTreeMap<String, String> {
    self
      .changed
      .iter()
      .map(|name| {
        let text = self.docs.get(name).map(Doc::render).unwrap_or_default();
        (name.clone(), Sha256::digest(text.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect())
      })
      .collect()
  }
}

/// `name` → (`name`, None), `name@family` → (`name`, Some(family))
fn split(name: &str) -> (&str, Option<&str>) {
  match name.split_once('@') {
    Some((base, family)) => (base, Some(family)),
    None => (name, None),
  }
}

fn compose(docs: &BTreeMap<String, Doc>, name: &str, family: &str) -> Doc {
  let mut doc = docs.get(name).cloned().unwrap_or_default();
  if family != BASE
    && let Some(top) = docs.get(&format!("{name}@{family}"))
  {
    doc.overlay(top);
  }
  doc
}

/// Replace each `{var}` of `vars` once; other braces stay
fn fill(text: &str, vars: &[(&str, &str)]) -> String {
  if vars.is_empty() {
    return text.to_owned();
  }
  let mut out = String::with_capacity(text.len());
  let mut rest = text;
  while let Some((before, after)) = rest.split_once('{') {
    out.push_str(before);
    match after.split_once('}').and_then(|(key, tail)| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| (*v, tail))) {
      Some((value, tail)) => {
        out.push_str(value);
        rest = tail;
      }
      None => {
        out.push('{');
        rest = after;
      }
    }
  }
  out.push_str(rest);
  out
}

/// The prompt names a folder's Markdown files give, with their paths: `mode/plan.md` → `mode/plan`
pub(super) fn names_in(dir: &Path, files: impl IntoIterator<Item = PathBuf>, depth: usize) -> Vec<(String, PathBuf)> {
  let mut out: Vec<(String, PathBuf)> = files
    .into_iter()
    .filter(|p| p.extension().is_some_and(|x| x == "md"))
    .filter_map(|p| {
      let rel = p.strip_prefix(dir).ok()?.with_extension("");
      let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
      (!parts.is_empty() && parts.len() <= depth).then(|| (parts.join("/"), p.clone()))
    })
    .collect();
  out.sort();
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_built_in_file_is_in_the_table_and_declares_the_vars_it_uses() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prompt/builtin");
    let mut files = vec![];
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
      for e in std::fs::read_dir(d).unwrap().flatten() {
        if e.path().is_dir() { stack.push(e.path()) } else { files.push(e.path()) }
      }
    }
    let on_disk: Vec<String> = names_in(&root, files, MAX_DEPTH).into_iter().map(|(n, _)| n).collect();
    let mut table: Vec<String> = BUILTIN.iter().map(|(n, _)| (*n).to_owned()).collect();
    table.sort();
    assert_eq!(on_disk, table);
    let lib = Library::builtin();
    for (name, _) in BUILTIN {
      let doc = lib.doc(name, BASE);
      for var in doc.list("vars") {
        assert!(doc.render().contains(&format!("{{{var}}}")), "{name} declares {var} but never uses it");
      }
    }
  }

  #[test]
  fn layers_override_by_name_and_family_and_a_text_that_drops_a_var_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let (data, work) = (dir.path().join("data"), dir.path().join("work"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    let user = data.join("prompts");
    let project = work.join(".acpira/prompts");
    std::fs::create_dir_all(user.join("mode")).unwrap();
    std::fs::create_dir_all(project.join("steer")).unwrap();
    std::fs::write(user.join("mode/plan.md"), "Plan into {plan}, carefully.").unwrap();
    std::fs::write(user.join("mode/agent@deepseek.md"), "Agent mode, DeepSeek wording.").unwrap();
    std::fs::write(user.join("mode/team.md"), "---\nvars: []\n---\nNo team here.").unwrap();
    std::fs::write(project.join("steer/note.md"), "From the project.").unwrap();
    std::fs::write(project.join("nope.md"), "Matches nothing.").unwrap();
    let lib = Library::load(&Places::new(&work, None).with_data(Some(&data)));
    assert_eq!(lib.render("mode/plan", BASE, &[("plan", "/p/plan.md")]), "Plan into /p/plan.md, carefully.");
    assert_eq!(lib.text("mode/agent", "deepseek"), "Agent mode, DeepSeek wording.");
    assert_eq!(lib.text("mode/agent", "claude"), Library::builtin().text("mode/agent", BASE));
    assert_eq!(lib.text("steer/note", BASE), "From the project.");
    assert_eq!(lib.overrides().keys().collect::<Vec<_>>(), ["mode/agent@deepseek", "mode/plan", "mode/team", "steer/note"]);

    // An override without {plan} would hide the plan file's path: the built-in text stays
    std::fs::write(user.join("mode/plan.md"), "Plan somewhere.").unwrap();
    let lib = Library::load(&Places::new(&work, None).with_data(Some(&data)));
    assert!(lib.render("mode/plan", BASE, &[("plan", "/p/plan.md")]).starts_with("Plan mode is on."));
  }

  #[test]
  fn vars_fill_in_one_pass_and_other_braces_stay() {
    assert_eq!(fill("{a} {b} {c} {a", &[("a", "{b}"), ("b", "2")]), "{b} 2 {c} {a");
    assert_eq!(fill("read_session {session, from}", &[("session", "x")]), "read_session {session, from}");
  }
}
