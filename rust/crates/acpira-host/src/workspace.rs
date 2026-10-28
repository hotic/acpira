//! A session folder's files as the pages read them (the desktop tool panel, @ search, attached images): this
//! machine's disk, or the SSH host the folder is on (`ssh://<alias>/<path>`, docs/dev/remote.md). On a host the
//! helper's `project` op runs the same `project_files` / `node_files` code there, so an `ssh://` folder is never taken
//! for a local path. Reads of a session folder outside the agent go through here (`tests::no_direct_folder_io`)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use acpira_shared::protocol::{FileHit, ProjectChange, ProjectEntry};
use acpira_shared::remote::parse_remote_folder;
use serde_json::{Value, json};

use crate::project_files::{self, ProjectError};
use crate::remote::client::RemoteClient;
use crate::remote::connections::Connections;

/// How long a page's read waits for the host's connection to come (back) up
const CONNECT_WAIT: Duration = Duration::from_secs(20);

pub enum Workspace {
  Local(PathBuf),
  Remote { client: RemoteClient, path: PathBuf },
}

/// The folder's own path: the host's for a folder on an SSH host (what the agent's edits name)
pub fn folder_path(folder: &str) -> PathBuf {
  match parse_remote_folder(folder) {
    Some(f) => PathBuf::from(f.path),
    None => PathBuf::from(folder),
  }
}

pub fn is_remote(folder: &str) -> bool {
  parse_remote_folder(folder).is_some()
}

/// Where a process for a session folder starts: the folder, or this machine's home for one on an SSH host
pub fn process_folder(folder: String) -> String {
  if is_remote(&folder) { crate::store::data_dir::home_dir().to_string_lossy().into_owned() } else { folder }
}

impl Workspace {
  /// The workspace of a session folder; one on an SSH host connects first when it is not connected
  pub async fn open(folder: &str, connections: Option<&Connections>) -> Result<Workspace, ProjectError> {
    let Some(f) = parse_remote_folder(folder) else { return Ok(Workspace::Local(PathBuf::from(folder))) };
    let connections = connections.ok_or_else(|| ProjectError::Other("this engine has no SSH connections".into()))?;
    let client = connections.ensure(&f.alias, CONNECT_WAIT).await.map_err(|e| ProjectError::Other(format!("{e:#}")))?;
    Ok(Workspace::Remote { client, path: PathBuf::from(f.path) })
  }

  pub fn path(&self) -> &Path {
    match self {
      Workspace::Local(p) | Workspace::Remote { path: p, .. } => p,
    }
  }

  /// One `project` op on the host; its `error` comes back as the local one would
  async fn call(client: &RemoteClient, root: &Path, op: &str, mut a: Value) -> Result<Value, ProjectError> {
    a["op"] = json!(op);
    a["cwd"] = json!(root);
    let r = client.call("project", a).await.map_err(|e| ProjectError::Other(e.to_string()))?;
    match r.value.get("error").and_then(Value::as_str) {
      Some(code) => Err(ProjectError::from_code(code)),
      None => Ok(r.value),
    }
  }

  fn field<T: serde::de::DeserializeOwned + Default>(v: &Value, key: &str) -> T {
    v.get(key).and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or_default()
  }

  pub async fn list_dir(&self, rel: &str) -> Result<Vec<ProjectEntry>, ProjectError> {
    match self {
      Workspace::Local(root) => project_files::list_dir(root, rel).await,
      Workspace::Remote { client, path } => Self::call(client, path, "listDir", json!({ "rel": rel })).await.map(|v| Self::field(&v, "entries")),
    }
  }

  pub async fn git_marks(&self) -> BTreeMap<String, String> {
    match self {
      Workspace::Local(root) => project_files::git_marks(root).await,
      Workspace::Remote { client, path } => Self::call(client, path, "gitMarks", json!({})).await.map(|v| Self::field(&v, "marks")).unwrap_or_default(),
    }
  }

