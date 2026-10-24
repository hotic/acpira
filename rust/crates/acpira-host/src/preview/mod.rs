//! Loopback preview proxies for the shell's browser panel (`previewOpen` / `previewClose` on the sidecar wire).
//!
//! The panel shows a local dev page in an iframe and needs a script inside it (the element picker). A page is not ours to
//! edit, so each target origin gets a proxy on 127.0.0.1 that forwards to it and adds one `<script>` to its HTML
//! (`proxy.rs`). Only loopback http targets are taken: this is a window onto the user's own dev servers, never an open
//! proxy. Proxies are leased per wire and panel: windows of one workspace share the persistent engine and may show the
//! same site, so a proxy closes when its last lease goes, and a wire that leaves (window reload, dropped connection) drops
//! its leases at once; the shell asks again when it reconnects. Ports are not kept across engines.
//! Behaviour inside a VS Code webview was checked first, see the page-inside-a-webview entry in docs/dev/host-architecture.md

mod proxy;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// What `previewOpen` answers: the proxied address of the page, and the token the picker stamps its messages with
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
  pub url: String,
  pub token: String,
}

/// A loopback http target, split for the proxy
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
  /// `http://localhost:5173`, what the page's own absolute links and redirects point at
  pub origin: String,
  /// `localhost:5173`, the Host header upstream expects
  pub authority: String,
  /// What to connect to (brackets stripped from an IPv6 literal)
  pub host: String,
  pub port: u16,
  /// Path and query of the page asked for, `/` at least
  pub path: String,
  /// `#…` of the page asked for, or empty: never sent upstream, but handed back so a hash route opens where it was
  pub fragment: String,
}

