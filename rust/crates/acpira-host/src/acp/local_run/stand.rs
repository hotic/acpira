//! The stand-in folder a CLI runs in for a folder on an SSH host: how its paths map to the host, which of the CLI's own
//! files stay here, and the gates that stop its native project tools (the hook file and `acpira tool-guard`, the
//! engine's refusal of their permission requests)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::{HookFormat, StandProfile};

/// A session folder on an SSH host and its stand-in folder here
#[derive(Clone, Debug, PartialEq)]
pub struct Stand {
  pub profile: Arc<StandProfile>,
  pub alias: String,
  /// The folder on the host, absolute
  pub host: String,
  /// Where the CLI runs here: `<data root>/remote/<profile dir>/<alias>/<host path>`
  pub local: PathBuf,
  /// The folders the session's process was told to keep the CLI's own files in (`OwnFiles::env_dirs` as an account
  /// set them, `AgentProcess::own_dirs`): the engine's own environment never names those
  pub own: Vec<PathBuf>,
}

/// `<data root>/remote/<dir>/<alias>/<host path>`
fn stand_folder(dir: &str, alias: &str, host: &str, data_root: &Path) -> PathBuf {
  let mut local = data_root.join("remote");
  local.extend(dir.split('/'));
  local.push(alias);
  let rel = host.trim_start_matches('/');
  if !rel.is_empty() {
    local.push(rel);
  }
  local
}

fn trim_host(host: &str) -> &str {
  if host.len() > 1 { host.trim_end_matches('/') } else { host }
}

impl Stand {
  pub fn new(profile: Arc<StandProfile>, alias: &str, host: &str, data_root: &Path) -> Stand {
    let host = trim_host(host);
    let local = stand_folder(&profile.dir, alias, host, data_root);
    Stand { profile, alias: alias.to_owned(), host: host.to_owned(), local, own: Vec::new() }
  }

  /// Makes the stand-in. A folder on the way that its owner cannot enter or write (a stray `chmod` on `<data root>/remote`
  /// left every session there failing with `spawn EACCES`) is opened to its owner and the stand-in made again:
  /// everything from `<data root>/remote` down is the engine's own. A profile with a hook gets its hook file here, so
  /// the CLI denies its built-in project tools before running them
  pub fn make(&self, data_root: &Path) -> std::io::Result<()> {
    // A host path Windows reads as a drive (`/C:/x`) or a climbing one would put the stand-in, and the files written
    // into it, somewhere among this machine's own
    if !self.local.starts_with(data_root.join("remote")) || self.local.components().any(|c| c == std::path::Component::ParentDir) {
      return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{} is outside the stand-in folders", self.local.display())));
    }
    match std::fs::create_dir_all(&self.local) {
      Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
        self.open_to_owner(data_root);
        std::fs::create_dir_all(&self.local)?;
      }
      Err(e) => return Err(e),
      Ok(()) => {}
    }
    if let Some(format) = self.profile.hook() {
      write_hooks(&self.local, &self.profile, format)?;
    }
    for (rel, text) in &self.profile.files {
      write_changed(&self.local.join(rel), text.as_bytes())?;
    }
    Ok(())
  }

  /// Top down, so a folder closed inside a closed one is reached once its parent is open
  #[cfg(unix)]
  fn open_to_owner(&self, data_root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rest) = self.local.strip_prefix(data_root) else { return };
    let mut dir = data_root.to_path_buf();
    for part in rest.components() {
      dir.push(part);
      let Ok(meta) = std::fs::symlink_metadata(&dir) else { return };
      let mode = meta.permissions().mode();
      if meta.is_dir() && mode & 0o700 != 0o700 {
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode | 0o700));
      }
    }
  }

  #[cfg(not(unix))]
  fn open_to_owner(&self, _data_root: &Path) {}

  /// A path the model wrote, as the host knows it: under the stand-in it moves to the host folder, a relative one is
  /// read against the host folder, anything else is the host's already
  pub fn host_path(&self, p: &str) -> String {
    map_host_path(&self.local, &self.host, p)
  }

  /// Whether `p` names the stand-in or something in it
  pub fn in_stand(&self, p: &str) -> bool {
    Path::new(p).starts_with(&self.local)
  }
}

fn map_host_path(local: &Path, host: &str, p: &str) -> String {
  // By components, not by string: on Windows the stand-in and the paths the CLI writes use either separator
  if let Ok(rest) = Path::new(p).strip_prefix(local) {
    let parts: Vec<_> = rest.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    return if parts.is_empty() { host.to_owned() } else { format!("{}/{}", host.trim_end_matches('/'), parts.join("/")) };
  }
  on_host(host, p)
}

/// A path outside any stand-in: an absolute one is the host's already, a relative one is read against the host folder
fn on_host(host: &str, p: &str) -> String {
  if p.starts_with('/') || Path::new(p).is_absolute() {
    return p.to_owned();
  }
  format!("{}/{}", host.trim_end_matches('/'), p.trim_start_matches("./"))
}

/// Resolves a path from any CLI's stand-in of `(alias, host)` or relative to `host`, for `openFile` and `stageFile`
/// where only the session's folder is known. A stand-in folder is `remote/<dir>/<alias>/…`, a generic profile's dir
/// being `stand/<agent>` (`super::generic`)
pub fn resolve_host_path(alias: &str, host: &str, data_root: &Path, p: &str) -> String {
  let host = trim_host(host);
  if let Ok(rest) = Path::new(p).strip_prefix(data_root.join("remote")) {
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let depth = if parts.first().map(String::as_str) == Some("stand") { 2 } else { 1 };
    if parts.len() > depth {
      let local = stand_folder(&parts[..depth].join("/"), alias, host, data_root);
      if Path::new(p).starts_with(&local) {
        return map_host_path(&local, host, p);
      }
    }
  }
  on_host(host, p)
}

