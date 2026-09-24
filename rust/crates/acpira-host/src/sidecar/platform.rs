//! The host platform over the envelope protocol: every IDE action becomes
//! a platformRequest to the shell, facts come from hello and later platformEvents. Capabilities the shell did not declare
//! fall back host-side where that makes sense (file search walks the workspace) and are otherwise logged

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use acpira_shared::protocol::FileHit;
use acpira_shared::sidecar::{Hello, PLATFORM_RPC_METHODS, PlanTarget, PlatformEvent, PlatformRequest, ShellEnv, SidecarMsg};

use crate::node_files::NodeFiles;
use crate::store::data_dir::home_dir;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether a settings change touched acpira.<section> (None: anything)
pub type Affects = Arc<dyn Fn(Option<&str>) -> bool + Send + Sync>;
pub type Send_ = Arc<dyn Fn(SidecarMsg) + Send + Sync>;

struct Pending {
  method: &'static str,
  tx: oneshot::Sender<Result<Value>>,
}

pub struct SidecarPlatform {
  send: Send_,
  stderr: Arc<dyn Fn(&str) + Send + Sync>,
  ignore_agents: bool,
  caps: HashSet<String>,
  settings: parking_lot::RwLock<Map<String, Value>>,
  env: parking_lot::RwLock<ShellEnv>,
  pending: parking_lot::Mutex<HashMap<String, Pending>>,
  settings_listeners: parking_lot::Mutex<Vec<Arc<dyn Fn(Affects) + Send + Sync>>>,
  focus_listeners: parking_lot::Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
  files: NodeFiles,
  seq: std::sync::atomic::AtomicU64,
  closed: parking_lot::Mutex<Option<String>>,
}

impl SidecarPlatform {
  pub fn new(send: Send_, hello: &Hello, stderr: Arc<dyn Fn(&str) + Send + Sync>, ignore_agents: bool) -> Arc<Self> {
    Arc::new_cyclic(|me: &std::sync::Weak<SidecarPlatform>| {
      let weak = me.clone();
      SidecarPlatform {
        send,
        stderr,
        ignore_agents,
        caps: hello.client.capabilities.iter().cloned().collect(),
        settings: parking_lot::RwLock::new(hello.settings.clone()),
        env: parking_lot::RwLock::new(hello.env.clone()),
        pending: Default::default(),
        settings_listeners: Default::default(),
        focus_listeners: Default::default(),
        files: NodeFiles::new(Arc::new(move || {
          weak.upgrade().map(|p| p.cwd()).unwrap_or_else(|| home_dir().to_string_lossy().into_owned())
        })),
        seq: Default::default(),
        closed: Default::default(),
      }
    })
  }

  pub fn blob_base(&self) -> Option<String> {
    self.env.read().blob_base.clone()
  }

  pub fn log(&self, line: &str) {
    (self.stderr)(line);
  }

  pub fn host_language(&self) -> String {
    self.env.read().host_language.clone()
  }

  pub fn home(&self) -> String {
    home_dir().to_string_lossy().into_owned()
  }

  pub fn cwd(&self) -> String {
    self.env.read().cwd.clone().unwrap_or_else(|| self.home())
  }

  pub fn read_setting(&self, key: &str) -> Option<Value> {
    if self.ignore_agents && key == "agents" {
      return None;
    }
    self.settings.read().get(key).cloned()
  }

  /// The snapshot changes at once so the push right after the write already shows it; the shell persists and echoes
  pub async fn write_setting(&self, key: &str, value: Value) -> Result<()> {
    self.settings.write().insert(key.to_owned(), value.clone());
    if self.caps.contains("writeSetting") {
      self.rpc(PlatformRequest::WriteSetting { key: key.to_owned(), value }).await?;
    }
    Ok(())
  }

  pub fn on_settings_changed(&self, f: Arc<dyn Fn(Affects) + Send + Sync>) {
    self.settings_listeners.lock().push(f);
  }

  pub fn on_window_focus(&self, f: Arc<dyn Fn() + Send + Sync>) {
    self.focus_listeners.lock().push(f);
  }

  pub fn toast(&self, level: &str, text: &str) {
    if !self.notify(PlatformRequest::Toast { level: level.to_owned(), text: text.to_owned() }) {
      (self.stderr)(&format!("[toast:{level}] {text}"));
    }
  }

  /// Without a terminal on the shell side the command is at least shown, so a login / install can be run by hand
  pub fn run_in_terminal(&self, title: String, command: String, args: Vec<String>, env: Option<BTreeMap<String, Option<String>>>) {
    let wire_env = env
      .as_ref()
      .map(|e| e.iter().map(|(k, v)| (k.clone(), v.clone().map(Value::from).unwrap_or(Value::Null))).collect::<Map<String, Value>>());
    if self.notify(PlatformRequest::RunInTerminal { title: title.clone(), command: command.clone(), args: args.clone(), env: wire_env }) {
      return;
    }
    let prefix: Vec<String> = env
      .unwrap_or_default()
      .into_iter()
      .map(|(k, v)| match v {
        None => format!("unset {k};"),
        Some(v) => format!("{k}={v}"),
      })
      .collect();
    let mut parts = vec![prefix.join(" "), command];
    parts.extend(args);
    let line: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    self.toast("error", &format!("{title}: {}", line.join(" ")));
  }

