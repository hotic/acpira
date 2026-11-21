//! Downloading and unpacking a release archive: the helper builds `fetch` takes from a platform's VSIX

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

const DOWNLOAD_TRIES: usize = 4;
const RETRY_BACKOFF_MS: [u64; 3] = [150, 500, 1500];

fn hex(digest: &[u8]) -> String {
  digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn part_path(to: &Path) -> PathBuf {
  let mut s = to.as_os_str().to_owned();
  s.push(".part");
  PathBuf::from(s)
}

/// Parses `Content-Range: bytes <from>-<end>/<total>` for a resumed request from `from`; returns `Some(total)` (`None`
/// when total is `*`) only when the range starts at `from`
fn content_range_total(raw: Option<&str>, from: u64) -> Option<Option<u64>> {
  let rest = raw?.trim().strip_prefix("bytes")?.trim_start();
  let (span, total_str) = rest.split_once('/')?;
  let (start, end) = span.split_once('-')?;
  if start.trim().parse::<u64>().ok()? != from {
    return None;
  }
  let end = end.trim().parse::<u64>().ok()?;
  if end < from {
    return None;
  }
  let total = match total_str.trim() {
    "*" => None,
    n => {
      let t = n.parse::<u64>().ok()?;
      if end >= t {
        return None;
      }
      Some(t)
    }
  };
  Some(total)
}

/// Marker error returned by `download_cancellable` when its cancellation flag turns true
#[derive(Debug)]
pub struct DownloadCancelled;

impl std::fmt::Display for DownloadCancelled {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("download cancelled")
  }
}

impl std::error::Error for DownloadCancelled {}

