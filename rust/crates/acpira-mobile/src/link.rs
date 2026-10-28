//! A link to a paired computer (acpira-net `link`, ALPN `acpira/link/1`): one envelope stream read by a task of its own,
//! one short stream per blob, and the connection's path changes. The gateway admits the device by its key before reading
//! anything and greets it with `Welcome`; it tells why it closed a link in the QUIC close code (`link::close`), which
//! becomes the `LinkFailure` Swift shows.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use acpira_net::endpoint::{self, ALPN_LINK};
use acpira_net::frame::FrameKind;
use acpira_net::iroh::endpoint::{Connection, ConnectionError};
use acpira_net::link::{self, BlobRequest, Control, EnvelopeRx, EnvelopeTx, close};
use acpira_net::{Endpoint, EndpointAddr};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{LinkFailure, LinkPath, Welcome, ms, on_runtime};

/// Frames read ahead of Swift. Bounded, so a reader that falls behind backs up into QUIC flow control and the gateway's
/// own limit drops the link (`Behind`), instead of this side buffering without end
const READ_AHEAD: usize = 256;
/// How long a closed envelope stream waits for the connection's close to say why
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// What arrives on a link besides its end
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LinkEvent {
  /// One sidecar envelope (JSON, no newline)
  Envelope { line: String },
  /// Where the computer can be reached now, to keep for the next dial
  Addresses { relay: Option<String>, addrs: Vec<String> },
  /// The gateway did not pass a message on: only the computer itself may send `what`
  Denied { what: String },
}

#[derive(uniffi::Object)]
pub struct Link {
  conn: Connection,
  welcome: Welcome,
  tx: Arc<Mutex<EnvelopeTx>>,
  events: Mutex<mpsc::Receiver<LinkEvent>>,
  /// How the link ended, once it did: `None` is closed from this side
  end: Arc<OnceLock<Option<LinkFailure>>>,
  paths: Mutex<mpsc::UnboundedReceiver<LinkPath>>,
  path_now: Arc<parking_lot::Mutex<Option<LinkPath>>>,
  pings: Arc<parking_lot::Mutex<HashMap<u64, oneshot::Sender<()>>>>,
  ping_seq: AtomicU64,
  closed_here: Arc<AtomicBool>,
  tasks: Vec<JoinHandle<()>>,
}

/// Dials, opens the envelope stream and waits for the gateway's `Welcome`, all within `within`. Runs on the runtime
pub(crate) async fn open(ep: Endpoint, addr: EndpointAddr, protocol_version: i64, within: Duration) -> Result<Link, LinkFailure> {
  let dialed = tokio::time::timeout(within, endpoint::dial(&ep, addr, ALPN_LINK, within)).await;
  // Nothing at the addresses, no route to the relay, or no answer in time: the computer is not there for this device
  let Ok(Ok(conn)) = dialed else { return Err(LinkFailure::Offline) };
  let greeted = tokio::time::timeout(within, async {
    let (tx, mut rx) = link::open_envelopes(&conn).await.ok()?;
    loop {
      let (kind, body) = rx.next().await.ok()??;
      if kind == FrameKind::Control
        && let Ok(Control::Welcome { name, version, protocol }) = serde_json::from_slice(&body)
      {
        return Some((tx, rx, Welcome { name, version, protocol_version: protocol }));
      }
    }
  })
  .await;
  let (tx, rx, welcome) = match greeted {
    Ok(Some(g)) => g,
    // The gateway closes a device it does not admit right after the handshake: read why
    Ok(None) => {
      let why = ended(&conn).await;
      conn.close(0u32.into(), b"no welcome");
      return Err(why.unwrap_or(LinkFailure::Offline));
    }
    Err(_) => {
      conn.close(0u32.into(), b"no welcome");
      return Err(LinkFailure::Offline);
    }
  };
  if welcome.protocol_version != protocol_version {
    conn.close(0u32.into(), b"incompatible");
    let side = if welcome.protocol_version > protocol_version { "update this app" } else { "update Acpira on the computer" };
    return Err(LinkFailure::Incompatible { message: format!("the computer speaks protocol {}, this app {protocol_version}: {side}", welcome.protocol_version) });
  }
  Ok(Link::start(conn, tx, rx, welcome))
}

