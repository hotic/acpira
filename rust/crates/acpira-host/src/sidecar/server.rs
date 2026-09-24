//! The sidecar's state machine over one wire: hello (version-checked) →
//! runtime → views. Control messages are processed strictly in order. A view's WebviewMsg runs its synchronous prefix
//! in order and continues concurrently, exactly like `void core.handle(m)` in the TS host: a `send` claims its turn
//! before a following `stop` is looked at, and the `stop` never queues behind the whole turn

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;

use acpira_shared::protocol::HostMsg;
use acpira_shared::sidecar::{Hello, SIDECAR_PROTOCOL_VERSION, ShellMsg, SidecarInfo, SidecarMsg};

use super::platform::SidecarPlatform;
use crate::bridge_core::BridgeCore;
use crate::runtime::HostRuntime;

pub struct ServerOpts {
  pub version: String,
  pub home: PathBuf,
  pub log: Arc<dyn Fn(&str) + Send + Sync>,
  /// Browser harness: the connecting page must not supply executable command lines
  pub ignore_client_agents: bool,
  /// The executable the ChatGPT connection prompt names (this binary)
  pub bridge_exe: Option<String>,
}

pub struct SidecarServer {
  opts: ServerOpts,
  out: mpsc::UnboundedSender<String>,
  platform: Option<Arc<SidecarPlatform>>,
  runtime: Option<Arc<HostRuntime>>,
  views: HashMap<String, Arc<BridgeCore>>,
  done: bool,
}

impl SidecarServer {
  pub fn new(opts: ServerOpts, out: mpsc::UnboundedSender<String>) -> Self {
    SidecarServer { opts, out, platform: None, runtime: None, views: HashMap::new(), done: false }
  }

  fn log(&self, line: &str) {
    (self.opts.log)(line);
  }

  fn sender(&self) -> Arc<dyn Fn(SidecarMsg) + Send + Sync> {
    let out = self.out.clone();
    Arc::new(move |m: SidecarMsg| {
      if let Ok(line) = serde_json::to_string(&m) {
        let _ = out.send(line);
      }
    })
  }

  fn send(&self, m: SidecarMsg) {
    if !self.done {
      (self.sender())(m);
    }
  }

  /// Serve until the shell asks to shut down or the wire closes; resolves to the exit code (2 = protocol rejected)
  pub async fn run(mut self, mut lines: mpsc::UnboundedReceiver<String>) -> i32 {
    while let Some(line) = lines.recv().await {
      if let Some(code) = self.on_line(&line).await {
        return code;
      }
    }
    // Lines received before EOF were all honoured above
    self.finish("wire closed", 0, false).await
  }

  /// A line that is not a JSON envelope is noise on the channel: logged and skipped, never fatal
  async fn on_line(&mut self, line: &str) -> Option<i32> {
    let text = line.trim();
    if text.is_empty() || self.done {
      return None;
    }
    let head: String = text.chars().take(120).collect();
    let parsed: Value = match serde_json::from_str(text) {
      Ok(v) => v,
      Err(_) => {
        self.log(&format!("ignoring non-JSON line from the shell: {head}"));
        return None;
      }
    };
    let kind = parsed.get("type").and_then(Value::as_str).map(str::to_owned);
    let Some(kind) = kind else {
      self.log(&format!("ignoring envelope without a type: {head}"));
      return None;
    };
    let m: ShellMsg = match serde_json::from_value(parsed) {
      Ok(m) => m,
      Err(e) => {
        self.log(&format!("{kind} failed: {e}"));
        return None;
      }
    };
    self.handle(m).await
  }

