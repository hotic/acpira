//! Paths and files on this host: the test root map, metadata, reads, writes and listings, and the `project` reads

use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::MAX_READ;
use crate::remote::wire::WireError;

/// Paths of the requests, through the test root map
fn root_map() -> Option<&'static (String, PathBuf)> {
  static MAP: std::sync::OnceLock<Option<(String, PathBuf)>> = std::sync::OnceLock::new();
  MAP
    .get_or_init(|| {
      let raw = std::env::var("ACPIRA_REMOTE_ROOT_MAP").ok()?;
      let (virt, real) = raw.split_once('=')?;
      Some((virt.trim_end_matches('/').to_owned(), PathBuf::from(real)))
    })
    .as_ref()
}

/// A request's path on this host. Must be absolute; under the test map, must lie inside the virtual root
pub fn host_path(raw: &str) -> Result<PathBuf, WireError> {
  if !raw.starts_with('/') {
    return Err(WireError::new("invalid", format!("not an absolute path: {raw}")));
  }
  match root_map() {
    None => Ok(PathBuf::from(raw)),
    Some((virt, real)) => match raw.strip_prefix(virt.as_str()) {
      Some(rest) if rest.is_empty() || rest.starts_with('/') => Ok(real.join(rest.trim_start_matches('/'))),
      _ => Err(WireError::new("denied", format!("outside the test root: {raw}"))),
    },
  }
}

/// Every string of an answer with the test map's real root written as its virtual one, so the engine only ever sees
/// the host's paths as it named them (no map: unchanged)
pub(super) fn unmapped(v: Value) -> Value {
  match root_map() {
    Some((virt, real)) => replace_strings(v, &real.to_string_lossy(), virt),
    None => v,
  }
}

/// The other way, for a request carrying paths inside larger values (a hook payload, a prepared tool call)
pub(super) fn mapped(v: Value) -> Value {
  match root_map() {
    Some((virt, real)) => replace_strings(v, virt, &real.to_string_lossy()),
    None => v,
  }
}

fn replace_strings(v: Value, from: &str, to: &str) -> Value {
  match v {
    Value::String(s) if s.contains(from) => Value::String(s.replace(from, to)),
    Value::Array(a) => Value::Array(a.into_iter().map(|x| replace_strings(x, from, to)).collect()),
    Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, replace_strings(x, from, to))).collect()),
    other => other,
  }
}

pub(super) fn path_arg(a: &Value, key: &str) -> Result<PathBuf, WireError> {
  host_path(a.get(key).and_then(Value::as_str).ok_or_else(|| WireError::new("invalid", format!("missing \"{key}\"")))?)
}

pub(super) fn ms(t: std::io::Result<SystemTime>) -> Option<u64> {
  t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64)
}

fn kind_of(ft: std::fs::FileType) -> &'static str {
  if ft.is_dir() {
    "dir"
  } else if ft.is_file() {
    "file"
  } else if ft.is_symlink() {
    "symlink"
  } else {
    "other"
  }
}

#[cfg(unix)]
pub(super) fn mode_of(m: &std::fs::Metadata) -> u32 {
  use std::os::unix::fs::PermissionsExt;
  m.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
pub(super) fn mode_of(_: &std::fs::Metadata) -> u32 {
  0o644
}

pub(super) fn stat_json(m: &std::fs::Metadata, link: bool) -> Value {
  json!({ "kind": kind_of(m.file_type()), "size": m.len(), "mode": mode_of(m), "mtimeMs": ms(m.modified()), "link": link })
}

/// Blocking file work off the runtime's workers
pub(super) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, WireError> + Send + 'static) -> Result<T, WireError> {
  tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(WireError::new("io", format!("the operation crashed: {e}"))))
}

pub(super) fn read_file(p: &Path, offset: u64, len: Option<u64>) -> Result<(std::fs::Metadata, Vec<u8>), WireError> {
  let what = p.to_string_lossy();
  let mut f = std::fs::File::open(p).map_err(|e| WireError::io(&e, &what))?;
  let meta = f.metadata().map_err(|e| WireError::io(&e, &what))?;
  if meta.is_dir() {
    return Err(WireError::new("isDir", format!("{what} is a folder")));
  }
  let want = match len {
    Some(n) => n.min(MAX_READ),
    None if meta.len().saturating_sub(offset) > MAX_READ => {
      return Err(WireError::new("tooLarge", format!("{what} is {} bytes; read it in windows", meta.len())));
    }
    None => meta.len().saturating_sub(offset),
  };
  f.seek(std::io::SeekFrom::Start(offset)).map_err(|e| WireError::io(&e, &what))?;
  let mut bytes = Vec::with_capacity(want as usize);
  f.take(want).read_to_end(&mut bytes).map_err(|e| WireError::io(&e, &what))?;
  Ok((meta, bytes))
}

