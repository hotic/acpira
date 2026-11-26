//! Acpira's private portable Node.js LTS runtime (`<ACPIRA_HOME>/runtime/node`).
//!
//! Only downloaded after the user confirms when installing an npm-based agent on a machine without Node.js. It never
//! modifies the system PATH or clashes with nvm / fnm / volta: its bin directory is appended at the very end of the
//! engine's effective PATH only when present, so any system Node.js always wins.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use serde::Deserialize;

use crate::acp::agents::installer::{Cancel, Log};
use crate::http::archive::{DownloadCancelled, download_cancellable, extract_zip, safe_entry_path};
use crate::platform::command::Os;

/// Default base URL for official Node.js release archives; overridable via `ACPIRA_NODE_DIST_URL`
const DEFAULT_DIST_URL: &str = "https://nodejs.org/dist";
pub const DIST_URL_ENV: &str = "ACPIRA_NODE_DIST_URL";
const META_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// Serializes concurrent managed-Node installs so two agent installs clicking "Install Node.js" share one download
static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
  TarGz,
  Zip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformTarget {
  /// Identifier in Node's `index.json` `files` array (e.g. `osx-arm64-tar`, `win-x64-zip`)
  pub index_token: &'static str,
  /// Suffix after `node-<version>-` in the archive filename (e.g. `darwin-arm64.tar.gz`)
  pub suffix: &'static str,
  pub kind: ArchiveKind,
  pub os: Os,
}

impl PlatformTarget {
  pub fn current() -> Result<Self> {
    Self::for_platform(std::env::consts::OS, std::env::consts::ARCH)
  }

  pub fn for_platform(os: &str, arch: &str) -> Result<Self> {
    match (os, arch) {
      ("macos", "aarch64") => Ok(Self { index_token: "osx-arm64-tar", suffix: "darwin-arm64.tar.gz", kind: ArchiveKind::TarGz, os: Os::Posix }),
      ("macos", "x86_64") => Ok(Self { index_token: "osx-x64-tar", suffix: "darwin-x64.tar.gz", kind: ArchiveKind::TarGz, os: Os::Posix }),
      ("linux", "x86_64") => Ok(Self { index_token: "linux-x64", suffix: "linux-x64.tar.gz", kind: ArchiveKind::TarGz, os: Os::Posix }),
      ("linux", "aarch64") => Ok(Self { index_token: "linux-arm64", suffix: "linux-arm64.tar.gz", kind: ArchiveKind::TarGz, os: Os::Posix }),
      ("windows", "x86_64") => Ok(Self { index_token: "win-x64-zip", suffix: "win-x64.zip", kind: ArchiveKind::Zip, os: Os::Windows }),
      ("windows", "aarch64") => Ok(Self { index_token: "win-arm64-zip", suffix: "win-arm64.zip", kind: ArchiveKind::Zip, os: Os::Windows }),
      _ => bail!("automatic Node.js download is not supported on {os}/{arch}"),
    }
  }
}

pub use crate::acp::login_path::{append_managed_bin, bin_dir_in, managed_bin_dir, managed_root, node_exe_in, npm_exe_in};

#[derive(Deserialize)]
struct DistRelease {
  version: String,
  #[serde(default)]
  lts: serde_json::Value,
  #[serde(default)]
  files: Vec<String>,
}

/// Latest LTS version from `nodejs.org/dist/index.json` that ships the archive for `target`
pub fn parse_latest_lts(index_json: &str, target: PlatformTarget) -> Result<String> {
  let releases: Vec<DistRelease> = serde_json::from_str(index_json).context("parse Node.js release index")?;
  releases
    .into_iter()
    .find(|r| {
      let is_lts = r.lts.as_str().is_some_and(|s| !s.is_empty()) || r.lts.as_bool() == Some(true);
      is_lts && r.version.starts_with('v') && r.files.iter().any(|f| f == target.index_token)
    })
    .map(|r| r.version)
    .ok_or_else(|| anyhow!("no Node.js LTS release found for {}", target.index_token))
}

