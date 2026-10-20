//! One preview proxy: HTTP/1.1 in front of a loopback dev server, one request per connection.
//!
//! Every response goes back with `Connection: close`, so the browser opens a connection per request and no message framing
//! has to be tracked across requests. Requests reach upstream as if the page had been opened directly: Host, Origin and a
//! Referer on the proxy are rewritten to the target, and redirects back to the target point at the proxy again. HTML gets
//! the picker's `<script>` after `<head>` (the body is decoded from chunked or length framing and goes out close-delimited)
//! and loses the CSP / frame headers and meta CSP that would keep it out of the panel or block the script. Everything
//! else, SSE included, streams through untouched, and WebSocket upgrades (Vite's HMR) are piped as raw bytes.
//! Cookies are rewritten to `SameSite=None; Secure`: inside the webview the page is a cross-site frame and Chromium treats
//! a cookie without SameSite as Lax, so a login would not stick (VS Code 1.142.0, docs/dev/host-architecture.md)

use std::io;
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use regex::bytes::Regex;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::Target;

pub(super) struct Ctx {
  pub target: Target,
  pub token: String,
  pub picker: Arc<Mutex<Arc<Vec<u8>>>>,
}

/// Where the picker is served; the proxy answers it itself
const PICKER_PATH: &str = "/__acpira/picker.js";
/// A request or response head larger than this is refused
const MAX_HEAD: usize = 64 * 1024;
/// How much HTML is held back looking for the end of the head before the script goes in anyway
const SNIFF: usize = 64 * 1024;
/// The most body bytes read at once, whatever a chunk header announces
const PIECE: usize = 16 * 1024;

/// Where the head is over: the held-back part then holds every meta CSP the page declares up front
static HEAD_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</head\s*>|<body[\s>]").unwrap());
static HEAD_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<head(?:\s[^>]*)?>").unwrap());
static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<html(?:\s[^>]*)?>").unwrap());
static DOCTYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<!doctype[^>]*>").unwrap());
static META_CSP: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#"(?i)<meta\b[^>]*http-equiv\s*=\s*["']?content-security-policy(?:-report-only)?["']?[^>]*>"#).unwrap());

pub(super) async fn accept(listener: TcpListener, ctx: Arc<Ctx>, mut stop: watch::Receiver<bool>) {
  loop {
    tokio::select! {
      _ = stop.changed() => return,
      conn = listener.accept() => {
        let Ok((client, _)) = conn else { continue };
        let (ctx, mut stop) = (ctx.clone(), stop.clone());
        tokio::spawn(async move {
          tokio::select! {
            _ = stop.changed() => {}
            _ = serve(client, &ctx) => {}
          }
        });
      }
    }
  }
}

struct Head {
  /// Request line or status line
  start: String,
  headers: Vec<(String, String)>,
}

impl Head {
  fn get(&self, name: &str) -> Option<&str> {
    self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
  }

  fn remove(&mut self, name: &str) {
    self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
  }

  fn set(&mut self, name: &str, value: String) {
    self.remove(name);
    self.headers.push((name.to_owned(), value));
  }

