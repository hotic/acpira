//! acpira-mobile: the phone's side of paired remote access, exported to Swift through UniFFI (the XCFramework and the
//! bindings come from apps/ios/scripts/build-net.sh; `AcpiraRemote` in apps/ios wraps them). Only Acpira's own protocol
//! crosses the boundary: a node started from the identity key Swift keeps in the Keychain, pairing by link or by code,
//! and a link to a paired computer's gateway carrying envelope lines, blobs and path changes. Nothing iroh-shaped is
//! exported, so an iroh upgrade stays inside acpira-net and this crate.
//!
//! Every call runs on this library's own small tokio runtime and hands its result back through a join handle or a
//! channel, which any executor can await: Swift polls these futures from its own threads. Every wait has a deadline,
//! since iroh's own waits may have none (`online()` never returns while no relay is reachable).

use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use acpira_net::endpoint::{self, Options, Relays};
use acpira_net::iroh::TransportAddr;
use acpira_net::pair::{self, DeviceKind, PairReply, Refusal};
use acpira_net::path::PathKind;
use acpira_net::ticket::PairTicket;
use acpira_net::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey};

mod link;

pub use link::{Link, LinkEvent};

uniffi::setup_scaffolding!();

static RUNTIME: OnceLock<std::io::Result<tokio::runtime::Runtime>> = OnceLock::new();

/// Two workers: the link is a handful of streams, and the phone's cores belong to the UI
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
  RUNTIME
    .get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("acpira-net").enable_all().build())
    .as_ref()
    .map_err(|e| format!("runtime: {e}"))
}

/// Runs `f` on the library's runtime; the caller may await it from any executor. Dropping the caller's future leaves
/// `f` running to its own deadline
async fn on_runtime<T: Send + 'static>(f: impl Future<Output = T> + Send + 'static) -> Result<T, String> {
  runtime()?.spawn(f).await.map_err(|e| format!("task: {e}"))
}

/// How long the home relay may stay down before the watchdog nudges iroh; the gateway waits as long
const RELAY_GRACE: Duration = Duration::from_secs(30);

fn ms(n: u64) -> Duration {
  Duration::from_millis(n.max(1))
}

/// What this device is, as the computer's device list shows it
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceType {
  Phone,
  Tablet,
  Computer,
}

impl From<DeviceType> for DeviceKind {
  fn from(t: DeviceType) -> Self {
    match t {
      DeviceType::Phone => DeviceKind::Phone,
      DeviceType::Tablet => DeviceKind::Tablet,
      DeviceType::Computer => DeviceKind::Computer,
    }
  }
}

/// Where traffic goes when no direct path works (acpira-shared `devices::Relays`)
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RelayChoice {
  /// n0's public relays and their lookup
  N0,
  /// Own relays; `token` is the relay's shared token
  Custom { urls: Vec<String>, token: Option<String> },
  /// LAN, Tailscale and punched holes only
  Off,
}

impl From<RelayChoice> for Relays {
  fn from(r: RelayChoice) -> Self {
    match r {
      RelayChoice::N0 => Relays::N0,
      RelayChoice::Custom { urls, token } => Relays::Custom { urls, token },
      RelayChoice::Off => Relays::Off,
    }
  }
}

/// The kind of path a link travels on now (acpira-shared `devices::PathKind`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkPath {
  Lan,
  Tailscale,
  Direct,
  Relay,
}

impl From<PathKind> for LinkPath {
  fn from(k: PathKind) -> Self {
    match k {
      PathKind::Lan => LinkPath::Lan,
      PathKind::Tailscale => LinkPath::Tailscale,
      PathKind::Direct => LinkPath::Direct,
      PathKind::Relay => LinkPath::Relay,
    }
  }
}

/// Who to dial: a computer's EndpointId (its identity, pinned at pairing) and where it was last seen. The addresses are
/// hints; the handshake proves the identity
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ComputerAddr {
  pub id: String,
  pub relay: Option<String>,
  /// `ip:port`, unparsable ones skipped
  pub addrs: Vec<String>,
}

impl ComputerAddr {
  fn endpoint_addr(&self) -> Option<EndpointAddr> {
    let id: EndpointId = self.id.trim().parse().ok()?;
    let ips = self.addrs.iter().filter_map(|a| a.trim().parse::<SocketAddr>().ok()).map(TransportAddr::Ip);
    let relay = self.relay.as_deref().and_then(|r| r.parse::<RelayUrl>().ok()).map(TransportAddr::Relay);
    Some(EndpointAddr::from_parts(id, ips.chain(relay)))
  }
}