/// Download `url` into `to` (resuming `<to>.part` with HTTP `Range` across transient connection drops and retries),
/// reporting `(bytes so far, total length)` after every chunk; returns the SHA-256 of the complete file
pub fn download_progress(url: &str, to: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<String> {
  download_cancellable(url, to, progress, &|| false)
}

/// Like `download_progress`, stopping between retries and after any read chunk when `cancelled()` turns true
pub fn download_cancellable(url: &str, to: &Path, progress: &mut dyn FnMut(u64, Option<u64>), cancelled: &dyn Fn() -> bool) -> Result<String> {
  if cancelled() {
    return Err(anyhow!(DownloadCancelled));
  }
  if let Some(dir) = to.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let part = part_path(to);
  let agent: ureq::Agent = crate::net_proxy::ureq_config(
    ureq::Agent::config_builder()
      .timeout_connect(Some(std::time::Duration::from_secs(30)))
      .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
      .timeout_recv_body(Some(std::time::Duration::from_secs(600)))
      .http_status_as_error(false),
  )
  .build()
  .into();
  for attempt in 0..DOWNLOAD_TRIES {
    if cancelled() {
      let _ = std::fs::remove_file(&part);
      return Err(anyhow!(DownloadCancelled));
    }
    if attempt > 0
      && let Some(&ms) = RETRY_BACKOFF_MS.get(attempt - 1)
    {
      std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    if cancelled() {
      let _ = std::fs::remove_file(&part);
      return Err(anyhow!(DownloadCancelled));
    }
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let mut req = agent.get(url);
    if have > 0 {
      req = req.header("Range", format!("bytes={have}-"));
    }
    let mut res = match req.call() {
      Ok(r) => r,
      Err(_) if attempt + 1 < DOWNLOAD_TRIES && !cancelled() => continue,
      Err(_) if cancelled() => {
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!(DownloadCancelled));
      }
      Err(e) => return Err(e).with_context(|| format!("download {url}")),
    };
    let status = res.status().as_u16();
    let content_len = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let (resumed, total) = if status == 416 {
      let _ = std::fs::remove_file(&part);
      if attempt + 1 < DOWNLOAD_TRIES {
        continue;
      }
      bail!("download {url}: HTTP {status}");
    } else if status == 206 {
      let hdr = res.headers().get("content-range").and_then(|v| v.to_str().ok());
      match (have > 0).then(|| content_range_total(hdr, have)).flatten() {
        Some(range_total) => (true, range_total.or_else(|| content_len.map(|n| have + n))),
        None => {
          let _ = std::fs::remove_file(&part);
          if attempt + 1 < DOWNLOAD_TRIES {
            continue;
          }
          bail!("download {url}: unexpected HTTP 206");
        }
      }
    } else if (200..300).contains(&status) {
      (false, content_len)
    } else if status == 408 || status == 429 || (500..600).contains(&status) {
      if attempt + 1 < DOWNLOAD_TRIES {
        continue;
      }
      bail!("download {url}: HTTP {status}");
    } else {
      let _ = std::fs::remove_file(&part);
      bail!("download {url}: HTTP {status}");
    };
    let mut file = std::fs::OpenOptions::new().create(true).write(true).read(true).truncate(!resumed).open(&part)?;
    let mut hash = Sha256::new();
    let mut got = if resumed {
      std::io::copy(&mut (&file).take(have), &mut hash)?;
      file.seek(SeekFrom::Start(have))?;
      progress(have, total);
      have
    } else {
      0
    };
    let mut reader = res.body_mut().as_reader();
    let mut buf = vec![0u8; 1 << 16];
    let read_err = loop {
      if cancelled() {
        drop(file);
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!(DownloadCancelled));
      }
      match reader.read(&mut buf) {
        Ok(0) => break None,
        Ok(n) => {
          let chunk = buf.get(..n).unwrap_or_default();
          file.write_all(chunk)?;
          hash.update(chunk);
          got += n as u64;
          progress(got, total);
        }
        Err(e) => break Some(e),
      }
    };
    file.sync_all()?;
    drop(file);
    if let Some(e) = read_err {
      if attempt + 1 < DOWNLOAD_TRIES && !cancelled() {
        continue;
      }
      if cancelled() {
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!(DownloadCancelled));
      }
      return Err(e).with_context(|| format!("download {url}"));
    }
    if let Some(t) = total
      && t != got
    {
      if got > t {
        let _ = std::fs::remove_file(&part);
      }
      if attempt + 1 < DOWNLOAD_TRIES {
        continue;
      }
      bail!("download {url}: got {got} of {t} bytes");
    }
    std::fs::rename(&part, to)?;
    return Ok(hex(&hash.finalize()));
  }
  bail!("download {url}: retries exhausted")
}

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], i: usize) -> u16 {
  u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
  u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// A zip entry name as a relative path inside the destination: no absolute paths, drive letters, `..` or empty names
pub fn safe_entry_path(name: &str) -> Result<PathBuf> {
  let unified = name.replace('\\', "/");
  if unified.starts_with('/') || unified.contains(':') {
    bail!("unsafe path in archive: {name}");
  }
  let mut out = PathBuf::new();
  for part in unified.split('/').filter(|p| !p.is_empty() && *p != ".") {
    let c = Path::new(part).components().next();
    if part == ".." || !matches!(c, Some(Component::Normal(_))) {
      bail!("unsafe path in archive: {name}");
    }
    out.push(part);
  }
  if out.as_os_str().is_empty() {
    bail!("empty path in archive");
  }
  Ok(out)
}

/// Unpacks a (non-zip64) zip with stored / deflated entries into `dest`, which must not exist yet. Symlinks are refused,
/// sizes and CRC-32 are checked, Unix permission bits kept. Returns the number of files written
pub fn extract_zip(archive: &Path, dest: &Path) -> Result<usize> {
  let mut f = std::fs::File::open(archive)?;
  let len = f.metadata()?.len();
  let tail_len = len.min(65_557);
  f.seek(SeekFrom::Start(len - tail_len))?;
  let mut tail = vec![0u8; tail_len as usize];
  f.read_exact(&mut tail)?;
  let at = (0..tail.len().saturating_sub(21)).rev().find(|&i| u32_at(&tail, i) == EOCD).ok_or_else(|| anyhow!("not a zip archive"))?;
  let count = u16_at(&tail, at + 10) as usize;
  let cd_size = u32_at(&tail, at + 12) as u64;
  let cd_offset = u32_at(&tail, at + 16) as u64;
  if count == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff {
    bail!("zip64 archives are not supported");
  }
  if cd_offset + cd_size > len {
    bail!("corrupt zip: central directory out of range");
  }
  f.seek(SeekFrom::Start(cd_offset))?;
  let mut cd = vec![0u8; cd_size as usize];
  f.read_exact(&mut cd)?;
  std::fs::create_dir(dest).with_context(|| format!("create {}", dest.display()))?;
  let mut p = 0usize;
  let mut files = 0;
  for _ in 0..count {
    if p + 46 > cd.len() || u32_at(&cd, p) != CENTRAL {
      bail!("corrupt zip: bad central directory entry");
    }
    let made_by_host = cd[p + 5];
    let flags = u16_at(&cd, p + 8);
    let method = u16_at(&cd, p + 10);
    let crc = u32_at(&cd, p + 16);
    let csize = u32_at(&cd, p + 20) as u64;
    let usize_ = u32_at(&cd, p + 24) as u64;
    let name_len = u16_at(&cd, p + 28) as usize;
    let extra_len = u16_at(&cd, p + 30) as usize;
    let comment_len = u16_at(&cd, p + 32) as usize;
    let attrs = u32_at(&cd, p + 38);
    let local = u32_at(&cd, p + 42) as u64;
    if p + 46 + name_len > cd.len() {
      bail!("corrupt zip: name out of range");
    }
    let name = String::from_utf8_lossy(&cd[p + 46..p + 46 + name_len]).into_owned();
    p += 46 + name_len + extra_len + comment_len;
    if flags & 1 != 0 {
      bail!("encrypted entry in archive: {name}");
    }
    let unix_mode = (made_by_host == 3).then_some(attrs >> 16).filter(|m| *m != 0);
    if unix_mode.is_some_and(|m| m & 0o170000 == 0o120000) {
      bail!("symlink in archive refused: {name}");
    }
    let rel = safe_entry_path(&name)?;
    let out = dest.join(&rel);
    if name.ends_with('/') || name.ends_with('\\') {
      std::fs::create_dir_all(&out)?;
      continue;
    }
    if let Some(parent) = out.parent() {
      std::fs::create_dir_all(parent)?;
    }
    f.seek(SeekFrom::Start(local))?;
    let mut head = [0u8; 30];
    f.read_exact(&mut head)?;
    if u32_at(&head, 0) != LOCAL {
      bail!("corrupt zip: bad local header for {name}");
    }
    let skip = u16_at(&head, 26) as i64 + u16_at(&head, 28) as i64;
    f.seek(SeekFrom::Current(skip))?;
    let raw = (&mut f).take(csize);
    let mut reader: Box<dyn Read> = match method {
      0 => Box::new(raw),
      8 => Box::new(flate2::read::DeflateDecoder::new(raw)),
      m => bail!("unsupported compression method {m} for {name}"),
    };
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&out).with_context(|| format!("write {}", out.display()))?;
    let mut sum = flate2::Crc::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut written: u64 = 0;
    loop {
      let n = reader.read(&mut buf)?;
      if n == 0 {
        break;
      }
      written += n as u64;
      if written > usize_ {
        bail!("corrupt zip: {name} is larger than declared");
      }
      sum.update(&buf[..n]);
      file.write_all(&buf[..n])?;
    }
    if written != usize_ || sum.sum() != crc {
      bail!("corrupt zip: {name} failed its size / CRC check");
    }
    file.sync_all()?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = unix_mode.map(|m| m & 0o777).unwrap_or(0o644) | 0o600;
      std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
    }
    files += 1;
  }
  Ok(files)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn entry_paths_cannot_leave_the_destination() {
    for bad in ["/etc/passwd", "../x", "a/../../x", "C:/x", "a\\..\\..\\x", "", "./"] {
      assert!(safe_entry_path(bad).is_err(), "{bad}");
    }
    assert_eq!(safe_entry_path("./a/b.txt").unwrap(), PathBuf::from("a").join("b.txt"));
    assert_eq!(safe_entry_path("extension/bin/acpira").unwrap(), PathBuf::from("extension").join("bin").join("acpira"));
  }

  const FILE: u32 = 0o100644;
  const LINK: u32 = 0o120777;

  /// A minimal zip: (name, data, unix mode, deflate)
  fn zip(entries: &[(&str, &[u8], u32, bool)]) -> Vec<u8> {
    let (mut out, mut central) = (vec![], vec![]);
    for (name, data, mode, deflate) in entries {
      let mut crc = flate2::Crc::new();
      crc.update(data);
      let body = if *deflate {
        let mut e = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
      } else {
        data.to_vec()
      };
      let method: u16 = if *deflate { 8 } else { 0 };
      let offset = out.len() as u32;
      let common = |v: &mut Vec<u8>| {
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&method.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&crc.sum().to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
      };
      out.extend_from_slice(&LOCAL.to_le_bytes());
      out.extend_from_slice(&20u16.to_le_bytes());
      common(&mut out);
      out.extend_from_slice(name.as_bytes());
      out.extend_from_slice(&body);
      central.extend_from_slice(&CENTRAL.to_le_bytes());
      central.extend_from_slice(&[20, 3]);
      central.extend_from_slice(&20u16.to_le_bytes());
      common(&mut central);
      central.extend_from_slice(&[0; 6]);
      central.extend_from_slice(&(mode << 16).to_le_bytes());
      central.extend_from_slice(&offset.to_le_bytes());
      central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&EOCD.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
  }

  fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
  }

  #[test]
  fn extraction_refuses_escapes_symlinks_and_corrupt_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let cases: [(&str, Vec<u8>, &str); 4] = [
      ("escape", zip(&[("ok", b"1", FILE, false), ("../../evil", b"x", FILE, false)]), "unsafe path"),
      ("absolute", zip(&[("/tmp/evil", b"x", FILE, true)]), "unsafe path"),
      ("link", zip(&[("lib", b"/etc", LINK, false)]), "symlink"),
      ("dup", zip(&[("a", b"1", FILE, false), ("a", b"2", FILE, false)]), "write"),
    ];
    for (name, bytes, why) in cases {
      let file = put(tmp.path(), &format!("{name}.zip"), &bytes);
      let e = extract_zip(&file, &tmp.path().join(format!("out-{name}"))).err().unwrap_or_else(|| panic!("{name}"));
      assert!(format!("{e:#}").contains(why), "{name}: {e:#}");
    }
    assert!(!tmp.path().join("evil").exists());
    // A flipped byte in stored data fails the CRC
    let mut bytes = zip(&[("data", b"hello world", FILE, false)]);
    let at = bytes.windows(5).position(|w| w == b"hello").unwrap();
    bytes[at] = b'j';
    let file = put(tmp.path(), "crc.zip", &bytes);
    assert!(extract_zip(&file, &tmp.path().join("out-crc")).err().unwrap().to_string().contains("CRC"));
    let file = put(tmp.path(), "nozip.zip", b"not a zip at all, just text that is long enough");
    assert!(extract_zip(&file, &tmp.path().join("out-nozip")).is_err());
    // Nested directories and both methods round-trip
    let file = put(tmp.path(), "nested.zip", &zip(&[("a/", b"", 0o40755, false), ("a/b/c.txt", b"deep", FILE, true)]));
    assert_eq!(extract_zip(&file, &tmp.path().join("out-nested")).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(tmp.path().join("out-nested/a/b/c.txt")).unwrap(), "deep");
  }

  #[test]
  fn interrupted_downloads_retry_and_resume_via_range() {
    use std::io::BufRead;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let body: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
    let expected = hex(&Sha256::digest(&body));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_srv = hits.clone();
    let served = body.clone();
    std::thread::spawn(move || {
      let mut cuts = 2usize;
      for stream in listener.incoming().flatten() {
        let call = hits_srv.fetch_add(1, Ordering::SeqCst);
        let mut r = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        if r.read_line(&mut line).is_err() {
          continue;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let mut range: Option<usize> = None;
        loop {
          let mut h = String::new();
          if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
          }
          if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
            range = v.trim().trim_end_matches('-').parse().ok();
          }
        }
        let mut s = stream;
        if path == "/missing" {
          let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
          continue;
        }
        // First hit drops before sending any HTTP status, as a reset proxy tunnel does
        if call == 0 {
          continue;
        }
        let from = range.unwrap_or(0);
        let slice = &served[from..];
        let head = if from > 0 {
          format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len() - 1,
            served.len(),
            slice.len()
          )
        } else {
          format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len())
        };
        let _ = s.write_all(head.as_bytes());
        let send_len = if cuts > 0 {
          cuts -= 1;
          (slice.len() / 2).max(1)
        } else {
          slice.len()
        };
        let _ = s.write_all(&slice[..send_len]);
      }
    });

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("asset.vsix");
    let mut seen = vec![];
    let got = download_progress(&format!("{base}/asset.vsix"), &out, &mut |done, total| seen.push((done, total))).unwrap();
    assert_eq!(got, expected);
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert!(!part_path(&out).exists());
    assert_eq!(hits.load(Ordering::SeqCst), 4, "1 reset + 2 cut streams + 1 final range completion");
    assert!(seen.windows(2).all(|w| w[1].0 >= w[0].0), "progress stays monotonic across range resumes: {seen:?}");
    assert_eq!(seen.last().copied(), Some((body.len() as u64, Some(body.len() as u64))));

    // Permanent 404 fails on the first request without retrying
    let before = hits.load(Ordering::SeqCst);
    let err = download_progress(&format!("{base}/missing"), &tmp.path().join("missing"), &mut |_, _| {}).unwrap_err();
    assert!(format!("{err:#}").contains("HTTP 404"), "{err:#}");
    assert_eq!(hits.load(Ordering::SeqCst), before + 1);
  }
}
