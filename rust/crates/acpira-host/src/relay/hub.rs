//! The sidecar end of the relay: a loopback listener the `acpira mcp` processes of this sidecar's agents call back into.
//! Every MCP entry a session sends carries a grant token minted for it alone; the grant names the session (held weakly),
//! the summoned thread whose CLI got the entry and its depth, so a caller can neither reach another session nor climb
//! out of its depth. A connection has a few seconds to send its request line, and only so many may be waiting at once

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use acpira_shared::sidecar::{BrowserOpened, BrowserReuse};

use crate::acp::session::AcpSession;
use crate::relay::roster::Roster;
use crate::relay::wire::{HostFeatures, HubOp, HubReply, HubRequest};
use crate::relay::{ENV_ADDR, ENV_TOKEN};
use crate::session_library::{LibraryCaller, SessionLibrary};
use crate::store::transcript_store::LogFn;
use crate::util::random_uuid;

/// A request line larger than this is refused (covers large `RemoteTool` writes / edits and `ToolHook` payloads,
/// matching `mcp_http::MAX_BODY`)
const MAX_REQUEST: usize = 16 * 1024 * 1024;
/// How long a connection may take to send its request line
const REQUEST_WITHIN: Duration = Duration::from_secs(10);
/// Connections that have not sent their request line yet; more are closed at once
const MAX_PENDING: usize = 32;

/// How long the agent's machine gets to accept a connection to the page before `open_browser` reports nothing listening
const LISTEN_CHECK: Duration = Duration::from_secs(2);

/// The shell side of `open_browser` (`SidecarPlatform`; a fake in tests)
pub trait HostBrowser: Send + Sync {
  /// Whether the serving shell can show a page inside the editor
  fn available(&self) -> bool;
  /// Whether a shell serving `wires` (driving / viewing wires, checked first) can show a page
  fn available_for(&self, _wires: &[u64]) -> bool {
    self.available()
  }
  /// Show the page; where the user's side shows it (a forwarded port has its own address) and whether an earlier tab
  /// was reused, or why it did not open. `folder`: the session folder on an SSH host whose loopback the page is on.
  /// `session`: the id and folder of the session whose agent asked
  fn open<'a>(&'a self, url: &'a str, folder: Option<&'a str>, session: Option<(&'a str, &'a str)>) -> Pin<Box<dyn Future<Output = Result<BrowserOpened>> + Send + 'a>>;
  /// `open`, preferring one of `wires` when multiple windows or peers are connected
  fn open_for<'a>(
    &'a self,
    url: &'a str,
    folder: Option<&'a str>,
    session: Option<(&'a str, &'a str)>,
    _wires: &'a [u64],
  ) -> Pin<Box<dyn Future<Output = Result<BrowserOpened>> + Send + 'a>> {
    self.open(url, folder, session)
  }
}

/// What one MCP entry's token stands for
struct Grant {
  session: Weak<AcpSession>,
  /// The summoned thread whose CLI was given the entry (None = the session's own agent)
  caller: Option<String>,
  depth: u32,
}

pub struct RelayHub {
  addr: String,
  grants: parking_lot::Mutex<HashMap<String, Grant>>,
  pending: AtomicUsize,
  pub roster: Arc<Roster>,
  log: LogFn,
  /// Set once the runtime has a shell platform; without one `open_browser` is not offered
  browser: parking_lot::RwLock<Option<Arc<dyn HostBrowser>>>,
  /// The session manager, for `search_sessions` / `read_session`; weak, since the manager's sessions hold grants here
  library: parking_lot::RwLock<Option<Weak<dyn SessionLibrary>>>,
  /// The user's plugins: their script tools, the tool groups switched off, activation
  plugins: parking_lot::RwLock<Option<Weak<crate::plugins::host::PluginHost>>>,
  /// Computer use: the engine's driver and the per-app approvals
  computer: parking_lot::RwLock<Option<Arc<crate::computer::ComputerHost>>>,
}

/// Leaves the pending count when the request line is in (or the connection gave up)
struct PendingSlot<'a>(&'a AtomicUsize);

impl Drop for PendingSlot<'_> {
  fn drop(&mut self) {
    self.0.fetch_sub(1, Ordering::Relaxed);
  }
}

