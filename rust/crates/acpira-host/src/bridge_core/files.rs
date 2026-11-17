//! Files a page opens, stages or searches: paths resolved against the session's folders, SSH host files copied in first

use std::path::Path;
use std::sync::Arc;

use acpira_shared::protocol::{FileHit, HostMsg, ProjectBase};
use acpira_shared::remote::normalize_remote_path;

use super::{BridgeCore, REMOTE_OPEN_MAX, spawn_on};
use crate::acp::transcript::normalize::file_url_to_path;
use crate::i18n::tp;
use crate::project_files;
use crate::store::data_dir::{home_dir, normalize};

impl BridgeCore {
  pub(super) async fn on_open_file(self: &Arc<Self>, session_id: String, path: String, line: Option<i64>, reveal: bool) -> anyhow::Result<()> {
    let (manager, platform) = (&self.manager, &self.platform);
    // Tool references are relative to the session they came from; a view showing another session ignores them
    let Some(active) = self.viewer.active_id().filter(|a| *a == session_id) else { return Ok(()) };
    let Some(cwd) = manager.session_cwd(&active) else { return Ok(()) };
    // A file on an SSH host opens as a read-only copy fetched here, so the editor of choice works with the file the
    // agent names; the copy has no folder worth revealing
    if let Some(f) = acpira_shared::remote::parse_remote_folder(&cwd) {
      let named = if crate::platform::file_url::is_file_url(&path) { file_url_to_path(&path) } else { Some(path.clone()) };
      let root = manager.data_root();
      let host_path = named
        .filter(|p| !reveal && *p != "~" && !p.starts_with("~/"))
        .and_then(|p| normalize_remote_path(&crate::acp::vendors::claude_remote::resolve_host_path(&f.alias, &f.path, &root, &p)));
      let Some(host_path) = host_path else {
        platform.toast("info", &tp("host.remoteFile", &[("path", &path), ("host", &f.alias)]));
        return Ok(());
      };
      let core = self.clone();
      let copy = remote_copy_path(&manager.data_root(), &f.alias, &host_path);
      spawn_on(self.wire, async move { core.open_remote_file(cwd, f.alias, host_path, copy, line.filter(|l| *l > 0)).await });
      return Ok(());
    }
    let resolved = if crate::platform::file_url::is_file_url(&path) {
      file_url_to_path(&path).ok_or_else(|| anyhow::anyhow!("File URL must be absolute"))
    } else {
      Ok(normalize(&resolve_open_path(&cwd, &path)).to_string_lossy().into_owned())
    };
    match resolved {
      Ok(mut p) => {
        let missing = |p: &str| {
          let p = p.to_owned();
          async move { tokio::fs::metadata(&p).await.is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) }
        };
        // The session folder wins. A name missing there may be relative to another folder of the project, or lead
        // with a folder further up (`acpira-web/out/x.mp4` said from `acpira/rust`): the first that exists opens
        if missing(&p).await {
          let roots: Vec<String> = std::iter::once(cwd.clone())
            .chain(manager.categories().other_folders(&cwd).into_iter().filter(|f| !crate::workspace::is_remote(f)).map(|f| crate::workspace::folder_path(&f).to_string_lossy().into_owned()))
            .collect();
          for cand in fallback_open_paths(&roots, &path) {
            if !missing(&cand).await {
              p = cand;
              break;
            }
          }
        }
        // Localize a missing link before the OS opener returns its own untranslated error.
        if missing(&p).await {
          platform.toast("error", &tp("host.fileMissing", &[("path", &p)]));
          return Ok(());
        }
        let res = if reveal {
          platform.reveal_in_os(&p).await
        } else {
          platform.open_resolved_file(&p, line.filter(|l| *l > 0)).await
        };
        if let Err(e) = res {
          platform.toast("error", &e.to_string());
        }
      }
      Err(e) => platform.toast("error", &e.to_string()),
    }
    Ok(())
  }

  pub(super) async fn on_search_files(self: &Arc<Self>, query: String, seq: i64) -> anyhow::Result<()> {
    let (manager, platform) = (&self.manager, &self.platform);
    // Always answer, even on failure: the webview holds a promise per seq
    let root = manager.folder_for(Some(&self.viewer), None);
    // A folder on an SSH host is searched there; the shell's own search only knows this machine
    if let Some(root) = root.clone().filter(|r| crate::workspace::is_remote(r)) {
      let core = self.clone();
      spawn_on(self.wire, async move {
        let files = match core.workspace(Some(root)).await {
          Ok(ws) => ws.search_files(&query).await,
          Err(_) => vec![],
        };
        core.post_now(HostMsg::Files { seq, files });
      });
      return Ok(());
    }
    let files: Vec<FileHit> = match platform.search_files(root.as_deref(), &query).await {
      Ok(f) => f,
      Err(e) => {
        platform.log(&format!("searchFiles failed: {e}"));
        vec![]
      }
    };
    self.post_now(HostMsg::Files { seq, files });
    Ok(())
  }

  pub(super) async fn on_image_blob(self: &Arc<Self>, session_id: String, path: String, seq: i64, shown: bool) -> anyhow::Result<()> {
    let manager = &self.manager;
    let (core, cwd) = (self.clone(), manager.session_cwd(&session_id));
    spawn_on(self.wire, async move {
      let blob = async {
        // A paired device may stage only what the session shows, never any path of this machine
        if shown && !core.manager.shows_file(&session_id, &path) {
          return None;
        }
        let cwd = cwd?;
        let resolved = core.staged_path(&cwd, &path)?;
        let ws = core.workspace(Some(cwd)).await.ok()?;
        core.manager.stage_file(&session_id, &ws, &resolved).await
      }
      .await;
      core.post_now(HostMsg::ImageBlob { seq, blob });
    });
    Ok(())
  }

  pub(super) async fn on_stage_file(self: &Arc<Self>, session_id: String, path: String, seq: i64) -> anyhow::Result<()> {
    let manager = &self.manager;
    let (core, cwd) = (self.clone(), manager.session_cwd(&session_id));
    spawn_on(self.wire, async move {
      let staged = async {
        let ws = core.workspace(cwd.clone()).await?;
        let resolved = cwd.as_deref().and_then(|c| core.staged_path(c, &path)).ok_or(project_files::ProjectError::Outside)?;
        core.manager.stage_any_file(&session_id, &ws, &resolved).await
      }
      .await;
      let msg = match staged {
        Ok((blob, size)) => HostMsg::StagedFile { seq, blob: Some(blob), size: Some(size), error: None },
        Err(e) => HostMsg::StagedFile { seq, blob: None, size: None, error: Some(e.code()) },
      };
      core.post_now(msg);
    });
    Ok(())
  }

  pub(super) async fn on_project_file(self: &Arc<Self>, session_id: String, path: String, base: Option<ProjectBase>, seq: i64) -> anyhow::Result<()> {
    let manager = &self.manager;
    let (core, mut root) = (self.clone(), manager.session_cwd(&session_id));
    // A `~` / `~/…` path on a local workspace resolves against the user's home, and an absolute path inside `root`
    // shortens to its relative path so git marks and HEAD lookups apply
    let path = match root.clone() {
      Some(r) if !crate::workspace::is_remote(&r) => {
        let expanded = if path == "~" {
          home_dir().to_string_lossy().into_owned()
        } else if let Some(rest) = path.strip_prefix("~/").or_else(|| if cfg!(windows) { path.strip_prefix("~\\") } else { None }) {
          home_dir().join(rest).to_string_lossy().into_owned()
        } else {
          path
        };
        let root_path = crate::workspace::folder_path(&r);
        match Path::new(&expanded).strip_prefix(&root_path) {
          Ok(rel) if !rel.as_os_str().is_empty() => project_files::relative(&root_path, &expanded),
          // A file in another folder of the session's project reads from that folder, its own git marks and HEAD
          _ => {
            let other = manager.categories().other_folders(&r).into_iter().find(|f| !crate::workspace::is_remote(f) && Path::new(&expanded).starts_with(crate::workspace::folder_path(f)));
            match other {
              Some(f) => {
                let rel = project_files::relative(&crate::workspace::folder_path(&f), &expanded);
                root = Some(f);
                rel
              }
              None => expanded,
            }
          }
        }
      }
      _ => path,
    };
    // The session's base is looked up now, while its view is at hand
    let session_base = match root.as_ref() {
      Some(r) => {
        let abs = crate::workspace::folder_path(r).join(&path);
        manager.edit_bases(&session_id).await.into_iter().find(|(p, _)| Path::new(p) == abs).and_then(|(_, b)| b)
      }
      None => None,
    };
    spawn_on(self.wire, async move {
      let ws = match core.workspace(root).await {
        Ok(ws) => ws,
        Err(e) => {
          core.post_now(HostMsg::ProjectFile { seq, text: None, hunks: vec![], error: Some(e.code()) });
          return;
        }
      };
      let text = match ws.read_text(&path).await {
        Ok(text) => text,
        Err(e) => {
          core.post_now(HostMsg::ProjectFile { seq, text: None, hunks: vec![], error: Some(e.code()) });
          return;
        }
      };
      let base_text = match base {
        Some(ProjectBase::Session) => session_base,
        Some(ProjectBase::Head) => ws.head_text(&path).await.or(session_base),
        None => None,
      };
      let (text, hunks) = match base_text {
        Some(b) => tokio::task::spawn_blocking(move || {
          let h = project_files::hunks(&b, &text);
          (text, h)
        })
        .await
        .unwrap_or_default(),
        None => (text, vec![]),
      };
      core.post_now(HostMsg::ProjectFile { seq, text: Some(text), hunks, error: None });
    });
    Ok(())
  }

  /// A session folder's files (`workspace.rs`); "noProject" without one
  /// The file a staging request names, read against the session's folder: a file URL or a path, `~` for this machine's
  /// home. On an SSH host the host's own path (the Claude stand-in moved back, a relative one read against the host
  /// folder, `/` joins even on a Windows engine); `~` would name this machine's home there, so it is refused
  fn staged_path(&self, cwd: &str, path: &str) -> Option<std::path::PathBuf> {
    let named = if crate::platform::file_url::is_file_url(path) { file_url_to_path(path)? } else { path.to_owned() };
    match acpira_shared::remote::parse_remote_folder(cwd) {
      Some(f) => {
        if named == "~" || named.starts_with("~/") {
          return None;
        }
        let resolved = crate::acp::vendors::claude_remote::resolve_host_path(&f.alias, &f.path, &self.manager.data_root(), &named);
        Some(std::path::PathBuf::from(normalize_remote_path(&resolved)?))
      }
      None => Some(normalize(&resolve_open_path(cwd, &named))),
    }
  }

  pub(super) async fn workspace(&self, root: Option<String>) -> Result<crate::workspace::Workspace, project_files::ProjectError> {
    let root = root.ok_or_else(|| project_files::ProjectError::Other("noProject".into()))?;
    crate::workspace::Workspace::open(&root, Some(&self.connections)).await
  }

  /// A file on an SSH host fetched into its read-only copy here (`remote_copy_path`), then opened as a local file is. Every
  /// open fetches again, so the copy shows the host's file as it is now
  async fn open_remote_file(&self, folder: String, alias: String, host_path: String, copy: std::path::PathBuf, line: Option<i64>) {
    let fetched = match self.workspace(Some(folder)).await {
      Ok(ws) => ws.read_file(Path::new(&host_path), REMOTE_OPEN_MAX).await,
      Err(e) => {
        self.platform.log(&format!("openFile {alias}:{host_path}: {e:?}"));
        None
      }
    };
    let Some(bytes) = fetched else {
      self.platform.toast("error", &tp("host.remoteFileFailed", &[("path", &host_path), ("host", &alias)]));
      return;
    };
    let res = match write_read_only(&copy, &bytes).await {
      Ok(()) => self.platform.open_resolved_file(&copy.to_string_lossy(), line).await,
      Err(e) => Err(e.into()),
    };
    if let Err(e) = res {
      self.platform.toast("error", &e.to_string());
    }
  }
}

