//! The handoff pack: what a summoned agent (a reviewer, a sidekick, the members of a group) gets about the changes a
//! session made, so its first round does not start by rediscovering them. Only deterministic sources, only git:
//!
//! 1. the diff of the changed files against HEAD (or a base the caller names), each hunk widened to its whole function
//!    (`git diff -W`, with git's own function rules picked per extension through a temporary attributes file);
//! 2. the full text of changed files up to a line limit;
//! 3. optionally, where the symbols whose definition lines changed are used elsewhere (`git grep -n -w`), capped;
//! 4. optionally, the paths the author read (off by default: a reviewer should not inherit the author's blind spots).
//!
//! Over the size cap the pack degrades in a fixed order: full texts go first (largest first), then whole functions
//! become plain hunks, and last only the paths and line counts stay, with the plain diffs of the smallest files that
//! still fit. The recipe (`Recipe`, a JSON file for `acpira handoff-pack --recipe`) is the policy the evaluation tunes.
//! The same `build` serves the relay (`ask_agent` `changes`), groups, the sidekick and the evaluation scripts

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// git's empty tree: the base of a repository without any commit yet
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// Per-file git processes running at once
const PARALLEL: usize = 8;
/// A file this large (bytes) is never read in whole, whatever its line count
const FULL_TEXT_BYTES: usize = 64 * 1024;
/// Definitions whose references are looked up, at most
const REF_NAMES: usize = 16;
/// References listed per definition, at most
const REFS_PER_NAME: usize = 12;
/// A name used more often than this is too common to list: only its count is given
const REFS_TOO_MANY: usize = 60;
const REF_LINE_MAX: usize = 160;
const READ_PATHS_MAX: usize = 60;

/// git's own function rules (`userdiff.c`) by extension; any other file keeps git's default rule, which already finds
/// top-level declarations of most C-like languages (JS / TS included)
const DRIVERS: [(&str, &str); 33] = [
  ("rs", "rust"),
  ("py", "python"),
  ("pyi", "python"),
  ("go", "golang"),
  ("java", "java"),
  ("kt", "kotlin"),
  ("kts", "kotlin"),
  ("c", "cpp"),
  ("h", "cpp"),
  ("cc", "cpp"),
  ("cpp", "cpp"),
  ("cxx", "cpp"),
  ("hpp", "cpp"),
  ("hh", "cpp"),
  ("m", "objc"),
  ("mm", "objc"),
  ("cs", "csharp"),
  ("rb", "ruby"),
  ("php", "php"),
  ("pl", "perl"),
  ("pm", "perl"),
  ("sh", "bash"),
  ("bash", "bash"),
  ("zsh", "bash"),
  ("css", "css"),
  ("scss", "css"),
  ("html", "html"),
  ("htm", "html"),
  ("md", "markdown"),
  ("ex", "elixir"),
  ("exs", "elixir"),
  ("scm", "scheme"),
  ("tex", "tex"),
];

/// Files whose diffs say nothing to a reader: they appear in the line counts only
const STAT_ONLY: [&str; 11] = [
  "Cargo.lock",
  "pnpm-lock.yaml",
  "package-lock.json",
  "yarn.lock",
  "bun.lockb",
  "go.sum",
  "poetry.lock",
  "uv.lock",
  "Gemfile.lock",
  "composer.lock",
  "Pipfile.lock",
];

/// What goes into a pack and how big it may get: the strategy the evaluation compares and tunes. Every field has a
/// default, so a recipe file names only what it changes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Recipe {
  /// Widen each hunk to its whole function (`git diff -W`); false = plain three-line hunks from the start
  pub function_context: bool,
  /// Changed files up to this many lines go in whole; 0 = never
  pub full_text_lines: usize,
  /// List where the symbols whose definition lines changed are used elsewhere
  pub refs: bool,
  /// Reference lines in all, at most
  pub refs_max: usize,
  /// List the paths the author read (the session's read and search rows)
  pub read_paths: bool,
  /// The pack's size cap in bytes, after which it degrades
  pub max_bytes: usize,
}

impl Default for Recipe {
  fn default() -> Self {
    Recipe { function_context: true, full_text_lines: 400, refs: false, refs_max: 60, read_paths: false, max_bytes: 60_000 }
  }
}

impl Recipe {
  /// A recipe file: JSON with any of the fields; unknown ones are ignored, numbers are clamped to sane bounds
  pub fn from_json(text: &str) -> Result<Recipe> {
    let r: Recipe = serde_json::from_str(text).map_err(|e| anyhow!("bad recipe: {e}"))?;
    Ok(r.clamped())
  }

  pub fn clamped(mut self) -> Recipe {
    self.full_text_lines = self.full_text_lines.min(5_000);
    self.refs_max = self.refs_max.min(500);
    // Under 2 KB nothing useful fits; over the text attachment limit the pack could not be sent
    self.max_bytes = self.max_bytes.clamp(2_000, acpira_shared::attachments::MAX_TEXT_BYTES / 2);
    self
  }
}

/// Which files a pack covers. `None` (the CLI, the evaluation) is every change since the base, untracked files included
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
  /// Files the session's edit rows named (absolute, or relative to the session's folder)
  pub files: Vec<String>,
  /// The session ran tools that can write anywhere (a command, an MCP tool, a subagent): every uncommitted change
  /// joins the named files, the rule the workspace gate's `row_reach` follows
  pub anywhere: bool,
  /// Paths the author read, for `Recipe::read_paths`
  pub read: Vec<String>,
}

/// How far a pack had to degrade to fit its cap
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
  /// Whole functions (or plain hunks, when the recipe asks for them) with every full text that was due
  Full,
  /// Some or all full texts dropped, largest first
  Trimmed,
  /// Plain three-line hunks, no full texts
  Plain,
  /// Paths and line counts, with the plain diffs of the smallest files that still fit
  Stat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
  /// Repository-relative path (the new one for a rename)
  pub path: String,
  /// `A` added (or untracked), `M` modified, `D` deleted, `R` renamed, `T` type changed
  pub status: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub old_path: Option<String>,
  /// Lines added / removed; None for a binary file
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub added: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub removed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
  pub text: String,
  pub files: Vec<PackFile>,
  pub level: Level,
  /// The base the diff is against, as a full object id (the empty tree for a repository without commits)
  pub base: String,
  /// The work tree's top level
  pub root: String,
  pub bytes: usize,
}

