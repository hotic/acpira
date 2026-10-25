//! The updater's downloads: https only and from the release hosts only, every redirect hop checked against them (the
//! host whitelist of acpira-host's `native_release.rs`); through the same proxy route as the engine; an interrupted
//! download resumes with a Range request; bytes are hashed as they are written and the file takes its final name only
//! once size and SHA-256 match the signed manifest

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::manifest::{Asset, hex};

/// Where a release's assets come from: the release download URL, then GitHub's asset storage it redirects to
/// (`release-assets.githubusercontent.com` since 2025, `objects.githubusercontent.com` before)
pub const RELEASE_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
const MAX_REDIRECTS: usize = 8;
const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

#[derive(Debug, Clone)]
pub struct Policy {
  pub hosts: Vec<String>,
  /// Plain http to the loopback interface: the smoke run's local release server, never a release build
  pub loopback_http: bool,
}

impl Policy {
  pub fn release() -> Policy {
    Policy { hosts: RELEASE_HOSTS.iter().map(|h| (*h).to_owned()).collect(), loopback_http: false }
  }

  pub fn check(&self, raw: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(raw).map_err(|e| format!("bad download address {raw}: {e}"))?;
    let host = u.host_str().unwrap_or_default();
    let ok = match u.scheme() {
      "https" => self.hosts.iter().any(|h| h == host),
      "http" => self.loopback_http && LOOPBACK.contains(&host),
      _ => false,
    };
    if !ok || !u.username().is_empty() {
      return Err(format!("refusing a download from {raw}: allowed https on {}", self.hosts.join(", ")));
    }
    Ok(u)
  }
}

/// An HTTP client for the updater: redirects followed by hand (each hop checked), statuses returned rather than
/// thrown, the proxy when one is in effect with loopback kept direct
pub fn agent(proxy: Option<&str>) -> ureq::Agent {
  let mut b = ureq::Agent::config_builder()
    .timeout_connect(Some(Duration::from_secs(30)))
    .timeout_recv_response(Some(Duration::from_secs(60)))
    .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
    .http_status_as_error(false)
    .max_redirects(0)
    .user_agent(format!("acpira-desktop/{}", env!("CARGO_PKG_VERSION")));
  if let Some(p) = proxy.and_then(proxy_of) {
    b = b.proxy(Some(p));
  }
  b.build().into()
}

fn proxy_of(url: &str) -> Option<ureq::Proxy> {
  let parsed = ureq::Proxy::new(url).ok()?;
  let mut b = ureq::Proxy::builder(parsed.protocol()).host(parsed.host()).port(parsed.port());
  if let Some(user) = parsed.username() {
    b = b.username(user);
  }
  if let Some(password) = parsed.password() {
    b = b.password(password);
  }
  for host in LOOPBACK.iter().chain(&["::1"]) {
    b = b.no_proxy(host);
  }
  b.build().ok()
}

type Response = ureq::http::Response<ureq::Body>;

/// GET `url` (from `range_from` on), following redirects within the policy
fn call(agent: &ureq::Agent, policy: &Policy, url: &str, range_from: Option<u64>) -> Result<Response, String> {
  let mut at = policy.check(url)?;
  for _ in 0..MAX_REDIRECTS {
    let mut req = agent.get(at.as_str());
    if let Some(n) = range_from {
      req = req.header("Range", format!("bytes={n}-"));
    }
    let res = req.call().map_err(|e| format!("download {}: {e}", at.host_str().unwrap_or_default()))?;
    if matches!(res.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
      let loc = res.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("a redirect without a location")?;
      let next = at.join(loc).map_err(|e| format!("bad redirect {loc}: {e}"))?;
      at = policy.check(next.as_str())?;
      continue;
    }
    return Ok(res);
  }
  Err(format!("too many redirects from {url}"))
}

/// A small body (the manifest, its signature) whole, at most `max` bytes
pub fn get_small(agent: &ureq::Agent, policy: &Policy, url: &str, max: usize) -> Result<Vec<u8>, String> {
  let mut res = call(agent, policy, url, None)?;
  let status = res.status().as_u16();
  if status != 200 {
    return Err(format!("{url}: HTTP {status}"));
  }
  let mut out = vec![];
  res.body_mut().as_reader().take(max as u64 + 1).read_to_end(&mut out).map_err(|e| format!("{url}: {e}"))?;
  if out.len() > max {
    return Err(format!("{url}: larger than {max} bytes"));
  }
  Ok(out)
}

fn part_of(to: &Path) -> PathBuf {
  let mut p = to.as_os_str().to_owned();
  p.push(".part");
  PathBuf::from(p)
}

/// Whether `to` already holds the asset (a finished earlier download)
pub fn complete(to: &Path, asset: &Asset) -> bool {
  fs::metadata(to).is_ok_and(|m| m.is_file() && m.len() == asset.size)
    && super::delta::sha256_file(to).is_ok_and(|s| s.eq_ignore_ascii_case(&asset.sha256))
}