/// The CLI's own folders here (`OwnFiles::dirs` under home, then the folders its variables name)
fn config_roots(profile: &StandProfile, home: &Path) -> Vec<PathBuf> {
  let mut out: Vec<PathBuf> = profile.own.dirs.iter().map(|d| home.join(d)).collect();
  for var in &profile.own.env_dirs {
    if let Some(dir) = std::env::var_os(var).filter(|d| !d.is_empty()) {
      out.push(PathBuf::from(dir));
    }
  }
  out
}

/// `config_roots` for a session's stand-in in the engine, which also knows the folders of the session's process
fn own_roots(stand: &Stand, home: &Path) -> Vec<PathBuf> {
  let mut out = config_roots(&stand.profile, home);
  out.extend(stand.own.iter().cloned());
  out
}

/// The folders of this machine whose files the CLI's tools reach for and must find here: its own folders (plans,
/// memory, artifacts, saved tool outputs) and Acpira's data root. Its temporary files are matched by prefix
pub fn here_roots(stand: &Stand, home: &Path, data_root: &Path) -> Vec<PathBuf> {
  let mut out = own_roots(stand, home);
  out.push(data_root.to_path_buf());
  out
}

/// The path a call of a remote tool names, if any
pub(super) fn path_arg<'a>(name: &str, args: &'a Value) -> Option<&'a str> {
  let key = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty());
  match name {
    "Read" | "Write" | "Edit" | "MultiEdit" => key("file_path").or_else(|| key("path")),
    "Grep" | "Glob" | "LS" => key("path"),
    _ => None,
  }
}

/// Whether a path climbs with `..`: matched by prefix, `~/.claude/plans/../../x` would count as inside `~/.claude`
fn climbs(p: &str) -> bool {
  Path::new(p).components().any(|c| c == std::path::Component::ParentDir)
}

/// Whether a call of a remote tool runs on this machine: a file tool whose path is one of the CLI's own files here
/// (`here_roots`, or its temporary files). Anything under `<data_root>/remote` is stand-in space, never a local file;
/// a path with `..` goes to the host rather than being resolved here; commands always run on the host
pub fn runs_here(name: &str, args: &Value, stand: &Stand, data_root: &Path, roots: &[PathBuf]) -> bool {
  let Some(p) = path_arg(name, args).filter(|p| (p.starts_with('/') || Path::new(p).is_absolute()) && !climbs(p)) else { return false };
  if stand.in_stand(p) || Path::new(p).starts_with(data_root.join("remote")) {
    return false;
  }
  stand.profile.own.temp_prefixes.iter().any(|t| p.starts_with(t.as_str())) || roots.iter().any(|r| Path::new(p).starts_with(r))
}

/// Whether a local write (`runs_here`) targets the CLI's own plan / artifact folder, which its native tools write
/// without showing a permission card
pub fn is_quiet_write(stand: &Stand, path: &str, home: &Path) -> bool {
  let p = Path::new(path);
  !climbs(path) && own_roots(stand, home).iter().any(|root| stand.profile.own.quiet_writes.iter().any(|w| p.starts_with(root.join(w))))
}

/// The paths in a native tool's arguments: string values (or string lists) under a key naming a path, file or
/// directory (`Path`, `AbsolutePath`, `TargetFile`, `file_path`, …), since the tools do not share one key
fn tool_paths(args: &Value) -> impl Iterator<Item = String> + '_ {
  let pathish = |k: &str| {
    let k = k.to_ascii_lowercase();
    k.contains("path") || k.contains("file") || k.contains("dir")
  };
  args.as_object().into_iter().flatten().filter(move |(k, _)| pathish(k)).flat_map(|(_, v)| match v {
    Value::String(s) => vec![s.trim().to_owned()],
    Value::Array(a) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_owned()).collect(),
    _ => Vec::new(),
  })
}

/// Whether a native file tool only touches the CLI's own files: it names at least one path and every one is absolute
/// (or `~/`) and inside one of its own folders. `..` is refused rather than resolved, so `~/.gemini/../x` cannot pass
fn own_paths(roots: &[PathBuf], paths: impl Iterator<Item = String>, home: &Path) -> bool {
  let mut any = false;
  for p in paths.filter(|p| !p.is_empty()) {
    let p = match p.strip_prefix("~/") {
      Some(rest) => home.join(rest),
      None => PathBuf::from(&p),
    };
    if !p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir) || !roots.iter().any(|r| p.starts_with(r)) {
      return false;
    }
    any = true;
  }
  any
}

/// `own_paths` of a native tool call's arguments
pub fn own_file(profile: &StandProfile, args: &Value, home: &Path) -> bool {
  own_paths(&config_roots(profile, home), tool_paths(args), home)
}

/// Where a CLI names the tool of a permission request: the top-level `name`, Grok's and Devin's `_meta`, else the title
fn native_names(tc: &Value) -> Vec<&str> {
  ["/name", "/_meta/x.ai~1tool/name", "/_meta/cognition.ai~1inferenceToolName", "/title"]
    .iter()
    .filter_map(|at| tc.pointer(at)?.as_str())
    .map(|s| s.strip_prefix("Run ").unwrap_or(s))
    .collect()
}

/// A request the engine cannot tell safe in a stand-in whose CLI no gate stops: a tool of kind `other` that is no MCP
/// server's (a CLI may file its shell there, as DSH does). `blocked_native` lets it by, so it must not ride the
/// engine's own full access past the user
pub fn unjudged(stand: &Stand, tc: &Value) -> bool {
  stand.profile.gates.is_empty()
    && tc.get("kind").and_then(Value::as_str) == Some("other")
    && !native_names(tc).first().is_some_and(|n| n.starts_with("mcp_"))
}

/// A location's path as the platform reads it: on Windows `agy` sends a file URL's path (`/C:/Users/me/x`), which is
/// not absolute there until the slash before the drive goes
pub fn located_path(p: &str) -> &str {
  located_path_for(p, cfg!(windows))
}

