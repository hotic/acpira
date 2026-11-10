//! A tunnel to the machine a project folder lives on (the computer, or one of its SSH hosts), for a webview on this
//! device to show a dev server there. Two ways in, both listening on this device's loopback only: a SOCKS5 proxy
//! (`port`) whose loopback targets each become one tunnel stream (acpira-net `tunnel`), and plain forwards of one port
//! (`forward`) for a webview that will not send loopback names through a proxy. Every other target is refused here as
//! well as at the gateway. The tunnel ends with `close`, when dropped, and when its link ends.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use acpira_net::iroh::endpoint::Connection;
use acpira_net::tunnel::{self as wire, TcpCode, TcpRequest};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;

use crate::socks::{self, reply};
use crate::{LinkFailure, runtime};

/// How long a tunnel stream may take to open: the stream, then the gateway's header (an SSH host's answer included)
const OPEN_WAIT: Duration = Duration::from_secs(20);
/// How long a SOCKS client has for its greeting and request
const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);

#[derive(uniffi::Object)]
pub struct Tunnel {
  shared: Arc<Shared>,
  socks_port: u16,
}

struct Shared {
  conn: Connection,
  folder: String,
  /// The link's budget of tunnel streams, shared by its tunnels (the gateway's `tunnel::MAX_STREAMS`)
  slots: Arc<Semaphore>,
  stop: watch::Sender<bool>,
  /// Port on the folder's machine → port on this device
  forwards: parking_lot::Mutex<HashMap<u16, u16>>,
  listeners: parking_lot::Mutex<Vec<JoinHandle<()>>>,
  last_error: parking_lot::Mutex<Option<String>>,
}

/// Why a tunnel did not open
pub(crate) enum OpenFailure {
  /// The gateway said why (an SSH host not connected, not a folder)
  Refused(String),
  /// The link failed under the check, or the device could not listen
  Link(String),
}

/// Checks with the gateway that the folder's machine can be reached, then listens for the proxy. Runs on the runtime
pub(crate) async fn open(conn: Connection, folder: String, slots: Arc<Semaphore>) -> Result<Tunnel, OpenFailure> {
  let check = TcpRequest { folder: folder.clone(), host: "localhost".into(), port: 0 };
  match wire::open(&conn, &check, OPEN_WAIT).await {
    Ok(Ok(_)) => {}
    Ok(Err(f)) => return Err(OpenFailure::Refused(f.message)),
    Err(e) => return Err(OpenFailure::Link(format!("{e:#}"))),
  }
  let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| OpenFailure::Link(format!("listen on 127.0.0.1: {e}")))?;
  let socks_port = listener.local_addr().map_err(|e| OpenFailure::Link(e.to_string()))?.port();
  let shared = Arc::new(Shared {
    conn,
    folder,
    slots,
    stop: watch::channel(false).0,
    forwards: Default::default(),
    listeners: Default::default(),
    last_error: Default::default(),
  });
  let accept = tokio::spawn(serve(listener, shared.clone(), None));
  // The link's end is the tunnel's
  let watcher = tokio::spawn({
    let s = shared.clone();
    async move {
      let mut stop = s.stop.subscribe();
      tokio::select! {
        _ = s.conn.closed() => s.close(),
        () = stopped(&mut stop) => {}
      }
    }
  });
  shared.listeners.lock().extend([accept, watcher]);
  Ok(Tunnel { shared, socks_port })
}

impl Shared {
  fn close(&self) {
    self.stop.send_replace(true);
    for t in self.listeners.lock().drain(..) {
      t.abort();
    }
    self.forwards.lock().clear();
  }

  fn note(&self, e: String) {
    *self.last_error.lock() = Some(e);
  }

