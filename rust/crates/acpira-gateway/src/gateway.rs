//! The gateway: the only process on this computer facing the network. It admits paired devices by EndpointId before
//! reading anything from them, pairs new ones while a pairing window is open, bridges each device's envelope stream to
//! the engine, serves its blob requests, and reports who is online on which path.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use acpira_net::endpoint::{self, ALPN_LINK, ALPN_PAIR, Options, Relays};
use acpira_net::link::{self, BlobRequest, Incoming, close};
use acpira_net::pair::{self, PairReply, PairingWindow, Refusal};
use acpira_shared::devices::{DeviceStatus, GatewayStatus, PairingStatus, PathKind};
use acpira_net::ticket::PairTicket;
use acpira_net::{Endpoint, identity};
use anyhow::Result;
use acpira_net::iroh::endpoint::Connection;

use crate::bridge::{self, Who};
use crate::engine::{EngineSpec, Log};
use crate::store::{Config, Device, Devices, Paths, now_ms};

pub struct Opts {
  pub home: PathBuf,
  pub engine: EngineSpec,
  /// Overrides `gateway.json` (tests: relays off)
  pub relays: Option<Relays>,
  /// Tests: IPv4 on this address only, and its bound sockets go into tickets (loopback is never worth one otherwise)
  pub bind: Option<SocketAddr>,
  pub log: Log,
}

struct Live {
  conn: Connection,
  path: Option<PathKind>,
}

pub struct Gateway {
  pub paths: Paths,
  ep: Endpoint,
  /// `gateway.json` `name` or the computer's own, read again on `reload`
  name: parking_lot::Mutex<String>,
  relays: Relays,
  test_bind: bool,
  devices: Devices,
  engine: EngineSpec,
  /// `gateway.json` `keepAwake`, read again on `reload`
  keep_awake: std::sync::atomic::AtomicBool,
  window: parking_lot::Mutex<Option<OpenWindow>>,
  live: parking_lot::Mutex<HashMap<String, Live>>,
  changed: tokio::sync::watch::Sender<u64>,
  /// The devices' notification registrations and the APNs sender
  push: Arc<crate::push::Push>,
  log: Log,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The open pairing window and, while it lasts, its Bonjour advertisement (dropped with it)
struct OpenWindow {
  w: PairingWindow,
  _advert: Option<crate::bonjour::Advert>,
}

impl Gateway {
  pub async fn start(o: Opts) -> Result<Arc<Gateway>> {
    let paths = Paths::new(&o.home);
    let cfg = Config::load(&paths.config);
    let relays = o.relays.unwrap_or(cfg.relays.clone());
    let secret = identity::load_or_create(&paths.identity)?;
    let ep = endpoint::bind(Options { secret, relays: relays.clone(), alpns: vec![ALPN_PAIR.to_vec(), ALPN_LINK.to_vec()], bind: o.bind }).await?;
    let name = display_name(&cfg).into();
    let g = Arc::new(Gateway {
      devices: Devices::load(&paths.devices),
      paths,
      ep,
      name,
      relays,
      test_bind: o.bind.is_some(),
      engine: o.engine,
      keep_awake: cfg.keep_awake.into(),
      window: Default::default(),
      live: Default::default(),
      changed: tokio::sync::watch::channel(0).0,
      push: crate::push::Push::load(&o.home, o.log.clone()),
      log: o.log,
    });
    (g.log)(&format!("gateway {} listening as {}", VERSION, g.ep.id()));
    tokio::spawn(accept_loop(g.clone()));
    tokio::spawn(relay_watch(g.clone()));
    if g.relays != Relays::Off {
      let log = g.log.clone();
      endpoint::watch_relay(g.ep.clone(), Duration::from_secs(30), move |l| log(l));
    }
    Ok(g)
  }

  pub fn endpoint(&self) -> &Endpoint {
    &self.ep
  }

  pub fn keep_awake(&self) -> bool {
    self.keep_awake.load(std::sync::atomic::Ordering::Relaxed)
  }

  pub fn name(&self) -> String {
    self.name.lock().clone()
  }