/// Write through a temporary file in the same folder, then rename over the target: a reader never sees half a file,
/// and an existing file keeps its permission bits. `if_mtime` / `if_absent` refuse to write over a file that changed
/// since it was read (`changed`)
pub(super) fn write_file(p: &Path, data: &[u8], mode: Option<u32>, mkdirs: bool, if_mtime: Option<u64>, if_absent: bool) -> Result<Value, WireError> {
  let what = p.to_string_lossy();
  let existing = std::fs::metadata(p).ok();
  if if_absent && existing.is_some() {
    return Err(WireError::new("changed", format!("{what} appeared meanwhile")));
  }
  if let Some(want) = if_mtime
    && existing.as_ref().and_then(|m| ms(m.modified())) != Some(want)
  {
    return Err(WireError::new("changed", format!("{what} changed since it was read")));
  }
  if existing.as_ref().is_some_and(std::fs::Metadata::is_dir) {
    return Err(WireError::new("isDir", format!("{what} is a folder")));
  }
  let dir = p.parent().ok_or_else(|| WireError::new("invalid", format!("{what} has no parent")))?;
  if mkdirs {
    std::fs::create_dir_all(dir).map_err(|e| WireError::io(&e, &dir.to_string_lossy()))?;
  }
  let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  let tmp = dir.join(format!(".{name}.acpira-{}", crate::util::random_hex(4)));
  // Permission bits only exist on Unix
  #[cfg(not(unix))]
  let _ = mode;
  let result = (|| -> std::io::Result<()> {
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      if let Some(bits) = mode.or_else(|| existing.as_ref().map(mode_of)) {
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(bits))?;
      }
    }
    std::fs::rename(&tmp, p)
  })();
  if let Err(e) = result {
    let _ = std::fs::remove_file(&tmp);
    return Err(WireError::io(&e, &what));
  }
  let meta = std::fs::metadata(p).map_err(|e| WireError::io(&e, &what))?;
  Ok(json!({ "size": meta.len(), "mtimeMs": ms(meta.modified()), "created": existing.is_none() }))
}

pub(super) fn list_dir(p: &Path, max: usize) -> Result<Value, WireError> {
  let what = p.to_string_lossy();
  let rd = std::fs::read_dir(p).map_err(|e| WireError::io(&e, &what))?;
  let mut entries = vec![];
  let mut cut = false;
  for e in rd.flatten() {
    if entries.len() >= max {
      cut = true;
      break;
    }
    let name = e.file_name().to_string_lossy().into_owned();
    let path = e.path();
    let link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
    // A link reads as what it points at; a dangling one as itself
    let meta = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path));
    let Ok(meta) = meta else { continue };
    let dir = meta.is_dir();
    let git = dir && path.join(".git").exists();
    entries.push(json!({ "name": name, "kind": kind_of(meta.file_type()), "size": meta.len(), "mtimeMs": ms(meta.modified()), "link": link, "git": git }));
  }
  entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
  Ok(json!({ "entries": entries, "cut": cut }))
}

/// The @ search index of the folders searched here, as an engine keeps one for its own folders
static FILES: std::sync::LazyLock<crate::node_files::NodeFiles> = std::sync::LazyLock::new(crate::node_files::NodeFiles::new);

/// The `project` op: `op` names the read, `cwd` the project folder, `rel` a path inside it. A refused read answers with
/// the error's code (`ProjectError::code`)
pub(super) async fn project(a: &Value) -> Result<Value, WireError> {
  use crate::project_files as pf;
  let root = path_arg(a, "cwd")?;
  let rel = a.get("rel").and_then(Value::as_str).unwrap_or("").to_owned();
  let failed = |e: pf::ProjectError| json!({ "error": e.code() });
  let out = match a.get("op").and_then(Value::as_str).unwrap_or("") {
    "listDir" => pf::list_dir(&root, &rel).await.map_or_else(failed, |entries| json!({ "entries": entries })),
    "gitMarks" => json!({ "marks": pf::git_marks(&root).await }),
    "readText" => match pf::resolve(&root, &rel) {
      Ok(file) => pf::read_text(&file).await.map_or_else(failed, |text| json!({ "text": text })),
      Err(e) => failed(e),
    },
    "headText" => json!({ "text": pf::head_text(&root, &rel).await }),
    "sessionChanges" => {
      let bases: Vec<(String, Option<String>)> = serde_json::from_value(mapped(a.get("bases").cloned().unwrap_or(Value::Null))).unwrap_or_default();
      json!({ "files": pf::session_changes(&root, bases).await })
    }
    "searchFiles" => {
      let query = a.get("query").and_then(Value::as_str).unwrap_or("");
      json!({ "files": FILES.search(&root.to_string_lossy(), query).await })
    }
    other => return Err(WireError::new("invalid", format!("unknown project read \"{other}\""))),
  };
  Ok(unmapped(out))
}
