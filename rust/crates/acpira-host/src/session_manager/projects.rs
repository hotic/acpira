//! Projects, chats and categories: the folders a project shell lists, chat folders, the category list, and the
//! debounced session index that follows the in-memory list

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::protocol::HostMsg;
use acpira_shared::transcript::*;

use super::{CHATS_DIR, INDEX_DEBOUNCE, INDEX_MAX_WAIT, SessionManager, Viewer};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::{t, tp};
use crate::store::transcript_store::sort_index;

impl SessionManager {
  /// A project shell's first connect: the folders sessions already ran in, newest first, become the list's first order
  pub(super) async fn seed_projects(&self) {
    if !(self.deps.projects)() || self.state.lock().categories.projects.is_some() {
      return;
    }
    // A host's own sessions are not this machine's history: a host lists every folder it ever ran one in
    let mut list: Vec<SessionSummary> = self.sessions().into_iter().filter(|s| !self.is_host_session(&s.id)).collect();
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    // Folders a session merely ran in are not projects: the home directory (where windows without a workspace start)
    // and folders that are gone
    let home = crate::store::data_dir::home_dir();
    let cwds: Vec<String> = list
      .into_iter()
      .map(|s| s.cwd)
      .filter(|c| !c.is_empty() && std::path::Path::new(c) != home && !self.is_chat(c) && (acpira_shared::remote::is_remote_folder(c) || std::path::Path::new(c).is_dir()))
      .collect();
    if cwds.is_empty() {
      return;
    }
    if let Err(e) = self.category_op(&CategoryOp::SeedProjects { cwds }).await {
      self.log(&format!("seeding the project list failed: {e}"));
    }
  }

  /// The folder a view means when something needs one: a new session, warming, a probe, native sessions, @ search, the
  /// settings page's project files, ChatGPT. An explicit folder wins; a shell with a workspace means its workspace (as it
  /// always did); a project shell (`ManagerDeps.projects`) means the session the view shows, else the newest session's
  /// project folder (a chat's is its own), else its first registered project, else nothing: no session starts in the home
  /// directory on its own
  pub fn folder_for(&self, v: Option<&Viewer>, explicit: Option<String>) -> Option<String> {
    if let Some(dir) = explicit.filter(|d| !d.trim().is_empty()) {
      return Some(dir);
    }
    if !(self.deps.projects)() {
      return Some((self.deps.cwd)());
    }
    if let Some(dir) = v.and_then(|v| v.active_id()).and_then(|id| self.session_cwd(&id)) {
      return Some(dir);
    }
    if let Some(s) = self.sessions().into_iter().filter(|s| !s.cwd.is_empty() && !self.is_chat(&s.cwd)).max_by(|a, b| a.updated_at.cmp(&b.updated_at)) {
      return Some(s.cwd);
    }
    self.state.lock().categories.projects.as_ref().and_then(|p| p.first().cloned())
  }

  /// A folder a project shell starts something in joins its list (at the end) the first time; a chat's folder never does
  pub(super) async fn register_project(&self, cwd: &str) {
    if !(self.deps.projects)() || self.is_chat(cwd) || self.state.lock().categories.projects.as_ref().is_some_and(|p| p.iter().any(|x| x == cwd)) {
      return;
    }
    if let Err(e) = self.category_op(&CategoryOp::AddProject { cwd: cwd.to_owned() }).await {
      self.log(&format!("registering project {cwd} failed: {e}"));
    }
  }

  /// Where chats live (`<ACPIRA_HOME>/chats`): a project shell's session that belongs to no project gets a fresh folder
  /// of its own here, since an ACP session always runs in one
  pub fn chat_root(&self) -> std::path::PathBuf {
    self.deps.lease_root.join(CHATS_DIR)
  }

  /// A chat's folder: directly inside `chat_root`
  pub fn is_chat(&self, cwd: &str) -> bool {
    let p = std::path::Path::new(cwd);
    p.file_name().is_some() && p.parent() == Some(self.chat_root().as_path())
  }

