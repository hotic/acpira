//! What the user is told about a connection: which kind of path iroh selected. iroh tries every candidate in one QUIC
//! connection and moves to the best one on its own; this only names the selected one.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use iroh::TransportAddr;

/// Tailscale: 100.64/10 or fd7a:115c:a1e0::/48
pub use acpira_shared::devices::PathKind;

pub fn classify(addr: &TransportAddr) -> PathKind {
  match addr {
    TransportAddr::Ip(sa) => classify_ip(sa.ip()),
    _ => PathKind::Relay,
  }
}

pub fn classify_ip(ip: IpAddr) -> PathKind {
  match ip {
    IpAddr::V4(v4) => classify_v4(v4),
    IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
      Some(v4) => classify_v4(v4),
      None => classify_v6(v6),
    },
  }
}

fn classify_v4(v4: Ipv4Addr) -> PathKind {
  let [a, b, ..] = v4.octets();
  if v4.is_private() || v4.is_link_local() || v4.is_loopback() {
    PathKind::Lan
  } else if a == 100 && (64..128).contains(&b) {
    PathKind::Tailscale
  } else {
    PathKind::Direct
  }
}

fn classify_v6(v6: Ipv6Addr) -> PathKind {
  let s = v6.segments();
  if s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0 {
    PathKind::Tailscale
  } else if v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local() {
    PathKind::Lan
  } else {
    PathKind::Direct
  }
}

/// Addresses never worth handing to a peer: a proxy's TUN interface (Clash, Surge and the like put theirs in the
/// 198.18.0.0/15 benchmarking range, where nothing real lives) and loopback
pub fn is_local_only(sa: &SocketAddr) -> bool {
  match sa.ip() {
    IpAddr::V4(v4) => {
      let [a, b, ..] = v4.octets();
      v4.is_loopback() || v4.is_unspecified() || (a == 198 && (b == 18 || b == 19))
    }
    IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ip(s: &str) -> PathKind {
    classify_ip(s.parse().unwrap())
  }

  #[test]
  fn names_the_path() {
    assert_eq!(ip("192.168.71.85"), PathKind::Lan);
    assert_eq!(ip("10.0.0.2"), PathKind::Lan);
    assert_eq!(ip("fe80::1"), PathKind::Lan);
    assert_eq!(ip("100.83.164.70"), PathKind::Tailscale);
    assert_eq!(ip("fd7a:115c:a1e0::1"), PathKind::Tailscale);
    assert_eq!(ip("100.128.0.1"), PathKind::Direct);
    assert_eq!(ip("116.234.48.222"), PathKind::Direct);
    assert_eq!(ip("240e:b8f:965b:f500::1"), PathKind::Direct);
    assert_eq!(ip("::ffff:192.168.1.2"), PathKind::Lan);
    assert!(is_local_only(&"198.18.0.1:5000".parse().unwrap()));
    assert!(!is_local_only(&"198.20.0.1:5000".parse().unwrap()));
  }
}
