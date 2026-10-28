//! Building an endpoint and dialing: the one place that decides relays and address lookup, shared by the gateway and the
//! phone. Every wait has a timeout: iroh's `online()` never returns while no relay is reachable.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher};
use n0_future::StreamExt;

use crate::path::{self, PathKind};

/// Pairing: the only ALPN a stranger may use, and only while a pairing window is open
pub const ALPN_PAIR: &[u8] = b"acpira/pair/1";
/// Everything after pairing; only paired devices are admitted
pub const ALPN_LINK: &[u8] = b"acpira/link/1";

pub use acpira_shared::devices::Relays;

pub struct Options {
  pub secret: SecretKey,
  pub relays: Relays,
  /// The ALPNs this endpoint answers; a dial-only endpoint passes none
  pub alpns: Vec<Vec<u8>>,
  /// Bind IPv4 to this address only (tests: 127.0.0.1:0)
  pub bind: Option<SocketAddr>,
}

pub async fn bind(o: Options) -> Result<Endpoint> {
  let b = match &o.relays {
    Relays::N0 => Endpoint::builder(presets::N0),
    Relays::Custom { urls, token } => {
      let urls: Vec<RelayUrl> = urls.iter().map(|u| u.parse::<RelayUrl>().with_context(|| format!("relay url {u}"))).collect::<Result<_>>()?;
      let mut map = RelayMap::from_iter(urls);
      if let Some(t) = token {
        map = map.with_auth_token(t.clone());
      }
      Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Custom(map))
    }
    Relays::Off => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
  };
  let mut b = b.secret_key(o.secret).alpns(o.alpns);
  if let Some(addr) = o.bind {
    b = b.clear_ip_transports().bind_addr(addr).map_err(|e| anyhow!("bind {addr}: {e}"))?;
  }
  b.bind().await.map_err(|e| anyhow!("endpoint: {e}"))
}

/// Whether the endpoint reached its home relay within `within` (with relays off: at once)
pub async fn online(ep: &Endpoint, within: Duration) -> bool {
  tokio::time::timeout(within, ep.online()).await.is_ok()
}

/// How this endpoint can be reached now: its home relay and the addresses worth giving a peer (no proxy TUN address,
/// no loopback). What goes into a ticket and the `addresses` update
pub fn reachable_at(ep: &Endpoint) -> (Option<RelayUrl>, Vec<SocketAddr>) {
  let a = ep.addr();
  let relay = a.relay_urls().next().cloned();
  (relay, worth_giving(a.ip_addrs().copied()))
}

/// At most a few addresses of each kind, the likeliest first: a Mac on IPv6 lists half a dozen rotating privacy
/// addresses, which would make the QR code dense without adding a path (iroh finds the rest once connected)
pub fn worth_giving(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
  let mut addrs: Vec<SocketAddr> = addrs.into_iter().filter(|sa| !path::is_local_only(sa)).collect();
  let class = |sa: &SocketAddr| match (path::classify_ip(sa.ip()), sa.is_ipv4()) {
    (PathKind::Lan, true) => 0,
    (PathKind::Tailscale, true) => 1,
    (PathKind::Direct, true) => 2,
    (PathKind::Direct, false) => 3,
    (PathKind::Lan, false) => 4,
    _ => 5,
  };
  const KEEP: [usize; 6] = [2, 1, 1, 2, 0, 1];
  addrs.sort_by_key(class);
  let mut kept = [0usize; 6];
  addrs.retain(|sa| {
    let c = class(sa);
    kept[c] += 1;
    kept[c] <= KEEP[c]
  });
  addrs
}

/// Dial with a deadline. The address may carry no IP at all: with n0 relays, lookup finds the peer's home relay
pub async fn dial(ep: &Endpoint, addr: EndpointAddr, alpn: &[u8], within: Duration) -> Result<Connection> {
  match tokio::time::timeout(within, ep.connect(addr, alpn)).await {
    Ok(Ok(c)) => Ok(c),
    Ok(Err(e)) => Err(anyhow!("{e}")),
    Err(_) => Err(anyhow!("no answer within {} s", within.as_secs())),
  }
}

/// The path a connection uses right now, and its round trip
pub fn selected_path(conn: &Connection) -> Option<(PathKind, Duration)> {
  conn.paths().iter().find(|p| p.is_selected()).map(|p| (path::classify(p.remote_addr()), p.rtt()))
}