/// Why a link ended, from the connection's close: `None` when this side closed it
fn failure_of(e: &ConnectionError) -> Option<LinkFailure> {
  Some(match e {
    ConnectionError::LocallyClosed => return None,
    ConnectionError::ApplicationClosed(_) => match link::close_code(e) {
      // Unknown: the computer's list has no entry for this device, which is how a removal looks once it reconnects
      Some(close::UNKNOWN | close::REVOKED) => LinkFailure::Revoked,
      Some(close::BEHIND) => LinkFailure::Behind,
      Some(close::ENGINE) => LinkFailure::Engine { message: "the computer's engine stopped".into() },
      // Shutting down (remote access turned off), replaced, or no reason given
      _ => LinkFailure::Offline,
    },
    ConnectionError::TimedOut | ConnectionError::Reset => LinkFailure::Offline,
    ConnectionError::VersionMismatch => LinkFailure::Incompatible { message: "update Acpira on both sides".into() },
    other => LinkFailure::Other { message: other.to_string() },
  })
}

/// Called once the envelope stream ended: the close usually lands right after the stream's end
async fn ended(conn: &Connection) -> Option<LinkFailure> {
  match tokio::time::timeout(CLOSE_GRACE, conn.closed()).await {
    Ok(e) => failure_of(&e),
    Err(_) => Some(LinkFailure::Other { message: "the computer ended the stream".into() }),
  }
}

impl Link {
  /// Starts the reader and the path watcher. Runs on the runtime (both spawn)
  fn start(conn: Connection, tx: EnvelopeTx, rx: EnvelopeRx, welcome: Welcome) -> Link {
    let tx = Arc::new(Mutex::new(tx));
    let (events_tx, events) = mpsc::channel(READ_AHEAD);
    let (paths_tx, paths) = mpsc::unbounded_channel();
    let path_now = Arc::new(parking_lot::Mutex::new(None));
    let watcher = {
      let now = path_now.clone();
      endpoint::on_path_change(&conn, move |k| {
        let k = LinkPath::from(k);
        *now.lock() = Some(k);
        let _ = paths_tx.send(k);
      })
    };
    let pings: Arc<parking_lot::Mutex<HashMap<u64, oneshot::Sender<()>>>> = Default::default();
    let end = Arc::new(OnceLock::new());
    let closed_here = Arc::new(AtomicBool::new(false));
    let reader = Reader { conn: conn.clone(), tx: tx.clone(), events: events_tx, pings: pings.clone(), end: end.clone(), closed_here: closed_here.clone() };
    let reader = tokio::spawn(reader.run(watcher, rx));
    Link {
      conn,
      welcome,
      tx,
      events: Mutex::new(events),
      end,
      paths: Mutex::new(paths),
      path_now,
      pings,
      ping_seq: AtomicU64::new(1),
      closed_here,
      tasks: vec![reader],
    }
  }

  /// The failure a write or a probe reports: how the link ended if it did, else what the connection says
  fn failure_now(&self, fallback: &str) -> LinkFailure {
    match self.end.get() {
      Some(Some(f)) => f.clone(),
      Some(None) => LinkFailure::Other { message: "the link is closed".into() },
      None => self.conn.close_reason().and_then(|e| failure_of(&e)).unwrap_or_else(|| LinkFailure::Other { message: fallback.into() }),
    }
  }
}

impl Drop for Link {
  fn drop(&mut self) {
    self.closed_here.store(true, Ordering::Relaxed);
    self.conn.close(0u32.into(), b"bye");
    for t in &self.tasks {
      t.abort();
    }
  }
}

#[uniffi::export]
impl Link {
  pub fn welcome(&self) -> Welcome {
    self.welcome.clone()
  }