  /// One connection from the webview: through the SOCKS handshake, or straight to `to` for a forward
  async fn connection(&self, mut sock: TcpStream, to: Option<u16>) {
    let _ = sock.set_nodelay(true);
    let proxied = to.is_none();
    let (host, port) = match to {
      Some(port) => ("localhost".to_owned(), port),
      None => {
        let (mut r, mut w) = sock.split();
        match tokio::time::timeout(HANDSHAKE_WAIT, socks::handshake(&mut r, &mut w)).await {
          Ok(Ok(target)) => target,
          _ => return,
        }
      }
    };
    // A forward's client hears only the connection close; a proxy's gets the reply code
    let refuse = async |sock: &mut TcpStream, code: u8| {
      if proxied {
        let _ = sock.write_all(&socks::answer(code)).await;
      }
    };
    if wire::loopback(&host).is_none() {
      self.note(format!("only localhost can be opened through the tunnel, not {host}"));
      return refuse(&mut sock, reply::NOT_ALLOWED).await;
    }
    let Ok(_slot) = self.slots.clone().try_acquire_owned() else {
      self.note(format!("too many connections at once (at most {})", wire::MAX_STREAMS));
      return refuse(&mut sock, reply::FAILURE).await;
    };
    let req = TcpRequest { folder: self.folder.clone(), host, port };
    match wire::open(&self.conn, &req, OPEN_WAIT).await {
      Ok(Ok(o)) => {
        if proxied && sock.write_all(&socks::answer(reply::OK)).await.is_err() {
          return;
        }
        wire::splice(sock, o.send, o.recv, o.first).await;
      }
      Ok(Err(f)) => {
        self.note(f.message);
        let code = match f.code {
          TcpCode::Denied => reply::NOT_ALLOWED,
          TcpCode::Offline => reply::HOST_UNREACHABLE,
          TcpCode::Refused => reply::REFUSED,
          TcpCode::Busy | TcpCode::Failed => reply::FAILURE,
        };
        refuse(&mut sock, code).await;
      }
      Err(e) => {
        self.note(format!("{e:#}"));
        refuse(&mut sock, reply::FAILURE).await;
      }
    }
  }
}

/// Resolves once the tunnel stops (the guard `wait_for` answers must not live across an await: it is not `Send`)
async fn stopped(stop: &mut watch::Receiver<bool>) {
  let _ = stop.wait_for(|v| *v).await;
}

/// Accepts the webview's connections until the tunnel stops; `to`: a forward's port, else the SOCKS proxy
async fn serve(listener: TcpListener, s: Arc<Shared>, to: Option<u16>) {
  let mut stop = s.stop.subscribe();
  loop {
    tokio::select! {
      () = stopped(&mut stop) => return,
      accepted = listener.accept() => {
        let sock = match accepted {
          Ok((sock, _)) => sock,
          // Out of descriptors, most likely: let some close
          Err(_) => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
          }
        };
        let s = s.clone();
        tokio::spawn(async move {
          let mut stop = s.stop.subscribe();
          tokio::select! {
            () = stopped(&mut stop) => {}
            () = s.connection(sock, to) => {}
          }
        });
      }
    }
  }
}

impl Drop for Tunnel {
  fn drop(&mut self) {
    self.shared.close();
  }
}

#[uniffi::export]
impl Tunnel {
  /// The SOCKS5 proxy on this device's 127.0.0.1 (no authentication): point the webview's loopback names at it
  pub fn port(&self) -> u16 {
    self.socks_port
  }

  /// A plain listener on this device's loopback piped to `port` on the folder's machine; answers its port, `port`
  /// itself when that is free here (so `http://localhost:5173` stays the same address), else another. Forwarding the
  /// same port again answers the same listener
  pub fn forward(&self, port: u16) -> Result<u16, LinkFailure> {
    let fail = |message: String| LinkFailure::Other { message };
    if port == 0 {
      return Err(fail("a port is needed".into()));
    }
    if *self.shared.stop.borrow() {
      return Err(fail("the tunnel is closed".into()));
    }
    let rt = runtime().map_err(fail)?;
    let mut forwards = self.shared.forwards.lock();
    if let Some(local) = forwards.get(&port) {
      return Ok(*local);
    }
    let v4 = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
      .or_else(|_| std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
      .map_err(|e| fail(format!("listen on 127.0.0.1: {e}")))?;
    let local = v4.local_addr().map_err(|e| fail(e.to_string()))?.port();
    // `localhost` may resolve to ::1 first: the same port there too, when it is free
    let v6 = std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, local)).ok();
    let mut listeners = self.shared.listeners.lock();
    for l in std::iter::once(v4).chain(v6) {
      l.set_nonblocking(true).map_err(|e| fail(e.to_string()))?;
      let s = self.shared.clone();
      listeners.push(rt.spawn(async move {
        if let Ok(l) = TcpListener::from_std(l) {
          serve(l, s, Some(port)).await;
        }
      }));
    }
    forwards.insert(port, local);
    Ok(local)
  }

  /// Why the last connection through this tunnel failed (nothing listening, a host gone), for the page to show
  pub fn last_error(&self) -> Option<String> {
    self.shared.last_error.lock().clone()
  }

  /// Stops the proxy and every forward, ending their connections
  pub fn close(&self) {
    self.shared.close();
  }
}
