//! Getting past a proxy's TUN. Clash, sing-box, Surge and the like route all IPv4 traffic into a virtual interface
//! (macOS: split routes 0/1 + 128/1 to a utun) and send UDP out through a remote proxy. iroh then learns the proxy's
//! exit as its public address, hole punching goes through the proxy or fails, and a phone stays on the relay. A UDP
//! socket bound to the physical interface's own address leaves through that interface instead (macOS scopes the route
//! lookup to the interface owning the source address, Windows sends by the strong host model, Clash Verge's
//! strict-route included), so the gateway binds its IPv4 sockets there while a proxy TUN
//! diverts traffic (a VPN tunnel is left alone, see `plan`). The proxy's DNS answers relay names with fake 198.18/15
//! addresses, which only mean something inside the TUN, so iroh's resolver moves to DNS over TLS while bypassing.
//!
//! The proxy TUN may hold the system's default route itself (Windows Clash Verge: a metric-0 default route to the TUN
//! beside the adapter's), so the physical interface is the best default route not owned by the TUN. netwatch cannot say
//! which that is on Windows (its WMI default route has no interface index), hence the routing table read here.
//!
//! macOS and Windows only: Linux proxies divert by policy routing, which a source address does not get past.

use std::net::Ipv4Addr;

use iroh::dns::{DnsResolver, NameserverConfig};

/// Where the gateway binds while a proxy TUN diverts traffic
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bypass {
  /// The physical interface: the best default route not owned by the TUN
  pub interface: String,
  /// Its IPv4 address: the IPv4 default socket binds here
  pub v4: Ipv4Addr,
  /// Its network prefix, for iroh's routing between bound sockets
  pub prefix: u8,
  /// A Tailscale address, bound beside it so a device on the tailnet keeps that path (the TUN-bound default would
  /// leave through the physical interface, where 100.64/10 leads nowhere)
  pub tailscale: Option<Ipv4Addr>,
}

/// One interface as the plan reads it
#[derive(Debug, Clone)]
pub struct Iface {
  pub name: String,
  pub up: bool,
  /// IPv4 addresses with their prefix length
  pub v4: Vec<(Ipv4Addr, u8)>,
}

/// The bypass to use now, or none: no proxy TUN diverting traffic, no physical interface to bind, or neither macOS nor
/// Windows. `ACPIRA_NET_BYPASS=0` turns it off: a test gateway that an Android emulator dials through the host's
/// loopback needs the wildcard IPv4 socket, which a bypass replaces with one on the physical address
pub async fn detect() -> Option<Bypass> {
  if std::env::var_os("ACPIRA_NET_BYPASS").is_some_and(|v| v == "0") {
    return None;
  }
  #[cfg(target_os = "macos")]
  {
    let (defaults, ifaces) = macos().await;
    plan(&defaults, &ifaces, route_source())
  }
  #[cfg(windows)]
  {
    let (defaults, ifaces) = tokio::task::spawn_blocking(windows::read).await.ok()?;
    plan(&defaults, &ifaces, route_source())
  }
  #[cfg(not(any(target_os = "macos", windows)))]
  None
}

/// macOS: netwatch's default route (the unscoped one, which a TUN's split routes leave in place) and its interfaces
#[cfg(target_os = "macos")]
async fn macos() -> (Vec<String>, Vec<Iface>) {
  let state = netwatch::interfaces::State::new().await;
  let ifaces: Vec<Iface> = state
    .interfaces
    .values()
    .map(|i| Iface {
      name: i.name().to_owned(),
      up: i.is_up(),
      v4: i
        .addrs()
        .filter_map(|n| match n {
          netwatch::interfaces::IpNet::V4(net) => Some((net.addr(), net.prefix_len())),
          _ => None,
        })
        .collect(),
    })
    .collect();
  (state.default_route_interface.into_iter().collect(), ifaces)
}

/// The source address the system picks for public IPv4 traffic: connecting a UDP socket sends nothing, it only runs the
/// route lookup. A TUN shows here as its own address
#[cfg(any(target_os = "macos", windows))]
fn route_source() -> Option<Ipv4Addr> {
  let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
  // Any public address does: 1.1.1.1 is a well-known one, and no datagram goes out
  s.connect((Ipv4Addr::new(1, 1, 1, 1), 53)).ok()?;
  match s.local_addr().ok()?.ip() {
    std::net::IpAddr::V4(a) if !a.is_unspecified() => Some(a),
    _ => None,
  }
}