fn located_path_for(p: &str, windows: bool) -> &str {
  match p.strip_prefix('/') {
    Some(rest) if windows && matches!(rest.as_bytes(), [d, b':', b'/' | b'\\', ..] if d.is_ascii_alphabetic()) => rest,
    _ => p,
  }
}

/// Whether `path` sits under `root`, comparing components case-insensitively on Windows where drive and folder casing
/// can differ between what a CLI reports and what `USERPROFILE` holds
fn path_under(path: &Path, root: &Path) -> bool {
  if path.starts_with(root) {
    return true;
  }
  #[cfg(windows)]
  {
    let mut p = path.components();
    for want in root.components() {
      let Some(got) = p.next() else { return false };
      if !got.as_os_str().eq_ignore_ascii_case(want.as_os_str()) {
        return false;
      }
    }
    true
  }
  #[cfg(not(windows))]
  {
    false
  }
}

/// Largest local file copied verbatim to a remote SSH host via `Write` (`source_local_path`)
pub const MAX_LOCAL_COPY_BYTES: u64 = 32 * 1024 * 1024;

/// Top-level engine state / credential files under `data_root` that must never be copied to a remote host
const PRIVATE_DATA_FILES: [&str; 4] = ["secrets.json", "accounts.json", "providers.json", "connections.json"];

/// Strips the Windows verbatim `\\?\` prefix returned by `std::fs::canonicalize` so canonical paths compare cleanly
/// against non-verbatim roots
fn strip_verbatim(path: PathBuf) -> PathBuf {
  #[cfg(windows)]
  {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
      return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
      return PathBuf::from(rest);
    }
  }
  path
}

fn canonical_clean(path: &Path) -> Option<PathBuf> {
  std::fs::canonicalize(path).ok().map(strip_verbatim)
}

/// Whether `path` names one of a local-run CLI's own files on this machine (its config / artifact / step-media
/// folders, the engine's data root outside `<data_root>/remote`, or its temp prefixes), returning the normalized local
/// path. Stand-in paths, relative paths, `~` paths, `..` climbs and SSH host paths return `None`
pub fn local_own_path(stand: &Stand, path: &str, home: &Path, data_root: &Path) -> Option<PathBuf> {
  let p = located_path(path);
  if p.is_empty() || p == "~" || p.starts_with("~/") || (cfg!(windows) && p.starts_with("~\\")) || climbs(p) {
    return None;
  }
  let candidate = Path::new(p);
  if !candidate.is_absolute() {
    return None;
  }
  if path_under(candidate, &stand.local) || path_under(candidate, &data_root.join("remote")) {
    return None;
  }
  let roots = here_roots(stand, home, data_root);
  let matches_temp = stand.profile.own.temp_prefixes.iter().any(|t| p.starts_with(t.as_str()));
  let matches_root = roots.iter().any(|r| path_under(candidate, r));
  (matches_temp || matches_root).then(|| crate::store::data_dir::normalize(candidate))
}

/// Validates `path` as a readable regular file owned by the local CLI (or Acpira's non-secret data directories) before
/// copying its bytes to an SSH host. Also canonicalizes to prevent symlink escape and blocks private Acpira state files
pub fn copy_source_path(stand: &Stand, path: &str, home: &Path, data_root: &Path) -> Result<PathBuf, String> {
  let Some(candidate) = local_own_path(stand, path, home, data_root) else {
    return Err(format!(
      "source_local_path must be an absolute path to one of your own local files (not a host, stand-in, or arbitrary local path): {path}"
    ));
  };
  let canonical = std::fs::canonicalize(&candidate)
    .map(strip_verbatim)
    .map_err(|e| format!("cannot read source_local_path {path}: {e}"))?;
  let canon_home = canonical_clean(home).unwrap_or_else(|| home.to_path_buf());
  let canon_data = canonical_clean(data_root).unwrap_or_else(|| data_root.to_path_buf());
  let mut canon_stand = stand.clone();
  canon_stand.local = canonical_clean(&stand.local)
    .unwrap_or_else(|| stand.local.strip_prefix(data_root).map(|rel| canon_data.join(rel)).unwrap_or_else(|_| stand.local.clone()));
  canon_stand.own = stand.own.iter().map(|d| canonical_clean(d).unwrap_or_else(|| d.clone())).collect();
  let canonical_str = canonical.to_string_lossy();
  let inside_temp = stand.profile.own.temp_prefixes.iter().any(|t| canonical_str.starts_with(t.as_str()));
  let roots = here_roots(&canon_stand, &canon_home, &canon_data);
  let inside_stand = path_under(&canonical, &canon_stand.local) || path_under(&canonical, &canon_data.join("remote"));
  let inside_root = roots.iter().any(|r| path_under(&canonical, r));
  if inside_stand || (!inside_temp && !inside_root) {
    return Err(format!("source_local_path resolves outside allowed local directories: {path}"));
  }
  if PRIVATE_DATA_FILES.iter().any(|f| canonical == canon_data.join(f)) {
    return Err(format!("source_local_path cannot copy private Acpira state file: {path}"));
  }
  let meta = std::fs::metadata(&canonical).map_err(|e| format!("cannot stat source_local_path {path}: {e}"))?;
  if !meta.is_file() {
    return Err(format!("source_local_path is not a regular file: {path}"));
  }
  if meta.len() > MAX_LOCAL_COPY_BYTES {
    return Err(format!(
      "source_local_path is too large ({} bytes; max {} MiB): {path}",
      meta.len(),
      MAX_LOCAL_COPY_BYTES / (1024 * 1024)
    ));
  }
  Ok(canonical)
}

