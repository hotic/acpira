//! Skills: the shared `SKILL.md` folders under `~/.agents/skills/` and the project's `.agents/skills/` (what the Shared
//! tab manages for every agent). Each one is a slash command the session advertises (`available_commands_update`,
//! next to `/compact`), so the composer lists and marks it; a `/name` anywhere in a prompt, at the start or after
//! whitespace, puts the skill's instructions behind the message, and the `skill` tool lets the model load one itself

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::prompt::{Places, doc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
  pub name: String,
  pub description: String,
  /// The frontmatter's `argument-hint`, shown under the composer while the arguments are empty
  pub hint: Option<String>,
  /// The `SKILL.md`
  pub path: PathBuf,
  /// The instructions without the frontmatter, read with the rest (a skill on an SSH host is not on this disk)
  pub body: String,
}

impl Skill {
  pub fn dir(&self) -> &Path {
    self.path.parent().unwrap_or(&self.path)
  }
}

/// The description of `/compact`, the one command of the agent's own
pub const COMPACT: &str = "compact";
const COMPACT_DESCRIPTION: &str = "Summarize the conversation so far to free context";

/// Every skill, by name, the project's over the user's. Dot-names and folders without a SKILL.md are skipped
pub fn scan(places: &Places) -> Vec<Skill> {
  let mut out: Vec<Skill> = vec![];
  for root in roots(places) {
    for dir in places.entries(&root, true) {
      let Some(folder) = dir.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')) else { continue };
      let path = dir.join("SKILL.md");
      let Some(text) = places.read(&path) else { continue };
      let (meta, body) = doc::split_frontmatter(&text);
      let text_of = |k: &str| meta.iter().rev().find(|(key, _)| key == k).and_then(|(_, v)| match v {
        doc::Meta::Text(t) => Some(t.trim().to_owned()).filter(|t| !t.is_empty()),
        doc::Meta::List(_) => None,
      });
      let name = text_of("name").filter(|n| !n.chars().any(char::is_whitespace)).unwrap_or(folder);
      let skill = Skill { name, description: text_of("description").unwrap_or_default(), hint: text_of("argument-hint"), path, body: body.trim().to_owned() };
      match out.iter_mut().find(|s| s.name == skill.name) {
        Some(s) => *s = skill,
        None => out.push(skill),
      }
    }
  }
  out.sort_by(|a, b| a.name.cmp(&b.name));
  out
}

/// The skill folders, the later winning: the user's, then the project's
pub(crate) fn roots(places: &Places) -> Vec<PathBuf> {
  let mut roots = vec![];
  if let Some(h) = &places.home {
    roots.push(h.join(".agents").join("skills"));
  }
  if let Some(r) = places.root.as_ref().or(Some(&places.cwd)) {
    roots.push(r.join(".agents").join("skills"));
  }
  roots.dedup();
  roots
}

/// The `availableCommands` of a session: `/compact` and `/goal` (`goal.rs`), then the skills
pub fn commands(skills: &[Skill]) -> Vec<Value> {
  let mut out = vec![json!({ "name": COMPACT, "description": COMPACT_DESCRIPTION }), crate::goal::slash_command()];
  for s in skills {
    let mut c = json!({ "name": s.name, "description": s.description });
    if let Some(h) = &s.hint {
      c["input"] = json!({ "hint": h });
    }
    out.push(c);
  }
  out
}

/// A `/compact` prompt: the text is the command, with or without arguments
pub fn is_compact(text: &str) -> bool {
  let t = text.trim();
  t == "/compact" || t.starts_with("/compact ")
}

/// A character that may end a `/name` token besides whitespace
fn boundary(c: char) -> bool {
  c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '，' | '。' | '！' | '？' | '；' | '：' | '）' | '」' | '』')
}

/// What may sit right before a `/` for it to start a skill: whitespace, or CJK text and its full-width punctuation, which
/// Chinese / Japanese / Korean prose writes without spaces (`然后/release`). The same ranges as `SIGIL_LEAD` in the
/// webview's `chat/slashCommands.ts`, so whatever the composer paints is what gets expanded
fn sigil_lead(c: char) -> bool {
  c.is_whitespace()
    || matches!(c,
      '\u{1100}'..='\u{11ff}' | '\u{2e80}'..='\u{2fdf}' | '\u{3000}'..='\u{30ff}' | '\u{3130}'..='\u{318f}'
      | '\u{31f0}'..='\u{31ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}'
      | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}' | '\u{20000}'..='\u{323af}')
}

