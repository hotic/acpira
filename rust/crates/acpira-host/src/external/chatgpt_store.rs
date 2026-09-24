//! ChatGPT mirrors (mirror of src/host/external/ChatGptBridgeStore.ts): a separate writer-owned store; opening a mirror
//! never creates an ACP process, and every host observes the same atomic records by polling

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value};

use acpira_shared::transcript::{SessionSummary, SessionView};

use super::chatgpt_binding::chatgpt_binding;
use super::chatgpt_events::{ChatGptRecord, apply_chatgpt_event, chatgpt_session_id, chatgpt_summary, chatgpt_view, is_chatgpt_id};
use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::store::transcript_store::{LogFn, sort_index};
use crate::util::{iso_of_ms, now_ms};

const POLL: Duration = Duration::from_millis(750);
const UNDO_MS: i64 = 30_000;
const RECORD_LIMIT: u64 = 64 * 1024 * 1024;

pub type ChangeListener = Arc<dyn Fn(Vec<String>) + Send + Sync>;

#[derive(Default)]
struct State {
  records: HashMap<String, ChatGptRecord>,
  stamps: HashMap<String, String>,
  published: HashMap<String, i64>,
}

pub struct ChatGptBridgeStore {
  pub dir: PathBuf,
  log: LogFn,
  /// The bridge executable (this binary) when the bridge is available
  exe: Option<String>,
  state: parking_lot::Mutex<State>,
  listeners: parking_lot::Mutex<Vec<(u64, ChangeListener)>>,
  seq: std::sync::atomic::AtomicU64,
  scanning: tokio::sync::Mutex<()>,
  disposed: std::sync::atomic::AtomicBool,
  timer: parking_lot::Mutex<Option<tokio::task::AbortHandle>>,
  me: Weak<ChatGptBridgeStore>,
}

impl ChatGptBridgeStore {
  pub fn new(dir: PathBuf, log: LogFn, exe: Option<String>) -> Arc<Self> {
    Arc::new_cyclic(|me| ChatGptBridgeStore {
      dir,
      log,
      exe,
      state: Default::default(),
      listeners: Default::default(),
      seq: Default::default(),
      scanning: tokio::sync::Mutex::new(()),
      disposed: Default::default(),
      timer: Default::default(),
      me: me.clone(),
    })
  }

  /// Load once and poll: fs watching misses writes on some external macOS volumes
  pub async fn init(&self, poll: bool) -> Result<()> {
    create_private_dir(&self.dir).await?;
    self.refresh().await;
    if poll && self.timer.lock().is_none() && !self.disposed.load(std::sync::atomic::Ordering::Acquire) {
      let weak = self.me.clone();
      let h = tokio::spawn(async move {
        loop {
          tokio::time::sleep(POLL).await;
          let Some(me) = weak.upgrade() else { return };
          if me.scanning.try_lock().is_ok() {
            me.refresh().await;
          }
        }
      });
      *self.timer.lock() = Some(h.abort_handle());
    }
    Ok(())
  }

  pub fn subscribe(&self, f: ChangeListener) -> u64 {
    let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    id
  }

  pub fn available(&self) -> bool {
    self.exe.is_some()
  }

  pub fn owns(&self, id: &str) -> bool {
    is_chatgpt_id(id)
  }

  pub fn view(&self, id: &str) -> Option<SessionView> {
    let st = self.state.lock();
    let r = st.records.get(id).filter(|r| r.deleted_at.is_none())?;
    let mut v = chatgpt_view(r, now_ms());
    if let Some(exe) = &self.exe
      && v.external.is_some()
    {
      let home = self.dir.parent().and_then(Path::parent).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
      let prompt = chatgpt_binding(&v, exe, &home);
      if let Some(ext) = v.external.as_mut() {
        ext.connection_prompt = Some(prompt);
      }
    }
    Some(v)
  }

  pub fn summaries(&self) -> Vec<SessionSummary> {
    let now = now_ms();
    let mut out: Vec<SessionSummary> =
      self.state.lock().records.values().filter(|r| r.deleted_at.is_none()).map(|r| chatgpt_summary(r, now)).collect();
    sort_index(&mut out);
    out
  }

  fn file(&self, id: &str) -> Result<PathBuf> {
    if !is_chatgpt_id(id) {
      bail!("Invalid ChatGPT mirror ID");
    }
    Ok(self.dir.join(format!("{id}.json")))
  }