/// One changed file as the pack sees it
#[derive(Debug, Clone)]
struct Changed {
  path: String,
  status: char,
  old_path: Option<String>,
  untracked: bool,
}

async fn git_raw(dir: &Path, args: &[&str]) -> Result<(i32, Vec<u8>)> {
  let mut cmd = tokio::process::Command::new("git");
  cmd.arg("-C").arg(dir).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
  // Never a pager, never a prompt for credentials or an editor
  cmd.env("GIT_PAGER", "cat").env("GIT_TERMINAL_PROMPT", "0").env("GIT_OPTIONAL_LOCKS", "0");
  #[cfg(windows)]
  {
    cmd.creation_flags(0x0800_0000);
  }
  let out = cmd.output().await.map_err(|e| anyhow!("git could not run: {e}"))?;
  Ok((out.status.code().unwrap_or(-1), out.stdout))
}

async fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
  match git_raw(dir, args).await? {
    (0, out) => Ok(out),
    (code, _) => Err(anyhow!("git {} failed (exit {code})", args.first().copied().unwrap_or(""))),
  }
}

fn text(out: Vec<u8>) -> String {
  String::from_utf8_lossy(&out).into_owned()
}

/// The attributes file that maps extensions to git's own function rules. The content never changes, so one copy in
/// the temp directory serves every pack (written tmp + rename, so a concurrent reader never sees half of it)
fn attributes_file() -> Result<PathBuf> {
  let body: String = DRIVERS.iter().map(|(ext, driver)| format!("*.{ext} diff={driver}\n")).collect();
  let path = std::env::temp_dir().join("acpira-handoff.gitattributes");
  if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, &body)?;
    std::fs::rename(&tmp, &path)?;
  }
  Ok(path)
}

/// The work tree's top level and the base commit (HEAD unless named; the empty tree before the first commit)
async fn locate(cwd: &Path, base: Option<&str>) -> Result<Option<(PathBuf, String)>> {
  let Ok(top) = git(cwd, &["rev-parse", "--show-toplevel"]).await else { return Ok(None) };
  let top = PathBuf::from(text(top).trim());
  let want = base.map(str::trim).filter(|b| !b.is_empty()).unwrap_or("HEAD");
  let spec = format!("{want}^{{commit}}");
  let base = match git(&top, &["rev-parse", "--verify", "--quiet", &spec]).await {
    Ok(id) => text(id).trim().to_owned(),
    Err(_) if want == "HEAD" => EMPTY_TREE.to_owned(),
    Err(_) => return Err(anyhow!("{want} is not a commit in {}", top.display())),
  };
  Ok(Some((top, base)))
}

/// Every change of the work tree against the base: tracked ones from `git diff --name-status`, then untracked files
async fn changes(top: &Path, base: &str) -> Result<Vec<Changed>> {
  let mut out = vec![];
  let raw = git(top, &["-c", "core.quotepath=false", "diff", "--name-status", "-z", "-M", "--no-ext-diff", base, "--"]).await?;
  let mut parts = raw.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned());
  while let Some(status) = parts.next() {
    let kind = status.chars().next().unwrap_or('M');
    if matches!(kind, 'R' | 'C') {
      let (Some(old), Some(new)) = (parts.next(), parts.next()) else { break };
      out.push(Changed { path: new, status: if kind == 'R' { 'R' } else { 'A' }, old_path: (kind == 'R').then_some(old), untracked: false });
    } else if let Some(path) = parts.next() {
      out.push(Changed { path, status: kind, old_path: None, untracked: false });
    }
  }
  let raw = git(top, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
  for p in raw.split(|b| *b == 0).filter(|p| !p.is_empty()) {
    out.push(Changed { path: String::from_utf8_lossy(p).into_owned(), status: 'A', old_path: None, untracked: true });
  }
  out.sort_by(|a, b| a.path.cmp(&b.path));
  out.dedup_by(|a, b| a.path == b.path);
  Ok(out)
}

/// A path as the repository names it: relative to the top level, `/`-separated; None when it lies outside
fn repo_relative(top: &Path, cwd: &Path, p: &str) -> Option<String> {
  let p = p.trim();
  if p.is_empty() {
    return None;
  }
  let path = Path::new(p);
  let abs = if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) };
  let canon = |x: &Path| std::fs::canonicalize(x).unwrap_or_else(|_| x.to_path_buf());
  // The file itself may be gone (a deletion): its folder settles the spelling
  let abs = match (abs.parent(), abs.file_name()) {
    (Some(dir), Some(name)) if !abs.exists() => canon(dir).join(name),
    _ => canon(&abs),
  };
  let rel = abs.strip_prefix(canon(top)).ok()?;
  let s = rel.to_string_lossy().replace('\\', "/");
  (!s.is_empty()).then_some(s)
}

fn is_stat_only(path: &str) -> bool {
  let name = path.rsplit('/').next().unwrap_or(path);
  STAT_ONLY.contains(&name) || name.ends_with(".min.js") || name.ends_with(".min.css") || name.ends_with(".map")
}

/// One file's diff: tracked files against the base, untracked ones against nothing
async fn file_diff(top: PathBuf, attrs: PathBuf, base: String, c: Changed, whole: bool) -> String {
  let attrs_cfg = format!("core.attributesFile={}", attrs.display());
  let ctx = if whole { "-W" } else { "-U3" };
  let mut args = vec!["-c", attrs_cfg.as_str(), "-c", "core.quotepath=false", "-c", "diff.noprefix=false", "diff", "--no-color", "--no-ext-diff", "--no-textconv", ctx];
  let r = if c.untracked {
    args.extend(["--no-index", "--", null_device(), c.path.as_str()]);
    git_raw(&top, &args).await
  } else {
    args.extend(["-M", base.as_str(), "--"]);
    if let Some(old) = &c.old_path {
      args.push(old.as_str());
    }
    args.push(c.path.as_str());
    git_raw(&top, &args).await
  };
  // `--no-index` answers 1 when the files differ, which they always do here
  match r {
    Ok((0 | 1, out)) => text(out),
    _ => String::new(),
  }
}

fn null_device() -> &'static str {
  if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// Run `f` over every file, `PARALLEL` git processes at a time, answering in the files' order
