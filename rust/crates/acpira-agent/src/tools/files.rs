//! The file set the search tools work on: inside a git work tree what git would track (`git ls-files` with untracked
//! files, `.gitignore` honoured), elsewhere a capped walk that skips the usual dependency and build folders. Paths are
//! relative to the search root with `/`

use std::collections::VecDeque;
use std::path::Path;
use std::process::{Command, Stdio};

use regex::Regex;

/// Never descended into by the fallback walk
const SKIP_DIRS: &[&str] =
  &[".git", "node_modules", "target", ".venv", "venv", "__pycache__", ".next", ".turbo", "dist", "build", ".gradle", ".cache", ".hg", ".svn"];

/// Files under `root`, sorted; the flag says the list was cut at `cap`
pub fn list(root: &Path, cap: usize) -> (Vec<String>, bool) {
  let mut files = git_files(root).unwrap_or_else(|| walk(root, cap + 1));
  files.sort();
  let cut = files.len() > cap;
  files.truncate(cap);
  (files, cut)
}

fn git_files(root: &Path) -> Option<Vec<String>> {
  let mut cmd = Command::new("git");
  cmd.arg("-C").arg(root).args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"]);
  cmd.stdin(Stdio::null()).stderr(Stdio::null());
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000);
  }
  let out = cmd.output().ok().filter(|o| o.status.success())?;
  let text = String::from_utf8_lossy(&out.stdout);
  // A tracked file deleted in the work tree is still listed by --cached
  Some(text.split('\0').filter(|p| !p.is_empty() && root.join(p).is_file()).map(str::to_owned).collect())
}

fn walk(root: &Path, cap: usize) -> Vec<String> {
  let mut files = vec![];
  let mut queue = VecDeque::from([root.to_path_buf()]);
  while let Some(dir) = queue.pop_front() {
    let Ok(rd) = std::fs::read_dir(&dir) else { continue };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
      if files.len() >= cap {
        return files;
      }
      let Ok(ft) = e.file_type() else { continue };
      let name = e.file_name().to_string_lossy().into_owned();
      if ft.is_dir() {
        if !SKIP_DIRS.contains(&name.as_str()) {
          queue.push_back(e.path());
        }
      } else if ft.is_file()
        && let Ok(rel) = e.path().strip_prefix(root)
      {
        files.push(rel.to_string_lossy().replace('\\', "/"));
      }
    }
  }
  files
}

/// A compiled glob: `**` crosses folders, `*` and `?` stay inside one, `{a,b}` alternates, `[...]` is a class. A pattern
/// without `/` matches the file name at any depth, like ripgrep's `--glob`
pub struct Glob {
  re: Regex,
  name_only: bool,
}

impl Glob {
  pub fn new(pattern: &str) -> Result<Glob, String> {
    let pattern = pattern.trim().trim_start_matches("./");
    if pattern.is_empty() {
      return Err("The glob pattern is empty".into());
    }
    let mut re = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut depth = 0;
    while i < chars.len() {
      let c = chars[i];
      match c {
        '*' if chars.get(i + 1) == Some(&'*') => {
          if chars.get(i + 2) == Some(&'/') {
            re.push_str("(?:.*/)?");
            i += 3;
          } else {
            re.push_str(".*");
            i += 2;
          }
          continue;
        }
        '*' => re.push_str("[^/]*"),
        '?' => re.push_str("[^/]"),
        '{' => {
          depth += 1;
          re.push_str("(?:");
        }
        '}' if depth > 0 => {
          depth -= 1;
          re.push(')');
        }
        ',' if depth > 0 => re.push('|'),
        '[' => {
          let end = chars[i + 1..].iter().position(|c| *c == ']').map(|p| p + i + 1).ok_or("Unclosed [ in the glob pattern")?;
          let mut class: String = chars[i + 1..end].iter().collect();
          if let Some(rest) = class.strip_prefix('!') {
            class = format!("^{rest}");
          }
          re.push('[');
          re.push_str(&class.replace('\\', "\\\\"));
          re.push(']');
          i = end + 1;
          continue;
        }
        other => re.push_str(&regex::escape(&other.to_string())),
      }
      i += 1;
    }
    re.push('$');
    let re = Regex::new(&re).map_err(|e| format!("Invalid glob pattern: {e}"))?;
    Ok(Glob { re, name_only: !pattern.contains('/') })
  }

  pub fn matches(&self, rel: &str) -> bool {
    if self.name_only { self.re.is_match(rel.rsplit('/').next().unwrap_or(rel)) } else { self.re.is_match(rel) }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn globs() {
    let g = |p: &str| Glob::new(p).unwrap();
    assert!(g("*.rs").matches("src/deep/a.rs") && !g("*.rs").matches("a.rsx"));
    assert!(g("src/**/*.ts").matches("src/a.ts") && g("src/**/*.ts").matches("src/x/y/a.ts") && !g("src/**/*.ts").matches("lib/a.ts"));
    assert!(g("*.{ts,tsx}").matches("a.tsx") && g("*.{ts,tsx}").matches("b.ts"));
    assert!(g("src/*.rs").matches("src/a.rs") && !g("src/*.rs").matches("src/x/a.rs"));
    assert!(g("test_?.py").matches("t/test_1.py") && g("[!a]*.md").matches("b.md") && !g("[!a]*.md").matches("a.md"));
    assert!(g("**/Cargo.toml").matches("Cargo.toml") && g("**").matches("any/thing"));
    assert!(Glob::new("[abc").is_err());
  }

  #[test]
  fn the_walk_skips_build_folders_and_git_honours_ignores() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for p in ["src/a.rs", "node_modules/x/i.js", "target/debug/b", "README.md"] {
      std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
      std::fs::write(root.join(p), "x").unwrap();
    }
    let (files, cut) = list(root, 100);
    assert_eq!((files, cut), (vec!["README.md".to_owned(), "src/a.rs".to_owned()], false));
    let (files, cut) = list(root, 1);
    assert!(cut && files.len() == 1);
    // Inside a repository the ignore file decides, not the skip list
    if Command::new("git").arg("-C").arg(root).args(["init", "-q"]).status().is_ok_and(|s| s.success()) {
      std::fs::write(root.join(".gitignore"), "README.md\n").unwrap();
      let (files, _) = list(root, 100);
      assert!(files.contains(&"node_modules/x/i.js".to_owned()) && !files.contains(&"README.md".to_owned()), "{files:?}");
    }
  }
}