  /// Sends envelopes: one JSON line, or several separated by newlines (each goes as its own frame)
  pub async fn send(&self, line: String) -> Result<(), LinkFailure> {
    let tx = self.tx.clone();
    let sent = on_runtime(async move {
      let mut tx = tx.lock().await;
      for l in line.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
        tx.send(FrameKind::Envelope, l.as_bytes()).await?;
      }
      std::io::Result::Ok(())
    })
    .await;
    match sent {
      Ok(Ok(())) => Ok(()),
      _ => Err(self.failure_now("the write failed")),
    }
  }

  /// The next envelope or link message; `None` once the link was closed from this side, the failure once it ended
  /// otherwise (and on every call after)
  pub async fn recv(&self) -> Result<Option<LinkEvent>, LinkFailure> {
    let next = self.events.lock().await.recv().await;
    match next {
      Some(e) => Ok(Some(e)),
      None => match self.end.get() {
        Some(Some(f)) => Err(f.clone()),
        _ => Ok(None),
      },
    }
  }

  /// One blob of a session (an image its transcript shows), within the deadline
  pub async fn fetch_blob(&self, session_id: String, name: String, within_ms: u64) -> Result<Vec<u8>, LinkFailure> {
    let conn = self.conn.clone();
    let req = BlobRequest { session_id, name };
    let got = on_runtime(async move { link::fetch_blob(&conn, &req, ms(within_ms)).await }).await.map_err(|message| LinkFailure::Other { message })?;
    got.map_err(|e| match self.end.get() {
      Some(Some(f)) => f.clone(),
      _ => LinkFailure::Other { message: format!("{e:#}") },
    })
  }

  /// A round trip through the gateway, in milliseconds. A probe: no answer within the deadline is `Offline`
  pub async fn ping(&self, within_ms: u64) -> Result<u64, LinkFailure> {
    let n = self.ping_seq.fetch_add(1, Ordering::Relaxed);
    let (answered, answer) = oneshot::channel();
    self.pings.lock().insert(n, answered);
    let tx = self.tx.clone();
    let rtt = on_runtime(async move {
      let t = Instant::now();
      tx.lock().await.send(FrameKind::Control, &link::control(&Control::Ping { n })).await.ok()?;
      tokio::time::timeout(ms(within_ms), answer).await.ok()?.ok()?;
      Some(t.elapsed().as_millis() as u64)
    })
    .await;
    self.pings.lock().remove(&n);
    match rtt {
      Ok(Some(rtt)) => Ok(rtt),
      _ => Err(match self.end.get() {
        Some(Some(f)) => f.clone(),
        _ => LinkFailure::Offline,
      }),
    }
  }

  /// The path the link travels on now (none selected yet: `None`)
  pub fn path(&self) -> Option<LinkPath> {
    *self.path_now.lock()
  }

  /// The next change of path; `None` once the link ended
  pub async fn next_path(&self) -> Option<LinkPath> {
    self.paths.lock().await.recv().await
  }

  /// Ends the link; `recv` then returns `None`
  pub fn close(&self) {
    self.closed_here.store(true, Ordering::Relaxed);
    self.conn.close(0u32.into(), b"bye");
  }
}

struct Reader {
  conn: Connection,
  tx: Arc<Mutex<EnvelopeTx>>,
  events: mpsc::Sender<LinkEvent>,
  pings: Arc<parking_lot::Mutex<HashMap<u64, oneshot::Sender<()>>>>,
  end: Arc<OnceLock<Option<LinkFailure>>>,
  closed_here: Arc<AtomicBool>,
}

impl Reader {
  async fn run(self, watcher: JoinHandle<()>, mut rx: EnvelopeRx) {
    while let Ok(Some((kind, body))) = rx.next().await {
      let event = match kind {
        FrameKind::Envelope => match String::from_utf8(body) {
          Ok(line) => LinkEvent::Envelope { line },
          Err(_) => continue,
        },
        FrameKind::Control => match serde_json::from_slice::<Control>(&body) {
          Ok(Control::Pong { n }) => {
            if let Some(t) = self.pings.lock().remove(&n) {
              let _ = t.send(());
            }
            continue;
          }
          Ok(Control::Ping { n }) => {
            // Answered off the read loop, which a long write holding the stream must not stall
            let tx = self.tx.clone();
            tokio::spawn(async move {
              let _ = tx.lock().await.send(FrameKind::Control, &link::control(&Control::Pong { n })).await;
            });
            continue;
          }
          Ok(Control::Addresses { relay, addrs }) => LinkEvent::Addresses { relay, addrs },
          Ok(Control::Denied { what }) => LinkEvent::Denied { what },
          // A second welcome, or a message a newer gateway sends that this build does not know
          Ok(Control::Welcome { .. }) | Err(_) => continue,
        },
      };
      // The Link was dropped
      if self.events.send(event).await.is_err() {
        break;
      }
    }
    let outcome = if self.closed_here.load(Ordering::Relaxed) { None } else { ended(&self.conn).await };
    if outcome.is_some() {
      // The gateway ended the stream but may keep the connection: nothing more comes on it
      self.conn.close(0u32.into(), b"ended");
    }
    let _ = self.end.set(outcome);
    watcher.abort();
  }
}