async fn per_file<F, Fut>(files: &[Changed], f: F) -> Vec<String>
where
  F: Fn(Changed) -> Fut,
  Fut: std::future::Future<Output = String> + Send + 'static,
{
  let mut out = vec![String::new(); files.len()];
  for (chunk_no, chunk) in files.chunks(PARALLEL).enumerate() {
    let mut set = tokio::task::JoinSet::new();
    for (i, c) in chunk.iter().enumerate() {
      let fut = f(c.clone());
      set.spawn(async move { (i, fut.await) });
    }
    while let Some(Ok((i, d))) = set.join_next().await {
      out[chunk_no * PARALLEL + i] = d;
    }
  }
  out
}

/// Added / removed lines per path; a binary file has none
async fn numstat(top: &Path, base: &str, files: &[Changed]) -> BTreeMap<String, (Option<u64>, Option<u64>)> {
  let mut out = BTreeMap::new();
  let tracked: Vec<&Changed> = files.iter().filter(|c| !c.untracked).collect();
  if !tracked.is_empty() {
    let mut args: Vec<&str> = vec!["-c", "core.quotepath=false", "diff", "--numstat", "-z", "-M", "--no-ext-diff", base, "--"];
    for c in &tracked {
      if let Some(old) = &c.old_path {
        args.push(old.as_str());
      }
      args.push(c.path.as_str());
    }
    if let Ok(raw) = git(top, &args).await {
      let mut parts = raw.split(|b| *b == 0).map(|p| String::from_utf8_lossy(p).into_owned());
      while let Some(entry) = parts.next() {
        if entry.is_empty() {
          continue;
        }
        let mut cols = entry.splitn(3, '\t');
        let (a, r, path) = (cols.next().unwrap_or(""), cols.next().unwrap_or(""), cols.next().unwrap_or(""));
        // A rename leaves the path column empty and names both paths next
        let path = if path.is_empty() {
          let _old = parts.next();
          parts.next().unwrap_or_default()
        } else {
          path.to_owned()
        };
        out.insert(path, (a.parse().ok(), r.parse().ok()));
      }
    }
  }
  for c in files.iter().filter(|c| c.untracked) {
    let n = std::fs::read(top.join(&c.path)).ok().filter(|b| !looks_binary(b)).map(|b| String::from_utf8_lossy(&b).lines().count() as u64);
    out.insert(c.path.clone(), (n, n.map(|_| 0)));
  }
  out
}

fn looks_binary(bytes: &[u8]) -> bool {
  bytes.iter().take(8000).any(|b| *b == 0)
}

/// The full text of a changed file when it is small enough: (path, text, line count)
fn full_text(top: &Path, c: &Changed, max_lines: usize) -> Option<(String, String, usize)> {
  if max_lines == 0 || c.status == 'D' || is_stat_only(&c.path) {
    return None;
  }
  let bytes = std::fs::read(top.join(&c.path)).ok()?;
  if bytes.len() > FULL_TEXT_BYTES || looks_binary(&bytes) {
    return None;
  }
  let text = String::from_utf8(bytes).ok()?;
  let lines = text.lines().count();
  (lines <= max_lines && lines > 0).then(|| (c.path.clone(), text, lines))
}

static DEFINITION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
  regex::Regex::new(
    r#"(?x)^[+-](?P<indent>\s*)
    (?:(?:pub(?:\([^)]*\))?|export|default|async|unsafe|abstract|final|public|private|protected|internal|override|open|declare|extern(?:\s+"[^"]*")?)\s+)*
    (?P<kw>fn|struct|enum|trait|union|type|mod|macro_rules!|function\*?|class|interface|def|func|fun|protocol|record|object|const|static)\s+
    (?:\([^)]*\)\s*)?
    (?P<name>[A-Za-z_$][\w$]*)"#,
  )
  .unwrap()
});

/// A class member written as a signature with its body opening on the line (TS / JS / Java / C# / Kotlin)
static METHOD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
  regex::Regex::new(
    r"^[+-]\s+(?:(?:public|private|protected|internal|static|async|readonly|override|final|abstract|virtual|get|set)\s+)*(?P<name>[A-Za-z_$][\w$]*)\s*(?:<[^>()]*>)?\([^)]*\)\s*(?::\s*[^={]+)?\{\s*$",
  )
  .unwrap()
});

const NOT_NAMES: [&str; 12] = ["if", "for", "while", "switch", "catch", "return", "function", "else", "constructor", "new", "await", "with"];
/// Trait and framework method names every codebase defines many times over: listing their uses says nothing
const COMMON_NAMES: [&str; 24] = [
  "default", "from", "into", "fmt", "drop", "clone", "eq", "ne", "cmp", "partial_cmp", "hash", "next", "deref", "deref_mut",
  "as_ref", "borrow", "to_string", "try_from", "from_str", "serialize", "deserialize", "main", "render", "toString",
];

/// A definition a diff touches: on removed lines, on added lines, or both (changed in place)
#[derive(Debug, Clone, PartialEq)]
struct Definition {
  name: String,
  removed: bool,
  added: bool,
}

/// Names whose definition lines a diff adds or removes, in order of appearance
fn changed_definitions(diff: &str) -> Vec<Definition> {
  let mut out: Vec<Definition> = vec![];
  for line in diff.lines() {
    if line.starts_with("+++") || line.starts_with("---") || !(line.starts_with('+') || line.starts_with('-')) {
      continue;
    }
    let name = if let Some(m) = DEFINITION.captures(line) {
      // A `const` / `static` inside a body is a local: only top-level ones are definitions others use
      let local = matches!(&m["kw"], "const" | "static") && !m["indent"].is_empty();
      (!local).then(|| m["name"].to_owned())
    } else {
      METHOD.captures(line).map(|m| m["name"].to_owned()).filter(|n| !line.contains("=>") && !NOT_NAMES.contains(&n.as_str()))
    };
    let Some(name) = name.filter(|n| n.chars().count() >= 3 && !NOT_NAMES.contains(&n.as_str()) && !COMMON_NAMES.contains(&n.as_str())) else {
      continue;
    };
    let plus = line.starts_with('+');
    match out.iter_mut().find(|d| d.name == name) {
      Some(d) => {
        d.added |= plus;
        d.removed |= !plus;
      }
      None => out.push(Definition { name, removed: !plus, added: plus }),
    }
  }
  out
}