  fn encode(&self) -> Vec<u8> {
    let mut out = format!("{}\r\n", self.start);
    for (k, v) in &self.headers {
      out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.into_bytes()
  }
}

/// Reads up to the blank line; returns the head and whatever body bytes came with it
async fn read_head(r: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<(Head, Vec<u8>)>> {
  let mut buf = Vec::with_capacity(4096);
  let mut chunk = [0u8; 4096];
  loop {
    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
      let text = String::from_utf8_lossy(&buf[..end]).into_owned();
      let mut lines = text.split("\r\n");
      let start = lines.next().unwrap_or_default().to_owned();
      let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
      return Ok(Some((Head { start, headers }, buf[end + 4..].to_vec())));
    }
    if buf.len() > MAX_HEAD {
      return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
    }
    let n = r.read(&mut chunk).await?;
    if n == 0 {
      return Ok(None);
    }
    buf.extend_from_slice(&chunk[..n]);
  }
}

/// Whether the browser reached the proxy under a name an attacker cannot point at this machine. The proxy rewrites Host and
/// Origin to the dev server's own, which would defeat a dev server's DNS-rebinding check (a page on `evil.test` whose name
/// now resolves to 127.0.0.1), so the name is checked here instead: an IP literal or `localhost` like Vite's default, or a
/// remote window's forwarding service whose DNS belongs to that service (Codespaces, dev tunnels; not exercised yet)
pub(super) fn trusted_host(host: &str) -> bool {
  let name = match host.strip_prefix('[') {
    Some(v6) => return v6.split_once(']').is_some_and(|(ip, rest)| ip.parse::<std::net::Ipv6Addr>().is_ok() && (rest.is_empty() || rest.starts_with(':'))),
    None => host.rsplit_once(':').map_or(host, |(h, _)| h).to_ascii_lowercase(),
  };
  name.parse::<std::net::Ipv4Addr>().is_ok()
    || name == "localhost"
    || [".localhost", ".github.dev", ".devtunnels.ms"].iter().any(|s| name.ends_with(s))
}

async fn serve(mut client: TcpStream, ctx: &Ctx) -> io::Result<()> {
  let Some((mut req, leftover)) = read_head(&mut client).await? else { return Ok(()) };
  if !req.get("host").is_some_and(trusted_host) {
    let body = "this address is not one the Acpira preview answers to";
    let head = format!(
      "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
      body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(body.as_bytes()).await?;
    return client.shutdown().await;
  }
  let mut parts = req.start.splitn(3, ' ');
  let (method, path) = (parts.next().unwrap_or_default().to_owned(), parts.next().unwrap_or_default().to_owned());
  // What the browser called us: a forwarded port in a remote window has its own address
  let proxy_origin = format!("http://{}", req.get("host").unwrap_or_default());
  if path.split('?').next() == Some(PICKER_PATH) {
    let body = ctx.picker.lock().clone();
    let head = format!(
      "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
      body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(&body).await?;
    return client.shutdown().await;
  }
  let upgrade = req.get("upgrade").is_some() && req.get("connection").is_some_and(|c| c.to_ascii_lowercase().contains("upgrade"));
  req.set("Host", ctx.target.authority.clone());
  if req.get("origin").is_some_and(|o| o.eq_ignore_ascii_case(&proxy_origin)) {
    req.set("Origin", ctx.target.origin.clone());
  }
  if let Some(rest) = req.get("referer").and_then(|r| r.strip_prefix(proxy_origin.as_str())).map(str::to_owned) {
    req.set("Referer", format!("{}{rest}", ctx.target.origin));
  }
  let mut upstream = match TcpStream::connect((ctx.target.host.as_str(), ctx.target.port)).await {
    Ok(s) => s,
    Err(e) => {
      let body = format!("{} is not answering: {e}", ctx.target.origin);
      let head = format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
      );
      client.write_all(head.as_bytes()).await?;
      client.write_all(body.as_bytes()).await?;
      return client.shutdown().await;
    }
  };
  if upgrade {
    // A WebSocket (HMR): the rewritten request, then both directions as they are
    upstream.write_all(&req.encode()).await?;
    upstream.write_all(&leftover).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    return Ok(());
  }
  // Compressed HTML could not take the script; loopback traffic does not need compression
  req.remove("accept-encoding");
  for h in ["connection", "keep-alive", "proxy-connection"] {
    req.remove(h);
  }
  req.set("Connection", "close".into());
  upstream.write_all(&req.encode()).await?;
  upstream.write_all(&leftover).await?;
  let (mut client_r, mut client_w) = client.into_split();
  let (up_r, mut up_w) = upstream.into_split();
  // The request body (if any) streams on while the response is read; this connection carries nothing after it
  let body = tokio::spawn(async move {
    let _ = tokio::io::copy(&mut client_r, &mut up_w).await;
  });
  let result = respond(&mut client_w, up_r, ctx, &method, &proxy_origin).await;
  body.abort();
  result
}

async fn respond(client: &mut (impl AsyncWrite + Unpin), upstream: impl AsyncRead + Unpin + Send, ctx: &Ctx, method: &str, proxy_origin: &str) -> io::Result<()> {
  let mut upstream = BufReader::new(upstream);
  let Some((mut res, leftover)) = read_head(&mut upstream).await? else { return Ok(()) };
  let status: u16 = res.start.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
  for h in ["content-security-policy", "content-security-policy-report-only", "x-frame-options", "keep-alive"] {
    res.remove(h);
  }
  if let Some(loc) = res.get("location").and_then(|l| strip_origin(l, &ctx.target.origin)).map(str::to_owned) {
    res.set("Location", format!("{proxy_origin}{loc}"));
  }
  let cookies: Vec<(String, String)> = res
    .headers
    .iter()
    .map(|(k, v)| if k.eq_ignore_ascii_case("set-cookie") { (k.clone(), cross_site_cookie(v)) } else { (k.clone(), v.clone()) })
    .collect();
  res.headers = cookies;
  res.set("Connection", "close".into());
  let html = res.get("content-type").is_some_and(|t| t.to_ascii_lowercase().contains("text/html"));
  let encoded = res.get("content-encoding").is_some_and(|e| !e.eq_ignore_ascii_case("identity"));
  let bodiless = method.eq_ignore_ascii_case("HEAD") || status < 200 || status == 204 || status == 304;
  if !html || encoded || bodiless {
    client.write_all(&res.encode()).await?;
    client.write_all(&leftover).await?;
    tokio::io::copy(&mut upstream, client).await?;
    return client.shutdown().await;
  }
  let chunked = res.get("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked"));
  let length: Option<u64> = res.get("content-length").and_then(|l| l.parse().ok());
  res.remove("content-length");
  res.remove("transfer-encoding");
  client.write_all(&res.encode()).await?;
  let mut body = BufReader::new(std::io::Cursor::new(leftover).chain(upstream));
  let mut held: Option<Vec<u8>> = Some(vec![]);
  let sink = |piece: Vec<u8>, held: &mut Option<Vec<u8>>| -> Option<Vec<u8>> {
    let Some(buf) = held.as_mut() else { return Some(piece) };
    buf.extend_from_slice(&piece);
    // Held until the head is complete, so a meta CSP that arrives in a later piece than `<head>` is dropped too
    if HEAD_END.is_match(buf) || buf.len() > SNIFF { held.take().map(|b| inject(&b, &ctx.token)) } else { None }
  };
  if chunked {
    let mut piece = vec![0u8; PIECE];
    loop {
      let mut line = String::new();
      // A size line is a few hex digits; one that runs on is not chunked framing
      if (&mut body).take(MAX_HEAD as u64).read_line(&mut line).await? == 0 {
        break;
      }
      let size = u64::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
      if size == 0 {
        break;
      }
      // Read in bounded pieces: the announced size is the server's word, not an allocation
      let mut left = size;
      while left > 0 {
        let want = left.min(PIECE as u64) as usize;
        body.read_exact(&mut piece[..want]).await?;
        left -= want as u64;
        if let Some(out) = sink(piece[..want].to_vec(), &mut held) {
          client.write_all(&out).await?;
        }
      }
      let mut crlf = [0u8; 2];
      body.read_exact(&mut crlf).await?;
    }
  } else {
    let mut limited: Box<dyn AsyncRead + Unpin + Send + '_> = match length {
      Some(n) => Box::new((&mut body).take(n)),
      None => Box::new(&mut body),
    };
    let mut chunk = vec![0u8; PIECE];
    loop {
      let n = limited.read(&mut chunk).await?;
      if n == 0 {
        break;
      }
      if let Some(out) = sink(chunk[..n].to_vec(), &mut held) {
        client.write_all(&out).await?;
      }
    }
  }
  if let Some(rest) = held.take() {
    client.write_all(&inject(&rest, &ctx.token)).await?;
  }
  client.shutdown().await
}