  /// A new chat's folder, made now: named by its day and a random tail, never by the prompt (a pasted key would land in
  /// the file system)
  pub(super) async fn new_chat_folder(&self) -> Result<String> {
    let root = self.chat_root();
    tokio::fs::create_dir_all(&root).await?;
    let day = crate::util::now_iso().chars().take(10).collect::<String>();
    loop {
      let dir = root.join(format!("{day}-{}", crate::util::random_hex(3)));
      match tokio::fs::create_dir(&dir).await {
        Ok(()) => return Ok(dir.to_string_lossy().into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
        Err(e) => return Err(e.into()),
      }
    }
  }

  /// A chat that is gone takes its folder along while nothing else runs there: whole (`all`) for one never prompted, else
  /// only when nothing was written there, so a deleted chat's outputs stay
  pub(super) async fn remove_chat_folder(&self, cwd: &str, all: bool) {
    if !self.is_chat(cwd) {
      return;
    }
    let used = {
      let st = self.state.lock();
      st.index.iter().any(|s| s.cwd == cwd) || st.live.values().any(|s| s.cwd == cwd)
    };
    if used {
      return;
    }
    let _ = if all { tokio::fs::remove_dir_all(cwd).await } else { tokio::fs::remove_dir(cwd).await };
  }

  /// The in-memory list changed: bring the disk index along shortly (debounced)
  pub(super) fn save_index(&self) {
    let now = tokio::time::Instant::now();
    let spawn = {
      let mut st = self.state.lock();
      let (first, _) = st.sync_due.unwrap_or((now, now));
      let due = (now + INDEX_DEBOUNCE).min(first + INDEX_MAX_WAIT);
      let fresh = st.sync_due.is_none();
      st.sync_due = Some((first, due));
      fresh
    };
    if !spawn {
      return;
    }
    let weak = self.me.clone();
    tokio::spawn(async move {
      loop {
        let Some(m) = weak.upgrade() else { return };
        let due = match m.state.lock().sync_due {
          Some((_, d)) => d,
          None => return,
        };
        if due > tokio::time::Instant::now() {
          drop(m);
          tokio::time::sleep_until(due).await;
          continue;
        }
        m.state.lock().sync_due = None;
        m.sync_index().await;
        return;
      }
    });
  }

  /// Index entries without a model (written before the field existed, or rewritten by an older engine that drops it)
  /// learn it from their records in the background, so the list's mark flips to the model's vendor without each session
  /// being opened first. Reading every record's controls takes a while on a large store; the list shows meanwhile
  pub(super) fn backfill_models(&self) {
    let ids: Vec<String> = self.state.lock().index.iter().filter(|s| s.model.is_none() && s.external.is_none()).map(|s| s.id.clone()).collect();
    if ids.is_empty() {
      return;
    }
    let me = self.me.clone();
    tokio::spawn(async move {
      let Some(store) = me.upgrade().map(|m| m.deps.store.clone()) else { return };
      let started = std::time::Instant::now();
      let found = store.read_models(&ids).await;
      let Some(m) = me.upgrade() else { return };
      m.log(&format!("model backfill: read {} records in {} ms", ids.len(), started.elapsed().as_millis()));
      if found {
        m.sync_index().await;
      }
    });
  }

  /// Reconcile the list with the sessions directory now
  pub async fn refresh_index(self: &Arc<Self>) {
    self.state.lock().sync_due = None;
    self.sync_index().await;
    self.sync_shapes().await;
    self.sync_categories().await;
    // Engines off the bus (Windows, older ones) change them too: picked up here as the rest of the list is
    self.reload_unread().await;
    if let Some(c) = &self.deps.chatgpt {
      c.refresh().await;
    }
  }

  /// Categories another window edited reach this one's list
  pub(super) async fn sync_categories(&self) {
    let disk = self.deps.store.load_categories().await;
    self.set_categories(disk);
  }

  /// Keep `list` and push it to every page when it differs from what they have
  fn set_categories(&self, list: SessionCategories) {
    {
      let mut st = self.state.lock();
      if st.categories == list {
        return;
      }
      st.categories = list.clone();
    }
    self.emit(HostMsg::Categories { categories: list });
  }

  pub fn categories(&self) -> SessionCategories {
    self.state.lock().categories.clone()
  }

  pub async fn category_op(&self, op: &CategoryOp) -> Result<()> {
    // A folder on an SSH host is listed in its one canonical spelling, so two spellings never make two projects
    let normalized;
    let op = match op {
      CategoryOp::AddProject { cwd } if acpira_shared::remote::is_remote_folder(cwd) => {
        let canonical = acpira_shared::remote::parse_remote_folder(cwd).map(|f| f.uri()).ok_or_else(|| anyhow!(tp("host.folderMissing", &[("folder", cwd)])))?;
        normalized = CategoryOp::AddProject { cwd: canonical };
        &normalized
      }
      other => other,
    };
    let list = self.deps.store.update_categories(op).await?;
    self.set_categories(list);
    self.bus.touch("categories");
    Ok(())
  }

  /// A folder something is about to start in, as it will be stored: a local folder must exist; a folder on an SSH host
  /// (`ssh://<alias>/<path>`, docs/dev/remote.md) is put in its canonical spelling and must be a folder on the host,
  /// which is connected for that. `missing`: the message key for a folder that is not there
  pub(crate) async fn check_folder(&self, dir: &str, missing: &str) -> Result<String> {
    let gone = || anyhow!(tp(missing, &[("folder", dir)]));
    if !acpira_shared::remote::is_remote_folder(dir) {
      return if tokio::fs::metadata(dir).await.is_ok_and(|m| m.is_dir()) { Ok(dir.to_owned()) } else { Err(gone()) };
    }
    let folder = acpira_shared::remote::parse_remote_folder(dir).ok_or_else(gone)?;
    let connections = self.deps.remote.clone().ok_or_else(gone)?;
    let client = connections.ensure(&folder.alias, std::time::Duration::from_secs(30)).await?;
    let stat = client.call("stat", serde_json::json!({ "path": folder.path })).await.map_err(|_| gone())?;
    if stat.value.get("kind").and_then(Value::as_str) != Some("dir") {
      return Err(gone());
    }
    Ok(folder.uri())
  }

  /// A category id the session may be filed under: one that exists and belongs to the session's project
  pub(super) fn category_for(&self, category: Option<String>, cwd: &str) -> Option<Option<String>> {
    match category {
      None => Some(None),
      Some(id) => self.state.lock().categories.categories.iter().any(|c| c.id == id && c.cwd == cwd).then_some(Some(id)),
    }
  }

  /// One run at a time; a request mid-run schedules exactly one more. A live session whose record left the directory
  /// although the store once had it was deleted by another window: this host follows suit
  pub(super) fn sync_index(self: &Arc<Self>) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move {
      let busy = {
        let mut st = me.state.lock();
        if st.sync_running {
          st.sync_again = true;
        } else {
          st.sync_running = true;
        }
        st.sync_again
      };
      if busy {
        let _wait = me.sync_lock.lock().await;
        return;
      }
      let _serial = me.sync_lock.lock().await;
      loop {
        let (mut mine, own) = {
          let mut st = me.state.lock();
          st.sync_again = false;
          let mut own: HashSet<String> = st.live.keys().cloned().collect();
          own.extend(st.touched.drain());
          (st.index.clone(), own)
        };
        // A live session's latest change may still wait for its flush: the shared file gets its current summary, not the stale copy
        for s in me.live_sessions() {
          let sum = s.summary();
          match mine.iter().position(|x| x.id == s.id) {
            Some(i) => mine[i] = sum,
            None => mine.push(sum),
          }
        }
        match me.deps.store.sync_index(&mine, &own).await {
          Ok(merged) => {
            let trash: HashSet<String> = me.state.lock().trash.keys().cloned().collect();
            let mut merged: Vec<SessionSummary> = merged.into_iter().filter(|s| !trash.contains(&s.id)).collect();
            let live = me.live_sessions();
            let mut gone: Vec<String> =
              live.iter().filter(|s| me.deps.store.knew(&s.id) && !merged.iter().any(|m| m.id == s.id)).map(|s| s.id.clone()).collect();
            // A mirrored session deleted by the engine driving it goes the same way
            let mirrors: Vec<String> = me.state.lock().mirrors.keys().cloned().collect();
            gone.extend(mirrors.into_iter().filter(|id| !merged.iter().any(|m| &m.id == id)));
            for id in &gone {
              me.forget(id);
            }
            for s in me.live_sessions() {
              let sum = s.summary();
              match merged.iter().position(|x| x.id == s.id) {
                Some(i) => merged[i] = sum,
                None => merged.push(sum),
              }
            }
            sort_index(&mut merged);
            let changed = {
              let mut st = me.state.lock();
              let changed = st.index != merged;
              if changed {
                st.index = merged;
              }
              changed
            };
            if changed {
              me.emit_sessions();
            }
            for id in gone {
              if !me.viewers_on(&id, None).is_empty() {
                (me.deps.toast)("info", &t("host.deletedElsewhere"));
              }
              me.rehome(&id).await;
            }
          }
          Err(e) => me.log(&format!("index sync failed: {e}")),
        }
        let again = {
          let mut st = me.state.lock();
          let again = st.sync_again;
          if !again {
            st.sync_running = false;
          }
          again
        };
        if !again {
          break;
        }
      }
    })
  }
}