/// Which definitions get their uses listed first: one removed outright (a use left behind breaks), then one changed in
/// place (a caller may now be wrong), then a new one in an existing file, then those of new files
fn definition_rank(d: &Definition, new_file: bool) -> u8 {
  match (new_file, d.removed, d.added) {
    (false, true, false) => 0,
    (false, true, true) => 1,
    (false, false, _) => 2,
    (true, ..) => 3,
  }
}

/// Where each name is used in the work tree (tracked and untracked files, prose left out), its own definitions and the
/// lines of new files (the diff already shows them whole) left out; the total capped at `max`
async fn references(top: &Path, names: &[String], new_files: &BTreeSet<String>, max: usize) -> Vec<(String, Vec<String>, usize)> {
  let mut out = vec![];
  let mut left = max;
  for name in names.iter().take(REF_NAMES) {
    if left == 0 {
      break;
    }
    let args = ["-c", "core.quotepath=false", "grep", "--untracked", "-n", "-w", "-I", "--no-color", "-F", "-e", name.as_str(), "--", ".", ":!*.md"];
    let Ok(raw) = git(top, &args).await else {
      // Exit 1 = no match: a name nothing else uses is worth saying too
      out.push((name.clone(), vec![], 0));
      continue;
    };
    let hits: Vec<String> = text(raw)
      .lines()
      .filter(|l| {
        // `path:line:text`
        let mut cols = l.splitn(3, ':');
        let (path, _, body) = (cols.next().unwrap_or(""), cols.next(), cols.next().unwrap_or(""));
        let as_diff = format!("+{body}");
        let defines = DEFINITION.captures(&as_diff).is_some_and(|m| &m["name"] == name.as_str())
          || METHOD.captures(&as_diff).is_some_and(|m| &m["name"] == name.as_str());
        !defines && !new_files.contains(path)
      })
      .map(|l| clip(l, REF_LINE_MAX))
      .collect();
    let total = hits.len();
    if total > REFS_TOO_MANY {
      out.push((name.clone(), vec![], total));
      continue;
    }
    let shown: Vec<String> = hits.into_iter().take(REFS_PER_NAME.min(left)).collect();
    left -= shown.len();
    out.push((name.clone(), shown, total));
  }
  out
}

fn clip(s: &str, max: usize) -> String {
  let t = s.trim_end();
  if t.chars().count() <= max {
    return t.to_owned();
  }
  let mut out: String = t.chars().take(max).collect();
  out.push('…');
  out
}

/// A fenced block whose fence no line inside can close
fn fenced(lang: &str, body: &str) -> String {
  let mut fence = "```".to_owned();
  while body.contains(&fence) {
    fence.push('`');
  }
  let nl = if body.ends_with('\n') { "" } else { "\n" };
  format!("{fence}{lang}\n{body}{nl}{fence}\n")
}

fn lang_of(path: &str) -> &'static str {
  match path.rsplit('.').next().unwrap_or("") {
    "rs" => "rust",
    "ts" | "tsx" | "mts" | "cts" => "ts",
    "js" | "jsx" | "mjs" | "cjs" => "js",
    "py" => "python",
    "go" => "go",
    "java" => "java",
    "kt" | "kts" => "kotlin",
    "swift" => "swift",
    "rb" => "ruby",
    "c" | "h" => "c",
    "cc" | "cpp" | "hpp" | "cxx" => "cpp",
    "cs" => "csharp",
    "json" => "json",
    "toml" => "toml",
    "yaml" | "yml" => "yaml",
    "md" => "markdown",
    "sh" | "bash" | "zsh" => "sh",
    "css" | "scss" => "css",
    "html" | "htm" => "html",
    _ => "",
  }
}

/// The work tree's changes against the base that the scope covers
async fn in_scope(cwd: &Path, top: &Path, base: &str, scope: Option<&Scope>) -> Result<Vec<Changed>> {
  let all = changes(top, base).await?;
  Ok(match scope {
    None => all,
    Some(s) if s.anywhere => all,
    Some(s) => {
      let named: BTreeSet<String> = s.files.iter().filter_map(|f| repo_relative(top, cwd, f)).collect();
      all.into_iter().filter(|c| named.contains(&c.path) || c.old_path.as_ref().is_some_and(|o| named.contains(o))).collect()
    }
  })
}

/// The files a pack of this scope would list now, with their line counts, against `base` (a pack's own base, so a
/// later round compares like with like): what a summoned thread is told changed since its last round. None when `cwd`
/// is not in a git work tree
pub async fn file_stats(cwd: &Path, base: Option<&str>, scope: Option<&Scope>) -> Result<Option<Vec<PackFile>>> {
  let Some((top, base)) = locate(cwd, base).await? else { return Ok(None) };
  let files = in_scope(cwd, &top, &base, scope).await?;
  let counts = numstat(&top, &base, &files).await;
  Ok(Some(
    files
      .iter()
      .map(|c| {
        let (added, removed) = counts.get(&c.path).copied().unwrap_or((None, None));
        PackFile { path: c.path.clone(), status: c.status.to_string(), old_path: c.old_path.clone(), added, removed }
      })
      .collect(),
  ))
}

/// What moved between two listings of the same scope and base, one entry per file: its counts now, new to the list,
/// or no longer changed. Empty when nothing moved
pub fn changed_since(before: &[PackFile], now: &[PackFile]) -> Vec<String> {
  let lines = |f: &PackFile| match (f.added, f.removed) {
    (Some(a), Some(r)) => format!("+{a} -{r}"),
    _ => "binary".to_owned(),
  };
  let mut out = vec![];
  for f in now {
    match before.iter().find(|b| b.path == f.path) {
      Some(b) if b.added == f.added && b.removed == f.removed && b.status == f.status => {}
      Some(_) => out.push(format!("{} ({} now)", f.path, lines(f))),
      None => out.push(format!("{} (newly changed, {})", f.path, lines(f))),
    }
  }
  for b in before.iter().filter(|b| !now.iter().any(|f| f.path == b.path)) {
    out.push(format!("{} (no longer changed)", b.path));
  }
  out
}

/// The recipe file in the data root, next to subagents.json: the strategy the evaluation tunes
pub const RECIPE_FILE: &str = "handoff.json";