/// Expected SHA-256 hex digest for `filename` from `SHASUMS256.txt`
pub fn parse_shasum(shasums: &str, filename: &str) -> Result<String> {
  for line in shasums.lines() {
    let mut parts = line.split_whitespace();
    let Some(hash) = parts.next() else { continue };
    let Some(name) = parts.next() else { continue };
    if name == filename && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
      return Ok(hash.to_ascii_lowercase());
    }
  }
  bail!("checksum for {filename} not found in SHASUMS256.txt")
}

fn fetch_text(url: &str) -> Result<String> {
  let agent: ureq::Agent = crate::net_proxy::ureq_config(
    ureq::Agent::config_builder()
      .timeout_connect(Some(META_TIMEOUT))
      .timeout_recv_response(Some(META_TIMEOUT))
      .timeout_recv_body(Some(META_TIMEOUT))
      .http_status_as_error(false),
  )
  .build()
  .into();
  let mut res = agent.get(url).call().with_context(|| format!("fetch {url}"))?;
  let status = res.status().as_u16();
  if !(200..300).contains(&status) {
    bail!("fetch {url}: HTTP {status}");
  }
  let mut body = String::new();
  // Release index is ~2 MB; cap at 16 MB so a broken response never exhausts memory
  res.body_mut().as_reader().take(16 * 1024 * 1024).read_to_string(&mut body).with_context(|| format!("read {url}"))?;
  Ok(body)
}

/// Validate that a relative symlink target inside `rel`'s parent directory never escapes the archive root
fn safe_symlink_target(rel: &Path, target: &Path) -> Result<()> {
  if target.is_absolute() {
    bail!("absolute symlink target refused: {}", target.display());
  }
  let mut depth: usize = rel.parent().map(|p| p.components().count()).unwrap_or(0);
  for comp in target.components() {
    match comp {
      Component::CurDir => {}
      Component::Normal(_) => depth += 1,
      Component::ParentDir => {
        depth = depth.checked_sub(1).ok_or_else(|| anyhow!("escaping symlink target refused: {}", target.display()))?;
      }
      Component::RootDir | Component::Prefix(_) => bail!("unsafe symlink target refused: {}", target.display()),
    }
  }
  Ok(())
}

/// Unpack a `.tar.gz` archive into `dest` (which must not exist yet), keeping executable bits and verified internal
/// relative symlinks (such as `bin/npm -> ../lib/node_modules/npm/bin/npm-cli.js`)
pub fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<usize> {
  let file = std::fs::File::open(archive)?;
  let gz = flate2::read::GzDecoder::new(file);
  let mut tar = tar::Archive::new(gz);
  std::fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
  let mut files = 0usize;
  for entry in tar.entries()? {
    let mut entry = entry?;
    let entry_type = entry.header().entry_type();
    if entry_type.is_pax_global_extensions() || entry_type.is_pax_local_extensions() || entry_type.is_gnu_longname() || entry_type.is_gnu_longlink() {
      continue;
    }
    let raw_path = entry.path()?.to_string_lossy().into_owned();
    let rel = safe_entry_path(&raw_path)?;
    let out = dest.join(&rel);
    if entry_type.is_dir() {
      std::fs::create_dir_all(&out)?;
    } else if entry_type.is_file() {
      if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
      }
      let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(&out).with_context(|| format!("write {}", out.display()))?;
      std::io::copy(&mut entry, &mut dst)?;
      dst.sync_all()?;
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        let mode = (entry.header().mode().unwrap_or(0o644) & 0o777) | 0o600;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
      }
      files += 1;
    } else if entry_type.is_symlink() {
      let target = entry.link_name()?.ok_or_else(|| anyhow!("symlink without target: {raw_path}"))?;
      safe_symlink_target(&rel, &target)?;
      if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
      }
      #[cfg(unix)]
      {
        std::os::unix::fs::symlink(target.as_ref(), &out).with_context(|| format!("symlink {}", out.display()))?;
        files += 1;
      }
      // Windows Node.js ships as a zip, so a tar symlink only appears in a malformed archive there.
      #[cfg(not(unix))]
      bail!("symlink in archive unsupported on this platform: {raw_path}");
    } else {
      bail!("unsupported tar entry type {:?} for {raw_path}", entry_type);
    }
  }
  Ok(files)
}