/// The engine's refusal under every profile: a permission request of a native tool (not one of Acpira's, `caller`
/// tells those apart) that runs a command, searches or lists, or that writes, reads or moves files other than the CLI's
/// own. A tool of an MCP server (`mcp__…`, `mcp_…`) is that server's business
pub fn blocked_native(stand: &Stand, tc: &Value, home: &Path, data_root: &Path) -> bool {
  let names = native_names(tc);
  if names.first().is_some_and(|n| n.starts_with("mcp_")) {
    return false;
  }
  let native = &stand.profile.native;
  if names.iter().any(|n| native.shell.iter().any(|t| t == n)) {
    return true;
  }
  let kind = tc.get("kind").and_then(Value::as_str).unwrap_or("");
  // A CLI the profile names no tools of is judged by kind alone, so a request saying none (nor its row) is refused
  if kind.is_empty() && native.shell.is_empty() && native.files.is_empty() {
    return true;
  }
  let file_tool = names.iter().any(|n| native.files.iter().any(|t| t == n));
  if !file_tool && !matches!(kind, "edit" | "delete" | "move") {
    return kind == "execute";
  }
  let input = tc.get("rawInput").or_else(|| tc.get("args")).unwrap_or(&Value::Null);
  // A model's arguments may come as a JSON string
  let parsed = input.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok());
  let input = parsed.as_ref().unwrap_or(input);
  let located = tc.get("locations").and_then(Value::as_array).into_iter().flatten().filter_map(|l| Some(located_path(l.get("path")?.as_str()?).to_owned()));
  let paths: Vec<String> = tool_paths(input).chain(located).collect();
  if paths.iter().any(|p| stand.in_stand(p) || Path::new(p).starts_with(data_root.join("remote"))) {
    return true;
  }
  !own_paths(&own_roots(stand, home), paths.into_iter(), home)
}

/// The engine subcommand a stand-in's hook runs (`guard`), with the agent's id after it
pub const GUARD_COMMAND: &str = "tool-guard";
/// What `agy`'s hook ran before `tool-guard`: still answered, for a hook file an older engine wrote
pub const AGY_GUARD_COMMAND: &str = "agy-guard";

fn deny_reason(profile: &StandProfile) -> String {
  format!(
    "This workspace is on the remote SSH host. Use {p}Read, {p}Write, {p}Edit, {p}Bash, {p}Grep or {p}Glob instead.",
    p = profile.prefix
  )
}

/// A hook's answer: what it prints and how it exits
#[derive(Debug, PartialEq, Eq)]
pub struct GuardVerdict {
  pub stdout: String,
  pub stderr: String,
  pub code: i32,
}

fn deny(format: &HookFormat, reason: &str) -> GuardVerdict {
  match format {
    HookFormat::Agents => GuardVerdict { stdout: json!({ "decision": "deny", "reason": reason }).to_string(), stderr: String::new(), code: 0 },
    HookFormat::Claude(_) => GuardVerdict {
      stdout: json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } }).to_string(),
      stderr: reason.to_owned(),
      code: 2,
    },
  }
}

/// `acpira tool-guard <agent>`: the `PreToolUse` hook of a stand-in. Reads the hook payload and denies a native shell
/// tool, and a file tool unless the call only touches the CLI's own files; an allowed call gets nothing back, which
/// leaves the CLI's own permission flow in charge. Only the call's own arguments count: `agy` puts its own `~/.gemini`
/// transcript paths in every payload, and reading the whole payload let every call through. An agent without a hook
/// profile here is denied everything, since the hook only exists in a stand-in
pub fn guard(agent: &str, payload: &str, home: &Path) -> Option<GuardVerdict> {
  let profile = super::builtin(agent).and_then(|l| l.stand().cloned());
  let Some((profile, format)) = profile.as_ref().and_then(|p| Some((p, p.hook()?))) else {
    return Some(deny(&HookFormat::Agents, "This workspace is on the remote SSH host; Acpira's guard knows no tools of this agent."));
  };
  let v: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
  let (name, args) = match format {
    HookFormat::Agents => (v.pointer("/toolCall/name"), v.pointer("/toolCall/args")),
    HookFormat::Claude(_) => (v.get("tool_name"), v.get("tool_input")),
  };
  let name = name.and_then(Value::as_str).unwrap_or("");
  let args = args.unwrap_or(&Value::Null);
  // Tolerate arguments sent as a JSON string
  let parsed = args.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok());
  let reason = deny_reason(profile);
  if profile.native.shell.iter().any(|t| t == name) {
    return Some(deny(format, &reason));
  }
  if own_file(profile, parsed.as_ref().unwrap_or(args), home) { None } else { Some(deny(format, &reason)) }
}

/// The hook's command: this engine binary as `tool-guard <agent>`. A shell test on the payload cannot tell a tool's path
/// from the CLI's own paths in every payload
fn guard_command(agent: &str) -> Option<String> {
  let exe = std::env::current_exe().ok()?;
  let exe = exe.to_str()?;
  // Unix hooks run under `sh -c`, Windows ones under `cmd /c`, which keeps a line with exactly two quotes as written
  #[cfg(unix)]
  let quoted = format!("'{}'", exe.replace('\'', r"'\''"));
  #[cfg(not(unix))]
  let quoted = format!("\"{exe}\"");
  Some(format!("{quoted} {GUARD_COMMAND} {agent}"))
}

/// A hook command that answers every call with a deny, needing nothing but the shell
fn deny_all_command(format: &HookFormat, reason: &str) -> String {
  let v = deny(format, reason);
  #[cfg(unix)]
  let cmd = {
    let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let mut cmd = format!("printf '%s\\n' {}", q(&v.stdout));
    if !v.stderr.is_empty() {
      cmd.push_str(&format!("; printf '%s\\n' {} >&2", q(&v.stderr)));
    }
    if v.code != 0 {
      cmd.push_str(&format!("; exit {}", v.code));
    }
    cmd
  };
  #[cfg(not(unix))]
  let cmd = {
    let mut cmd = format!("powershell -NoProfile -Command \"[Console]::Out.WriteLine('{}')", v.stdout.replace('\'', "''").replace('"', "\\\""));
    if !v.stderr.is_empty() {
      cmd.push_str(&format!("; [Console]::Error.WriteLine('{}')", v.stderr.replace('\'', "''").replace('"', "\\\"")));
    }
    cmd.push_str(&format!("; exit {}\"", v.code));
    cmd
  };
  cmd
}