/// The recipe in `dir/handoff.json`; a missing or unreadable file is the default
pub fn recipe_in(dir: &Path) -> Recipe {
  std::fs::read_to_string(dir.join(RECIPE_FILE)).ok().and_then(|t| Recipe::from_json(&t).ok()).unwrap_or_default().clamped()
}

/// Everything the pack may hold, before the cap decides how much of it goes in
struct Parts {
  header: String,
  stat: String,
  whole: Vec<String>,
  plain: Option<Vec<String>>,
  texts: Vec<(String, String, usize)>,
  refs: String,
  read: String,
}

fn section(title: &str, body: &str) -> String {
  format!("## {title}\n\n{body}\n")
}

fn assemble(p: &Parts, diffs: &[&str], diff_title: &str, texts: &[&(String, String, usize)], refs: bool, footer: &str) -> String {
  let mut out = p.header.clone();
  out.push_str(&section("Changed files", &fenced("", &p.stat)));
  let joined: String = diffs.iter().filter(|d| !d.is_empty()).copied().collect::<Vec<_>>().join("");
  if !joined.trim().is_empty() {
    out.push_str(&section(diff_title, &fenced("diff", &joined)));
  }
  if !texts.is_empty() {
    let body: String = texts.iter().map(|(path, t, n)| format!("### {path} ({n} lines)\n\n{}", fenced(lang_of(path), t))).collect::<Vec<_>>().join("\n");
    out.push_str(&section("Full text of small changed files", &body));
  }
  if refs && !p.refs.is_empty() {
    out.push_str(&section("Where changed definitions are used", &p.refs));
  }
  if refs && !p.read.is_empty() {
    out.push_str(&section("Files the author read", &p.read));
  }
  out.push_str(footer);
  out
}

/// Build the pack for the work tree that holds `cwd`. None when `cwd` is not in a git work tree or nothing in scope
/// changed
pub async fn build(cwd: &Path, base: Option<&str>, scope: Option<&Scope>, recipe: &Recipe) -> Result<Option<Pack>> {
  let recipe = recipe.clone().clamped();
  let Some((top, base)) = locate(cwd, base).await? else { return Ok(None) };
  let files = in_scope(cwd, &top, &base, scope).await?;
  if files.is_empty() {
    return Ok(None);
  }
  let attrs = attributes_file()?;
  let counts = numstat(&top, &base, &files).await;
  let diffable: Vec<Changed> = files.iter().filter(|c| !is_stat_only(&c.path)).cloned().collect();
  let (t, a, b) = (top.clone(), attrs.clone(), base.clone());
  let whole = per_file(&diffable, move |c| file_diff(t.clone(), a.clone(), b.clone(), c, recipe.function_context)).await;

  let pack_files: Vec<PackFile> = files
    .iter()
    .map(|c| {
      let (added, removed) = counts.get(&c.path).copied().unwrap_or((None, None));
      PackFile { path: c.path.clone(), status: c.status.to_string(), old_path: c.old_path.clone(), added, removed }
    })
    .collect();
  let stat: String = pack_files
    .iter()
    .map(|f| {
      let name = match &f.old_path {
        Some(old) => format!("{old} -> {}", f.path),
        None => f.path.clone(),
      };
      let lines = match (f.added, f.removed) {
        (Some(a), Some(r)) => format!("+{a} -{r}"),
        _ => "binary".to_owned(),
      };
      let note = if is_stat_only(&f.path) { "  (diff left out)" } else { "" };
      format!("{} {name}  {lines}{note}\n", f.status)
    })
    .collect();
  let short = if base == EMPTY_TREE { "an empty repository".to_owned() } else { format!("commit {}", &base[..base.len().min(12)]) };
  let root_name = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  let header = format!(
    "# Handoff: uncommitted changes\n\nThe changes in {root_name} against {short}, as git sees them now (paths relative to {}). Generated by Acpira from git alone; read the files themselves for anything this leaves out.\n\n",
    top.display()
  );
  let texts: Vec<(String, String, usize)> = diffable.iter().filter_map(|c| full_text(&top, c, recipe.full_text_lines)).collect();
  let refs = if recipe.refs {
    let new_files: BTreeSet<String> = diffable.iter().filter(|c| c.status == 'A').map(|c| c.path.clone()).collect();
    let mut ranked: Vec<(u8, String)> = vec![];
    for (c, d) in diffable.iter().zip(&whole) {
      for def in changed_definitions(d) {
        let rank = definition_rank(&def, new_files.contains(&c.path));
        match ranked.iter_mut().find(|(_, n)| *n == def.name) {
          Some(r) => r.0 = r.0.min(rank),
          None => ranked.push((rank, def.name)),
        }
      }
    }
    // Stable: within a rank, the order the diff shows them in
    ranked.sort_by_key(|(r, _)| *r);
    let names: Vec<String> = ranked.into_iter().map(|(_, n)| n).collect();
    let found = references(&top, &names, &new_files, recipe.refs_max).await;
    found
      .iter()
      .map(|(name, hits, total)| match (hits.is_empty(), *total) {
        (true, 0) => format!("`{name}`: not used elsewhere\n"),
        (true, n) => format!("`{name}`: {n} uses, too many to list\n"),
        (false, n) => {
          let more = if n > hits.len() { format!("… and {} more\n", n - hits.len()) } else { String::new() };
          format!("`{name}`:\n{}{more}", fenced("", &hits.join("\n")))
        }
      })
      .collect::<Vec<_>>()
      .join("\n")
  } else {
    String::new()
  };
  let read = if recipe.read_paths {
    let mut seen = BTreeSet::new();
    let listed: Vec<String> = scope
      .map(|s| s.read.as_slice())
      .unwrap_or(&[])
      .iter()
      .filter_map(|p| repo_relative(&top, cwd, p).or_else(|| Some(p.trim().to_owned()).filter(|p| !p.is_empty())))
      .filter(|p| seen.insert(p.clone()))
      .take(READ_PATHS_MAX)
      .map(|p| format!("- {p}\n"))
      .collect();
    listed.concat()
  } else {
    String::new()
  };
  let mut parts = Parts { header, stat, whole, plain: None, texts, refs, read };
  let first_title = if recipe.function_context { "Diff (each hunk widened to its whole function)" } else { "Diff" };
  let footer = |level: &str, dropped: usize, extra: &str| {
    let texts = if dropped > 0 { format!(" {dropped} full texts were left out to stay under the size cap.") } else { String::new() };
    format!("\n(Pack: {level}.{texts}{extra})\n")
  };

  // 1. Everything; 2. without full texts, largest first
  let mut kept: Vec<&(String, String, usize)> = parts.texts.iter().collect();
  kept.sort_by_key(|(_, t, _)| std::cmp::Reverse(t.len()));
  let diffs: Vec<&str> = parts.whole.iter().map(String::as_str).collect();
  loop {
    let dropped = parts.texts.len() - kept.len();
    let level = if dropped == 0 { "everything that was due" } else { "full texts trimmed" };
    let mut shown = kept.clone();
    shown.sort_by(|a, b| a.0.cmp(&b.0));
    let out = assemble(&parts, &diffs, first_title, &shown, true, &footer(level, dropped, ""));
    if out.len() <= recipe.max_bytes {
      let lvl = if dropped == 0 { Level::Full } else { Level::Trimmed };
      return Ok(Some(finish(out, pack_files, lvl, base, &top)));
    }
    if kept.is_empty() {
      break;
    }
    kept.remove(0);
  }
  // 3. Plain hunks
  if recipe.function_context {
    let (t, a, b) = (top.clone(), attrs.clone(), base.clone());
    parts.plain = Some(per_file(&diffable, move |c| file_diff(t.clone(), a.clone(), b.clone(), c, false)).await);
  }
  let plain: Vec<&str> = parts.plain.as_ref().unwrap_or(&parts.whole).iter().map(String::as_str).collect();
  let out = assemble(&parts, &plain, "Diff", &[], true, &footer("plain hunks, no full texts", parts.texts.len(), ""));
  if out.len() <= recipe.max_bytes {
    return Ok(Some(finish(out, pack_files, Level::Plain, base, &top)));
  }
  // 4. Paths and counts, plus the plain diffs of the smallest files that still fit
  let mut order: Vec<usize> = (0..plain.len()).collect();
  order.sort_by_key(|i| plain[*i].len());
  let mut picked: BTreeSet<usize> = BTreeSet::new();
  let base_out = assemble(&parts, &[], "Diff", &[], false, &footer("paths and line counts", parts.texts.len(), " Only the smallest diffs fit; read the other files themselves."));
  let mut budget = recipe.max_bytes.saturating_sub(base_out.len() + 64);
  for i in order {
    let n = plain[i].len();
    if n == 0 || n > budget {
      continue;
    }
    budget -= n;
    picked.insert(i);
  }
  let shown: Vec<&str> = picked.iter().map(|i| plain[*i]).collect();
  let mut out = assemble(&parts, &shown, "Diff of the smallest changes", &[], false, &footer("paths and line counts", parts.texts.len(), " Only the smallest diffs fit; read the other files themselves."));
  if out.len() > recipe.max_bytes {
    // Even the file list is over the cap: cut it, saying so
    let mut cut: String = out.chars().take(recipe.max_bytes.saturating_sub(80)).collect();
    cut.push_str("\n… (cut at the size cap)\n");
    out = cut;
  }
  Ok(Some(finish(out, pack_files, Level::Stat, base, &top)))
}