fn format_mb(bytes: u64) -> String {
  format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

/// Download, verify, and unpack the latest official Node.js LTS into `dest_root` if not already present.
/// Progress is written into `log` and pushed via `update`. Returns `Err(DownloadCancelled)` when cancelled.
pub async fn ensure_managed_node(
  dest_root: PathBuf,
  target: PlatformTarget,
  dist_base: Option<String>,
  cancel: Cancel,
  log: Arc<Mutex<Log>>,
  update: Arc<dyn Fn() + Send + Sync>,
) -> Result<String> {
  let _guard = INSTALL_LOCK.lock().await;
  if node_exe_in(&dest_root, target.os).is_file() && npm_exe_in(&dest_root, target.os).is_file() {
    return Ok("cached".into());
  }
  let base = dist_base
    .or_else(|| std::env::var(DIST_URL_ENV).ok().filter(|s| !s.trim().is_empty()))
    .unwrap_or_else(|| DEFAULT_DIST_URL.to_owned());
  let base = base.trim_end_matches('/').to_owned();
  tokio::task::spawn_blocking(move || install_blocking(&dest_root, target, &base, &cancel, &log, &update))
    .await
    .map_err(|e| anyhow!("Node.js installer task failed: {e}"))?
}

fn install_blocking(
  dest_root: &Path,
  target: PlatformTarget,
  base: &str,
  cancel: &Cancel,
  log: &Arc<Mutex<Log>>,
  update: &Arc<dyn Fn() + Send + Sync>,
) -> Result<String> {
  let push = |line: String| {
    log.lock().push(line);
    update();
  };
  if cancel.is_cancelled() {
    return Err(anyhow!(DownloadCancelled));
  }
  push(format!("$ Checking latest Node.js LTS from {base}…"));
  let index = fetch_text(&format!("{base}/index.json"))?;
  let version = parse_latest_lts(&index, target)?;
  let filename = format!("node-{version}-{}", target.suffix);
  if cancel.is_cancelled() {
    return Err(anyhow!(DownloadCancelled));
  }
  let shasums = fetch_text(&format!("{base}/{version}/SHASUMS256.txt"))?;
  let expected = parse_shasum(&shasums, &filename)?;

  let parent = dest_root.parent().ok_or_else(|| anyhow!("invalid runtime destination: {}", dest_root.display()))?;
  std::fs::create_dir_all(parent)?;
  let nonce = uuid::Uuid::new_v4().simple().to_string();
  let archive_path = parent.join(format!(".{filename}.{nonce}"));
  let staging = parent.join(format!(".node-staging-{nonce}"));

  let download_url = format!("{base}/{version}/{filename}");
  push(format!("Downloading {filename}…"));
  let mut last_push = Instant::now() - PROGRESS_EVERY;
  let result = download_cancellable(
    &download_url,
    &archive_path,
    &mut |done, total| {
      if last_push.elapsed() >= PROGRESS_EVERY || total.is_some_and(|t| done >= t) {
        last_push = Instant::now();
        let line = match total {
          Some(t) if t > 0 => format!("Downloading {filename} ({} / {} MB)", format_mb(done), format_mb(t)),
          _ => format!("Downloading {filename} ({} MB)", format_mb(done)),
        };
        log.lock().set_progress(line);
        update();
      }
    },
    &|| cancel.is_cancelled(),
  );
  let actual = match result {
    Ok(h) => h,
    Err(e) => {
      let _ = std::fs::remove_file(&archive_path);
      return Err(e);
    }
  };
  if actual != expected {
    let _ = std::fs::remove_file(&archive_path);
    bail!("SHA-256 mismatch for {filename}: expected {expected}, got {actual}");
  }
  if cancel.is_cancelled() {
    let _ = std::fs::remove_file(&archive_path);
    return Err(anyhow!(DownloadCancelled));
  }

  push(format!("Extracting {filename}…"));
  let unpack = match target.kind {
    ArchiveKind::Zip => extract_zip(&archive_path, &staging),
    ArchiveKind::TarGz => extract_tar_gz(&archive_path, &staging),
  };
  let _ = std::fs::remove_file(&archive_path);
  if let Err(e) = unpack {
    let _ = std::fs::remove_dir_all(&staging);
    return Err(e);
  }

  // Official Node archives wrap everything under `node-<version>-<platform>/`
  let inner_name = filename.strip_suffix(".tar.gz").or_else(|| filename.strip_suffix(".zip")).unwrap_or(&filename);
  let unpacked = staging.join(inner_name);
  let source = if node_exe_in(&unpacked, target.os).is_file() {
    unpacked
  } else if node_exe_in(&staging, target.os).is_file() {
    staging.clone()
  } else {
    let _ = std::fs::remove_dir_all(&staging);
    bail!("extracted archive did not contain Node.js executable");
  };
  if !npm_exe_in(&source, target.os).is_file() {
    let _ = std::fs::remove_dir_all(&staging);
    bail!("extracted archive did not contain npm");
  }

  // Atomic-ish directory swap on the same volume, restoring the prior copy on failure
  let backup = parent.join(format!(".node-old-{nonce}"));
  let had_old = dest_root.exists();
  if had_old {
    std::fs::rename(dest_root, &backup).with_context(|| format!("move old runtime {}", dest_root.display()))?;
  }
  if let Err(e) = std::fs::rename(&source, dest_root) {
    if had_old {
      let _ = std::fs::rename(&backup, dest_root);
    }
    let _ = std::fs::remove_dir_all(&staging);
    return Err(e).with_context(|| format!("install runtime at {}", dest_root.display()));
  }
  if had_old {
    let _ = std::fs::remove_dir_all(&backup);
  }
  let _ = std::fs::remove_dir_all(&staging);
  push(format!("Installed Node.js {version} to {}", dest_root.display()));
  Ok(version)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_latest_lts_and_matching_shasum() {
    let index = r#"[
      {"version":"v27.0.0","lts":false,"files":["osx-arm64-tar","win-x64-zip"]},
      {"version":"v26.13.1","lts":"Lithium","files":["osx-arm64-tar","win-x64-zip","linux-x64"]},
      {"version":"v24.11.0","lts":"Krypton","files":["osx-arm64-tar","win-x64-zip"]}
    ]"#;
    let mac = PlatformTarget::for_platform("macos", "aarch64").unwrap();
    let win = PlatformTarget::for_platform("windows", "x86_64").unwrap();
    assert_eq!(parse_latest_lts(index, mac).unwrap(), "v26.13.1");
    assert_eq!(parse_latest_lts(index, win).unwrap(), "v26.13.1");
    let sha = "a".repeat(64);
    let sums = format!("{}  other.tar.gz\n{sha}  node-v26.13.1-darwin-arm64.tar.gz\n", "b".repeat(64));
    assert_eq!(parse_shasum(&sums, "node-v26.13.1-darwin-arm64.tar.gz").unwrap(), sha);
    assert!(parse_shasum(&sums, "missing.zip").is_err());
  }

  #[test]
  fn managed_bin_is_appended_at_lowest_priority_without_duplicates() {
    let posix = Path::new("/home/u/.acpira/runtime/node/bin");
    assert_eq!(append_managed_bin(Some("/usr/bin:/bin".into()), Some(posix), Os::Posix).as_deref(), Some("/usr/bin:/bin:/home/u/.acpira/runtime/node/bin"));
    assert_eq!(append_managed_bin(Some("/usr/bin:/home/u/.acpira/runtime/node/bin/".into()), Some(posix), Os::Posix).as_deref(), Some("/usr/bin:/home/u/.acpira/runtime/node/bin/"));
    let win = Path::new(r"C:\Users\u\.acpira\runtime\node");
    assert_eq!(
      append_managed_bin(Some(r"C:\Windows\System32;c:/users/u/.acpira/runtime/node/".into()), Some(win), Os::Windows).as_deref(),
      Some(r"C:\Windows\System32;c:/users/u/.acpira/runtime/node/")
    );
    assert_eq!(append_managed_bin(None, Some(win), Os::Windows).as_deref(), Some(r"C:\Users\u\.acpira\runtime\node"));
  }

  #[test]
  fn symlink_target_validation_keeps_internal_links_and_blocks_escapes() {
    assert!(safe_symlink_target(Path::new("node-v26/bin/npm"), Path::new("../lib/node_modules/npm/bin/npm-cli.js")).is_ok());
    assert!(safe_symlink_target(Path::new("node-v26/bin/npm"), Path::new("../../../etc/passwd")).is_err());
    assert!(safe_symlink_target(Path::new("node-v26/bin/npm"), Path::new("/etc/passwd")).is_err());
  }

  /// (path, file body, symlink target, mode)
  #[cfg(unix)]
  type TarEntry<'a> = (&'a str, Option<&'a [u8]>, Option<&'a str>, u32);

  #[cfg(unix)]
  fn make_tar_gz(entries: &[TarEntry]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    for (path, body, link, mode) in entries {
      let mut h = tar::Header::new_gnu();
      h.set_mode(*mode);
      if let Some(target) = link {
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        builder.append_link(&mut h, path, target).unwrap();
      } else if let Some(data) = body {
        h.set_entry_type(tar::EntryType::Regular);
        h.set_size(data.len() as u64);
        builder.append_data(&mut h, path, *data).unwrap();
      }
    }
    builder.into_inner().unwrap().finish().unwrap()
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn downloads_verifies_and_extracts_managed_node_from_mock_dist() {
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, Write};
    let target = PlatformTarget::for_platform("macos", "aarch64").unwrap();
    let filename = format!("node-v26.13.1-{}", target.suffix);
    let archive_bytes = make_tar_gz(&[
      ("node-v26.13.1-darwin-arm64/bin/node", Some(b"#!/bin/sh\necho v26.13.1\n"), None, 0o755),
      ("node-v26.13.1-darwin-arm64/lib/node_modules/npm/bin/npm-cli.js", Some(b"#!/bin/sh\necho 11.20.0\n"), None, 0o755),
      ("node-v26.13.1-darwin-arm64/bin/npm", None, Some("../lib/node_modules/npm/bin/npm-cli.js"), 0o777),
    ]);
    let sha: String = Sha256::digest(&archive_bytes).iter().map(|b| format!("{b:02x}")).collect();
    let index = r#"[{"version":"v26.13.1","lts":"Lithium","files":["osx-arm64-tar"]}]"#.to_owned();
    let sums = format!("{sha}  {filename}\n");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (index_s, sums_s, archive_s, file_s) = (index.clone(), sums.clone(), archive_bytes.clone(), filename.clone());
    std::thread::spawn(move || {
      for mut stream in listener.incoming().flatten() {
        let mut r = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        if r.read_line(&mut line).is_err() {
          continue;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/");
        loop {
          let mut h = String::new();
          if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
          }
        }
        let body: Option<&[u8]> = if path == "/index.json" {
          Some(index_s.as_bytes())
        } else if path == "/v26.13.1/SHASUMS256.txt" {
          Some(sums_s.as_bytes())
        } else if path == format!("/v26.13.1/{file_s}") {
          Some(&archive_s)
        } else {
          None
        };
        match body {
          Some(b) => {
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(b);
          }
          None => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
          }
        }
      }
    });

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("runtime").join("node");
    let log = Arc::new(Mutex::new(Log::default()));
    let ver = ensure_managed_node(dest.clone(), target, Some(base), Cancel::default(), log.clone(), Arc::new(|| {})).await.unwrap();
    assert_eq!(ver, "v26.13.1");
    assert!(node_exe_in(&dest, Os::Posix).is_file());
    assert!(npm_exe_in(&dest, Os::Posix).is_file());
    assert!(log.lock().lines().iter().any(|l| l.contains("Installed Node.js v26.13.1")));
  }
}