  async fn read(&self, id: &str) -> Result<Option<ChatGptRecord>> {
    let file = self.file(id)?;
    let info = match tokio::fs::symlink_metadata(&file).await {
      Ok(i) => i,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
      Err(e) => return Err(e.into()),
    };
    if !info.is_file() || info.file_type().is_symlink() || info.len() > RECORD_LIMIT {
      bail!("Invalid or oversized ChatGPT mirror file");
    }
    let r: ChatGptRecord = serde_json::from_slice(&tokio::fs::read(&file).await?)?;
    let valid = r.version == 1
      && r.id == id
      && chatgpt_session_id(&r.source_key).ok().as_deref() == Some(id)
      && Path::new(&r.cwd).is_absolute()
      && crate::util::ms_of_iso(&r.last_event_at).is_some();
    if !valid {
      bail!("Invalid ChatGPT mirror record");
    }
    Ok(Some(r))
  }

  async fn save(&self, r: &ChatGptRecord) -> Result<()> {
    let body = serde_json::to_vec(r)?;
    if body.len() as u64 > RECORD_LIMIT {
      bail!("Mirror exceeds 64 MiB; start a new mirror");
    }
    write_atomic(&self.file(&r.id)?, &body, Some(0o600)).await
  }

  pub async fn open(&self, source_key: &str, cwd: &str, title: &str) -> Result<SessionView> {
    let id = chatgpt_session_id(source_key)?;
    if !Path::new(cwd).is_absolute() {
      bail!("An absolute project directory is required");
    }
    let resolved = tokio::fs::canonicalize(cwd).await?;
    if !tokio::fs::metadata(&resolved).await?.is_dir() {
      bail!("Project path is not a directory");
    }
    let resolved = resolved.to_string_lossy().into_owned();
    create_private_dir(&self.dir).await?;
    let file = self.file(&id)?;
    with_file_lock(&file, || async {
      if let Some(old) = self.read(&id).await? {
        if old.deleted_at.is_some() {
          bail!("Mirror was deleted; use a new session key");
        }
        if old.cwd != resolved {
          bail!("Session key is already bound to another project");
        }
        return Ok(());
      }
      let at = iso_of_ms(now_ms());
      let title: String = title.trim().chars().take(160).collect();
      self
        .save(&ChatGptRecord {
          version: 1,
          id: id.clone(),
          source_key: source_key.to_owned(),
          cwd: resolved.clone(),
          title: if title.is_empty() { "ChatGPT".into() } else { title },
          created_at: at.clone(),
          updated_at: at.clone(),
          last_event_at: at,
          revision: 1,
          turns: vec![],
          active_turn_id: None,
          active_event_at: None,
          pinned: None,
          deleted_at: None,
          receipts: Map::new(),
        })
        .await
    })
    .await?;
    self.refresh().await;
    self.view(&id).ok_or_else(|| anyhow!("Mirror not found after open"))
  }

  async fn mutate(&self, id: &str, apply: impl FnOnce(&ChatGptRecord) -> Result<Option<ChatGptRecord>>) -> Result<()> {
    create_private_dir(&self.dir).await?;
    let file = self.file(id)?;
    with_file_lock(&file, || async {
      let r = self.read(id).await?.ok_or_else(|| anyhow!("Unknown ChatGPT mirror; connect this conversation first"))?;
      if let Some(next) = apply(&r)? {
        self.save(&next).await?;
      }
      Ok(())
    })
    .await?;
    self.refresh().await;
    Ok(())
  }

  pub async fn accept(&self, id: &str, event: &Value) -> Result<()> {
    self.mutate(id, |r| apply_chatgpt_event(r, event, now_ms())).await
  }

  pub async fn rename(&self, id: &str, title: &str) -> Result<()> {
    if title.trim().is_empty() {
      return Ok(());
    }
    let title: String = title.trim().chars().take(160).collect();
    self
      .mutate(id, |r| {
        if r.deleted_at.is_some() {
          bail!("Mirror was deleted");
        }
        Ok(Some(ChatGptRecord { title, revision: r.revision + 1, ..r.clone() }))
      })
      .await
  }

  pub async fn pin(&self, id: &str, pinned: bool) -> Result<()> {
    self
      .mutate(id, |r| {
        if r.deleted_at.is_some() {
          bail!("Mirror was deleted");
        }
        Ok(Some(ChatGptRecord { pinned: pinned.then_some(true), revision: r.revision + 1, ..r.clone() }))
      })
      .await
  }

  pub async fn delete(&self, id: &str) -> Result<()> {
    self
      .mutate(id, |r| {
        Ok(r.deleted_at.is_none().then(|| ChatGptRecord { deleted_at: Some(now_ms()), revision: r.revision + 1, ..r.clone() }))
      })
      .await
  }