  async fn handle(&mut self, m: ShellMsg) -> Option<i32> {
    match m {
      ShellMsg::Hello(h) => return self.hello(*h).await,
      ShellMsg::Shutdown => return Some(self.finish("shutdown requested", 0, true).await),
      _ => {}
    }
    let (Some(runtime), Some(platform)) = (self.runtime.clone(), self.platform.clone()) else {
      self.log(&format!("{} before hello, ignored", shell_kind(&m)));
      return None;
    };
    match m {
      ShellMsg::AttachView { view_id, host, initial } => {
        if self.views.contains_key(&view_id) {
          self.log(&format!("attachView: {view_id} already attached, ignored"));
          return None;
        }
        let send = self.sender();
        let vid = view_id.clone();
        let post = Arc::new(move |message: HostMsg| send(SidecarMsg::HostMessage { view_id: vid.clone(), message }));
        let core = runtime.attach_view(host, initial, platform.blob_base(), post);
        self.views.insert(view_id, core);
      }
      ShellMsg::DetachView { view_id } => {
        if let Some(core) = self.views.remove(&view_id) {
          runtime.detach_view(&core);
        }
      }
      ShellMsg::WebviewMessage { view_id, message } => match self.views.get(&view_id) {
        Some(core) => crate::util::run_prefix(core.clone().handle(message)),
        None => {
          let kind = message.get("type").and_then(Value::as_str).unwrap_or("undefined");
          self.log(&format!("webviewMessage for unknown view {view_id} ({kind}), ignored"));
        }
      },
      ShellMsg::PlatformResponse { request_id, result, error } => platform.on_response(&request_id, result, error),
      ShellMsg::PlatformEvent { event } => platform.on_event(event),
      ShellMsg::Hello(_) | ShellMsg::Shutdown => unreachable!("handled above"),
    }
    None
  }

  async fn hello(&mut self, m: Hello) -> Option<i32> {
    if m.protocol_version != SIDECAR_PROTOCOL_VERSION {
      self.send(SidecarMsg::HelloReject {
        request_id: m.request_id,
        protocol_version: SIDECAR_PROTOCOL_VERSION,
        reason: format!("protocol version {} is not {SIDECAR_PROTOCOL_VERSION}", m.protocol_version),
      });
      return Some(self.finish(&format!("protocol version mismatch ({})", m.protocol_version), 2, false).await);
    }
    if let Some(rt) = &self.runtime {
      // A second hello on the same wire is a shell bug, but a harmless one
      let sessions_dir = rt.sessions_dir.to_string_lossy().into_owned();
      self.send(self.hello_ok(m.request_id, sessions_dir));
      return None;
    }
    let log = self.opts.log.clone();
    let platform = SidecarPlatform::new(self.sender(), &m, log, self.opts.ignore_client_agents);
    self.platform = Some(platform.clone());
    match HostRuntime::create(platform, self.opts.home.clone(), self.opts.bridge_exe.clone()).await {
      Ok(rt) => {
        let sessions_dir = rt.sessions_dir.to_string_lossy().into_owned();
        self.runtime = Some(rt);
        self.send(self.hello_ok(m.request_id, sessions_dir));
        None
      }
      Err(e) => {
        self.send(SidecarMsg::HelloReject {
          request_id: m.request_id,
          protocol_version: SIDECAR_PROTOCOL_VERSION,
          reason: format!("runtime failed to start: {e}"),
        });
        Some(self.finish(&format!("runtime failed: {e}"), 1, false).await)
      }
    }
  }

  fn hello_ok(&self, request_id: String, sessions_dir: String) -> SidecarMsg {
    SidecarMsg::HelloOk {
      request_id,
      protocol_version: SIDECAR_PROTOCOL_VERSION,
      sidecar: SidecarInfo { version: self.opts.version.clone(), pid: std::process::id() },
      sessions_dir,
    }
  }

  /// Tear everything down once: pending platform RPCs fail, sessions flush, the wire gets shutdownOk when asked
  async fn finish(&mut self, reason: &str, code: i32, ack: bool) -> i32 {
    if self.done {
      return code;
    }
    self.log(&format!("sidecar finishing: {reason}"));
    if let Some(p) = &self.platform {
      p.dispose(reason);
    }
    self.views.clear();
    if let Some(rt) = self.runtime.take() {
      rt.dispose().await;
    }
    if ack {
      self.send(SidecarMsg::ShutdownOk);
    }
    self.done = true;
    code
  }
}

fn shell_kind(m: &ShellMsg) -> &'static str {
  match m {
    ShellMsg::Hello(_) => "hello",
    ShellMsg::AttachView { .. } => "attachView",
    ShellMsg::DetachView { .. } => "detachView",
    ShellMsg::WebviewMessage { .. } => "webviewMessage",
    ShellMsg::PlatformResponse { .. } => "platformResponse",
    ShellMsg::PlatformEvent { .. } => "platformEvent",
    ShellMsg::Shutdown => "shutdown",
  }
}