  /// A file of the folder by its relative path, or an explicit local path (a plan or session-edited file outside the project),
  /// as the viewer shows it (`project_files::read_text`)
  pub async fn read_text(&self, rel: &str) -> Result<String, ProjectError> {
    match self {
      Workspace::Local(root) => {
        let p = Path::new(rel);
        let resolved = if p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
          p.to_path_buf()
        } else {
          project_files::resolve(root, rel)?
        };
        project_files::read_text(&resolved).await
      }
      Workspace::Remote { client, path } => Self::call(client, path, "readText", json!({ "rel": rel })).await.map(|v| Self::field(&v, "text")),
    }
  }

  pub async fn head_text(&self, rel: &str) -> Option<String> {
    if Path::new(rel).is_absolute() {
      return None;
    }
    match self {
      Workspace::Local(root) => project_files::head_text(root, rel).await,
      Workspace::Remote { client, path } => Self::call(client, path, "headText", json!({ "rel": rel })).await.ok().and_then(|v| Self::field(&v, "text")),
    }
  }

  pub async fn session_changes(&self, bases: Vec<(String, Option<String>)>) -> Vec<ProjectChange> {
    match self {
      Workspace::Local(root) => project_files::session_changes(root, bases).await,
      Workspace::Remote { client, path } => {
        Self::call(client, path, "sessionChanges", json!({ "bases": bases })).await.map(|v| Self::field(&v, "files")).unwrap_or_default()
      }
    }
  }

  /// @ search over a folder on an SSH host (`node_files` there). A local folder is the shell's to search
  pub async fn search_files(&self, query: &str) -> Vec<FileHit> {
    match self {
      Workspace::Local(_) => vec![],
      Workspace::Remote { client, path } => {
        Self::call(client, path, "searchFiles", json!({ "query": query })).await.map(|v| Self::field(&v, "files")).unwrap_or_default()
      }
    }
  }

  /// An image file the user attached by its path, at most `max` bytes; None when it is missing, larger or unreadable
  pub async fn read_file(&self, path: &Path, max: u64) -> Option<Vec<u8>> {
    match self {
      Workspace::Local(_) => {
        let meta = tokio::fs::metadata(path).await.ok()?;
        if meta.len() > max {
          return None;
        }
        tokio::fs::read(path).await.ok()
      }
      Workspace::Remote { client, .. } => {
        let st = client.call("stat", json!({ "path": path })).await.ok()?;
        if st.value.get("size").and_then(Value::as_u64)? > max {
          return None;
        }
        client.call("read", json!({ "path": path })).await.ok().map(|r| r.data)
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  /// Calls that hand out a session's folder, and the files allowed to make them: every use there was checked to go
  /// through `Workspace` (or `process_folder`, `is_remote`) or to treat the folder only as a name. A new file on this
  /// list is a decision: read the folder through `Workspace` first
  const FOLDER_CALLERS: &[(&str, &[&str])] = &[
    (".session_cwd(", &["bridge_core.rs", "session_manager.rs"]),
    ("folder_for(", &["bridge_core.rs", "session_manager.rs", "runtime.rs"]),
  ];

  #[test]
  fn no_direct_folder_io() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found: Vec<(String, String)> = vec![];
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
      for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
          stack.push(p);
        } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("workspace.rs") {
          let text = std::fs::read_to_string(&p).unwrap();
          let name = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
          for (needle, _) in FOLDER_CALLERS {
            if text.contains(needle) {
              found.push(((*needle).to_owned(), name.clone()));
            }
          }
        }
      }
    }
    let unexpected: Vec<&(String, String)> =
      found.iter().filter(|(needle, file)| !FOLDER_CALLERS.iter().any(|(n, files)| n == needle && files.contains(&file.as_str()))).collect();
    assert!(unexpected.is_empty(), "a session folder is used here outside `Workspace`: {unexpected:?}");
  }
}
