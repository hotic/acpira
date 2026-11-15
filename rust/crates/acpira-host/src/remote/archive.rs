//! Downloading and unpacking a release archive: the helper builds `fetch` takes from a platform's VSIX

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

fn hex(digest: &[u8]) -> String {
  digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download `url` into `to`, reporting (bytes so far, content length) after every chunk; the SHA-256 of what arrived
pub fn download_progress(url: &str, to: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<String> {
  let agent: ureq::Agent = crate::net_proxy::ureq_config(
    ureq::Agent::config_builder()
      .timeout_connect(Some(std::time::Duration::from_secs(30)))
      .timeout_recv_body(Some(std::time::Duration::from_secs(600)))
      .http_status_as_error(false),
  )
  .build()
  .into();
  let mut res = agent.get(url).call().with_context(|| format!("download {url}"))?;
  let status = res.status().as_u16();
  if !(200..300).contains(&status) {
    bail!("download {url}: HTTP {status}");
  }
  let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
  let mut reader = res.body_mut().as_reader();
  let mut file = std::fs::File::create(to)?;
  let mut hash = Sha256::new();
  let mut buf = vec![0u8; 1 << 16];
  let mut got: u64 = 0;
  loop {
    let n = reader.read(&mut buf).with_context(|| format!("download {url}"))?;
    if n == 0 {
      break;
    }
    file.write_all(&buf[..n])?;
    hash.update(&buf[..n]);
    got += n as u64;
    progress(got, total);
  }
  file.sync_all()?;
  if let Some(t) = total
    && t != got
  {
    bail!("download {url}: got {got} of {t} bytes");
  }
  Ok(hex(&hash.finalize()))
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
}