/// Where a file on an SSH host is copied to open here: `<data root>/run/remote/files/<alias>/<host path>`, a cache apart
/// from the Claude stand-in folders, which must stay empty. `host_path` is normalized (no `..`); each segment is made a
/// safe local name, so neither an alias like `..` nor a character Windows refuses leaves the cache or fails the write
pub(super) fn remote_copy_path(data_root: &Path, alias: &str, host_path: &str) -> std::path::PathBuf {
  let safe = |seg: &str| -> String {
    let s: String = seg.chars().map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\') { '_' } else { c }).collect();
    if s.chars().all(|c| c == '.') { s.replace('.', "_") } else { s }
  };
  let mut p = data_root.join("run").join("remote").join("files").join(safe(alias));
  for seg in host_path.split('/').filter(|s| !s.is_empty()) {
    p.push(safe(seg));
  }
  p
}

/// The copy written tmp + rename and left read-only, so an edit there is not mistaken for one on the host. The old copy
/// goes first: Windows refuses to rename over a read-only file
async fn write_read_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
  tokio::fs::create_dir_all(dir).await?;
  let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
  tokio::fs::write(&tmp, bytes).await?;
  let mut perms = tokio::fs::metadata(&tmp).await?.permissions();
  perms.set_readonly(true);
  tokio::fs::set_permissions(&tmp, perms).await?;
  let _ = tokio::fs::remove_file(path).await;
  tokio::fs::rename(&tmp, path).await
}

