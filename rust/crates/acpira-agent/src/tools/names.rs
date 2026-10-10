//! Tool-name recovery. Weaker models call `read_file`, `Bash`, `functions.grep` or `run_command` for the tools they
//! were given; a name that maps to exactly one tool is taken as that tool (a few times a turn, see `turn.rs`), anything
//! else gets an error that lists the real names, so a turn never ends silently on a misspelt call

use super::{BASH, EDIT, GLOB, GREP, LIST, READ, TODO, WRITE};

pub const ALL: [&str; 8] = [READ, WRITE, EDIT, BASH, GREP, GLOB, LIST, TODO];

/// Other names models use for the tools, already folded (lowercase, letters and digits only)
const ALIASES: &[(&str, &[&str])] = &[
  (READ, &["readfile", "view", "viewfile", "cat", "openfile", "fileread", "readtextfile"]),
  (WRITE, &["writefile", "createfile", "filewrite", "writetofile", "savefile"]),
  (EDIT, &["editfile", "strreplace", "strreplaceeditor", "replace", "replaceinfile", "fileedit", "searchreplace", "modifyfile"]),
  (BASH, &["shell", "sh", "run", "runcommand", "runshell", "runshellcommand", "execute", "exec", "executecommand", "terminal", "command", "cmd", "runterminalcmd", "powershell"]),
  (GREP, &["search", "grepsearch", "rg", "ripgrep", "searchcode", "codesearch", "findinfiles", "searchfilecontent"]),
  (GLOB, &["find", "findfiles", "filesearch", "globsearch", "searchfiles", "findbyname"]),
  (LIST, &["ls", "listdir", "listdirectory", "listfiles", "dir", "tree"]),
  (TODO, &["todowrite", "todos", "updatetodos", "todolist", "writetodos", "updatetodo", "todoupdate"]),
];

#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
  /// The name as given
  Exact(&'static str),
  /// Read as this tool
  Corrected(&'static str),
  /// The message for the model
  Unknown(String),
}

fn fold(s: &str) -> String {
  s.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn by_folded(key: &str) -> Option<&'static str> {
  ALL.into_iter().find(|t| fold(t) == key).or_else(|| ALIASES.iter().find(|(_, names)| names.contains(&key)).map(|(t, _)| *t))
}

fn available() -> String {
  ALL.join(", ")
}

pub fn resolve(name: &str) -> Resolved {
  if let Some(t) = ALL.into_iter().find(|t| *t == name) {
    return Resolved::Exact(t);
  }
  // A namespace in front (`functions.read`, `default_api:read`, `tools/read`) is dropped first
  let bare = name.rsplit(['.', ':', '/']).next().unwrap_or(name);
  if let Some(t) = by_folded(&fold(bare)) {
    return Resolved::Corrected(t);
  }
  // Then the words of the name (`bash_tool`, `ReadFileTool`): a single tool among them is the one meant
  let mut words: Vec<String> = vec![];
  let mut cur = String::new();
  for c in bare.chars() {
    if !c.is_ascii_alphanumeric() || (c.is_ascii_uppercase() && !cur.is_empty() && !cur.ends_with(|p: char| p.is_ascii_uppercase())) {
      if !cur.is_empty() {
        words.push(std::mem::take(&mut cur).to_lowercase());
      }
      if !c.is_ascii_alphanumeric() {
        continue;
      }
    }
    cur.push(c);
  }
  if !cur.is_empty() {
    words.push(cur.to_lowercase());
  }
  // Adjacent pairs too, so `read_file_tool` finds `readfile`
  let pairs: Vec<String> = words.windows(2).map(|w| format!("{}{}", w[0], w[1])).collect();
  let mut hits: Vec<&'static str> = words.iter().chain(&pairs).filter_map(|w| by_folded(w)).collect();
  hits.sort_unstable();
  hits.dedup();
  match hits[..] {
    [one] => Resolved::Corrected(one),
    [] => Resolved::Unknown(format!("Unknown tool \"{name}\". Available tools: {}.", available())),
    _ => Resolved::Unknown(format!("The tool name \"{name}\" is ambiguous (it could be {}). Call one of: {}.", hits.join(" or "), available())),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_are_recovered_only_when_unique() {
    assert_eq!(resolve("read"), Resolved::Exact(READ));
    for (given, tool) in [
      ("Read", READ),
      ("read_file", READ),
      ("functions.bash", BASH),
      ("default_api:grep", GREP),
      ("RUN_COMMAND", BASH),
      ("str_replace_editor", EDIT),
      ("bash_tool", BASH),
      ("ReadFileTool", READ),
      ("list-dir", LIST),
      ("TodoWrite", TODO),
    ] {
      assert_eq!(resolve(given), Resolved::Corrected(tool), "{given}");
    }
    let Resolved::Unknown(m) = resolve("read_and_edit") else { panic!() };
    assert!(m.contains("ambiguous") && m.contains("edit or read"), "{m}");
    let Resolved::Unknown(m) = resolve("teleport") else { panic!() };
    assert!(m.contains("Unknown tool \"teleport\"") && m.contains("read, write, edit, bash, grep, glob, list, todo"), "{m}");
  }
}