/// What a pairing link names, for the confirmation step before pairing
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkTicket {
  pub id: String,
  pub name: String,
  pub relay: Option<String>,
  pub addrs: Vec<String>,
}

/// A computer this device just paired with, and where to dial it next time
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Paired {
  pub id: String,
  pub name: String,
  pub relay: Option<String>,
  pub addrs: Vec<String>,
}

/// The gateway's greeting on a new link
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Welcome {
  /// The computer's name now (it may have been renamed since pairing)
  pub name: String,
  /// Its gateway's Acpira version
  pub version: String,
  /// Its engine's `SIDECAR_PROTOCOL_VERSION`
  pub protocol_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum NodeFailure {
  /// The identity key is not 32 bytes
  BadKey,
  Other { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum PairFailure {
  /// No pairing window is open on the computer
  Closed,
  Expired,
  /// Used already (scanned twice, or by someone else first)
  Used,
  Wrong,
  /// Not an `acpira://pair/…` link this version reads
  InvalidLink,
  /// The computer did not answer in time
  Unreachable,
  /// The computer answered but could not prove it knows the code
  Impostor,
  Other { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum LinkFailure {
  /// Nothing answered in time, the path died, or the gateway is stopping (remote access turned off)
  Offline,
  /// The computer does not admit this device: it was removed from the computer's list
  Revoked,
  /// The two sides speak different protocol versions; `message` names the side to update
  Incompatible { message: String },
  /// This side read too slowly and the gateway dropped it; a new link starts from whole views
  Behind,
  /// The computer's engine failed to start or ended
  Engine { message: String },
  Other { message: String },
}

macro_rules! display_as_debug {
  ($($t:ty),*) => {$(
    impl fmt::Display for $t {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
      }
    }
    impl std::error::Error for $t {}
  )*};
}
display_as_debug!(NodeFailure, PairFailure, LinkFailure);

fn other_pair(e: impl fmt::Display) -> PairFailure {
  PairFailure::Other { message: e.to_string() }
}

/// A fresh identity key, for Swift to keep in the Keychain
#[uniffi::export]
pub fn generate_key() -> Vec<u8> {
  SecretKey::generate().to_bytes().to_vec()
}

/// The sidecar protocol this library's gateway counterpart speaks, for a contract test against the Swift mirror
#[uniffi::export]
pub fn protocol_version() -> i64 {
  acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION
}

/// Reads a pairing link (or a bare ticket) without dialing anything
#[uniffi::export]
pub fn decode_link(link: String) -> Result<LinkTicket, PairFailure> {
  let t = PairTicket::decode(&link).map_err(|_| PairFailure::InvalidLink)?;
  Ok(LinkTicket { id: t.id.to_string(), name: t.name, relay: t.relay.map(|u| u.to_string()), addrs: t.addrs.iter().map(ToString::to_string).collect() })
}

/// Binds a node with the device's identity. It answers nothing: a device only dials
#[uniffi::export]
pub async fn start_node(secret: Vec<u8>, relays: RelayChoice) -> Result<Arc<Node>, NodeFailure> {
  let key = <[u8; 32]>::try_from(secret.as_slice()).map_err(|_| NodeFailure::BadKey)?;
  Node::start_bound(key, relays, None).await
}

/// The device's endpoint. Closing it (the app went to the background) ends every link; a new one starts from the same
/// key
#[derive(uniffi::Object)]
pub struct Node {
  ep: Endpoint,
}

impl Node {
  /// `bind`: IPv4 on this address only (tests: loopback)
  pub async fn start_bound(secret: [u8; 32], relays: RelayChoice, bind: Option<SocketAddr>) -> Result<Arc<Node>, NodeFailure> {
    let relays: Relays = relays.into();
    let watched = relays != Relays::Off;
    let o = Options { secret: SecretKey::from_bytes(&secret), relays, alpns: vec![], bind };
    let ep = on_runtime(async move {
      let ep = endpoint::bind(o).await?;
      // A home relay that stays down after a network change gets iroh to look again (ends with the endpoint)
      if watched {
        endpoint::watch_relay(ep.clone(), RELAY_GRACE, |l| eprintln!("acpira-net: {l}"));
      }
      anyhow::Ok(ep)
    })
    .await
    .map_err(|message| NodeFailure::Other { message })?
    .map_err(|e| NodeFailure::Other { message: format!("{e:#}") })?;
    Ok(Arc::new(Node { ep }))
  }
}

#[uniffi::export]
impl Node {
  /// This device's EndpointId, as the computer lists it
  pub fn id(&self) -> String {
    self.ep.id().to_string()
  }

  /// Whether the home relay is connected (true with relays off): false while online means no relay is reachable
  pub fn relay_connected(&self) -> bool {
    endpoint::relay_connected(&self.ep)
  }

  /// The system saw the network change (NWPathMonitor): re-probe interfaces and paths now instead of on iroh's timer
  pub fn network_changed(&self) {
    if let Ok(rt) = runtime() {
      let ep = self.ep.clone();
      rt.spawn(async move { ep.network_change().await });
    }
  }

  /// Pair through a scanned QR code or a pasted link: the link names the computer and carries its one-time secret
  pub async fn pair_link(&self, link: String, name: String, kind: DeviceType, version: String, within_ms: u64) -> Result<Paired, PairFailure> {
    let ticket = PairTicket::decode(&link).map_err(|_| PairFailure::InvalidLink)?;
    let ep = self.ep.clone();
    let t = ticket.clone();
    let reply = on_runtime(async move { pair::pair_with(&ep, &t, &name, kind.into(), &version, ms(within_ms)).await }).await.map_err(other_pair)?;
    // Every way the dial or the exchange fails is the computer not answering (gone, asleep, unreachable from here)
    let reply = reply.map_err(|_| PairFailure::Unreachable)?;
    let given = ticket.addrs.iter().map(ToString::to_string).collect();
    paired(ticket.id.to_string(), reply, ticket.relay.map(|u| u.to_string()), given)
  }

  /// Pair with a computer found on this network (Bonjour) and the 6-digit code its pairing card shows
  pub async fn pair_code(&self, computer: ComputerAddr, code: String, name: String, kind: DeviceType, version: String, within_ms: u64) -> Result<Paired, PairFailure> {
    let Some(addr) = computer.endpoint_addr() else { return Err(other_pair("not a computer id")) };
    let ep = self.ep.clone();
    let reply = on_runtime(async move { pair::pair_with_code(&ep, addr, &code, &name, kind.into(), &version, ms(within_ms)).await }).await.map_err(other_pair)?;
    let reply = reply.map_err(|e| {
      // The broadcast's answer could not prove the code: someone else answering in the computer's name
      if e.chain().any(|c| c.is::<pair::CodeNotProven>()) { PairFailure::Impostor } else { PairFailure::Unreachable }
    })?;
    paired(computer.id.trim().to_owned(), reply, computer.relay, computer.addrs)
  }

  /// Opens a link to a paired computer's engine. Within `within_ms` the computer must answer the dial and greet; the
  /// caller picks it as its probe (short on the LAN, longer through a relay)
  pub async fn connect(&self, computer: ComputerAddr, protocol_version: i64, within_ms: u64) -> Result<Arc<Link>, LinkFailure> {
    let Some(addr) = computer.endpoint_addr() else { return Err(LinkFailure::Other { message: "not a computer id".into() }) };
    let ep = self.ep.clone();
    let link = on_runtime(link::open(ep, addr, protocol_version, ms(within_ms))).await.map_err(|message| LinkFailure::Other { message })??;
    Ok(Arc::new(link))
  }

  /// Closes the endpoint and every link on it (they end as closed from this side)
  pub async fn close(&self) {
    let ep = self.ep.clone();
    let _ = on_runtime(async move { tokio::time::timeout(Duration::from_secs(3), ep.close()).await }).await;
  }
}

fn paired(id: String, reply: PairReply, ticket_relay: Option<String>, ticket_addrs: Vec<String>) -> Result<Paired, PairFailure> {
  match reply {
    PairReply::Paired { name, relay, addrs } => {
      // The computer's own report first; the ticket's addresses stay as hints (they are all a loopback-only test has)
      let mut all = addrs;
      for a in ticket_addrs {
        if !all.contains(&a) {
          all.push(a);
        }
      }
      all.truncate(12);
      Ok(Paired { id, name, relay: relay.or(ticket_relay), addrs: all })
    }
    PairReply::Refused { reason } => Err(match reason {
      Refusal::Closed => PairFailure::Closed,
      Refusal::Expired => PairFailure::Expired,
      Refusal::Used => PairFailure::Used,
      Refusal::Wrong => PairFailure::Wrong,
    }),
  }
}