  pub async fn restore(&self, id: &str) -> Result<()> {
    self
      .mutate(id, |r| {
        let Some(at) = r.deleted_at else { return Ok(None) };
        if now_ms() - at > UNDO_MS {
          bail!("The undo window has expired");
        }
        Ok(Some(ChatGptRecord { deleted_at: None, revision: r.revision + 1, ..r.clone() }))
      })
      .await
  }

  pub async fn refresh(&self) {
    if self.disposed.load(std::sync::atomic::Ordering::Acquire) {
      return;
    }
    let _serial = self.scanning.lock().await;
    if let Err(e) = self.scan().await {
      (self.log)(&format!("ChatGPT mirror refresh: {e}"));
    }
  }

  async fn scan(&self) -> Result<()> {
    let mut files = vec![];
    match tokio::fs::read_dir(&self.dir).await {
      Ok(mut rd) => {
        while let Some(e) = rd.next_entry().await? {
          files.push(e.file_name().to_string_lossy().into_owned());
        }
      }
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
      Err(e) => return Err(e.into()),
    }
    let mut found = HashSet::new();
    for f in files {
      let Some(id) = f.strip_suffix(".json").filter(|id| is_chatgpt_id(id)) else { continue };
      found.insert(id.to_owned());
      let stamp = match tokio::fs::symlink_metadata(self.file(id)?).await {
        Ok(m) => {
          let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
          #[cfg(unix)]
          let ino = std::os::unix::fs::MetadataExt::ino(&m);
          #[cfg(not(unix))]
          let ino = 0u64;
          format!("{ino}:{mtime}:{}", m.len())
        }
        Err(e) => {
          self.forget(id, &e.to_string());
          continue;
        }
      };
      if self.state.lock().stamps.get(id) == Some(&stamp) {
        continue;
      }
      match self.read(id).await {
        Ok(Some(r)) => {
          let mut st = self.state.lock();
          st.records.insert(id.to_owned(), r);
          st.stamps.insert(id.to_owned(), stamp);
        }
        Ok(None) => {}
        Err(e) => self.forget(id, &e.to_string()),
      }
    }
    let changed = {
      let mut st = self.state.lock();
      st.records.retain(|id, _| found.contains(id));
      st.stamps.retain(|id, _| found.contains(id));
      let now = now_ms();
      let next: HashMap<String, i64> = st
        .records
        .iter()
        .filter(|(_, r)| r.deleted_at.is_none())
        .map(|(id, r)| (id.clone(), chatgpt_view(r, now).rev.unwrap_or(0)))
        .collect();
      let mut ids: HashSet<&String> = st.published.keys().collect();
      ids.extend(next.keys());
      let changed: Vec<String> = ids.into_iter().filter(|id| st.published.get(*id) != next.get(*id)).cloned().collect();
      st.published = next;
      changed
    };
    if !changed.is_empty() && !self.disposed.load(std::sync::atomic::Ordering::Acquire) {
      let ls: Vec<ChangeListener> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
      for f in ls {
        f(changed.clone());
      }
    }
    // A tiny tombstone after undo expires keeps a delayed writer from resurrecting deleted history
    let expired: Vec<String> = self
      .state
      .lock()
      .records
      .values()
      .filter(|r| r.deleted_at.is_some_and(|d| now_ms() - d > UNDO_MS) && !r.turns.is_empty())
      .map(|r| r.id.clone())
      .collect();
    for id in expired {
      let file = self.file(&id)?;
      with_file_lock(&file, || async {
        if let Some(latest) = self.read(&id).await?
          && latest.deleted_at.is_some_and(|d| now_ms() - d > UNDO_MS)
        {
          self
            .save(&ChatGptRecord { turns: vec![], receipts: Map::new(), active_turn_id: None, revision: latest.revision + 1, ..latest })
            .await?;
        }
        Ok(())
      })
      .await?;
    }
    Ok(())
  }

  fn forget(&self, id: &str, why: &str) {
    let mut st = self.state.lock();
    st.records.remove(id);
    st.stamps.remove(id);
    drop(st);
    (self.log)(&format!("ChatGPT {id}: {why}"));
  }

  pub async fn dispose(&self) {
    self.disposed.store(true, std::sync::atomic::Ordering::Release);
    if let Some(h) = self.timer.lock().take() {
      h.abort();
    }
    let _ = self.scanning.lock().await;
    self.listeners.lock().clear();
  }
}

async fn create_private_dir(dir: &Path) -> Result<()> {
  tokio::fs::create_dir_all(dir).await?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await;
  }
  Ok(())
}