/// Downloads `asset` to `to` (resuming `to.part` when an earlier try left one). `progress(done, total)` follows the
/// bytes; `cancel` stops it between chunks, keeping the part for the next try
pub fn download(
  agent: &ureq::Agent,
  policy: &Policy,
  asset: &Asset,
  to: &Path,
  progress: &mut dyn FnMut(u64, u64),
  cancel: &AtomicBool,
) -> Result<(), String> {
  if complete(to, asset) {
    progress(asset.size, asset.size);
    return Ok(());
  }
  let part = part_of(to);
  let wrap = |e: std::io::Error| format!("{}: {e}", part.display());
  let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
  if have >= asset.size {
    let _ = fs::remove_file(&part);
    have = 0;
  }
  let mut res = call(agent, policy, &asset.url, (have > 0).then_some(have))?;
  let status = res.status().as_u16();
  let resumed = status == 206
    && have > 0
    && res.headers().get("content-range").and_then(|v| v.to_str().ok()).is_some_and(|r| r.starts_with(&format!("bytes {have}-")));
  if status == 416 || (status == 206 && !resumed) {
    // The server cannot continue this part: start over on the next try
    let _ = fs::remove_file(&part);
    return Err(format!("{}: the download could not resume (HTTP {status})", asset.url));
  }
  if !resumed && status != 200 {
    return Err(format!("{}: HTTP {status}", asset.url));
  }
  let mut hash = Sha256::new();
  let mut file = OpenOptions::new().create(true).write(true).read(true).truncate(!resumed).open(&part).map_err(wrap)?;
  if resumed {
    std::io::copy(&mut (&file).take(have), &mut hash).map_err(wrap)?;
    file.seek(SeekFrom::Start(have)).map_err(wrap)?;
  } else {
    have = 0;
  }
  progress(have, asset.size);
  let mut reader = res.body_mut().as_reader();
  let mut buf = vec![0u8; 1 << 16];
  loop {
    if cancel.load(Ordering::Relaxed) {
      return Err("cancelled".into());
    }
    let n = reader.read(&mut buf).map_err(|e| format!("{}: {e}", asset.url))?;
    if n == 0 {
      break;
    }
    have += n as u64;
    if have > asset.size {
      drop(file);
      let _ = fs::remove_file(&part);
      return Err(format!("{}: larger than the manifest says", asset.url));
    }
    file.write_all(&buf[..n]).map_err(wrap)?;
    hash.update(&buf[..n]);
    progress(have, asset.size);
  }
  file.sync_all().map_err(wrap)?;
  drop(file);
  if have != asset.size {
    return Err(format!("{}: the connection closed at {have} of {} bytes", asset.url, asset.size));
  }
  let sha = hex(&hash.finalize());
  if !sha.eq_ignore_ascii_case(&asset.sha256) {
    let _ = fs::remove_file(&part);
    return Err(format!("{}: SHA-256 {sha} does not match the manifest", asset.url));
  }
  fs::rename(&part, to).map_err(wrap)
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use std::io::BufRead;
  use std::net::TcpListener;
  use std::sync::Arc;
  use std::sync::atomic::AtomicUsize;

  /// A tiny release server: `/redirect/<path>` answers 302 to `/<path>`, `/away` redirects off the loopback, any other
  /// path is a file from `files` with Range support. With `cut` set, the first full request sends half the body and
  /// closes, as a dropped connection does
  pub(crate) struct Server {
    pub base: String,
    pub hits: Arc<AtomicUsize>,
  }

  pub(crate) fn serve(files: Vec<(String, Vec<u8>)>, cut: bool) -> Server {
    serve_built(cut, |_| files)
  }

  /// `serve` with the files made once the server's address is known (a manifest names its own URLs)
  pub(crate) fn serve_built(cut: bool, make: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let files = make(&base);
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
      let mut cut = cut;
      for stream in l.incoming() {
        let Ok(mut s) = stream else { continue };
        counter.fetch_add(1, Ordering::SeqCst);
        let mut r = std::io::BufReader::new(s.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
        let mut range: Option<u64> = None;
        loop {
          let mut h = String::new();
          if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
          }
          if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
            range = v.trim().trim_end_matches('-').parse().ok();
          }
        }
        let reply = |s: &mut std::net::TcpStream, head: String, body: &[u8]| {
          let _ = s.write_all(head.as_bytes());
          let _ = s.write_all(body);
        };
        if let Some(rest) = path.strip_prefix("/redirect") {
          reply(&mut s, format!("HTTP/1.1 302 Found\r\nLocation: {rest}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"), b"");
          continue;
        }
        if path == "/away" {
          reply(
            &mut s,
            "HTTP/1.1 302 Found\r\nLocation: http://example.com/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            b"",
          );
          continue;
        }
        let Some((_, body)) = files.iter().find(|(p, _)| format!("/{p}") == path) else {
          reply(&mut s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), b"");
          continue;
        };
        match range {
          Some(from) if (from as usize) < body.len() => {
            let rest = &body[from as usize..];
            let head = format!(
              "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
              body.len() - 1,
              body.len(),
              rest.len()
            );
            reply(&mut s, head, rest);
          }
          Some(_) => reply(&mut s, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), b""),
          None => {
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let sent = if cut { &body[..body.len() / 2] } else { &body[..] };
            cut = false;
            reply(&mut s, head, sent);
          }
        }
      }
    });
    Server { base, hits }
  }

  pub(crate) fn local() -> Policy {
    Policy { hosts: vec![], loopback_http: true }
  }

  fn asset(url: String, body: &[u8]) -> Asset {
    Asset { url, size: body.len() as u64, sha256: hex(&Sha256::digest(body)) }
  }

  #[test]
  fn only_https_on_the_release_hosts_passes_the_policy() {
    let p = Policy::release();
    assert!(p.check("https://github.com/hotic/acpira/releases/latest/download/desktop-update.json").is_ok());
    assert!(p.check("https://release-assets.githubusercontent.com/x").is_ok());
    for bad in [
      "http://github.com/x",
      "https://github.com.evil.example/x",
      "https://evil.example/github.com/x",
      "https://user@github.com/x",
      "http://127.0.0.1:8080/x",
      "file:///etc/passwd",
    ] {
      assert!(p.check(bad).is_err(), "{bad}");
    }
    assert!(local().check("http://127.0.0.1:8080/x").is_ok());
    assert!(local().check("http://10.0.0.1/x").is_err());
  }

  #[test]
  fn a_dropped_download_resumes_where_it_stopped_and_is_checked_whole() {
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let srv = serve(vec![("pkg".into(), body.clone())], true);
    let d = tempfile::tempdir().unwrap();
    let to = d.path().join("pkg");
    let a = asset(format!("{}/redirect/pkg", srv.base), &body);
    let agent = agent(None);
    let cancel = AtomicBool::new(false);
    let mut seen = vec![];
    let e = download(&agent, &local(), &a, &to, &mut |done, _| seen.push(done), &cancel).unwrap_err();
    assert!(e.contains("closed") || e.contains("pkg"), "{e}");
    let part = fs::metadata(part_of(&to)).unwrap().len();
    assert!(part > 0 && part < body.len() as u64, "{part}");
    // The second try asks for the rest only
    let mut first = None;
    download(&agent, &local(), &a, &to, &mut |done, _| _ = first.get_or_insert(done), &cancel).unwrap();
    assert_eq!(first, Some(part));
    assert_eq!(fs::read(&to).unwrap(), body);
    assert!(!part_of(&to).exists());
    // Done already: no request at all
    let hits = srv.hits.load(Ordering::SeqCst);
    download(&agent, &local(), &a, &to, &mut |_, _| {}, &cancel).unwrap();
    assert_eq!(srv.hits.load(Ordering::SeqCst), hits);
  }

  #[test]
  fn a_wrong_digest_or_size_is_refused_and_leaves_nothing_behind() {
    let body = b"the real package".to_vec();
    let srv = serve(vec![("pkg".into(), body.clone())], false);
    let d = tempfile::tempdir().unwrap();
    let to = d.path().join("pkg");
    let cancel = AtomicBool::new(false);
    let mut wrong = asset(format!("{}/pkg", srv.base), &body);
    wrong.sha256 = hex(&Sha256::digest(b"another package"));
    assert!(download(&agent(None), &local(), &wrong, &to, &mut |_, _| {}, &cancel).unwrap_err().contains("SHA-256"));
    assert!(!to.exists() && !part_of(&to).exists());
    let mut short = asset(format!("{}/pkg", srv.base), &body);
    short.size = 4;
    assert!(download(&agent(None), &local(), &short, &to, &mut |_, _| {}, &cancel).unwrap_err().contains("larger"));
    assert!(!to.exists() && !part_of(&to).exists());
    // A redirect off the allowed hosts is not followed
    let away = asset(format!("{}/away", srv.base), &body);
    assert!(download(&agent(None), &local(), &away, &to, &mut |_, _| {}, &cancel).unwrap_err().contains("refusing"));
    assert!(get_small(&agent(None), &local(), &format!("{}/missing", srv.base), 10).unwrap_err().contains("404"));
    assert_eq!(get_small(&agent(None), &local(), &format!("{}/redirect/pkg", srv.base), 100).unwrap(), body);
    assert!(get_small(&agent(None), &local(), &format!("{}/pkg", srv.base), 4).unwrap_err().contains("larger"));
  }
}