fn resolve_open_path(cwd: &str, path: &str) -> std::path::PathBuf {
  if path == "~" {
    return home_dir();
  }
  if let Some(rest) = path.strip_prefix("~/").or_else(|| if cfg!(windows) { path.strip_prefix("~\\") } else { None }) {
    return home_dir().join(rest);
  }
  Path::new(cwd).join(path)
}

/// Where else a relative name may live, in order: under each other folder of the project, then under each folder above
/// the roots (nearest first, never the filesystem root). Only a plain relative name qualifies: absolute, `~` and
/// explicitly anchored (`./`, `../`) paths mean exactly what they say. `roots[0]` is the session folder, already tried
pub(super) fn fallback_open_paths(roots: &[String], path: &str) -> Vec<String> {
  let rel = Path::new(path);
  let plain = matches!(rel.components().next(), Some(std::path::Component::Normal(_)));
  // Path::components drops a leading `./`, so check the text itself
  if !plain || path.starts_with('~') || path.starts_with("./") || (cfg!(windows) && path.starts_with(".\\")) {
    return vec![];
  }
  let under_roots = roots.iter().skip(1).map(|r| Path::new(r).to_path_buf());
  let above_roots = roots.iter().flat_map(|r| Path::new(r).ancestors().skip(1).filter(|a| a.parent().is_some()).map(Path::to_path_buf));
  let mut out: Vec<String> = Vec::new();
  for base in under_roots.chain(above_roots) {
    let cand = normalize(&base.join(rel)).to_string_lossy().into_owned();
    if !out.contains(&cand) {
      out.push(cand);
    }
  }
  out
}