impl RelayHub {
  pub async fn start(roster: Arc<Roster>, log: LogFn) -> Result<Arc<RelayHub>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?.to_string();
    let hub = Arc::new(RelayHub { addr, grants: Default::default(), pending: AtomicUsize::new(0), roster, log, browser: Default::default(), library: Default::default(), plugins: Default::default(), computer: Default::default() });
    let weak = Arc::downgrade(&hub);
    tokio::spawn(async move {
      while let Ok((sock, _)) = listener.accept().await {
        let Some(hub) = weak.upgrade() else { break };
        tokio::spawn(hub.serve(sock));
      }
    });
    Ok(hub)
  }

  /// Where `open_browser` calls go (the runtime's platform)
  pub fn set_browser(&self, browser: Arc<dyn HostBrowser>) {
    *self.browser.write() = Some(browser);
  }

  /// Where `search_sessions` / `read_session` calls go (the runtime's session manager)
  pub fn set_library(&self, library: Weak<dyn SessionLibrary>) {
    *self.library.write() = Some(library);
  }

  /// Where the plugin tools come from (the runtime's plugin host)
  pub fn set_plugins(&self, plugins: Weak<crate::plugins::host::PluginHost>) {
    *self.plugins.write() = Some(plugins);
  }

  /// Where the computer-use tools go (the runtime's driver host)
  pub fn set_computer(&self, computer: Arc<crate::computer::ComputerHost>) {
    *self.computer.write() = Some(computer);
  }

  pub fn plugin_host(&self) -> Option<Arc<crate::plugins::host::PluginHost>> {
    self.plugins.read().as_ref().and_then(Weak::upgrade)
  }

  /// The listener's address, for a test client
  pub fn addr(&self) -> &str {
    &self.addr
  }

  /// The MCP entry's `env` (ACP name / value pairs) for one session's agent, or for the CLI of one of its summoned
  /// threads one level deeper: one grant per session, thread and depth, dropped once the session is gone. The same
  /// one every time, so every session request of a process carries the same entry: `agy` takes an entry whose env
  /// changed for another server and registers it again beside the first (`docs/dev/agent-quirks-antigravity.md`)
  pub fn env(&self, session: &Arc<AcpSession>, caller: Option<&str>, depth: u32) -> Value {
    let token = {
      let mut grants = self.grants.lock();
      grants.retain(|_, g| g.session.strong_count() > 0);
      let held = grants.iter().find(|(_, g)| std::ptr::eq(g.session.as_ptr(), Arc::as_ptr(session)) && g.caller.as_deref() == caller && g.depth == depth);
      match held {
        Some((token, _)) => token.clone(),
        None => {
          let token = random_uuid();
          grants.insert(token.clone(), Grant { session: Arc::downgrade(session), caller: caller.map(str::to_owned), depth });
          token
        }
      }
    };
    json!([{ "name": ENV_ADDR, "value": self.addr }, { "name": ENV_TOKEN, "value": token }])
  }

  async fn serve(self: Arc<Self>, sock: TcpStream) {
    if self.pending.fetch_add(1, Ordering::Relaxed) >= MAX_PENDING {
      self.pending.fetch_sub(1, Ordering::Relaxed);
      return;
    }
    let slot = PendingSlot(&self.pending);
    let (read, mut write) = sock.into_split();
    let reply = |r: HubReply| {
      let mut s = serde_json::to_string(&r).unwrap_or_default();
      s.push('\n');
      s
    };
    // Read up to `MAX_REQUEST + 1` bytes so an oversized line without a newline can be distinguished from EOF
    let mut reader = BufReader::new(read).take((MAX_REQUEST + 1) as u64);
    let line = match tokio::time::timeout(REQUEST_WITHIN, read_request_line(&mut reader, MAX_REQUEST)).await {
      Ok(Ok(Some(line))) => line,
      Ok(Ok(None)) => {
        drop(slot);
        let msg = format!("request exceeds {} MiB limit", MAX_REQUEST / (1024 * 1024));
        let _ = write.write_all(reply(HubReply::Error(msg)).as_bytes()).await;
        let _ = write.shutdown().await;
        return;
      }
      _ => return,
    };
    drop(slot);
    let req: HubRequest = match serde_json::from_str(line.trim()) {
      Ok(r) => r,
      Err(e) => {
        let _ = write.write_all(reply(HubReply::Error(format!("bad request: {e}"))).as_bytes()).await;
        return;
      }
    };
    let grant = self.grants.lock().get(&req.token).map(|g| (g.session.clone(), g.caller.clone(), g.depth));
    let Some((session, caller, depth)) = grant else {
      (self.log)("relay: request with an unknown token refused");
      return;
    };
    match req.op {
      HubOp::List => {
        // A grant at the depth limit has the entry for its caller and the session tools, but nobody to summon
        let (mut personas, groups) = if depth >= crate::relay::MAX_DEPTH { (vec![], vec![]) } else { (self.roster.enabled(), self.roster.enabled_groups()) };
        // The root's own sidekick goes first: the executor its built-in agent picked, not a persona of the roster
        if caller.is_none()
          && let Some(p) = session.upgrade().and_then(|s| s.sidekick_persona())
        {
          personas.insert(0, p);
        }
        let _ = write.write_all(reply(HubReply::Roster { personas, groups }).as_bytes()).await;
      }
      HubOp::Features => {
        let session = session.upgrade();
        let wires = self.preferred_wires(session.as_deref());
        let browser = self.browser.read().as_ref().is_some_and(|b| b.available_for(&wires));
        let remote = session.as_ref().is_some_and(|s| s.remote_folder().is_some());
        // The project tools go to the session's own agent; a summoned child's CLI keeps its own
        let remote_tools = session.as_ref().filter(|_| caller.is_none()).and_then(|s| s.remote_stand()).map(|st| st.alias);
        let plugins_root = self.plugin_host().map(|p| p.store().root().display().to_string());
        let computer = acpira_computer::SUPPORTED && self.computer.read().is_some();
        let mut features = HostFeatures { browser, summoned: caller.is_some(), remote, remote_tools, computer, plugins_root, instructions: None };
        features.instructions = session.as_ref().map(|s| crate::relay_wire::instructions(&s.prompts(), &features));
        let _ = write.write_all(reply(HubReply::Features(features)).as_bytes()).await;
      }
      HubOp::HostHooks { cwd } => {
        let r = match session.upgrade() {
          Some(s) => match s.describe_host_hooks(&cwd).await {
            Ok(text) => HubReply::Done(text),
            Err(e) => HubReply::Error(e),
          },
          None => HubReply::Error("The Acpira session that started this agent is gone.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::HostFiles { paths, widget, video } => {
        let r = match session.upgrade() {
          Some(s) => match s.fetch_shown(&paths, widget, video).await {
            Ok(()) => HubReply::Done(String::new()),
            Err(e) => HubReply::Error(e),
          },
          None => HubReply::Error("The Acpira session that started this agent is gone.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::RemoteTool { name, arguments, tool_use_id } => {
        let Some(s) = session.upgrade().filter(|_| caller.is_none()) else {
          let _ = write.write_all(reply(HubReply::Error("The Acpira session that started this agent is gone.".into())).as_bytes()).await;
          return;
        };
        // The caller says nothing more after its request: its socket closing is the call's cancel, which drops the run
        // (and with it the command's process group on the host)
        reader.set_limit(1);
        let mut rest = [0u8; 1];
        let r = tokio::select! {
          r = s.on_mcp_remote_tool(&name, arguments, tool_use_id) => r,
          _ = reader.read(&mut rest) => return,
        };
        let r = match r {
          Ok(result) => HubReply::Tool(result),
          Err(e) => HubReply::Error(e),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::HostStdio { name } => {
        let opened = match session.upgrade() {
          Some(s) => s.open_host_stdio(&name).await,
          None => Err("The Acpira session that started this agent is gone.".into()),
        };
        let call = match opened {
          Ok(call) => call,
          Err(e) => {
            let _ = write.write_all(reply(HubReply::Error(e)).as_bytes()).await;
            return;
          }
        };
        if write.write_all(reply(HubReply::Done(String::new())).as_bytes()).await.is_err() {
          return;
        }
        // The rest of the connection is the server's: what the shim sends is its stdin, as long as it likes
        reader.set_limit(u64::MAX);
        pump_host_stdio(call, reader, write).await;
        return;
      }
      HubOp::OpenBrowser { url } => {
        let session = session.upgrade();
        let wires = self.preferred_wires(session.as_deref());
        let browser = self.browser.read().clone();
        let r = match browser {
          Some(b) if b.available_for(&wires) => self.open_browser(b.as_ref(), &url, session, &wires).await,
          _ => HubReply::Error("This editor cannot show web pages inside it; give the user the URL instead.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::Plugins => {
        let r = match self.plugin_host() {
          Some(p) => {
            p.reload();
            let loaded = p.loaded();
            HubReply::Plugins { tools: loaded.tool_defs(), off: loaded.groups_off() }
          }
          None => HubReply::Plugins { tools: vec![], off: vec![] },
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::PluginCall { name, arguments } => {
        let r = match (self.plugin_host(), session.upgrade()) {
          (Some(p), Some(s)) => {
            let ctx = crate::plugins::run::CallContext { workspace: s.cwd.clone(), session: s.id.clone() };
            drop(s);
            match p.call(&name, &arguments, &ctx).await {
              Ok(text) => HubReply::Done(text),
              Err(e) => HubReply::Error(e),
            }
          }
          (None, _) => HubReply::Error("Plugins are not available in this host.".into()),
          (_, None) => HubReply::Error("The Acpira session that started this agent is gone.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::Computer { name, arguments } => {
        // Off unless the user switched the group on, whatever the caller's tool list said when it asked
        let on = self.plugin_host().is_some_and(|p| p.loaded().state.tool_group_on(acpira_shared::host_tools::COMPUTER_GROUP));
        let computer = self.computer.read().clone();
        let r = match (computer, session.upgrade()) {
          _ if !on => HubReply::Error("Computer use is switched off on Acpira's plugin page.".into()),
          (Some(c), Some(_)) => c.call(&name, arguments).await,
          (None, _) => HubReply::Error("Computer use is not available in this host.".into()),
          (_, None) => HubReply::Error("The Acpira session that started this agent is gone.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::ToolHook { phase, name, arguments, result } => {
        let r = match session.upgrade() {
          Some(s) => HubReply::Done(s.plugin_tool(&phase, &name, arguments, result).await.to_string()),
          None => HubReply::Done("{}".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::PluginGuide => {
        let r = match self.plugin_host() {
          Some(p) => HubReply::Done(crate::plugins::guide::text(&p)),
          None => HubReply::Error("Plugins are not available in this host.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::PluginActivate { id } => {
        let r = match (self.plugin_host(), session.upgrade()) {
          (Some(p), Some(s)) => s.plugin_activate(p as Arc<dyn crate::acp::session::ext::PluginCardPort>, id).await,
          _ => HubReply::Error("Plugins are not available in this host.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::SearchSessions(_) | HubOp::ReadSession(_) => {
        let r = self.library_op(&session, caller.is_some(), req.op).await;
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::Ask(_) | HubOp::AskCaller(_) => {
        let Some(session) = session.upgrade() else {
          let _ = write.write_all(reply(HubReply::Error("The Acpira session that started this agent is gone.".into())).as_bytes()).await;
          return;
        };
        let (tx, mut rx) = mpsc::unbounded_channel::<HubReply>();
        match (req.op, caller) {
          (HubOp::Ask(args), caller) => {
            tokio::spawn(session.relay_ask(self.roster.clone(), caller, depth, args, tx));
          }
          (HubOp::AskCaller(args), Some(thread)) => {
            tokio::spawn(session.relay_ask_caller(thread, args, tx));
          }
          _ => {
            let _ = write.write_all(reply(HubReply::Error("Only a summoned agent has a caller to ask.".into())).as_bytes()).await;
            return;
          }
        }
        // Lines until the final one; a closed socket (the tool call was abandoned) leaves the child running
        while let Some(r) = rx.recv().await {
          let last = !matches!(r, HubReply::Progress(_));
          if write.write_all(reply(r).as_bytes()).await.is_err() || last {
            break;
          }
        }
      }
    }
    let _ = write.shutdown().await;
  }
}

impl RelayHub {
  /// The caller is the grant's session (a summoned thread's CLI reads it as its parent, not as its own conversation)
  async fn library_op(&self, session: &Weak<AcpSession>, summoned: bool, op: HubOp) -> HubReply {
    let Some(s) = session.upgrade() else {
      return HubReply::Error("The Acpira session that started this agent is gone.".into());
    };
    let caller = LibraryCaller { session_id: s.id.clone(), cwd: s.cwd.clone(), summoned };
    drop(s);
    let Some(library) = self.library.read().as_ref().and_then(Weak::upgrade) else {
      return HubReply::Error("Reading Acpira sessions is not available in this host.".into());
    };
    let r = match op {
      HubOp::SearchSessions(args) => library.search_sessions(&caller, args).await,
      HubOp::ReadSession(args) => library.read_session(&caller, args).await,
      _ => return HubReply::Error("unexpected op".into()),
    };
    match r {
      Ok(text) => HubReply::Done(text),
      Err(e) => HubReply::Error(e),
    }
  }

  /// The wires associated with `session` (the wire that last drove it, then any currently viewing it), in priority order
  fn preferred_wires(&self, session: Option<&AcpSession>) -> Vec<u64> {
    let mut wires = Vec::new();
    if let Some(s) = session {
      if let Some(w) = s.last_wire() {
        wires.push(w);
      }
      if let Some(lib) = self.library.read().as_ref().and_then(Weak::upgrade) {
        for w in lib.session_wires(&s.id) {
          if !wires.contains(&w) {
            wires.push(w);
          }
        }
      }
    }
    wires
  }

  /// A loopback page must be served on this machine first (the one the agent and its servers run on): nothing
  /// listening there is reported as such, before the shell forwards a port and checks from the user's side
  /// The listen check runs where the agent's commands do: this machine, or the SSH host of the session's folder (whose
  /// loopback the page is on, so the shell is told the folder)
  async fn open_browser(&self, browser: &dyn HostBrowser, url: &str, session: Option<Arc<AcpSession>>, wires: &[u64]) -> HubReply {
    let remote = session.as_ref().filter(|s| s.remote_folder().is_some());
    if let Some((host, port)) = loopback_target(url) {
      let listening = match remote {
        Some(s) => s.host_listening(&host, port, LISTEN_CHECK).await,
        None => matches!(tokio::time::timeout(LISTEN_CHECK, TcpStream::connect((host.as_str(), port))).await, Ok(Ok(_))),
      };
      if !listening {
        return HubReply::Error(format!(
          "Nothing is listening on {host}:{port} on the machine you run on, so the page was not opened. Start the server first, then call open_browser again."
        ));
      }
    }
    let folder = remote.map(|s| s.cwd.clone());
    match browser.open_for(url, folder.as_deref(), session.as_ref().map(|s| (s.id.as_str(), s.cwd.as_str())), wires).await {
      Ok(opened) => {
        let what = match opened.reused {
          Some(BrowserReuse::Revealed) => format!("{url} was already open in a browser tab in the user's editor; brought that tab to the front without reloading it."),
          Some(BrowserReuse::Navigated) => format!("Opened {url} in the browser tab already showing that site in the user's editor."),
          None => format!("Opened {url} in a browser tab in the user's editor."),
        };
        let via = if opened.url == url { String::new() } else { format!(" Their side loads it as {}.", opened.url) };
        HubReply::Done(format!("{what}{via}"))
      }
      Err(e) => {
        (self.log)(&format!("open_browser {url}: {e:#}"));
        HubReply::Error(format!("The page was not opened: {e:#}"))
      }
    }
  }
}

/// What the hub sends an `acpira mcp-host` shim after its `Done` line: a tag byte, a big-endian u32 length and that
/// many bytes. `End` carries the exit code as text, or why the server stopped
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Frame {
  Out = 1,
  Err = 2,
  End = 3,
}

pub fn frame(tag: Frame, bytes: &[u8]) -> Vec<u8> {
  let mut out = Vec::with_capacity(bytes.len() + 5);
  out.push(tag as u8);
  out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
  out.extend_from_slice(bytes);
  out
}

/// A project MCP server on the host, joined to its shim: the shim's bytes to the server's stdin (their end closes it),
/// the server's output back as frames. A shim that goes away drops the call, which stops the server there
async fn pump_host_stdio(
  mut call: Box<dyn crate::remote_seam::RemoteStream>,
  mut reader: tokio::io::Take<BufReader<tokio::net::tcp::OwnedReadHalf>>,
  mut write: tokio::net::tcp::OwnedWriteHalf,
) {
  use crate::remote_seam::RemoteEvent as Event;
  let mut buf = vec![0u8; 16 * 1024];
  let mut reading = true;
  let end = loop {
    tokio::select! {
      n = reader.read(&mut buf), if reading => match n {
        Ok(0) | Err(_) => {
          call.close_stdin();
          reading = false;
        }
        Ok(n) => call.send_stdin(&buf[..n]),
      },
      ev = call.next_event() => match ev {
        Some(Event::Chunk { stream, bytes }) => {
          let tag = if stream == "err" { Frame::Err } else { Frame::Out };
          if write.write_all(&frame(tag, &bytes)).await.is_err() {
            return;
          }
        }
        Some(Event::Done(Ok(r))) => break r.get("code").and_then(Value::as_i64).unwrap_or(1).to_string(),
        Some(Event::Done(Err(e))) => break e,
        None => break "the connection to the host ended".to_owned(),
      },
    }
  };
  let _ = write.write_all(&frame(Frame::End, end.as_bytes())).await;
  let _ = write.shutdown().await;
}

pub use crate::relay_wire::loopback_target;

/// Reads one newline-terminated request line up to `max` bytes; returns `Ok(None)` when the line exceeds `max`
async fn read_request_line<R: tokio::io::AsyncRead + Unpin>(reader: &mut tokio::io::Take<BufReader<R>>, max: usize) -> std::io::Result<Option<String>> {
  let mut line = String::new();
  reader.read_line(&mut line).await?;
  if line.len() > max || (line.len() == max && !line.ends_with('\n')) {
    return Ok(None);
  }
  Ok(Some(line))
}

/// The MCP server's side of one request (blocking: `acpira mcp` is synchronous); `on_line` sees every reply line
pub fn call(addr: &str, req: &HubRequest, mut on_line: impl FnMut(HubReply) -> bool) -> std::io::Result<()> {
  use std::io::{BufRead, Write};
  let mut sock = std::net::TcpStream::connect(addr)?;
  let mut body = serde_json::to_string(req).map_err(std::io::Error::other)?;
  body.push('\n');
  sock.write_all(body.as_bytes())?;
  let reader = std::io::BufReader::new(sock);
  for line in reader.lines() {
    let line = line?;
    if line.trim().is_empty() {
      continue;
    }
    let Ok(r) = serde_json::from_str::<HubReply>(&line) else { continue };
    if !on_line(r) {
      break;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::io::AsyncWriteExt;

  #[tokio::test]
  async fn request_lines_over_256_kib_are_accepted_and_oversized_lines_are_rejected_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let logged = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let sink = logged.clone();
    let hub = RelayHub::start(Arc::new(Roster::new(dir.path())), Arc::new(move |msg: &str| sink.lock().push(msg.to_owned()))).await.unwrap();

    // A 400 KiB valid JSON request reaches token validation instead of being truncated at the old 256 KiB cap
    let req = HubRequest {
      token: "missing-grant".into(),
      op: HubOp::RemoteTool {
        name: "Write".into(),
        arguments: json!({ "file_path": "big.txt", "content": "x".repeat(400 * 1024) }),
        tool_use_id: None,
      },
    };
    let mut sock = TcpStream::connect(hub.addr()).await.unwrap();
    let mut body = serde_json::to_string(&req).unwrap();
    body.push('\n');
    sock.write_all(body.as_bytes()).await.unwrap();
    let mut resp = String::new();
    let _ = BufReader::new(sock).read_line(&mut resp).await.unwrap();
    assert!(resp.is_empty(), "valid JSON over 256 KiB should reach token check, not fail JSON parse: {resp}");
    assert!(logged.lock().iter().any(|l| l.contains("unknown token")), "{:?}", *logged.lock());

    // Bounded reader distinguishes an oversized line from a valid line cleanly
    let under = b"{\"ok\":true}\n";
    let mut r_ok = BufReader::new(&under[..]).take(17);
    assert_eq!(read_request_line(&mut r_ok, 16).await.unwrap().as_deref(), Some("{\"ok\":true}\n"));
    let over = b"0123456789abcdef0\n";
    let mut r_over = BufReader::new(&over[..]).take(17);
    assert_eq!(read_request_line(&mut r_over, 16).await.unwrap(), None);
  }
}
