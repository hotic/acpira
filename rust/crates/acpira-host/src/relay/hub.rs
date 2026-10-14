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
use crate::store::transcript_store::LogFn;
use crate::util::random_uuid;

/// A request line larger than this is refused (prompts are short; the child reads files itself)
const MAX_REQUEST: usize = 256 * 1024;
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
  /// Show the page; where the user's side shows it (a forwarded port has its own address) and whether an earlier tab
  /// was reused, or why it did not open
  fn open<'a>(&'a self, url: &'a str) -> Pin<Box<dyn Future<Output = Result<BrowserOpened>> + Send + 'a>>;
}

impl HostBrowser for crate::sidecar::platform::SidecarPlatform {
  fn available(&self) -> bool {
    self.can_open_browser()
  }
  fn open<'a>(&'a self, url: &'a str) -> Pin<Box<dyn Future<Output = Result<BrowserOpened>> + Send + 'a>> {
    Box::pin(self.open_browser(url))
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
    let hub = Arc::new(RelayHub { addr, grants: Default::default(), pending: AtomicUsize::new(0), roster, log, browser: Default::default() });
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

  /// The listener's address, for a test client
  pub fn addr(&self) -> &str {
    &self.addr
  }

  /// The MCP entry's `env` (ACP name / value pairs) for one session's agent, or for the CLI of one of its summoned
  /// threads one level deeper: a fresh grant, dropped once the session is gone
  pub fn env(&self, session: &Arc<AcpSession>, caller: Option<&str>, depth: u32) -> Value {
    let token = random_uuid();
    {
      let mut grants = self.grants.lock();
      grants.retain(|_, g| g.session.strong_count() > 0);
      grants.insert(token.clone(), Grant { session: Arc::downgrade(session), caller: caller.map(str::to_owned), depth });
    }
    json!([{ "name": ENV_ADDR, "value": self.addr }, { "name": ENV_TOKEN, "value": token }])
  }

  async fn serve(self: Arc<Self>, sock: TcpStream) {
    if self.pending.fetch_add(1, Ordering::Relaxed) >= MAX_PENDING {
      self.pending.fetch_sub(1, Ordering::Relaxed);
      return;
    }
    let slot = PendingSlot(&self.pending);
    let (read, mut write) = sock.into_split();
    let mut line = String::new();
    let mut reader = BufReader::new(read).take(MAX_REQUEST as u64);
    if !matches!(tokio::time::timeout(REQUEST_WITHIN, reader.read_line(&mut line)).await, Ok(Ok(_))) {
      return;
    }
    drop(slot);
    let reply = |r: HubReply| {
      let mut s = serde_json::to_string(&r).unwrap_or_default();
      s.push('\n');
      s
    };
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
        let _ = write.write_all(reply(HubReply::Personas(self.roster.enabled())).as_bytes()).await;
      }
      HubOp::Features => {
        let browser = self.browser.read().as_ref().is_some_and(|b| b.available());
        let _ = write.write_all(reply(HubReply::Features(HostFeatures { browser })).as_bytes()).await;
      }
      HubOp::OpenBrowser { url } => {
        let browser = self.browser.read().clone();
        let r = match browser {
          Some(b) if b.available() => self.open_browser(b.as_ref(), &url).await,
          _ => HubReply::Error("This editor cannot show web pages inside it; give the user the URL instead.".into()),
        };
        let _ = write.write_all(reply(r).as_bytes()).await;
      }
      HubOp::Ask(args) => {
        let Some(session) = session.upgrade() else {
          let _ = write.write_all(reply(HubReply::Error("The Acpira session that started this agent is gone.".into())).as_bytes()).await;
          return;
        };
        let (tx, mut rx) = mpsc::unbounded_channel::<HubReply>();
        tokio::spawn(session.relay_ask(self.roster.clone(), caller, depth, args, tx));
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
  /// A loopback page must be served on this machine first (the one the agent and its servers run on): nothing
  /// listening there is reported as such, before the shell forwards a port and checks from the user's side
  async fn open_browser(&self, browser: &dyn HostBrowser, url: &str) -> HubReply {
    if let Some((host, port)) = loopback_target(url) {
      let connect = tokio::time::timeout(LISTEN_CHECK, TcpStream::connect((host.as_str(), port))).await;
      if !matches!(connect, Ok(Ok(_))) {
        return HubReply::Error(format!(
          "Nothing is listening on {host}:{port} on the machine you run on, so the page was not opened. Start the server first, then call open_browser again."
        ));
      }
    }
    match browser.open(url).await {
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

/// Host and port of an http(s) URL whose host is this machine's loopback (or the any-address a dev server prints);
/// None for anything else, which the agent's machine cannot vouch for
pub fn loopback_target(url: &str) -> Option<(String, u16)> {
  let (scheme, rest) = url.split_once("://")?;
  let default_port = match scheme.to_ascii_lowercase().as_str() {
    "http" => 80,
    "https" => 443,
    _ => return None,
  };
  let authority = rest.split(['/', '?', '#']).next()?;
  let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
  let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
    let (host, after) = v6.split_once(']')?;
    (host.to_owned(), after.strip_prefix(':'))
  } else {
    match authority.split_once(':') {
      Some((h, p)) => (h.to_owned(), Some(p)),
      None => (authority.to_owned(), None),
    }
  };
  let port = match port {
    Some(p) => p.parse().ok()?,
    None => default_port,
  };
  let host = host.to_ascii_lowercase();
  // The any-address is not connectable everywhere; a server bound to it answers on the loopback
  let connect = match host.as_str() {
    "localhost" | "127.0.0.1" | "::1" => host,
    "0.0.0.0" => "127.0.0.1".into(),
    "::" => "::1".into(),
    _ => return None,
  };
  Some((connect, port))
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
  use super::loopback_target;

  #[test]
  fn loopback_targets_are_found_with_their_ports_and_other_hosts_are_left_alone() {
    assert_eq!(loopback_target("http://localhost:3317/"), Some(("localhost".into(), 3317)));
    assert_eq!(loopback_target("http://127.0.0.1:5173/#/x"), Some(("127.0.0.1".into(), 5173)));
    assert_eq!(loopback_target("https://LOCALHOST"), Some(("localhost".into(), 443)));
    assert_eq!(loopback_target("http://0.0.0.0:8080?q=1"), Some(("127.0.0.1".into(), 8080)));
    assert_eq!(loopback_target("http://[::1]:9000/a"), Some(("::1".into(), 9000)));
    assert_eq!(loopback_target("http://user@localhost:81"), Some(("localhost".into(), 81)));
    assert_eq!(loopback_target("https://example.com:8443/"), None);
    assert_eq!(loopback_target("file:///tmp/x.html"), None);
    assert_eq!(loopback_target("http://localhost:notaport/"), None);
  }
}