/// Calls `f` with the kind of every newly selected path until the connection closes
pub fn on_path_change(conn: &Connection, f: impl Fn(PathKind) + Send + 'static) -> tokio::task::JoinHandle<()> {
  let mut events = conn.path_events();
  let conn = conn.clone();
  tokio::spawn(async move {
    if let Some((k, _)) = selected_path(&conn) {
      f(k);
    }
    let mut last = None;
    while let Some(e) = events.next().await {
      let kind = match e {
        iroh::endpoint::PathEvent::Selected { remote_addr, .. } => path::classify(&remote_addr),
        // Dropped events: read the selection again
        iroh::endpoint::PathEvent::Lagged { .. } => match selected_path(&conn) {
          Some((k, _)) => k,
          None => continue,
        },
        _ => continue,
      };
      if last != Some(kind) {
        last = Some(kind);
        f(kind);
      }
    }
  })
}

/// Whether the home relay is connected right now (`Relays::Off`: true, there is none to lose)
pub fn relay_connected(ep: &Endpoint) -> bool {
  let list = ep.home_relay_status().get();
  list.is_empty() || list.iter().any(|s| s.is_connected())
}

/// When to nudge iroh about a home relay that stays down: a relay connection can die quietly across sleep or a network
/// switch and not come back by itself, and telling iroh the network changed makes it probe and reconnect. The first
/// nudge comes after `first` of downtime, later ones back off to `MAX_NUDGE_WAIT`
pub struct RelayDog {
  first: Duration,
  wait: Duration,
  down_since: Option<std::time::Instant>,
}

const MAX_NUDGE_WAIT: Duration = Duration::from_secs(5 * 60);

impl RelayDog {
  pub fn new(first: Duration) -> Self {
    Self { first, wait: first, down_since: None }
  }

  /// One look at the relay; true when iroh should hear the network changed now
  pub fn tick(&mut self, connected: bool, now: std::time::Instant) -> bool {
    if connected {
      self.down_since = None;
      self.wait = self.first;
      return false;
    }
    let since = *self.down_since.get_or_insert(now);
    if now.duration_since(since) < self.wait {
      return false;
    }
    self.down_since = Some(now);
    self.wait = (self.wait * 2).min(MAX_NUDGE_WAIT);
    true
  }
}

/// Watch the home relay until the endpoint closes, nudging iroh when it stays down (`RelayDog`)
pub fn watch_relay(ep: Endpoint, first: Duration, log: impl Fn(&str) + Send + 'static) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    let mut dog = RelayDog::new(first);
    while !ep.is_closed() {
      if dog.tick(relay_connected(&ep), std::time::Instant::now()) {
        log("the home relay stayed down: telling iroh the network changed");
        ep.network_change().await;
      }
      tokio::time::sleep(Duration::from_secs(2)).await;
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_relay_dog_nudges_after_downtime_and_backs_off() {
    let t0 = std::time::Instant::now();
    let s = Duration::from_secs;
    let mut d = RelayDog::new(s(30));
    assert!(!d.tick(false, t0));
    assert!(!d.tick(false, t0 + s(29)));
    assert!(d.tick(false, t0 + s(30)));
    // Next nudge waits twice as long, counted from the last one
    assert!(!d.tick(false, t0 + s(89)));
    assert!(d.tick(false, t0 + s(90)));
    // Back up: the wait starts over
    assert!(!d.tick(true, t0 + s(100)));
    assert!(!d.tick(false, t0 + s(101)));
    assert!(d.tick(false, t0 + s(131)));
    // The wait never grows past the cap
    let mut d = RelayDog::new(s(200));
    let mut t = t0;
    d.tick(false, t);
    for _ in 0..4 {
      t += s(1000);
      assert!(d.tick(false, t));
    }
    assert_eq!(d.wait, MAX_NUDGE_WAIT);
  }

  #[test]
  fn a_ticket_keeps_a_few_of_each_kind() {
    let given: Vec<SocketAddr> = [
      "[240e::1]:5", "[240e::2]:5", "[240e::3]:5", "[fe80::1]:5", "198.18.0.1:5", "100.83.164.70:5", "64.204.67.35:5", "192.168.71.85:5",
      "192.168.71.147:5", "10.0.0.9:5", "127.0.0.1:5",
    ]
    .iter()
    .map(|s| s.parse().unwrap())
    .collect();
    let kept: Vec<String> = worth_giving(given).iter().map(ToString::to_string).collect();
    assert_eq!(kept, ["192.168.71.85:5", "192.168.71.147:5", "100.83.164.70:5", "64.204.67.35:5", "[240e::1]:5", "[240e::2]:5"]);
  }
}