/// Writes the stand-in's hook file when its content changed: shell tools denied outright by the shell, file tools
/// through `tool-guard` (everything denied when the engine binary cannot be named)
fn write_hooks(local: &Path, profile: &StandProfile, format: &HookFormat) -> std::io::Result<()> {
  let shell_reason = format!("This workspace is on the remote SSH host. Use {p}Bash, {p}Grep, or {p}Glob instead.", p = profile.prefix);
  let shell_cmd = deny_all_command(format, &shell_reason);
  let file_cmd = guard_command(&profile.agent).unwrap_or_else(|| deny_all_command(format, &deny_reason(profile)));
  let (file, shell_matcher, file_matcher) = match format {
    HookFormat::Agents => (local.join(".agents").join("hooks.json"), profile.native.shell.join("|"), profile.native.files.join("|")),
    // Claude Code's matchers are regular expressions: anchored, so `Read` is not also `ReadMcpResource`
    HookFormat::Claude(rel) => (local.join(rel), format!("^({})$", profile.native.shell.join("|")), format!("^({})$", profile.native.files.join("|"))),
  };
  let doc = json!({
    "hooks": {
      "PreToolUse": [
        { "matcher": shell_matcher, "hooks": [{ "type": "command", "command": shell_cmd }] },
        { "matcher": file_matcher, "hooks": [{ "type": "command", "command": file_cmd }] },
      ]
    }
  });
  write_changed(&file, &serde_json::to_vec_pretty(&doc).unwrap_or_default())
}

