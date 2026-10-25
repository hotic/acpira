//! The `proxy` setting resolved to a proxy address: what the engine (acpira-host `net_proxy`) and the desktop shell's
//! own downloads (acpira-desktop `update`) both route through. Pure apart from the loopback probe, which callers inject
//!
//! - `auto` (default): a local proxy on 127.0.0.1:7890 (Clash / mihomo's mixed port) while one listens there; otherwise
//!   nothing is added and the inherited environment (`HTTPS_PROXY` …) stays in charge
//! - `off`: nothing is added
//! - anything else: a proxy URL (`http://`, `https://`, `socks5://`, `socks5h://`) used as is
//!
//! `ACPIRA_PROXY` overrides the setting.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// The port `auto` looks at on the loopback interface
pub const AUTO_PORT: u16 = 7890;
/// The variable that overrides the setting
pub const OVERRIDE_ENV: &str = "ACPIRA_PROXY";
/// A loopback connect either succeeds or is refused at once; this only bounds a firewall that drops the SYN
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMode {
  Auto,
  Off,
  Url(String),
}

impl ProxyMode {
  /// A setting value; an empty, unknown or malformed value reads as `auto` (`crate::settings::proxy_setting`)
  pub fn parse(raw: &str) -> ProxyMode {
    match crate::settings::proxy_setting(&serde_json::Value::from(raw)).as_str() {
      "auto" => ProxyMode::Auto,
      "off" => ProxyMode::Off,
      url => ProxyMode::Url(url.to_owned()),
    }
  }

  /// `ACPIRA_PROXY` when set, else the setting's value (None: no setting yet, which is `auto`)
  pub fn effective(setting: Option<&str>) -> ProxyMode {
    if let Ok(v) = std::env::var(OVERRIDE_ENV) {
      return ProxyMode::parse(&v);
    }
    setting.map(ProxyMode::parse).unwrap_or(ProxyMode::Auto)
  }
}

/// The proxy URL for a mode, with the loopback probe injected; None leaves the inherited environment alone
pub fn resolve(mode: &ProxyMode, listening: impl Fn() -> bool) -> Option<String> {
  match mode {
    ProxyMode::Off => None,
    ProxyMode::Url(u) => Some(u.clone()),
    ProxyMode::Auto => listening().then(|| format!("http://127.0.0.1:{AUTO_PORT}")),
  }
}

/// One loopback connect to `port`
pub fn port_listening(port: u16) -> bool {
  TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), PROBE_TIMEOUT).is_ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn setting_values_parse_into_auto_off_or_a_url() {
    assert_eq!(ProxyMode::parse(""), ProxyMode::Auto);
    assert_eq!(ProxyMode::parse(" Auto "), ProxyMode::Auto);
    assert_eq!(ProxyMode::parse("off"), ProxyMode::Off);
    assert_eq!(ProxyMode::parse("DIRECT"), ProxyMode::Off);
    assert_eq!(ProxyMode::parse("http://127.0.0.1:7897/"), ProxyMode::Url("http://127.0.0.1:7897".into()));
    assert_eq!(ProxyMode::parse("10.0.0.2:3128"), ProxyMode::Url("http://10.0.0.2:3128".into()));
    assert_eq!(ProxyMode::parse("socks5h://u:p@proxy:1080"), ProxyMode::Url("socks5h://u:p@proxy:1080".into()));
    // Malformed values never break the network: they fall back to auto
    assert_eq!(ProxyMode::parse("ftp://x:21"), ProxyMode::Auto);
    assert_eq!(ProxyMode::parse("http://:8080"), ProxyMode::Auto);
    assert_eq!(ProxyMode::parse("http://host:1/path"), ProxyMode::Auto);
    assert_eq!(ProxyMode::parse("not a proxy"), ProxyMode::Auto);
  }

  #[test]
  fn auto_uses_the_local_port_only_while_it_listens() {
    let on = || true;
    let down = || false;
    assert_eq!(resolve(&ProxyMode::Auto, on).as_deref(), Some("http://127.0.0.1:7890"));
    assert_eq!(resolve(&ProxyMode::Auto, down), None);
    assert_eq!(resolve(&ProxyMode::Off, on), None);
    assert_eq!(resolve(&ProxyMode::Url("http://p:1".into()), down).as_deref(), Some("http://p:1"));
  }

  #[test]
  fn the_loopback_probe_sees_a_listener_and_a_closed_port() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    assert!(port_listening(port));
    drop(l);
    assert!(!port_listening(port));
  }
}