/// The decision, apart from the system calls: bypass when public traffic leaves from a proxy TUN's address and a
/// default route (`defaults`: interface names, best first) leads through a real interface with a usable IPv4 address,
/// one that is not the TUN. A VPN tunnel (WireGuard and OpenVPN also take public traffic) is left alone: its user wants
/// traffic inside it, privacy or company rules
pub fn plan(defaults: &[String], ifaces: &[Iface], route_src: Option<Ipv4Addr>) -> Option<Bypass> {
  let src = route_src.filter(|a| is_proxy_tun(*a))?;
  let (iface, v4, prefix) = defaults
    .iter()
    .filter_map(|d| ifaces.iter().find(|i| &i.name == d))
    .filter(|i| i.up && !is_virtual(&i.name) && !i.v4.iter().any(|(a, _)| *a == src))
    .find_map(|i| i.v4.iter().find(|(a, _)| usable(*a)).map(|&(a, p)| (i, a, p)))?;
  let tailscale = ifaces.iter().filter(|i| i.up).flat_map(|i| i.v4.iter()).map(|(a, _)| *a).find(|a| is_tailscale(*a));
  Some(Bypass { interface: iface.name.clone(), v4, prefix, tailscale })
}

/// DNS over TLS to two public resolvers, past the proxy's fake-IP answers. TCP to them may still go through the TUN;
/// only the answers matter
pub fn resolver() -> DnsResolver {
  DnsResolver::builder()
    .add_nameserver_config(NameserverConfig::tls(Ipv4Addr::new(223, 5, 5, 5).into()))
    .add_nameserver_config(NameserverConfig::tls(Ipv4Addr::new(1, 1, 1, 1).into()))
    .build()
}

/// Names of virtual interfaces a default route may point at: VPN and proxy tunnels, never a way out of their own TUN
fn is_virtual(name: &str) -> bool {
  let n = name.to_ascii_lowercase();
  ["utun", "tun", "tap", "wintun", "wg", "meta", "mihomo", "clash", "sing", "tailscale", "zerotier"].iter().any(|p| n.starts_with(p))
}

/// A proxy's TUN address: 198.18/15 (Clash and mihomo, Surge, Stash) or sing-box's default 172.19.0.0/30
fn is_proxy_tun(a: Ipv4Addr) -> bool {
  let [x, y, z, w] = a.octets();
  (x == 198 && (y == 18 || y == 19)) || (x == 172 && y == 19 && z == 0 && w < 4)
}

/// An address worth binding: not loopback, link-local or a proxy TUN's 198.18/15
fn usable(a: Ipv4Addr) -> bool {
  let [x, y, ..] = a.octets();
  !(a.is_loopback() || a.is_link_local() || a.is_unspecified() || (x == 198 && (y == 18 || y == 19)) || is_tailscale(a))
}

/// Tailscale's CGNAT range, 100.64/10
fn is_tailscale(a: Ipv4Addr) -> bool {
  let [x, y, ..] = a.octets();
  x == 100 && (64..128).contains(&y)
}

/// Windows: the IPv4 routing and address tables, through the IP Helper API
#[cfg(windows)]
mod windows {
  use std::net::Ipv4Addr;