/// Writes one of the stand-in's own files, with its folders, unless it already reads so. Renamed into place: a CLI
/// starting in the same stand-in meanwhile never reads half a gate
fn write_changed(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
  if std::fs::read(file).ok().as_deref() == Some(bytes) {
    return Ok(());
  }
  if let Some(dir) = file.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let mut tmp = file.as_os_str().to_owned();
  tmp.push(format!(".{}.tmp", crate::util::random_uuid()));
  let tmp = PathBuf::from(tmp);
  std::fs::write(&tmp, bytes)?;
  std::fs::rename(&tmp, file).inspect_err(|_| {
    let _ = std::fs::remove_file(&tmp);
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::acp::local_run::{Gate, builtin, generic};

  fn profile(id: &str) -> Arc<StandProfile> {
    builtin(id).and_then(|l| l.stand().cloned()).unwrap()
  }

  fn stand() -> Stand {
    Stand::new(profile("claude"), "example-host", "/home/u/proj/", Path::new("/Users/me/.acpira"))
  }

  fn agy_stand() -> Stand {
    Stand::new(profile("antigravity"), "example-host", "/home/u/proj/", Path::new("/Users/me/.acpira"))
  }

  #[cfg(unix)]
  #[test]
  fn a_closed_folder_on_the_way_is_opened_for_the_stand_in() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let remote = root.path().join("remote");
    std::fs::create_dir_all(remote.join("hosts")).unwrap();
    std::fs::set_permissions(&remote, std::fs::Permissions::from_mode(0o000)).unwrap();
    let s = Stand::new(profile("claude"), "example-host", "/mnt/proj", root.path());
    s.make(root.path()).unwrap();
    assert!(s.local.is_dir());
    assert!(!s.local.join(".agents").exists());
    assert_eq!(std::fs::metadata(&remote).unwrap().permissions().mode() & 0o777, 0o700);
  }

  #[test]
  fn antigravity_stand_in_writes_pre_tool_use_hooks_keeping_claude_empty() {
    let root = tempfile::tempdir().unwrap();
    let s = Stand::new(profile("antigravity"), "example-host", "/mnt/proj", root.path());
    s.make(root.path()).unwrap();
    let hooks: Value = serde_json::from_slice(&std::fs::read(s.local.join(".agents/hooks.json")).unwrap()).unwrap();
    let rules = hooks["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert!(rules[0]["matcher"].as_str().unwrap().contains("run_command"));
    assert!(rules[1]["matcher"].as_str().unwrap().contains("read_file"));
    assert!(rules[1]["hooks"][0]["command"].as_str().unwrap().ends_with(&format!(" {GUARD_COMMAND} antigravity")));
  }

  #[test]
  fn a_profiles_own_files_land_in_the_stand_in() {
    let root = tempfile::tempdir().unwrap();
    let s = Stand::new(profile("dsh"), "example-host", "/mnt/proj", root.path());
    s.make(root.path()).unwrap();
    let patch = std::fs::read_to_string(s.local.join(".acpira/dsh.patch.yml")).unwrap();
    assert!(patch.starts_with("- id: tool-bash\n  disabled: true\n"), "{patch}");
    assert!(s.profile.args().any(|a| a == ".acpira/dsh.patch.yml"));
    let droid = Stand::new(profile("droid"), "example-host", "/mnt/proj", root.path());
    droid.make(root.path()).unwrap();
    let hooks: Value = serde_json::from_slice(&std::fs::read(droid.local.join(".factory/settings.json")).unwrap()).unwrap();
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "^(Execute|Grep|Glob|LS)$");
  }

  #[test]
  fn a_claude_style_hook_file_is_anchored_and_names_the_agent() {
    let root = tempfile::tempdir().unwrap();
    let mut p = generic("mine", "Mine");
    p.gates.push(Gate::Hook(HookFormat::Claude(".mine/settings.json".into())));
    p.native.shell = vec!["Execute".into()];
    p.native.files = vec!["Read".into(), "Create".into()];
    let s = Stand::new(Arc::new(p), "example-host", "/mnt/proj", root.path());
    s.make(root.path()).unwrap();
    assert!(s.local.starts_with(root.path().join("remote/stand/mine/example-host")));
    let hooks: Value = serde_json::from_slice(&std::fs::read(s.local.join(".mine/settings.json")).unwrap()).unwrap();
    let rules = hooks["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(rules[0]["matcher"], "^(Execute)$");
    assert_eq!(rules[1]["matcher"], "^(Read|Create)$");
    assert!(rules[1]["hooks"][0]["command"].as_str().unwrap().ends_with(&format!(" {GUARD_COMMAND} mine")));
    #[cfg(unix)]
    {
      let out = std::process::Command::new("sh").arg("-c").arg(rules[0]["hooks"][0]["command"].as_str().unwrap()).output().unwrap();
      assert_eq!(out.status.code(), Some(2), "the shell deny blocks as Claude Code's hooks read it");
      assert!(String::from_utf8_lossy(&out.stderr).contains("mcp__acpira__Bash"));
      let v: Value = serde_json::from_slice(&out.stdout).unwrap();
      assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    }
  }

  #[test]
  fn the_guard_reads_only_the_tool_paths() {
    // An absolute home on every platform: `/Users/me` has no drive on Windows, so it is not absolute there
    let me = if cfg!(windows) { "C:/Users/me" } else { "/Users/me" };
    let home = Path::new(me);
    // Every payload carries agy's own ~/.gemini transcript and artifact paths, which must not let a call through
    let payload = |args: Value| {
      json!({
        "toolCall": { "id": "t1", "name": "view_file", "args": args },
        "transcriptPath": format!("{me}/.gemini/antigravity-cli/brain/c1/transcript.jsonl"),
        "artifactDirectoryPath": format!("{me}/.gemini/antigravity-cli/brain/c1"),
        "hookEventName": "PreToolUse",
      })
      .to_string()
    };
    let denied = |args: Value| guard("antigravity", &payload(args), home).is_some();
    assert!(denied(json!({ "AbsolutePath": "/etc/hosts" })));
    assert!(denied(json!({ "TargetFile": "src/a.rs", "CodeContent": "x" })));
    assert!(denied(json!({ "AbsolutePath": format!("{me}/.gemini/../.ssh/id_ed25519") })));
    assert!(denied(json!({ "TargetFile": format!("{me}/.gemini/x.md"), "DirectoryPath": format!("{me}/proj") })));
    assert!(denied(json!({ "Query": format!("{me}/.gemini/x") })));
    assert!(denied(json!({})));
    assert!(guard("antigravity", "not json", home).is_some());
    assert!(!denied(json!({ "AbsolutePath": format!("{me}/.gemini/skills/a/SKILL.md") })));
    assert!(!denied(json!({ "TargetFile": "~/.gemini/antigravity-cli/brain/c1/plan.md", "CodeContent": "/* x */" })));
    assert!(!denied(json!(format!("{{\"Path\":\"{me}/.gemini/x.md\"}}"))));
    // A shell tool is denied whatever it names; the deny is agy's JSON on stdout
    let shell = json!({ "toolCall": { "name": "run_command", "args": { "Cwd": format!("{me}/.gemini") } } }).to_string();
    let v = guard("antigravity", &shell, home).unwrap();
    assert_eq!((v.code, serde_json::from_str::<Value>(&v.stdout).unwrap()["decision"].clone()), (0, json!("deny")));
    // An agent with no hook here is denied everything
    assert!(guard("kimi", &payload(json!({ "AbsolutePath": format!("{me}/.gemini/x.md") })), home).is_some());
    // Claude Code's payload, as Droid and Grok send it: a deny is exit code 2
    let claude = |tool: &str, input: Value| guard("droid", &json!({ "tool_name": tool, "tool_input": input }).to_string(), home);
    assert_eq!(claude("Execute", json!({ "command": "ls" })).map(|v| v.code), Some(2));
    assert!(claude("Create", json!({ "file_path": "/etc/hosts", "content": "x" })).is_some());
    assert!(claude("Read", json!({ "file_path": format!("{me}/.factory/skills/a/SKILL.md") })).is_none());
  }

  #[test]
  fn the_stand_in_maps_to_the_host_folder() {
    let data = Path::new("/Users/me/.acpira");
    let s = stand();
    assert_eq!(s.local, PathBuf::from("/Users/me/.acpira/remote/hosts/example-host/home/u/proj"));
    assert_eq!(s.host_path("/Users/me/.acpira/remote/hosts/example-host/home/u/proj"), "/home/u/proj");
    assert_eq!(s.host_path("/Users/me/.acpira/remote/hosts/example-host/home/u/proj/src/a.rs"), "/home/u/proj/src/a.rs");
    // A sibling whose name only starts the same is not inside
    assert_eq!(s.host_path("/Users/me/.acpira/remote/hosts/example-host/home/u/project2/a"), "/Users/me/.acpira/remote/hosts/example-host/home/u/project2/a");
    assert_eq!(s.host_path("src/a.rs"), "/home/u/proj/src/a.rs");
    assert_eq!(s.host_path("./a.rs"), "/home/u/proj/a.rs");
    assert_eq!(s.host_path("/etc/hosts"), "/etc/hosts");
    assert_eq!(Stand::new(profile("claude"), "h", "/", Path::new("/d")).host_path("/d/remote/hosts/h/x"), "/x");
    assert_eq!(resolve_host_path("example-host", "/home/u/proj", data, "/Users/me/.acpira/remote/antigravity/example-host/home/u/proj/src/lib.rs"), "/home/u/proj/src/lib.rs");
    assert_eq!(resolve_host_path("example-host", "/home/u/proj/", data, "/Users/me/.acpira/remote/hosts/example-host/home/u/proj/a"), "/home/u/proj/a");
    assert_eq!(resolve_host_path("example-host", "/home/u/proj", data, "/Users/me/.acpira/remote/stand/droid/example-host/home/u/proj/b.rs"), "/home/u/proj/b.rs");
    assert_eq!(resolve_host_path("example-host", "/home/u/proj", data, "src/c.rs"), "/home/u/proj/src/c.rs");
    assert_eq!(resolve_host_path("example-host", "/home/u/proj", data, "/etc/hosts"), "/etc/hosts");
    // Another host's stand-in is not this folder
    assert_eq!(resolve_host_path("example-host", "/home/u/proj", data, "/Users/me/.acpira/remote/stand/droid/other-host/home/u/proj/b.rs"), "/Users/me/.acpira/remote/stand/droid/other-host/home/u/proj/b.rs");
  }

  #[test]
  fn clis_own_files_stay_here_and_the_rest_goes_to_the_host() {
    let data = Path::new("/Users/me/.acpira");
    let home = Path::new("/Users/me");
    let s = stand();
    let roots = here_roots(&s, home, data);
    let here = |name: &str, args: Value| runs_here(name, &args, &s, data, &roots);
    assert!(here("Write", json!({ "file_path": "/Users/me/.claude/plans/p.md", "content": "x" })));
    assert!(is_quiet_write(&s, "/Users/me/.claude/plans/p.md", home));
    assert!(!is_quiet_write(&s, "/Users/me/.acpira/plans/p.md", home), "the data root is no plan folder");
    assert!(here("Read", json!({ "file_path": "/private/tmp/claude-501/out.txt" })));
    assert!(!here("Read", json!({ "file_path": "/Users/me/.acpira/remote/hosts/example-host/home/u/proj/a.rs" })));
    assert!(!here("Read", json!({ "file_path": "src/a.rs" })));
    assert!(!here("Bash", json!({ "command": "cat /Users/me/.claude/x" })));
    assert!(!here("Grep", json!({ "pattern": "x" })));
    // `..` never climbs out of the CLI's own folders into this machine's files: the call goes to the host
    let climb = "/Users/me/.claude/plans/../../Projects/demo/a.rs";
    assert!(!here("Write", json!({ "file_path": climb, "content": "x" })));
    assert!(!is_quiet_write(&s, climb, home));
    assert!(!here("Write", json!({ "file_path": "/Users/me/.acpira/../Projects/a.rs", "content": "x" })));

    let agy = agy_stand();
    let agy_roots = here_roots(&agy, home, data);
    assert!(runs_here("Write", &json!({ "file_path": "/Users/me/.gemini/antigravity-cli/brain/id/plan.md", "content": "x" }), &agy, data, &agy_roots));
    assert!(is_quiet_write(&agy, "/Users/me/.gemini/antigravity-cli/brain/id/plan.md", home));
    assert!(!runs_here("Read", &json!({ "file_path": "/Users/me/.acpira/remote/antigravity/example-host/home/u/proj/a.rs" }), &agy, data, &agy_roots));
    assert!(!runs_here("Read", &json!({ "file_path": "/tmp/claude-501/out.txt" }), &agy, data, &agy_roots), "Claude's temp files are Claude's");
  }

  #[test]
  fn native_tools_are_refused_unless_they_touch_the_clis_own_files() {
    let data = Path::new("/Users/me/.acpira");
    let home = Path::new("/Users/me");
    let agy = agy_stand();
    assert!(blocked_native(&agy, &json!({ "name": "run_command", "rawInput": { "CommandLine": "ls" } }), home, data));
    assert!(blocked_native(&agy, &json!({ "name": "read_file", "rawInput": { "Path": "/etc/hosts" } }), home, data));
    assert!(blocked_native(&agy, &json!({ "name": "read_file", "rawInput": { "Path": "/Users/me/.acpira/remote/antigravity/example-host/home/u/proj/a" } }), home, data));
    // agy's own files pass under an absolute home (`/Users/me` has no drive on Windows)
    let me = if cfg!(windows) { "C:/Users/me" } else { "/Users/me" };
    let own = format!("{me}/.gemini/skills/a/SKILL.md");
    assert!(!blocked_native(&agy, &json!({ "name": "read_file", "rawInput": { "Path": own } }), Path::new(me), data));
    // A location outside its own folders counts as much as an argument
    let tc = json!({ "name": "write_to_file", "kind": "edit", "rawInput": { "TargetFile": own }, "locations": [{ "path": "/etc/hosts" }] });
    assert!(blocked_native(&agy, &tc, Path::new(me), data));
    // On Windows agy's locations are a file URL's path
    #[cfg(windows)]
    assert!(!blocked_native(&agy, &json!({ "name": "view_file", "kind": "read", "locations": [{ "path": format!("/{own}") }] }), Path::new(me), data));
    // A saved or plugin account's gemini dir is the CLI's own only for the session whose process was given it
    let store = format!("{me}/.acpira/accounts/a1/.gemini");
    let media = json!({ "name": "view_file", "kind": "read", "rawInput": { "AbsolutePath": format!("{store}/antigravity-cli/brain/c1/media_0.png") } });
    assert!(blocked_native(&agy, &media, Path::new(me), data));
    let mut account = agy_stand();
    account.own = vec![PathBuf::from(&store)];
    assert!(!blocked_native(&account, &media, Path::new(me), data));
    assert!(is_quiet_write(&account, &format!("{store}/antigravity-cli/brain/c1/plan.md"), Path::new(me)));
    // A question, a mode switch, a fetch or another MCP server's tool is not the gate's business
    assert!(!blocked_native(&agy, &json!({ "toolCallId": "interaction_1", "title": "Pick one", "kind": "other" }), home, data));
    assert!(!blocked_native(&agy, &json!({ "title": "Ready to code?", "kind": "switch_mode" }), home, data));
    assert!(!blocked_native(&agy, &json!({ "title": "Run mcp_playwright_click", "kind": "execute" }), home, data));

    // A CLI the profile knows nothing of: refused by kind
    let mine = Stand::new(Arc::new(generic("mine", "Mine")), "example-host", "/home/u/proj", data);
    assert!(blocked_native(&mine, &json!({ "title": "exec", "kind": "execute", "rawInput": { "command": "ls" } }), home, data));
    assert!(blocked_native(&mine, &json!({ "title": "Edit a.rs", "kind": "edit", "locations": [{ "path": "/home/u/proj/a.rs" }] }), home, data));
    assert!(blocked_native(&mine, &json!({ "title": "Delete", "kind": "delete" }), home, data), "a write naming no path cannot be checked");
    assert!(!blocked_native(&mine, &json!({ "title": "Read a.rs", "kind": "read", "locations": [{ "path": "/home/u/proj/a.rs" }] }), home, data));
    assert!(!blocked_native(&mine, &json!({ "title": "Fetch", "kind": "fetch" }), home, data));
    assert!(!blocked_native(&mine, &json!({ "title": "mcp__acpira__ask_agent", "kind": "execute" }), home, data));
    assert!(blocked_native(&mine, &json!({ "toolCallId": "t1", "title": "Terminal" }), home, data), "no kind, no way to tell it safe");
    // Kind `other` is let by, and keeps its card under the engine's full access where no gate stands behind it
    let other = json!({ "title": "shell", "kind": "other", "rawInput": { "cmd": "ls" } });
    assert!(!blocked_native(&mine, &other, home, data) && unjudged(&mine, &other));
    assert!(!unjudged(&mine, &json!({ "title": "mcp__github__create_issue", "kind": "other" })));
    assert!(!unjudged(&mine, &json!({ "title": "Read a.rs", "kind": "read" })));

    // Grok names the tool in `_meta`, under a title that is the command; DSH files every tool under `other`
    let grok = Stand::new(profile("grok"), "example-host", "/home/u/proj", data);
    let run = json!({ "title": "Execute `hostname`", "kind": "other", "_meta": { "x.ai/tool": { "name": "run_terminal_command" } } });
    assert!(blocked_native(&grok, &run, home, data));
    let dsh = Stand::new(profile("dsh"), "example-host", "/home/u/proj", data);
    assert!(blocked_native(&dsh, &json!({ "title": "write", "kind": "other", "rawInput": { "file_path": "/tmp/x", "content": "x" } }), home, data));
    assert!(!blocked_native(&dsh, &json!({ "title": "todo_write", "kind": "other" }), home, data));
    assert!(!unjudged(&dsh, &json!({ "title": "todo_write", "kind": "other" })), "its gate took the shell away");
  }

  #[test]
  fn local_own_path_resolves_cli_files_and_rejects_host_or_stand_paths() {
    let me = if cfg!(windows) { "C:/Users/dev" } else { "/Users/me" };
    let home = Path::new(me);
    let data = home.join(".acpira");
    let agy = Stand::new(profile("antigravity"), "example-host", "/home/u/proj", &data);
    let media = format!("{me}/.acpira/plugins/.data/asgard-gateway/agy-sessions/s1/.gemini/antigravity-cli/brain/b1/.system_generated/steps/114/media_0.png");
    let expected = crate::store::data_dir::normalize(Path::new(&media));
    assert_eq!(local_own_path(&agy, &media, home, &data), Some(expected.clone()));
    #[cfg(windows)]
    assert_eq!(local_own_path(&agy, &format!("/{media}"), home, &data), Some(expected));
    assert_eq!(
      local_own_path(&agy, &format!("{me}/.gemini/antigravity-cli/brain/b1/plan.md"), home, &data),
      Some(crate::store::data_dir::normalize(Path::new(&format!("{me}/.gemini/antigravity-cli/brain/b1/plan.md"))))
    );
    // Stand-in paths, host paths, relative paths, `~`, and `..` climbs never resolve as local CLI files
    assert_eq!(local_own_path(&agy, &agy.local.join("src/a.rs").to_string_lossy(), home, &data), None);
    assert_eq!(local_own_path(&agy, "/home/u/proj/a.rs", home, &data), None);
    assert_eq!(local_own_path(&agy, "src/a.rs", home, &data), None);
    assert_eq!(local_own_path(&agy, "~/.gemini/antigravity-cli/brain/b1/plan.md", home, &data), None);
    assert_eq!(local_own_path(&agy, &format!("{me}/.gemini/../../etc/passwd"), home, &data), None);
    // On Windows only drive paths lose a leading slash
    assert_eq!(located_path_for("/C:/Users/me/a.png", true), "C:/Users/me/a.png");
    assert_eq!(located_path_for("/c:relative.png", true), "/c:relative.png");
    assert_eq!(located_path_for("/C:/Users/me/a.png", false), "/C:/Users/me/a.png");
  }

  #[test]
  fn copy_source_path_allows_cli_artifacts_and_blocks_private_state_and_escapes() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let data = home.join(".acpira");
    let artifact_dir = home.join(".gemini/antigravity-cli/brain/b1");
    let outside = root.path().join("outside.txt");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let png = artifact_dir.join("icon.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(&outside, b"secret").unwrap();
    let secrets = data.join("secrets.json");
    std::fs::write(&secrets, b"{}").unwrap();

    let agy = Stand::new(profile("antigravity"), "example-host", "/home/u/proj", &data);
    assert!(copy_source_path(&agy, &png.to_string_lossy(), &home, &data).is_ok());
    assert!(copy_source_path(&agy, &outside.to_string_lossy(), &home, &data).is_err());
    assert!(copy_source_path(&agy, &secrets.to_string_lossy(), &home, &data).is_err());
    assert!(copy_source_path(&agy, &artifact_dir.to_string_lossy(), &home, &data).is_err(), "directories are rejected");
    #[cfg(unix)]
    {
      let link = artifact_dir.join("escape.txt");
      std::os::unix::fs::symlink(&outside, &link).unwrap();
      let err = copy_source_path(&agy, &link.to_string_lossy(), &home, &data).unwrap_err();
      assert!(err.contains("outside allowed local directories"), "{err}");
    }
  }
}