/// Only `http://localhost`, `127.0.0.1` or `[::1]` with an optional port; anything else is refused with the reason
pub fn parse_target(url: &str) -> Result<Target, String> {
  let rest = url
    .strip_prefix("http://")
    .or_else(|| url.get(..7).filter(|p| p.eq_ignore_ascii_case("http://")).map(|_| &url[7..]))
    .ok_or_else(|| format!("only http:// pages on this machine can be previewed: {url}"))?;
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let (authority, tail) = rest.split_at(end);
  let (tail, fragment) = tail.find('#').map_or((tail, ""), |i| tail.split_at(i));
  let path = if tail.is_empty() { "/".to_owned() } else if tail.starts_with('?') { format!("/{tail}") } else { tail.to_owned() };
  let (host, port) = match authority.rsplit_once(':') {
    Some((h, p)) if !h.ends_with(':') && (h.starts_with('[') == h.ends_with(']')) => {
      (h, p.parse::<u16>().map_err(|_| format!("bad port in {url}"))?)
    }
    _ => (authority, 80),
  };
  let bare = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
  if !matches!(bare.as_str(), "localhost" | "127.0.0.1" | "::1") || authority.contains('@') {
    return Err(format!("only pages on this machine (localhost, 127.0.0.1, ::1) can be previewed: {url}"));
  }
  let authority = authority.to_ascii_lowercase();
  Ok(Target { origin: format!("http://{authority}"), authority, host: bare, port, path, fragment: fragment.to_owned() })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Lease {
  wire: u64,
  panel: String,
}

struct Proxy {
  port: u16,
  token: String,
  picker: Arc<Mutex<Arc<Vec<u8>>>>,
  leases: HashSet<Lease>,
  stop: watch::Sender<bool>,
}

/// Every proxy this engine runs, by target origin (after its SSH host for a dev server there)
pub struct Previews {
  proxies: Mutex<HashMap<String, Proxy>>,
  /// The newest open asked for per lease. An open runs on its own task, so a close, a dropped wire or a later open of the
  /// same panel can finish first; an open that is no longer the newest gives up instead of taking a lease. Locked after
  /// `proxies` when both are held
  asked: Mutex<(u64, HashMap<Lease, u64>)>,
  log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Previews {
  pub fn new(log: Arc<dyn Fn(&str) + Send + Sync>) -> Arc<Self> {
    Arc::new(Previews { proxies: Default::default(), asked: Default::default(), log })
  }

  /// Lease the proxy for `url` to this wire's panel, starting it when no one holds it. A panel that moves to another site
  /// gives up its lease on the old one. `picker` is the script the proxy serves to the page (the shell's bundled file).
  /// The open is ordered when this is called, before the returned future runs: call it in wire order
  /// `remote`: the page is a dev server on this SSH host (the session folder's), reached through its helper; a proxy is
  /// kept per host and origin, so two hosts' `localhost:3000` never meet
  pub fn open(
    self: &Arc<Self>,
    wire: u64,
    panel: &str,
    url: &str,
    picker: impl Into<PathBuf>,
    remote: Option<(String, Arc<crate::remote::connections::Connections>)>,
  ) -> impl Future<Output = Result<Opened, String>> + Send + 'static {
    let lease = Lease { wire, panel: panel.to_owned() };
    let asked = {
      let mut asked = self.asked.lock();
      asked.0 += 1;
      let n = asked.0;
      asked.1.insert(lease.clone(), n);
      n
    };
    let (this, url, picker) = (self.clone(), url.to_owned(), picker.into());
    async move { this.open_asked(lease, asked, &url, &picker, remote).await }
  }

  fn still_asked(&self, lease: &Lease, asked: u64) -> bool {
    self.asked.lock().1.get(lease) == Some(&asked)
  }

  async fn open_asked(
    &self,
    lease: Lease,
    asked: u64,
    url: &str,
    picker: &std::path::Path,
    remote: Option<(String, Arc<crate::remote::connections::Connections>)>,
  ) -> Result<Opened, String> {
    const SUPERSEDED: &str = "the panel closed or moved on before the page opened";
    let target = parse_target(url)?;
    // The proxies' key: the origin, after the host it is on
    let key = match &remote {
      Some((alias, _)) => format!("ssh://{alias} {}", target.origin),
      None => target.origin.clone(),
    };
    let dial = match remote {
      Some((alias, connections)) => proxy::Dial::Remote { connections, alias },
      None => proxy::Dial::Local,
    };
    let script = tokio::fs::read(picker).await.map_err(|e| format!("picker script {}: {e}", picker.display()))?;
    // Released before the new one is taken, so moving within a site never closes it
    {
      let mut proxies = self.proxies.lock();
      if !self.still_asked(&lease, asked) {
        return Err(SUPERSEDED.into());
      }
      if let Some(p) = proxies.get_mut(&key) {
        *p.picker.lock() = Arc::new(script);
        p.leases.insert(lease.clone());
        let opened = Opened { url: format!("http://127.0.0.1:{}{}{}", p.port, target.path, target.fragment), token: p.token.clone() };
        let others: Vec<String> = proxies.keys().filter(|o| **o != key).cloned().collect();
        for origin in others {
          self.drop_lease(&mut proxies, &origin, &lease);
        }
        return Ok(opened);
      }
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("could not listen on 127.0.0.1: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().to_string();
    let picker = Arc::new(Mutex::new(Arc::new(script)));
    let (stop, stopped) = watch::channel(false);
    let ctx = Arc::new(proxy::Ctx { target: target.clone(), dial, token: token.clone(), picker: picker.clone() });
    tokio::spawn(proxy::accept(listener, ctx, stopped));
    let mut proxies = self.proxies.lock();
    // Closed or moved on while binding: the new listener goes and nothing is leased
    if !self.still_asked(&lease, asked) {
      let _ = stop.send(true);
      return Err(SUPERSEDED.into());
    }
    let others: Vec<String> = proxies.keys().cloned().collect();
    for origin in others {
      self.drop_lease(&mut proxies, &origin, &lease);
    }
    // Another open for the same site may have won the race while this one was binding: keep that one
    if let Some(p) = proxies.get_mut(&key) {
      let _ = stop.send(true);
      p.leases.insert(lease);
      return Ok(Opened { url: format!("http://127.0.0.1:{}{}{}", p.port, target.path, target.fragment), token: p.token.clone() });
    }
    (self.log)(&format!("preview proxy for {key} on 127.0.0.1:{port}"));
    proxies.insert(key, Proxy { port, token: token.clone(), picker, leases: HashSet::from([lease]), stop });
    Ok(Opened { url: format!("http://127.0.0.1:{port}{}{}", target.path, target.fragment), token })
  }

  /// The panel closed
  pub fn close(&self, wire: u64, panel: &str) {
    let lease = Lease { wire, panel: panel.to_owned() };
    let mut proxies = self.proxies.lock();
    self.asked.lock().1.remove(&lease);
    let origins: Vec<String> = proxies.keys().cloned().collect();
    for origin in origins {
      self.drop_lease(&mut proxies, &origin, &lease);
    }
  }

  /// The wire left: every panel it had goes with it
  pub fn release_wire(&self, wire: u64) {
    let mut proxies = self.proxies.lock();
    self.asked.lock().1.retain(|l, _| l.wire != wire);
    let gone: Vec<(String, Lease)> =
      proxies.iter().flat_map(|(o, p)| p.leases.iter().filter(|l| l.wire == wire).map(|l| (o.clone(), l.clone()))).collect();
    for (origin, lease) in gone {
      self.drop_lease(&mut proxies, &origin, &lease);
    }
  }

  /// The engine ends
  pub fn close_all(&self) {
    for (_, p) in self.proxies.lock().drain() {
      let _ = p.stop.send(true);
    }
  }

  /// Ports listening right now, for tests and logs
  pub fn ports(&self) -> Vec<u16> {
    self.proxies.lock().values().map(|p| p.port).collect()
  }

  fn drop_lease(&self, proxies: &mut HashMap<String, Proxy>, origin: &str, lease: &Lease) {
    let Some(p) = proxies.get_mut(origin) else { return };
    if !p.leases.remove(lease) || !p.leases.is_empty() {
      return;
    }
    let _ = p.stop.send(true);
    (self.log)(&format!("preview proxy for {origin} closed (127.0.0.1:{})", p.port));
    proxies.remove(origin);
  }
}

#[cfg(test)]
mod tests {
  use super::parse_target;

  #[test]
  fn only_loopback_http_targets_are_taken() {
    let t = parse_target("http://localhost:5173/a/b?x=1#frag").unwrap();
    assert_eq!((t.origin.as_str(), t.authority.as_str(), t.host.as_str(), t.port, t.path.as_str()), ("http://localhost:5173", "localhost:5173", "localhost", 5173, "/a/b?x=1"));
    assert_eq!(t.fragment, "#frag");
    let t = parse_target("http://localhost:5173#/settings").unwrap();
    assert_eq!((t.path.as_str(), t.fragment.as_str()), ("/", "#/settings"));
    let t = parse_target("http://127.0.0.1").unwrap();
    assert_eq!((t.port, t.path.as_str()), (80, "/"));
    let t = parse_target("http://[::1]:8765?q").unwrap();
    assert_eq!((t.host.as_str(), t.port, t.path.as_str()), ("::1", 8765, "/?q"));
    for bad in ["https://localhost:3000/", "http://example.com/", "http://10.0.0.2:80/", "http://user@localhost:1/", "http://localhost:x/", "file:///etc"] {
      assert!(parse_target(bad).is_err(), "{bad}");
    }
  }
}