  /// Read `gateway.json` again. Keep-awake and the name apply at once (an open pairing code shows the new name; paired
  /// devices see it when they next connect); relays are bound into the endpoint, so a change there takes a restart
  /// (the settings page restarts the LaunchAgent)
  pub fn reload(&self) {
    let cfg = Config::load(&self.paths.config);
    self.keep_awake.store(cfg.keep_awake, std::sync::atomic::Ordering::Relaxed);
    *self.name.lock() = display_name(&cfg);
    self.bump();
  }

  fn bump(&self) {
    self.changed.send_modify(|n| *n += 1);
  }

  /// A receiver that changes whenever the status may have
  pub fn watch(&self) -> tokio::sync::watch::Receiver<u64> {
    self.changed.subscribe()
  }

  pub fn status(&self) -> GatewayStatus {
    let (relay, _) = endpoint::reachable_at(&self.ep);
    let live = self.live.lock();
    let devices = self
      .devices
      .list()
      .into_iter()
      .map(|d| {
        let l = live.get(&d.id);
        DeviceStatus { online: l.is_some(), path: l.and_then(|l| l.path), device: d }
      })
      .collect();
    let pairing = self.window.lock().as_ref().filter(|o| o.w.is_open()).map(|o| pairing_status(self.ticket(o.w.secret()).link(), o.w.code(), o.w.expires_in()));
    GatewayStatus {
      id: self.ep.id().to_string(),
      name: self.name(),
      version: VERSION.to_owned(),
      relays: self.relays.clone(),
      relay: relay.map(|u| u.to_string()),
      relay_connected: endpoint::relay_connected(&self.ep),
      devices,
      pairing,
    }
  }

  fn ticket(&self, secret: [u8; 32]) -> PairTicket {
    let (relay, mut addrs) = endpoint::reachable_at(&self.ep);
    if self.test_bind {
      addrs = self.ep.bound_sockets();
    }
    PairTicket { id: self.ep.id(), name: self.name(), secret, relay, addrs }
  }

  /// Open a pairing window (a fresh secret and code: the previous ones stop working) and advertise it on the network
  pub fn open_pairing(self: &Arc<Self>) -> PairingStatus {
    let w = PairingWindow::open(pair::WINDOW);
    let p = pairing_status(self.ticket(w.secret()).link(), w.code(), w.expires_in());
    // Tests bind loopback and must not announce themselves on the real network
    let advert = if self.test_bind { None } else { crate::bonjour::advertise(&self.name(), &self.ep.id().to_string(), &self.lan_addrs()) };
    let secret = w.secret();
    *self.window.lock() = Some(OpenWindow { w, _advert: advert });
    self.bump();
    // The window closes on its own: drop it, and with it the advertisement, unless a newer one replaced it
    let me = Arc::downgrade(self);
    tokio::spawn(async move {
      tokio::time::sleep(pair::WINDOW + Duration::from_millis(100)).await;
      let Some(g) = me.upgrade() else { return };
      let mut slot = g.window.lock();
      if slot.as_ref().is_some_and(|o| o.w.secret() == secret) {
        slot.take();
      }
      drop(slot);
      g.bump();
    });
    p
  }

  /// LAN addresses for the advertisement: a phone on this network dials one of them
  fn lan_addrs(&self) -> Vec<SocketAddr> {
    let (_, addrs) = endpoint::reachable_at(&self.ep);
    addrs.into_iter().filter(|a| acpira_net::path::classify_ip(a.ip()) == PathKind::Lan).collect()
  }

  pub fn cancel_pairing(&self) {
    self.window.lock().take();
    self.bump();
  }

  /// Forget a device and drop its connection at once
  pub fn revoke(&self, id: &str) -> Result<bool> {
    let removed = self.devices.remove(id)?;
    self.push.forget(id);
    if let Some(l) = self.live.lock().remove(id) {
      l.conn.close(close::REVOKED.into(), b"revoked");
    }
    self.bump();
    Ok(removed)
  }

  /// A session wants the user: tell the phones that registered for notifications
  pub fn notify(&self, note: acpira_shared::devices::PushNote) {
    self.push.send(note, self.ep.id().to_string());
  }

  pub fn rename(&self, id: &str, name: &str) -> Result<bool> {
    let r = self.devices.rename(id, name)?;
    self.bump();
    Ok(r)
  }

  pub async fn stop(&self) {
    for (_, l) in self.live.lock().drain() {
      l.conn.close(close::SHUTDOWN.into(), b"shutdown");
    }
    self.ep.close().await;
  }