/// The script goes right after `<head>`; a page without one gets it after `<html>`, then after the doctype, then first.
/// Meta CSPs in the held-back part are dropped (they belong in the head, which this covers)
pub(super) fn inject(html: &[u8], token: &str) -> Vec<u8> {
  let html = META_CSP.replace_all(html, &b""[..]);
  let tag = format!(r#"<script src="{PICKER_PATH}?t={token}"></script>"#);
  let at = [&*HEAD_TAG, &*HTML_TAG, &*DOCTYPE].iter().find_map(|re| re.find(&html)).map(|m| m.end()).unwrap_or(0);
  let mut out = Vec::with_capacity(html.len() + tag.len());
  out.extend_from_slice(&html[..at]);
  out.extend_from_slice(tag.as_bytes());
  out.extend_from_slice(&html[at..]);
  out
}

/// `http://localhost:5173/x` → `/x` when it points at the target (the Location a dev server writes), else None
fn strip_origin<'a>(url: &'a str, origin: &str) -> Option<&'a str> {
  let rest = url.get(..origin.len()).filter(|p| p.eq_ignore_ascii_case(origin)).map(|_| &url[origin.len()..])?;
  (rest.is_empty() || rest.starts_with(['/', '?', '#'])).then_some(rest)
}

/// A cookie that keeps working inside the panel's cross-site frame. `Secure` is accepted on http://127.0.0.1, which is a
/// potentially trustworthy origin
pub(super) fn cross_site_cookie(line: &str) -> String {
  let mut parts: Vec<&str> = line.split(';').map(str::trim).collect();
  parts.retain(|p| {
    let name = p.split('=').next().unwrap_or("").trim().to_ascii_lowercase();
    name != "samesite" && name != "secure"
  });
  parts.push("SameSite=None");
  parts.push("Secure");
  parts.join("; ")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_script_lands_after_head_and_meta_csp_goes() {
    let page = br#"<!DOCTYPE html><html lang="en"><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'"><title>x</title></head><header>h</header></html>"#;
    let out = String::from_utf8(inject(page, "T")).unwrap();
    assert_eq!(out, r#"<!DOCTYPE html><html lang="en"><head><script src="/__acpira/picker.js?t=T"></script><title>x</title></head><header>h</header></html>"#);
    assert_eq!(String::from_utf8(inject(b"<!doctype html><p>no head", "T")).unwrap(), r#"<!doctype html><script src="/__acpira/picker.js?t=T"></script><p>no head"#);
    assert_eq!(String::from_utf8(inject(b"<p>bare", "T")).unwrap(), r#"<script src="/__acpira/picker.js?t=T"></script><p>bare"#);
  }

  #[test]
  fn only_names_that_cannot_be_rebound_reach_the_proxy() {
    for ok in ["127.0.0.1:5173", "localhost:41000", "LOCALHOST", "[::1]:8080", "app.localhost:3000", "x-41000.app.github.dev", "abc-41000.usw2.devtunnels.ms", "192.168.1.4:80"] {
      assert!(trusted_host(ok), "{ok}");
    }
    for bad in ["evil.test:41000", "localhost.evil.test", "github.dev.evil.test", "[::1", "[evil]:80", ""] {
      assert!(!trusted_host(bad), "{bad}");
    }
  }

  #[test]
  fn cookies_become_cross_site_and_redirects_come_back() {
    assert_eq!(cross_site_cookie("sid=1; Path=/; HttpOnly"), "sid=1; Path=/; HttpOnly; SameSite=None; Secure");
    assert_eq!(cross_site_cookie("a=b; SameSite=Strict; Secure; Max-Age=5"), "a=b; Max-Age=5; SameSite=None; Secure");
    assert_eq!(strip_origin("http://localhost:5173/login?x", "http://localhost:5173"), Some("/login?x"));
    assert_eq!(strip_origin("http://localhost:51730/", "http://localhost:5173"), None);
    assert_eq!(strip_origin("https://example.com/", "http://localhost:5173"), None);
  }
}
