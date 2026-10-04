//! The sidecar end of the relay: a loopback listener the `acpira mcp` processes of this sidecar's agents call back into.
//! Every MCP entry a session sends carries a grant token minted for it alone; the grant names the session (held weakly),
//! the summoned thread whose CLI got the entry and its depth, so a caller can neither reach another session nor climb
//! out of its depth. A connection has a few seconds to send its request line, and only so many may be waiting at once

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::acp::session::AcpSession;
use crate::relay::roster::Roster;
use crate::relay::wire::{HubOp, HubReply, HubRequest};
use crate::relay::{ENV_ADDR, ENV_TOKEN};
use crate::store::transcript_store::LogFn;
use crate::util::random_uuid;

/// A request line larger than this is refused (prompts are short; the child reads files itself)
const MAX_REQUEST: usize = 256 * 1024;
/// How long a connection may take to send its request line
const REQUEST_WITHIN: Duration = Duration::from_secs(10);
/// Connections that have not sent their request line yet; more are closed at once
const MAX_PENDING: usize = 32;

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
    let hub = Arc::new(RelayHub { addr, grants: Default::default(), pending: AtomicUsize::new(0), roster, log });
    let weak = Arc::downgrade(&hub);
    tokio::spawn(async move {
      while let Ok((sock, _)) = listener.accept().await {
        let Some(hub) = weak.upgrade() else { break };
        tokio::spawn(hub.serve(sock));
      }
    });
    Ok(hub)
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