  /// Settle a pairing against the open window (`f` checks the secret or the code exchange) and admit the device
  fn settle(&self, conn_id: acpira_net::EndpointId, req: &pair::PairRequest, f: impl FnOnce(&mut PairingWindow) -> Result<(), Refusal>) -> PairReply {
    let mut slot = self.window.lock();
    let Some(o) = slot.as_mut() else { return PairReply::Refused { reason: Refusal::Closed } };
    if let Err(reason) = f(&mut o.w) {
      // Too many wrong tries close it
      if !o.w.is_open() {
        slot.take();
        drop(slot);
        self.bump();
      }
      return PairReply::Refused { reason };
    }
    slot.take();
    drop(slot);
    let device = Device { id: conn_id.to_string(), name: clip(&req.name), kind: req.kind, paired_at: now_ms(), version: req.version.clone(), last_seen: None };
    if let Err(e) = self.devices.put(device) {
      (self.log)(&format!("could not save the paired device: {e}"));
      return PairReply::Refused { reason: Refusal::Closed };
    }
    (self.log)(&format!("paired {} ({})", req.name, conn_id.fmt_short()));
    self.bump();
    let (relay, addrs) = endpoint::reachable_at(&self.ep);
    PairReply::Paired { name: self.name(), relay: relay.map(|u| u.to_string()), addrs: addrs.iter().map(ToString::to_string).collect() }
  }
}

/// The name devices see: `gateway.json`'s when set, else the computer's own
fn display_name(cfg: &Config) -> String {
  cfg.name.as_deref().map(clip).filter(|n| !n.is_empty()).unwrap_or_else(computer_name)
}

fn clip(s: &str) -> String {
  s.trim().chars().take(64).collect()
}

async fn accept_loop(g: Arc<Gateway>) {
  while let Some(incoming) = g.ep.accept().await {
    let g = g.clone();
    tokio::spawn(async move {
      let Ok(conn) = incoming.await else { return };
      if conn.alpn() == ALPN_PAIR {
        if let Err(e) = pair::answer(&conn, &*g).await {
          (g.log)(&format!("pairing failed: {e}"));
        }
        return;
      }
      let id = conn.remote_id().to_string();
      // Admission before anything is read from it
      let Some(device) = g.devices.get(&id) else {
        conn.close(close::UNKNOWN.into(), b"unknown device");
        return;
      };
      serve_device(g, conn, device).await;
    });
  }
}

async fn serve_device(g: Arc<Gateway>, conn: Connection, device: Device) {
  let id = device.id.clone();
  // A reconnect replaces the device's previous connection
  if let Some(old) = g.live.lock().insert(id.clone(), Live { conn: conn.clone(), path: None }) {
    old.conn.close(close::SHUTDOWN.into(), b"replaced");
  }
  g.devices.seen(&id);
  g.bump();
  (g.log)(&format!("{} connected", device.name));
  let paths = {
    let g = g.clone();
    let (id, conn2) = (id.clone(), conn.clone());
    endpoint::on_path_change(&conn, move |kind| {
      if let Some(l) = g.live.lock().get_mut(&id).filter(|l| l.conn.stable_id() == conn2.stable_id()) {
        l.path = Some(kind);
      }
      g.bump();
    })
  };
  let sessions_dir = Arc::new(parking_lot::Mutex::new(None::<PathBuf>));
  let who = Who { device: device.name.clone(), kind: device.kind, computer: g.name(), version: VERSION.to_owned() };
  let mut current: Option<tokio::task::JoinHandle<()>> = None;
  loop {
    let incoming = tokio::select! {
      r = link::accept(&conn) => r,
      _ = conn.closed() => break,
    };
    match incoming {
      Ok(Incoming::Envelopes(tx, rx)) => {
        // One envelope stream per connection: a new one replaces the old
        if let Some(t) = current.take() {
          t.abort();
        }
        let (g2, conn2, who, dir) = (g.clone(), conn.clone(), who.clone(), sessions_dir.clone());
        let (push, device_id) = (g.push.clone(), id.clone());
        let on_push: bridge::OnPush = Arc::new(move |target| push.set(&device_id, target));
        current = Some(tokio::spawn(async move {
          let ended = bridge::run(&g2.engine, &g2.log, &who, tx, rx, dir, on_push).await;
          (g2.log)(&format!("{}: {ended:?}", who.device));
          match ended {
            bridge::Ended::Behind => conn2.close(close::BEHIND.into(), b"behind"),
            bridge::Ended::Engine(_) => conn2.close(close::ENGINE.into(), b"engine"),
            _ => {}
          }
        }));
      }
      Ok(Incoming::Blob(req, send)) => {
        let dir = sessions_dir.lock().clone();
        tokio::spawn(async move {
          let data = read_blob(dir.as_deref(), &req).await;
          let _ = link::send_blob(send, data).await;
        });
      }
      Err(_) => break,
    }
  }
  if let Some(t) = current.take() {
    t.abort();
  }
  paths.abort();
  {
    let mut live = g.live.lock();
    if live.get(&id).is_some_and(|l| l.conn.stable_id() == conn.stable_id()) {
      live.remove(&id);
    }
  }
  g.devices.seen(&id);
  g.bump();
  (g.log)(&format!("{} disconnected", device.name));
}

/// A file of a session's blob directory: both names single plain path segments, so nothing outside it is reachable
async fn read_blob(sessions_dir: Option<&Path>, req: &BlobRequest) -> std::result::Result<Vec<u8>, String> {
  let plain = |s: &str| !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
  let Some(dir) = sessions_dir else { return Err("not connected to the engine yet".into()) };
  if !plain(&req.session_id) || !plain(&req.name) {
    return Err("no such blob".into());
  }
  let path = dir.join(&req.session_id).join(&req.name);
  match tokio::fs::metadata(&path).await {
    Ok(m) if m.is_file() && m.len() <= link::MAX_BLOB => tokio::fs::read(&path).await.map_err(|e| e.to_string()),
    Ok(m) if m.is_file() => Err("blob too large".into()),
    _ => Err("no such blob".into()),
  }
}

/// The home relay comes and goes (a network change, a relay restart): the status shows it
async fn relay_watch(g: Arc<Gateway>) {
  let mut last = None;
  loop {
    let now = (endpoint::relay_connected(&g.ep), endpoint::reachable_at(&g.ep).0);
    if last.as_ref() != Some(&now) {
      last = Some(now);
      g.bump();
    }
    if g.ep.is_closed() {
      return;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
  }
}

/// The computer's name as the user set it (macOS: Sharing settings), else the host name
pub fn computer_name() -> String {
  #[cfg(target_os = "macos")]
  if let Ok(o) = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output()
    && o.status.success()
  {
    let n = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    if !n.is_empty() {
      return n;
    }
  }
  let mut buf = [0u8; 256];
  // SAFETY: the buffer is valid for its length; gethostname NUL-terminates within it on success
  let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
  let n = if ok { String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == 0).unwrap_or(0)]).into_owned() } else { String::new() };
  if n.is_empty() { "Computer".into() } else { n.trim_end_matches(".local").to_owned() }
}

fn pairing_status(link: String, code: &str, expires_in: Duration) -> PairingStatus {
  PairingStatus { qr: qr_rows(&link), link, code: code.to_owned(), expires_in_ms: expires_in.as_millis() as u64 }
}

impl pair::Desk for Gateway {
  fn id(&self) -> acpira_net::EndpointId {
    self.ep.id()
  }

  fn redeem(&self, device: acpira_net::EndpointId, req: &pair::PairRequest) -> PairReply {
    self.settle(device, req, |w| w.redeem(&req.secret))
  }

  fn code(&self) -> Result<String, Refusal> {
    let slot = self.window.lock();
    let o = slot.as_ref().ok_or(Refusal::Closed)?;
    o.w.check().map(|()| o.w.code().to_owned())
  }

  fn try_code(&self, device: acpira_net::EndpointId, req: &pair::PairRequest, proven: bool) -> PairReply {
    self.settle(device, req, |w| w.try_code(proven))
  }
}

/// A QR code as rows of `1` (dark) and `0`; empty when the text does not fit one
pub fn qr_rows(text: &str) -> Vec<String> {
  let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else { return Vec::new() };
  let w = code.width();
  code.to_colors().chunks(w).map(|row| row.iter().map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' }).collect()).collect()
}
