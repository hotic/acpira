//! The network route for everything the engine starts or fetches: agent processes (their model requests), installers,
//! login terminals and the engine's own downloads (native releases, quotas, the model catalogue).
//!
//! Setting `proxy`:
//! - `auto` (default): a local proxy on 127.0.0.1:7890 (Clash / mihomo's mixed port) while one listens there; otherwise
//!   nothing is added and the inherited environment (`HTTPS_PROXY` …) stays in charge
//! - `off`: nothing is added
//! - anything else: a proxy URL (`http://`, `https://`, `socks5://`, `socks5h://`) used as is
//!
//! `ACPIRA_PROXY` overrides the setting: CLI subcommands (`acpira install-agent`) and the harness have no settings, and
//! the in-app installer hands the route it chose to them through it.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// The port `auto` looks at on the loopback interface
pub const AUTO_PORT: u16 = 7890;
/// The variable that overrides the setting
pub const OVERRIDE_ENV: &str = "ACPIRA_PROXY";
/// How long one loopback probe answers `auto` before the port is looked at again (a proxy app started or quit)
const PROBE_TTL: Duration = Duration::from_secs(10);
/// A loopback connect either succeeds or is refused at once; this only bounds a firewall that drops the SYN
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
/// Never sent through the proxy: the engine's own sockets and local model gateways
const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMode {
  Auto,
  Off,
  Url(String),
}

impl ProxyMode {
  /// A setting value; an empty, unknown or malformed value reads as `auto` (`acpira_shared::settings::proxy_setting`)
  pub fn parse(raw: &str) -> ProxyMode {
    match acpira_shared::settings::proxy_setting(&serde_json::Value::from(raw)).as_str() {
      "auto" => ProxyMode::Auto,
      "off" => ProxyMode::Off,
      url => ProxyMode::Url(url.to_owned()),
    }
  }
}

static MODE: Mutex<Option<ProxyMode>> = Mutex::new(None);
static PROBE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

/// The `proxy` setting as the settings center read it (re-applied on every change)
pub fn set_setting(raw: &str) {
  *MODE.lock() = Some(ProxyMode::parse(raw));
  // A changed choice is answered from a fresh look at the port
  *PROBE.lock() = None;
}

/// The mode in effect: `ACPIRA_PROXY` first, then the setting, `auto` before any setting arrived
pub fn mode() -> ProxyMode {
  if let Ok(v) = std::env::var(OVERRIDE_ENV) {
    return ProxyMode::parse(&v);
  }
  MODE.lock().clone().unwrap_or(ProxyMode::Auto)
}

/// The proxy URL everything started now should use; None leaves the inherited environment alone
pub fn current() -> Option<String> {
  resolve(&mode(), auto_listening)
}

/// `current` with the loopback probe injected (tests)
pub fn resolve(mode: &ProxyMode, listening: impl Fn() -> bool) -> Option<String> {
  match mode {
    ProxyMode::Off => None,
    ProxyMode::Url(u) => Some(u.clone()),
    ProxyMode::Auto => listening().then(|| format!("http://127.0.0.1:{AUTO_PORT}")),
  }
}

/// Whether something accepts connections on 127.0.0.1:AUTO_PORT, cached for PROBE_TTL
fn auto_listening() -> bool {
  let mut probe = PROBE.lock();
  if let Some((at, up)) = *probe
    && at.elapsed() < PROBE_TTL
  {
    return up;
  }
  let up = port_listening(AUTO_PORT);
  *probe = Some((Instant::now(), up));
  up
}

/// One loopback connect to `port`
pub fn port_listening(port: u16) -> bool {
  TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), PROBE_TIMEOUT).is_ok()
}

/// The variables that route a child through `url`: both spellings of the proxy variables (curl only reads lower-case
/// `http_proxy`, most others the upper-case ones), loopback kept direct on top of any existing NO_PROXY, and Node's
/// opt-in for its built-in fetch / http to honour them (Node 22.21+ / 24.5+; older versions ignore it)
pub fn env_pairs(url: &str) -> Vec<(String, String)> {
  let inherited = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
  let mut no_proxy: Vec<String> = inherited.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
  for host in LOOPBACK {
    if !no_proxy.iter().any(|h| h == host) {
      no_proxy.push(host.to_owned());
    }
  }
  let no_proxy = no_proxy.join(",");
  let mut out = vec![];
  for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
    out.push((key.to_owned(), url.to_owned()));
    out.push((key.to_ascii_lowercase(), url.to_owned()));
  }
  out.push(("NO_PROXY".into(), no_proxy.clone()));
  out.push(("no_proxy".into(), no_proxy));
  out.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
  out
}

/// Route a child process through the current proxy (no-op when there is none). Called before the caller's own env, so
/// an agent definition or account that sets these variables still wins
pub fn apply(cmd: &mut tokio::process::Command) -> Option<String> {
  let url = current()?;
  for (k, v) in env_pairs(&url) {
    cmd.env(k, v);
  }
  Some(url)
}

/// The same variables for a terminal launched by the IDE shell
pub fn terminal_env() -> BTreeMap<String, Option<String>> {
  current().map(|url| env_pairs(&url).into_iter().map(|(k, v)| (k, Some(v))).collect()).unwrap_or_default()
}

/// The current proxy on a ureq agent config; without one ureq keeps reading the environment itself. A SOCKS URL is left
/// to the environment as well (this build of ureq speaks HTTP CONNECT only)
pub fn ureq_config(
  b: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
  match current().filter(|u| u.starts_with("http://") || u.starts_with("https://")).and_then(|u| ureq::Proxy::new(&u).ok()) {
    Some(p) => b.proxy(Some(p)),
    None => b,
  }
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

  #[test]
  fn env_pairs_cover_both_spellings_keep_loopback_direct_and_enable_node() {
    let pairs: BTreeMap<String, String> = env_pairs("http://127.0.0.1:7890").into_iter().collect();
    for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
      assert_eq!(pairs[k], "http://127.0.0.1:7890", "{k}");
    }
    for host in LOOPBACK {
      assert!(pairs["NO_PROXY"].split(',').any(|h| h == host));
    }
    assert_eq!(pairs["NO_PROXY"], pairs["no_proxy"]);
    assert_eq!(pairs["NODE_USE_ENV_PROXY"], "1");
  }
}