/// The skills a text calls up, each once in order of first mention, with the rest of its line as the arguments. A
/// `/name` counts at the start or after whitespace / CJK text (`sigil_lead`) and ends at whitespace, punctuation or the
/// end, the same boundary the composer paints; the longest name wins where one is a prefix of another
pub fn mentioned<'a>(text: &str, skills: &'a [Skill]) -> Vec<(&'a Skill, String)> {
  let mut out: Vec<(&Skill, String)> = vec![];
  let mut prev: Option<char> = None;
  for (i, c) in text.char_indices() {
    let leading = c == '/' && prev.is_none_or(sigil_lead);
    prev = Some(c);
    if !leading {
      continue;
    }
    let rest = text.get(i + 1..).unwrap_or_default();
    let hit = skills
      .iter()
      .filter(|s| rest.strip_prefix(s.name.as_str()).is_some_and(|after| after.chars().next().is_none_or(boundary)))
      .max_by_key(|s| s.name.len());
    let Some(skill) = hit else { continue };
    if out.iter().any(|(s, _)| s.name == skill.name) {
      continue;
    }
    let after = rest.strip_prefix(skill.name.as_str()).unwrap_or_default();
    let args = after.lines().next().unwrap_or("").trim().to_owned();
    out.push((skill, args));
  }
  out
}

/// The text put behind a prompt for the skills it calls up
pub fn expansion(mentions: &[(&Skill, String)]) -> String {
  let mut out = String::new();
  for (skill, args) in mentions {
    let body = skill.body.replace("$ARGUMENTS", args);
    out.push_str(&format!("\n\n<skill name=\"{}\" path=\"{}\">\n{body}\n</skill>", skill.name, skill.path.display()));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn skills_are_scanned_mentioned_and_expanded() {
    let dir = tempfile::tempdir().unwrap();
    let (home, work) = (dir.path().join("home"), dir.path().join("work"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    for (root, folder, text) in [
      (&home, "release", "---\nname: release\ndescription: Cut a release\nargument-hint: version\n---\nTag $ARGUMENTS and push.\n"),
      (&home, "dig", "---\ndescription: user copy\n---\nuser dig\n"),
      (&work, "dig", "---\nname: dig\ndescription: project copy\n---\nproject dig\n"),
      (&work, "dig-deep", "no frontmatter\n"),
      (&work, ".hidden", "---\nname: hidden\n---\n"),
      (&work, "empty", ""),
    ] {
      let d = root.join(".agents/skills").join(folder);
      std::fs::create_dir_all(&d).unwrap();
      if !text.is_empty() {
        std::fs::write(d.join("SKILL.md"), text).unwrap();
      }
    }
    let skills = scan(&Places::new(&work, Some(&home)));
    // The same folders as an SSH host sends them (`HostFiles::gather` there): nothing is read from this disk afterwards
    let sent = std::sync::Arc::new(crate::prompt::HostFiles::gather(&work, Some(&home)));
    std::fs::remove_dir_all(&work).unwrap();
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(scan(&Places::on_host(&work, sent)), skills);
    // A dot-name and a folder without a SKILL.md are skipped; the project's dig wins over the user's
    assert_eq!(skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["dig", "dig-deep", "release"]);
    assert_eq!(skills[0].description, "project copy");
    assert_eq!(skills[2].hint.as_deref(), Some("version"));
    let cmds = commands(&skills);
    assert_eq!((&cmds[0]["name"], &cmds[1]["name"]), (&json!("compact"), &json!("goal")));
    assert_eq!(cmds[4], json!({ "name": "release", "description": "Cut a release", "input": { "hint": "version" } }));

    // Leading, mid-sentence, before punctuation; a path, a partial name and a second mention are not
    let m = mentioned("please /release 1.2 then /dig, not /tmp/release or /releaser or /release again", &skills);
    assert_eq!(m.iter().map(|(s, a)| (s.name.as_str(), a.as_str())).collect::<Vec<_>>(), [("release", "1.2 then /dig, not /tmp/release or /releaser or /release again"), ("dig", ", not /tmp/release or /releaser or /release again")]);
    let m = mentioned("/dig-deep\nsecond line", &skills);
    assert_eq!((m[0].0.name.as_str(), m[0].1.as_str()), ("dig-deep", ""));
    assert!(mentioned("a/release", &skills).is_empty());
    // CJK prose writes no space before the slash
    let m = mentioned("然后/release 1.2", &skills);
    assert_eq!((m[0].0.name.as_str(), m[0].1.as_str()), ("release", "1.2"));
    let text = expansion(&mentioned("/release 1.2", &skills));
    assert!(text.contains("<skill name=\"release\" path=") && text.contains("\nTag 1.2 and push.\n</skill>"), "{text}");
    assert_eq!(skills[1].body, "no frontmatter");
    assert!(is_compact(" /compact ") && is_compact("/compact now") && !is_compact("/compaction") && !is_compact("x /compact"));
  }
}
