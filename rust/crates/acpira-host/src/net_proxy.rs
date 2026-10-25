//! The network route for everything the engine starts or fetches: agent processes (their model requests), installers,
//! login terminals and the engine's own downloads (native releases, quotas, the model catalogue).
//!
//! Setting `proxy`:
//! - `auto` (default): a local proxy on 127.0.0.1:7890 (Clash / mihomo's mixed port) while one listens there; otherwise
//!   nothing is added and the inherited environment (`HTTPS_PROXY` …) stays in charge
//! - `off`: nothing is added
//! - anything else: a proxy URL (`http://`, `https://`, `socks5://`, `socks5h://`) used as is, by the children and the
//!   engine's own HTTP client alike
//!
//! `ACPIRA_PROXY` overrides the setting: CLI subcommands (`acpira install-agent`) and the harness have no settings, and
//! the in-app installer hands the route it chose to them through it. The mode and its resolution are
//! `acpira_shared::proxy`, which the desktop shell's own downloads use too.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub use acpira_shared::proxy::{AUTO_PORT, OVERRIDE_ENV, ProxyMode, port_listening, resolve};
use parking_lot::Mutex;

/// How long one loopback probe answers `auto` before the port is looked at again (a proxy app started or quit)
const PROBE_TTL: Duration = Duration::from_secs(10);
/// Never sent through the proxy: the engine's own sockets and local model gateways
const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

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
  if std::env::var_os(OVERRIDE_ENV).is_some() {
    return ProxyMode::effective(None);
  }
  MODE.lock().clone().unwrap_or(ProxyMode::Auto)
}

/// The proxy URL everything started now should use; None leaves the inherited environment alone
pub fn current() -> Option<String> {
  resolve(&mode(), auto_listening)
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

/// The variables that route a child through `url`: both spellings of the proxy variables (curl only reads lower-case
/// `http_proxy`, most others the upper-case ones), loopback kept direct on top of any existing NO_PROXY, and Node's
/// opt-in for its built-in fetch / http to honour them (Node 22.21+ / 24.5+; older versions ignore it)
pub fn env_pairs(url: &str) -> Vec<(String, String)> {
  let no_proxy = no_proxy_hosts().join(",");
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

/// The hosts kept off the proxy: the inherited NO_PROXY, then loopback
fn no_proxy_hosts() -> Vec<String> {
  let inherited = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
  let mut hosts: Vec<String> = inherited.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
  for host in LOOPBACK {
    if !hosts.iter().any(|h| h == host) {
      hosts.push(host.to_owned());
    }
  }
  hosts
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

/// The current proxy on a ureq agent config, HTTP or SOCKS, with the same hosts kept direct as for a child (local
/// model servers must not go through a remote proxy); without one ureq keeps reading the environment itself
pub fn ureq_config(
  b: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
  match current().and_then(|u| ureq_proxy(&u)) {
    Some(p) => b.proxy(Some(p)),
    None => b,
  }
}

/// A proxy URL as ureq's proxy with the no-proxy hosts; `Proxy::new` takes none, so the parsed URL is rebuilt
fn ureq_proxy(url: &str) -> Option<ureq::Proxy> {
  let parsed = ureq::Proxy::new(url).ok()?;
  let mut b = ureq::Proxy::builder(parsed.protocol()).host(parsed.host()).port(parsed.port());
  if let Some(user) = parsed.username() {
    b = b.username(user);
  }
  if let Some(password) = parsed.password() {
    b = b.password(password);
  }
  // ureq compares a target's host as the URL spells it, so an IPv6 loopback also needs its bracketed form
  for host in no_proxy_hosts().iter().map(String::as_str).chain(["[::1]"]) {
    b = b.no_proxy(host);
  }
  b.build().ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_engine_proxy_keeps_loopback_direct_for_http_and_socks() {
    for url in ["http://u:p@10.0.0.2:3128", "socks5h://10.0.0.2:1080", "socks5://10.0.0.2:1080"] {
      let p = ureq_proxy(url).unwrap_or_else(|| panic!("{url} was dropped"));
      assert_eq!((p.host(), p.port()), ("10.0.0.2", if url.starts_with("http") { 3128 } else { 1080 }), "{url}");
      for local in ["http://127.0.0.1:1234/v1", "http://localhost:11434", "http://[::1]:8080"] {
        assert!(p.is_no_proxy(&local.parse().unwrap()), "{url} proxies {local}");
      }
      assert!(!p.is_no_proxy(&"https://api.example.com".parse().unwrap()), "{url}");
    }
    let http = ureq_proxy("http://u:p@10.0.0.2:3128").unwrap();
    assert_eq!((http.username(), http.password()), (Some("u"), Some("p")));
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