fn finish(text: String, files: Vec<PackFile>, level: Level, base: String, top: &Path) -> Pack {
  let bytes = text.len();
  Pack { text, files, level, base, root: top.to_string_lossy().into_owned(), bytes }
}

/// `acpira handoff-pack --cwd DIR [--base REV] [--recipe FILE] [--json]`: the pack of a work tree, printed. Exit 0
/// with the pack, 1 with nothing to pack (or not a git work tree), 2 on a usage or recipe error
pub async fn cli(args: &[String]) -> i32 {
  let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
  let cwd = flag("--cwd").map(PathBuf::from).or_else(|| std::env::current_dir().ok());
  let Some(cwd) = cwd else {
    eprintln!("usage: acpira handoff-pack --cwd DIR [--base REV] [--recipe FILE] [--json]");
    return 2;
  };
  let recipe = match flag("--recipe") {
    Some(f) => match std::fs::read_to_string(&f).map_err(anyhow::Error::from).and_then(|t| Recipe::from_json(&t)) {
      Ok(r) => r,
      Err(e) => {
        eprintln!("{f}: {e}");
        return 2;
      }
    },
    None => Recipe::default(),
  };
  match build(&cwd, flag("--base").as_deref(), None, &recipe).await {
    Ok(Some(pack)) => {
      if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string(&pack).unwrap_or_default());
      } else {
        print!("{}", pack.text);
      }
      0
    }
    Ok(None) => {
      eprintln!("nothing to hand off: {} is not in a git work tree, or nothing changed", cwd.display());
      1
    }
    Err(e) => {
      eprintln!("{e:#}");
      2
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sh(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().map(|o| o.status.success());
    assert_eq!(ok.ok(), Some(true), "git {args:?}");
  }

  fn repo() -> Option<tempfile::TempDir> {
    std::process::Command::new("git").arg("--version").output().ok()?;
    let dir = tempfile::tempdir().unwrap();
    sh(dir.path(), &["init", "-q"]);
    sh(dir.path(), &["config", "user.email", "t@t"]);
    sh(dir.path(), &["config", "user.name", "t"]);
    sh(dir.path(), &["config", "commit.gpgsign", "false"]);
    Some(dir)
  }

  fn write(dir: &Path, path: &str, body: &str) {
    let p = dir.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
  }

  fn commit(dir: &Path) {
    sh(dir, &["add", "-A"]);
    sh(dir, &["commit", "-q", "-m", "base"]);
  }

  const LIB: &str = "use std::fmt;\n\npub fn alpha(a: u32) -> u32 {\n    let b = a + 1;\n    let c = b * 2;\n    let d = c * 3;\n    let e = d * 4;\n    let f = e * 5;\n    let g = f * 6;\n    g\n}\n\npub fn beta() -> u32 {\n    alpha(1)\n}\n";

  #[tokio::test]
  async fn a_hunk_widens_to_its_function_and_a_small_file_comes_in_whole() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    write(root, "src/lib.rs", LIB);
    write(root, "src/user.rs", "fn caller() -> u32 { crate::alpha(2) }\n");
    commit(root);
    write(root, "src/lib.rs", &LIB.replace("let g = f * 6;", "let g = f * 7;"));
    let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
    assert_eq!(pack.level, Level::Full);
    assert_eq!(pack.files.len(), 1);
    assert_eq!((pack.files[0].added, pack.files[0].removed), (Some(1), Some(1)));
    // -W: the whole of `alpha`, its first line included, though the change is seven lines below it
    let diff = pack.text.split("## Diff").nth(1).unwrap();
    assert!(diff.contains("\n pub fn alpha(a: u32) -> u32 {\n") && diff.contains("+    let g = f * 7;"), "{}", pack.text);
    assert!(pack.text.contains("### src/lib.rs (15 lines)"), "{}", pack.text);
    // Off by default: references and read paths
    assert!(!pack.text.contains("Where changed definitions are used"));
    // A plain recipe keeps three lines of context, and no full text
    let plain = Recipe { function_context: false, full_text_lines: 0, ..Recipe::default() };
    let pack = build(root, None, None, &plain).await.unwrap().unwrap();
    assert!(!pack.text.contains("\n pub fn alpha(a: u32) -> u32 {\n") && !pack.text.contains("Full text"), "{}", pack.text);
  }

  #[tokio::test]
  async fn new_renamed_deleted_and_binary_files_are_listed_and_a_lockfile_only_counted() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    let long: String = (0..60).map(|i| format!("line {i}\n")).collect();
    write(root, "old_name.txt", &long);
    write(root, "gone.txt", "bye\n");
    write(root, "Cargo.lock", "a\n");
    std::fs::write(root.join("logo.bin"), [0u8, 1, 2, 3]).unwrap();
    commit(root);
    sh(root, &["mv", "old_name.txt", "new_name.txt"]);
    std::fs::remove_file(root.join("gone.txt")).unwrap();
    write(root, "fresh.py", "def hello():\n    return 1\n");
    write(root, "Cargo.lock", "a\nb\n");
    std::fs::write(root.join("logo.bin"), [0u8, 9, 9]).unwrap();
    let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
    let by: BTreeMap<&str, &PackFile> = pack.files.iter().map(|f| (f.path.as_str(), f)).collect();
    assert_eq!(by["new_name.txt"].status, "R");
    assert_eq!(by["new_name.txt"].old_path.as_deref(), Some("old_name.txt"));
    assert_eq!(by["gone.txt"].status, "D");
    assert_eq!((by["fresh.py"].status.as_str(), by["fresh.py"].added), ("A", Some(2)));
    assert_eq!(by["logo.bin"].added, None, "binary");
    assert!(pack.text.contains("old_name.txt -> new_name.txt"));
    assert!(pack.text.contains("Cargo.lock  +1 -0  (diff left out)"), "{}", pack.text);
    assert!(!pack.text.contains("+b\n"), "the lockfile's diff stays out");
    assert!(pack.text.contains("+def hello():"), "an untracked file shows as new: {}", pack.text);
    assert!(pack.text.contains("Binary files"), "{}", pack.text);
  }

  #[tokio::test]
  async fn references_are_found_capped_and_definitions_left_out() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    write(root, "src/lib.rs", LIB);
    let users: String = (0..30).map(|i| format!("fn use{i}() -> u32 {{ crate::alpha({i}) }}\n")).collect();
    write(root, "src/user.rs", &users);
    commit(root);
    write(root, "src/lib.rs", &LIB.replace("pub fn alpha(a: u32) -> u32 {", "pub fn alpha(a: u32, b: u32) -> u32 {"));
    let recipe = Recipe { refs: true, refs_max: 5, ..Recipe::default() };
    let pack = build(root, None, None, &recipe).await.unwrap().unwrap();
    let refs = pack.text.split("## Where changed definitions are used").nth(1).unwrap();
    assert!(refs.contains("`alpha`:"), "{refs}");
    // The use in the same file comes first (git grep goes by path), then four of the thirty in user.rs
    assert_eq!(refs.matches("src/user.rs:").count(), 4, "capped at refs_max: {refs}");
    assert!(refs.contains("and 26 more"), "the rest counted: {refs}");
    assert!(!refs.contains("src/lib.rs:3:pub fn alpha"), "its own definition is not a use: {refs}");
    assert!(refs.contains("src/lib.rs:14:    alpha(1)"), "a use in the same file is: {refs}");
  }

  #[test]
  fn definitions_are_read_off_changed_lines_in_several_languages() {
    let diff = "+pub(crate) async fn load_all(x: u32) {\n-def parse_header(line):\n+export class SessionView {\n+  async refresh(force: boolean): Promise<void> {\n+    const local = 1;\n+export const LIMIT = 5;\n+func (s *Server) Handle(w http.ResponseWriter) {\n+    if (ready()) {\n+fn ab() {}\n context fn not_changed() {}\n";
    let names: Vec<String> = changed_definitions(diff).into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["load_all", "parse_header", "SessionView", "refresh", "LIMIT", "Handle"]);
    // Removed, changed in place, added; and the names every codebase defines are skipped
    let defs = changed_definitions("-fn gone() {}\n-fn kept(a: u8) {}\n+fn kept(a: u16) {}\n+fn fresh() {}\n+    fn default() -> Self {\n");
    assert_eq!(defs.iter().map(|d| (d.name.as_str(), d.removed, d.added)).collect::<Vec<_>>(), [("gone", true, false), ("kept", true, true), ("fresh", false, true)]);
    let rank = |i: usize, new_file| definition_rank(&defs[i], new_file);
    assert!(rank(0, false) < rank(1, false) && rank(1, false) < rank(2, false) && rank(2, false) < rank(2, true));
  }

  #[tokio::test]
  async fn over_the_cap_full_texts_go_first_then_functions_then_all_but_the_stat() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    // One long function edited at its end, a small file, and a file whose every line changes
    let body: String = (0..300).map(|i| format!("    let v{i} = {i};\n")).collect();
    let src = format!("pub fn big() {{\n{body}}}\n");
    write(root, "big.rs", &src);
    write(root, "small.rs", "pub fn small() -> u8 {\n    1\n}\n");
    write(root, "huge.txt", &(0..150).map(|i| format!("x{i}\n")).collect::<String>());
    commit(root);
    write(root, "big.rs", &src.replace("let v299 = 299;", "let v299 = 300;"));
    write(root, "small.rs", "pub fn small() -> u8 {\n    2\n}\n");
    write(root, "huge.txt", &(0..150).map(|i| format!("y{i}\n")).collect::<String>());
    let full = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
    assert_eq!(full.level, Level::Full);
    assert!(full.text.contains("### big.rs (302 lines)"));
    let cap = |max_bytes| Recipe { max_bytes, ..Recipe::default() };
    // Just under the full size: the largest full text goes, the others stay
    let trimmed = build(root, None, None, &cap(full.bytes - 200)).await.unwrap().unwrap();
    assert_eq!(trimmed.level, Level::Trimmed, "{}", trimmed.text);
    assert!(trimmed.text.contains("### small.rs") && trimmed.text.contains("### huge.txt") && !trimmed.text.contains("### big.rs"));
    // The whole 300-line function does not fit: plain hunks, no full texts
    let plain = build(root, None, None, &cap(4_000)).await.unwrap().unwrap();
    assert_eq!(plain.level, Level::Plain, "{}", plain.text);
    assert!(plain.text.contains("+    let v299 = 300;") && !plain.text.contains("    let v10 = 10;") && !plain.text.contains("### small.rs"));
    assert!(plain.bytes <= 4_000);
    // At the floor: every path with its counts, the diffs of the two small changes, not the 150-line one
    let stat = build(root, None, None, &cap(2_000)).await.unwrap().unwrap();
    assert_eq!(stat.level, Level::Stat, "{}", stat.text);
    assert!(stat.bytes <= 2_000, "{}", stat.bytes);
    assert!(stat.text.contains("M huge.txt  +150 -150") && stat.text.contains("M big.rs  +1 -1"), "{}", stat.text);
    assert!(stat.text.contains("+    2\n") && stat.text.contains("+    let v299 = 300;") && !stat.text.contains("+y1\n"), "{}", stat.text);
  }

  #[tokio::test]
  async fn a_scope_keeps_the_named_files_unless_the_session_could_write_anywhere() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    write(root, "a.txt", "a\n");
    write(root, "b.txt", "b\n");
    commit(root);
    write(root, "a.txt", "a2\n");
    write(root, "b.txt", "b2\n");
    let named = Scope { files: vec![root.join("a.txt").to_string_lossy().into_owned(), "nowhere/else.txt".into()], ..Default::default() };
    let pack = build(root, None, Some(&named), &Recipe::default()).await.unwrap().unwrap();
    assert_eq!(pack.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    let anywhere = Scope { anywhere: true, ..named.clone() };
    let pack = build(root, None, Some(&anywhere), &Recipe::default()).await.unwrap().unwrap();
    assert_eq!(pack.files.len(), 2);
    // A named file with no change left: nothing to hand off
    let clean = Scope { files: vec!["nowhere/else.txt".into()], ..Default::default() };
    assert!(build(root, None, Some(&clean), &Recipe::default()).await.unwrap().is_none());
    // Read paths only when the recipe asks
    let read = Scope { read: vec!["docs/spec.md".into(), root.join("b.txt").to_string_lossy().into_owned()], ..named };
    let pack = build(root, None, Some(&read), &Recipe { read_paths: true, ..Recipe::default() }).await.unwrap().unwrap();
    assert!(pack.text.contains("## Files the author read\n\n- docs/spec.md\n- b.txt\n"), "{}", pack.text);
  }

  #[tokio::test]
  async fn a_base_can_be_named_and_a_repository_without_commits_diffs_against_nothing() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    write(root, "x.txt", "one\n");
    let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
    assert_eq!(pack.base, EMPTY_TREE);
    commit(root);
    write(root, "x.txt", "two\n");
    commit(root);
    // Clean against HEAD, one change against HEAD~1
    assert!(build(root, None, None, &Recipe::default()).await.unwrap().is_none());
    let pack = build(root, Some("HEAD~1"), None, &Recipe::default()).await.unwrap().unwrap();
    assert!(pack.text.contains("-one\n+two"), "{}", pack.text);
    assert!(build(root, Some("no-such-rev"), None, &Recipe::default()).await.is_err());
    // Not a work tree at all
    let plain = tempfile::tempdir().unwrap();
    assert!(build(plain.path(), None, None, &Recipe::default()).await.unwrap().is_none());
  }

  #[test]
  fn a_recipe_file_names_only_what_it_changes_and_is_clamped() {
    let r = Recipe::from_json(r#"{ "refs": true, "maxBytes": 10, "unknown": 1 }"#).unwrap();
    assert!(r.refs && r.function_context);
    assert_eq!(r.max_bytes, 2_000);
    assert!(Recipe::from_json("not json").is_err());
    // The data root's file, or the default when it is missing or broken
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(recipe_in(dir.path()), Recipe::default());
    std::fs::write(dir.path().join(RECIPE_FILE), r#"{ "refs": true }"#).unwrap();
    assert!(recipe_in(dir.path()).refs);
    std::fs::write(dir.path().join(RECIPE_FILE), "{").unwrap();
    assert_eq!(recipe_in(dir.path()), Recipe::default());
  }

  #[tokio::test]
  async fn a_later_listing_against_the_packs_base_names_only_what_moved() {
    let Some(dir) = repo() else { return };
    let root = dir.path();
    write(root, "a.txt", "one\n");
    write(root, "b.txt", "one\n");
    commit(root);
    write(root, "a.txt", "two\n");
    write(root, "b.txt", "two\n");
    let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
    // A commit in between does not move the base: the listing stays against the pack's own
    sh(root, &["commit", "-q", "-am", "wip"]);
    write(root, "a.txt", "three\nfour\n");
    write(root, "b.txt", "one\n");
    write(root, "c.txt", "new\n");
    let now = file_stats(root, Some(&pack.base), None).await.unwrap().unwrap();
    assert_eq!(changed_since(&pack.files, &now), ["a.txt (+2 -1 now)", "c.txt (newly changed, +1 -0)", "b.txt (no longer changed)"]);
    assert!(changed_since(&now, &now).is_empty());
    // A scope keeps to its files
    let scope = Scope { files: vec!["a.txt".into()], ..Default::default() };
    let only = file_stats(root, Some(&pack.base), Some(&scope)).await.unwrap().unwrap();
    assert_eq!(only.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
  }
}