  use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceLuidToAlias, FreeMibTable, GetIpForwardTable2, GetIpInterfaceEntry, GetUnicastIpAddressTable, MIB_IPFORWARD_TABLE2,
    MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_TABLE,
  };
  use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
  use windows_sys::Win32::Networking::WinSock::{AF_INET, IpDadStatePreferred, SOCKADDR_INET};

  use super::Iface;

  /// Default routes' interface names, best (lowest route plus interface metric) first, and every interface's IPv4
  /// addresses. Names are the adapters' aliases ("Ethernet", "Meta"), so `is_virtual` reads them as elsewhere
  pub fn read() -> (Vec<String>, Vec<Iface>) {
    let mut ifaces: Vec<(u32, Iface)> = Vec::new();
    let mut table: *mut MIB_UNICASTIPADDRESS_TABLE = std::ptr::null_mut();
    // SAFETY: the API allocates the table, read within its NumEntries and freed with FreeMibTable
    unsafe {
      if GetUnicastIpAddressTable(AF_INET, &mut table) == 0 && !table.is_null() {
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        for r in rows {
          let Some(a) = v4(&r.Address) else { continue };
          let at = match ifaces.iter().position(|(i, _)| *i == r.InterfaceIndex) {
            Some(at) => at,
            None => {
              ifaces.push((r.InterfaceIndex, Iface { name: alias(&r.InterfaceLuid), up: true, v4: Vec::new() }));
              ifaces.len() - 1
            }
          };
          // A tentative or duplicate address cannot be bound yet
          if r.DadState == IpDadStatePreferred {
            ifaces[at].1.v4.push((a, r.OnLinkPrefixLength));
          }
        }
        FreeMibTable(table.cast());
      }
    }
    let mut defaults: Vec<(u32, u32)> = Vec::new();
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    // SAFETY: as above
    unsafe {
      if GetIpForwardTable2(AF_INET, &mut table) == 0 && !table.is_null() {
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        for r in rows.iter().filter(|r| r.DestinationPrefix.PrefixLength == 0) {
          defaults.push((r.Metric.saturating_add(interface_metric(r.InterfaceIndex)), r.InterfaceIndex));
        }
        FreeMibTable(table.cast());
      }
    }
    defaults.sort_unstable();
    let names = defaults.iter().filter_map(|(_, idx)| ifaces.iter().find(|(i, _)| i == idx).map(|(_, f)| f.name.clone())).collect();
    (names, ifaces.into_iter().map(|(_, f)| f).collect())
  }

  fn v4(sa: &SOCKADDR_INET) -> Option<Ipv4Addr> {
    // SAFETY: si_family says which member of the union is set
    unsafe { (sa.si_family == AF_INET).then(|| Ipv4Addr::from(sa.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes())) }
  }

  /// The interface's IPv4 metric, which Windows adds to a route's own
  fn interface_metric(index: u32) -> u32 {
    // SAFETY: a zeroed row with family and index set is what GetIpInterfaceEntry reads
    unsafe {
      let mut row: MIB_IPINTERFACE_ROW = std::mem::zeroed();
      row.Family = AF_INET;
      row.InterfaceIndex = index;
      if GetIpInterfaceEntry(&mut row) == 0 { row.Metric } else { u32::MAX / 2 }
    }
  }

  fn alias(luid: &NET_LUID_LH) -> String {
    let mut buf = [0u16; 257];
    // SAFETY: the buffer's length is passed in characters, and the alias comes back NUL-terminated within it
    let ok = unsafe { ConvertInterfaceLuidToAlias(luid, buf.as_mut_ptr(), buf.len()) } == 0;
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    if ok { String::from_utf16_lossy(&buf[..end]) } else { String::new() }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn iface(name: &str, addrs: &[(&str, u8)]) -> Iface {
    Iface { name: name.into(), up: true, v4: addrs.iter().map(|(a, p)| (a.parse().unwrap(), *p)).collect() }
  }

  fn mac() -> Vec<Iface> {
    vec![
      iface("en0", &[("192.168.71.147", 24)]),
      iface("en1", &[("192.168.71.85", 24)]),
      iface("utun4", &[("100.83.164.70", 32)]),
      iface("utun1024", &[("198.18.0.1", 30)]),
      iface("lo0", &[("127.0.0.1", 8)]),
    ]
  }

  #[test]
  fn a_tun_taking_public_traffic_is_bypassed_on_the_default_interface() {
    let b = plan(&names(&["en0"]), &mac(), Some("198.18.0.1".parse().unwrap())).unwrap();
    assert_eq!(b, Bypass { interface: "en0".into(), v4: "192.168.71.147".parse().unwrap(), prefix: 24, tailscale: Some("100.83.164.70".parse().unwrap()) });
  }

  #[test]
  fn traffic_on_the_physical_interface_or_inside_a_vpn_is_left_alone() {
    assert_eq!(plan(&names(&["en0"]), &mac(), Some("192.168.71.147".parse().unwrap())), None);
    // A full-tunnel WireGuard / OpenVPN
    assert_eq!(plan(&names(&["en0"]), &mac(), Some("10.64.0.2".parse().unwrap())), None);
    // sing-box's default TUN address is a proxy's
    assert!(plan(&names(&["en0"]), &mac(), Some("172.19.0.1".parse().unwrap())).is_some());
  }

  #[test]
  fn a_tun_holding_the_best_default_route_is_passed_over() {
    // Windows with Clash Verge: a metric-0 default route to the TUN, the adapter's next; aliases as Windows names them
    let win = vec![
      iface("Meta", &[("198.18.0.1", 30)]),
      iface("以太网 2", &[("192.168.71.96", 24)]),
      iface("Tailscale", &[("100.113.77.63", 32)]),
      iface("Loopback Pseudo-Interface 1", &[("127.0.0.1", 8)]),
    ];
    let b = plan(&names(&["Meta", "以太网 2"]), &win, Some("198.18.0.1".parse().unwrap())).unwrap();
    assert_eq!((b.interface.as_str(), b.v4, b.prefix), ("以太网 2", "192.168.71.96".parse().unwrap(), 24));
    assert_eq!(b.tailscale, Some("100.113.77.63".parse().unwrap()));
  }

  #[test]
  fn no_real_default_route_or_address_gives_nothing_to_bind() {
    let src = Some("198.18.0.1".parse().unwrap());
    // Only the tunnel has a default route (a full-tunnel utun, the TUN alone)
    assert_eq!(plan(&names(&["utun1024"]), &mac(), src), None);
    assert_eq!(plan(&names(&["Meta"]), &[iface("Meta", &[("198.18.0.1", 30)])], src), None);
    assert_eq!(plan(&[], &mac(), src), None);
    assert_eq!(plan(&names(&["en0"]), &[iface("en0", &[("169.254.3.4", 16)])], src), None);
    assert_eq!(plan(&names(&["en0"]), &mac(), None), None);
  }

  fn names(n: &[&str]) -> Vec<String> {
    n.iter().map(|s| (*s).to_owned()).collect()
  }
}