  pub async fn open_resolved_file(&self, path: &str, line: Option<i64>) -> Result<()> {
    if self.caps.contains("openResolvedFile") {
      self.rpc(PlatformRequest::OpenResolvedFile { path: path.to_owned(), line }).await?;
    } else {
      (self.stderr)(&format!("openResolvedFile unsupported by the shell: {path}{}", line.map(|l| format!(":{l}")).unwrap_or_default()));
    }
    Ok(())
  }

  pub async fn open_plan_document(&self, target: PlanTarget) -> Result<()> {
    if self.caps.contains("openPlanDocument") {
      self.rpc(PlatformRequest::OpenPlanDocument { target }).await?;
    } else {
      (self.stderr)("openPlanDocument unsupported by the shell");
    }
    Ok(())
  }

  pub fn open_external(&self, url: &str) {
    if !self.notify(PlatformRequest::OpenExternal { url: url.to_owned() }) {
      (self.stderr)(&format!("openExternal unsupported by the shell: {url}"));
    }
  }

  pub async fn reveal_in_os(&self, path: &str) -> Result<()> {
    if self.caps.contains("revealInOS") {
      self.rpc(PlatformRequest::RevealInOs { path: path.to_owned() }).await?;
    } else {
      (self.stderr)(&format!("revealInOS unsupported by the shell: {path}"));
    }
    Ok(())
  }

  pub fn open_in_editor(&self, session_id: Option<String>) {
    if !self.notify(PlatformRequest::OpenInEditor { session_id }) {
      (self.stderr)("openInEditor unsupported by the shell");
    }
  }

  pub async fn search_files(&self, query: &str) -> Result<Vec<FileHit>> {
    if !self.caps.contains("searchFiles") {
      return Ok(self.files.search(query).await);
    }
    let hits = self.rpc(PlatformRequest::SearchFiles { query: query.to_owned() }).await?;
    Ok(hits.as_array().map(|a| a.iter().filter_map(|h| serde_json::from_value::<FileHit>(h.clone()).ok()).collect()).unwrap_or_default())
  }

  /// Fire-and-forget IDE actions; false when the shell did not declare the method
  fn notify(&self, request: PlatformRequest) -> bool {
    if !self.caps.contains(request.method()) || self.closed.lock().is_some() {
      return false;
    }
    (self.send)(SidecarMsg::PlatformRequest { request_id: None, request });
    true
  }

  async fn rpc(&self, request: PlatformRequest) -> Result<Value> {
    debug_assert!(PLATFORM_RPC_METHODS.contains(&request.method()));
    let method = request.method();
    let id = format!("p{}", self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1);
    let (tx, rx) = oneshot::channel();
    {
      // Checked under the pending lock: close() records the reason before it drains, so a request is either refused here or failed there
      let mut pending = self.pending.lock();
      if let Some(reason) = self.closed.lock().clone() {
        return Err(anyhow!("sidecar closed ({reason})"));
      }
      pending.insert(id.clone(), Pending { method, tx });
    }
    (self.send)(SidecarMsg::PlatformRequest { request_id: Some(id.clone()), request });
    match tokio::time::timeout(RPC_TIMEOUT, rx).await {
      Ok(Ok(r)) => r,
      Ok(Err(_)) => Err(anyhow!("{method}: sidecar closed")),
      Err(_) => {
        self.pending.lock().remove(&id);
        Err(anyhow!("{method}: no response from the shell within {}s", RPC_TIMEOUT.as_secs()))
      }
    }
  }

  pub fn on_response(&self, request_id: &str, result: Option<Value>, error: Option<String>) {
    let Some(p) = self.pending.lock().remove(request_id) else {
      (self.stderr)(&format!("platformResponse for unknown request {request_id}"));
      return;
    };
    let _ = p.tx.send(match error {
      Some(e) => Err(anyhow!(e)),
      None => Ok(result.unwrap_or(Value::Null)),
    });
  }

  pub fn on_event(&self, ev: PlatformEvent) {
    match ev {
      PlatformEvent::WindowFocus => {
        let ls: Vec<_> = self.focus_listeners.lock().clone();
        for f in ls {
          f();
        }
      }
      PlatformEvent::SettingsChanged { keys, settings } => {
        *self.settings.write() = settings;
        let affects: Affects = Arc::new(move |section: Option<&str>| match section {
          None => !keys.is_empty(),
          Some(s) => keys.iter().any(|k| k == s || k.starts_with(&format!("{s}."))),
        });
        let ls: Vec<_> = self.settings_listeners.lock().clone();
        for f in ls {
          f(affects.clone());
        }
      }
      PlatformEvent::EnvChanged { env } => {
        let language_changed = {
          let mut cur = self.env.write();
          let changed = env.host_language.as_ref().is_some_and(|l| *l != cur.host_language);
          if let Some(c) = env.cwd {
            cur.cwd = Some(c);
          }
          if let Some(l) = env.host_language {
            cur.host_language = l;
          }
          if let Some(b) = env.blob_base {
            cur.blob_base = Some(b);
          }
          changed
        };
        if language_changed {
          let affects: Affects = Arc::new(|section: Option<&str>| section.is_none() || section == Some("language"));
          let ls: Vec<_> = self.settings_listeners.lock().clone();
          for f in ls {
            f(affects.clone());
          }
        }
      }
    }
  }

  /// Every request still waiting fails now
  pub fn dispose(&self, reason: &str) {
    *self.closed.lock() = Some(reason.to_owned());
    let pending: Vec<Pending> = self.pending.lock().drain().map(|(_, p)| p).collect();
    for p in pending {
      let _ = p.tx.send(Err(anyhow!("{}: {reason}", p.method)));
    }
  }
}
