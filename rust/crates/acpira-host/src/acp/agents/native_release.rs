//! Agents shipped as a native release archive instead of an install script or an npm package: Google's Antigravity ACP
//! server (ACP Registry entry `antigravity-acp`). The archive is pinned per platform with the SHA-256 observed when the
//! version was verified; `acpira install-agent <id>` downloads, checks and unpacks it under
//! `$ACPIRA_HOME/agents/<id>/<version>/` and only then points `current` at it, so a running server keeps its own files.
//! The archive is kept whole: the server finds `localharness_external` next to its own path (agy_acp_server 1.2.1
//! `_configure_localharness_path`: `dirname(argv[0])`, then `dirname(sys.executable)`)

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::store::data_dir::acpira_home;

#[derive(Debug, PartialEq, Eq)]
pub struct NativeAsset {
  /// ACP Registry platform key: `darwin-aarch64`, `linux-x86_64`, `windows-aarch64`, …
  pub platform: &'static str,
  pub url: &'static str,
  /// Observed when this version was verified (the registry entry carries no digest), not a vendor signature
  pub sha256: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NativeRelease {
  pub agent: &'static str,
  pub registry_id: &'static str,
  pub version: &'static str,
  /// Download hosts an asset URL may name (https only)
  pub hosts: &'static [&'static str],
  pub assets: &'static [NativeAsset],
  /// The launcher inside the archive, per OS family
  pub posix_cmd: &'static str,
  pub windows_cmd: &'static str,
  /// Arguments the registry entry passes on Linux only
  pub linux_args: &'static [&'static str],
  pub docs: &'static str,
}

/// agy-acp-server 1.3.0 as listed in the ACP Registry on 2026-10-08; digests computed from the downloads that day
pub static ANTIGRAVITY: NativeRelease = NativeRelease {
  agent: "antigravity",
  registry_id: "antigravity-acp",
  version: "1.3.0",
  hosts: &["dl.google.com"],
  assets: &[
    NativeAsset {
      platform: "darwin-aarch64",
      url: "https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.3.0-darwin-arm64.zip",
      sha256: "7cd97045f7b4fe81175a107cdf16f9c51484e3c78a5162cae415338bb6aa5b88",
    },
    NativeAsset {
      platform: "darwin-x86_64",
      url: "https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.3.0-darwin-x86_64.zip",
      sha256: "bb23956b89984bf5d354af2c3725e6c57f0cc1b7228e77a0e91c9c2bc1d47646",
    },
    NativeAsset {
      platform: "linux-x86_64",
      url: "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.3.0-linux-x86_64.zip",
      sha256: "9fb60956af0a9d76220a4db91ca9ac88e2a2372ad68f985ab5fceace6b825b96",
    },
    NativeAsset {
      platform: "linux-aarch64",
      url: "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.3.0-linux-arm64.zip",
      sha256: "500b0bc0fb858e88f4df404d4cedf80bf9298c178291e39e383d6c50b111cbdf",
    },
    NativeAsset {
      platform: "windows-x86_64",
      url: "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-1.3.0-windows-x86_64.zip",
      sha256: "65215e0688681fa3116e048a9eab27ef53af1bbd6f3da3f1c52bd4911d8b17f9",
    },
    NativeAsset {
      platform: "windows-aarch64",
      url: "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-1.3.0-windows-arm64.zip",
      sha256: "4a0f469720e9beb9438a979f543fdbfad5022ebe0992c052c590bd78b3144ca3",
    },
  ],
  posix_cmd: "agy_acp_server.par",
  windows_cmd: "agy_acp_server.exe",
  linux_args: &["--uid="],
  docs: "https://antigravity.google/docs/ide/extensions",
};

pub fn release_of(agent: &str) -> Option<&'static NativeRelease> {
  [&ANTIGRAVITY].into_iter().find(|r| r.agent == agent)
}

/// The registry's platform key for an `std::env::consts` OS / ARCH pair
pub fn platform_key(os: &str, arch: &str) -> Option<String> {
  let os = match os {
    "macos" => "darwin",
    "linux" => "linux",
    "windows" => "windows",
    _ => return None,
  };
  let arch = match arch {
    "aarch64" => "aarch64",
    "x86_64" => "x86_64",
    _ => return None,
  };
  Some(format!("{os}-{arch}"))
}

pub fn current_platform() -> Option<String> {
  platform_key(std::env::consts::OS, std::env::consts::ARCH)
}

impl NativeRelease {
  pub fn asset(&self, platform: &str) -> Option<&NativeAsset> {
    self.assets.iter().find(|a| a.platform == platform)
  }

  /// (launcher, args) on a platform key
  pub fn launch(&self, platform: &str) -> (&'static str, Vec<String>) {
    let cmd = if platform.starts_with("windows-") { self.windows_cmd } else { self.posix_cmd };
    let args = if platform.starts_with("linux-") { self.linux_args.iter().map(|a| (*a).to_owned()).collect() } else { vec![] };
    (cmd, args)
  }

  pub fn dir(&self, root: &Path) -> PathBuf {
    root.join("agents").join(self.agent)
  }

  /// The version `current` points at, when that install is complete
  pub fn installed(&self, root: &Path, platform: &str) -> Option<(String, PathBuf)> {
    let dir = self.dir(root);
    let version = std::fs::read_to_string(dir.join("current")).ok()?.trim().to_owned();
    if !valid_version(&version) {
      return None;
    }
    let bin = dir.join(&version).join(self.launch(platform).0);
    bin.is_file().then_some((version, bin))
  }

  /// The launcher of the managed install on this machine, where the registry looks first
  pub fn managed_binary(&self) -> Option<String> {
    let platform = current_platform()?;
    self.installed(&acpira_home(), &platform).map(|(_, p)| p.to_string_lossy().into_owned())
  }
}

/// A version is one directory name: digits, letters, dots, dashes
fn valid_version(v: &str) -> bool {
  !v.is_empty() && v.len() <= 64 && v != "." && v != ".." && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// What `acpira install-agent` reports back
pub enum Installed {
  Fresh { version: String, path: PathBuf, sha256: String },
  Already { version: String, path: PathBuf },
}

/// Download (or take `archive`, a copy fetched by hand), verify and unpack the pinned archive for `platform` under
/// `root`, then switch `current` to it. Blocking. `say` receives progress lines
pub fn install(
  release: &NativeRelease,
  root: &Path,
  platform: &str,
  force: bool,
  archive: Option<&Path>,
  say: &mut dyn FnMut(&str),
) -> Result<Installed> {
  let asset = release.asset(platform).ok_or_else(|| {
    anyhow!("{} {} has no build for {platform} (available: {})", release.registry_id, release.version, platforms(release))
  })?;
  check_url(release, asset.url)?;
  let dir = release.dir(root);
  if !force
    && let Some((version, path)) = release.installed(root, platform)
    && version == release.version
  {
    return Ok(Installed::Already { version, path });
  }
  std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
  let _lock = InstallLock::take(&dir.join("install.lock"))?;
  let previous = std::fs::read_to_string(dir.join("current")).ok().map(|s| s.trim().to_owned()).filter(|v| valid_version(v));
  let stage = dir.join(format!(".stage-{}", crate::util::random_hex(6)));
  std::fs::create_dir_all(&stage)?;
  let result = (|| -> Result<Installed> {
    let (archive, sha256) = match archive {
      Some(local) => {
        // Hash a private copy and unpack that same copy: the original could change between the check and the unpack
        say(&format!("Using {}", local.display()));
        let to = stage.join("archive.zip");
        let sha = copy_hashed(local, &to)?;
        (to, sha)
      }
      None => {
        let to = stage.join("archive.zip");
        say(&format!("Downloading {}", asset.url));
        let sha = download(asset.url, &to, say)?;
        (to, sha)
      }
    };
    if !sha256.eq_ignore_ascii_case(asset.sha256) {
      bail!("SHA-256 mismatch for {}: expected {}, got {sha256}. Nothing was installed", asset.url, asset.sha256);
    }
    say(&format!("SHA-256 {sha256} (matches the digest recorded for {} {})", release.registry_id, release.version));
    let pkg = stage.join("pkg");
    let files = extract_zip(&archive, &pkg)?;
    say(&format!("Unpacked {files} file(s)"));
    let (cmd, _) = release.launch(platform);
    if !pkg.join(cmd).is_file() {
      bail!("the archive has no {cmd}");
    }
    let target = dir.join(release.version);
    // A same-version reinstall: the old copy moves aside (a running server keeps its open files) and comes back if the
    // new one cannot take its place; it is deleted only once `current` is written
    let aside = std::fs::symlink_metadata(&target)
      .is_ok()
      .then(|| dir.join(format!(".old-{}-{}", release.version, crate::util::random_hex(6))));
    if let Some(a) = &aside {
      std::fs::rename(&target, a).with_context(|| format!("move {} aside", target.display()))?;
    }
    let published = std::fs::rename(&pkg, &target)
      .with_context(|| format!("move into {}", target.display()))
      .and_then(|_| write_atomic_sync(&dir.join("current"), format!("{}\n", release.version).as_bytes()));
    if let Err(e) = published {
      if let Some(a) = &aside {
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::rename(a, &target);
      }
      return Err(e);
    }
    if let Some(a) = &aside {
      let _ = std::fs::remove_dir_all(a);
    }
    Ok(Installed::Fresh { version: release.version.to_owned(), path: target.join(cmd), sha256 })
  })();
  let _ = std::fs::remove_dir_all(&stage);
  let installed = result?;
  // A same-version reinstall replaced nothing: the version before it stays
  let replaced = previous.as_deref().filter(|p| *p != release.version);
  prune(&dir, release.version, replaced, previous.is_none() || replaced.is_some());
  Ok(installed)
}

fn platforms(release: &NativeRelease) -> String {
  release.assets.iter().map(|a| a.platform).collect::<Vec<_>>().join(", ")
}

/// https and one of the release's hosts, nothing else
fn check_url(release: &NativeRelease, url: &str) -> Result<()> {
  let rest = url.strip_prefix("https://").ok_or_else(|| anyhow!("refusing a non-https download: {url}"))?;
  let host = rest.split(['/', '?', '#']).next().unwrap_or("");
  if !release.hosts.contains(&host) {
    bail!("refusing a download from {host}: allowed {}", release.hosts.join(", "));
  }
  Ok(())
}

/// Keep the new version and the one it replaced; older versions (when `versions`) and leftovers of interrupted installs go
fn prune(dir: &Path, current: &str, previous: Option<&str>, versions: bool) {
  let Ok(entries) = std::fs::read_dir(dir) else { return };
  for e in entries.flatten() {
    let name = e.file_name().to_string_lossy().into_owned();
    let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
    if !is_dir || name == current || Some(name.as_str()) == previous {
      continue;
    }
    // Another installer's stage cannot be here: this installer holds the lock
    if name.starts_with(".stage-") || name.starts_with(".old-") || (versions && valid_version(&name)) {
      let _ = std::fs::remove_dir_all(e.path());
    }
  }
}

fn hex(digest: &[u8]) -> String {
  digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Copies `from` to `to` and returns the SHA-256 of the bytes written
fn copy_hashed(from: &Path, to: &Path) -> Result<String> {
  let mut src = std::fs::File::open(from).with_context(|| format!("open {}", from.display()))?;
  let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
  let mut hash = Sha256::new();
  let mut buf = vec![0u8; 1 << 16];
  loop {
    let n = src.read(&mut buf)?;
    if n == 0 {
      break;
    }
    hash.update(&buf[..n]);
    out.write_all(&buf[..n])?;
  }
  out.sync_all()?;
  Ok(hex(&hash.finalize()))
}

/// Streams the body to `to` while hashing it; returns the hex SHA-256
fn download(url: &str, to: &Path, say: &mut dyn FnMut(&str)) -> Result<String> {
  let agent: ureq::Agent = ureq::Agent::config_builder()
    .timeout_connect(Some(std::time::Duration::from_secs(30)))
    .timeout_recv_body(Some(std::time::Duration::from_secs(600)))
    .http_status_as_error(false)
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
  let mut shown = 0;
  loop {
    let n = reader.read(&mut buf).with_context(|| format!("download {url}"))?;
    if n == 0 {
      break;
    }
    file.write_all(&buf[..n])?;
    hash.update(&buf[..n]);
    got += n as u64;
    if let Some(t) = total.filter(|t| *t > 0) {
      let pct = (got * 100 / t) as usize;
      if pct >= shown + 10 {
        shown = pct - pct % 10;
        say(&format!("  {shown}% of {} MB", t / 1_000_000));
      }
    }
  }
  file.sync_all()?;
  if let Some(t) = total
    && t != got
  {
    bail!("download {url}: got {got} of {t} bytes");
  }
  Ok(hex(&hash.finalize()))
}

fn write_atomic_sync(path: &Path, data: &[u8]) -> Result<()> {
  let tmp = path.with_extension(format!("tmp-{}", crate::util::random_hex(6)));
  std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, path)).inspect_err(|_| {
    let _ = std::fs::remove_file(&tmp);
  })?;
  Ok(())
}

/// One installer per agent directory at a time, across processes: an OS lock on `install.lock` (flock / LockFileEx),
/// which the OS releases when the holder exits however it ends. The file itself stays: deleting it would let a second
/// installer lock a fresh inode while the first still holds the old one
struct InstallLock(#[allow(dead_code)] std::fs::File);

impl InstallLock {
  fn take(path: &Path) -> Result<InstallLock> {
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).with_context(|| format!("open {}", path.display()))?;
    match f.try_lock() {
      Ok(()) => Ok(InstallLock(f)),
      Err(std::fs::TryLockError::WouldBlock) => bail!("another install of this agent is running; try again when it finishes"),
      Err(std::fs::TryLockError::Error(e)) => Err(anyhow::Error::new(e).context(format!("lock {}", path.display()))),
    }
  }
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

/// The command line that runs the installer in a host terminal: this executable's own `install-agent`
pub fn install_command(agent: &str, windows: bool) -> Option<String> {
  let exe = std::env::current_exe().ok()?.to_string_lossy().into_owned();
  Some(if windows { format!("& '{}' install-agent {agent}", exe.replace('\'', "''")) } else { format!("'{}' install-agent {agent}", exe.replace('\'', "'\\''")) })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn platform_keys_follow_the_registry_names() {
    assert_eq!(platform_key("macos", "aarch64").as_deref(), Some("darwin-aarch64"));
    assert_eq!(platform_key("linux", "x86_64").as_deref(), Some("linux-x86_64"));
    assert_eq!(platform_key("windows", "aarch64").as_deref(), Some("windows-aarch64"));
    assert_eq!(platform_key("freebsd", "x86_64"), None);
    assert_eq!(platform_key("linux", "riscv64"), None);
  }

  #[test]
  fn every_platform_has_an_asset_and_its_own_launch_line() {
    for os in ["macos", "linux", "windows"] {
      for arch in ["aarch64", "x86_64"] {
        let key = platform_key(os, arch).unwrap();
        let asset = ANTIGRAVITY.asset(&key).unwrap_or_else(|| panic!("{key}"));
        assert_eq!(asset.sha256.len(), 64);
        check_url(&ANTIGRAVITY, asset.url).unwrap();
        let (cmd, args) = ANTIGRAVITY.launch(&key);
        match os {
          "windows" => assert_eq!((cmd, args.len()), ("agy_acp_server.exe", 0)),
          "linux" => assert_eq!((cmd, args), ("agy_acp_server.par", vec!["--uid=".to_owned()])),
          _ => assert_eq!((cmd, args.len()), ("agy_acp_server.par", 0)),
        }
      }
    }
  }

  #[test]
  fn downloads_stay_on_https_and_the_listed_hosts() {
    assert!(check_url(&ANTIGRAVITY, "http://dl.google.com/x.zip").is_err());
    assert!(check_url(&ANTIGRAVITY, "https://dl.google.com.evil.example/x.zip").is_err());
    assert!(check_url(&ANTIGRAVITY, "https://example.com/dl.google.com/x.zip").is_err());
    assert!(check_url(&ANTIGRAVITY, "https://dl.google.com/agy-extensions/x.zip").is_ok());
  }

  #[test]
  fn entry_paths_cannot_leave_the_destination() {
    for bad in ["/etc/passwd", "../x", "a/../../x", "C:/x", "a\\..\\..\\x", "", "./"] {
      assert!(safe_entry_path(bad).is_err(), "{bad}");
    }
    assert_eq!(safe_entry_path("./a/b.txt").unwrap(), PathBuf::from("a").join("b.txt"));
    assert_eq!(safe_entry_path("agy_acp_server.par").unwrap(), PathBuf::from("agy_acp_server.par"));
  }

  const EXEC: u32 = 0o100755;
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

  fn server_zip(version: &str) -> Vec<u8> {
    zip(&[
      ("agy_acp_server.par", format!("#!/bin/sh\necho {version}\n").as_bytes(), EXEC, true),
      ("localharness_external", b"harness", EXEC, false),
    ])
  }

  /// A release pinned to `bytes`' digest, sharing the antigravity layout
  fn release(version: &'static str, bytes: &[u8]) -> &'static NativeRelease {
    let sha256 = hex(&Sha256::digest(bytes));
    let assets = vec![NativeAsset { platform: "darwin-aarch64", url: "https://dl.google.com/x.zip", sha256: Box::leak(sha256.into_boxed_str()) }];
    Box::leak(Box::new(NativeRelease { version, assets: Box::leak(assets.into_boxed_slice()), ..ANTIGRAVITY }))
  }

  fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
  }

  fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
  }

  #[test]
  fn an_install_switches_current_and_keeps_only_the_version_it_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, downloads) = (tmp.path().join("home"), tmp.path());
    let mut quiet = |_: &str| {};
    for v in ["1.0.0", "1.1.0", "1.2.0"] {
      let bytes = server_zip(v);
      let r = release(v, &bytes);
      let file = put(downloads, &format!("{v}.zip"), &bytes);
      let Installed::Fresh { version, path, .. } = install(r, &root, "darwin-aarch64", false, Some(&file), &mut quiet).unwrap() else { panic!("{v}") };
      assert_eq!(version, v);
      assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("#!/bin/sh\necho {v}\n"));
      assert_eq!(r.installed(&root, "darwin-aarch64").unwrap().1, path);
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
      }
    }
    let dir = ANTIGRAVITY.dir(&root);
    assert_eq!(listing(&dir), ["1.1.0", "1.2.0", "current", "install.lock"]);
    assert_eq!(std::fs::read_to_string(dir.join("current")).unwrap().trim(), "1.2.0");
    // The same version again is a no-op unless forced
    let bytes = server_zip("1.2.0");
    let r = release("1.2.0", &bytes);
    let file = put(downloads, "again.zip", &bytes);
    assert!(matches!(install(r, &root, "darwin-aarch64", false, Some(&file), &mut quiet).unwrap(), Installed::Already { .. }));
    assert!(matches!(install(r, &root, "darwin-aarch64", true, Some(&file), &mut quiet).unwrap(), Installed::Fresh { .. }));
    assert_eq!(listing(&dir), ["1.1.0", "1.2.0", "current", "install.lock"]);
  }

  #[test]
  fn a_digest_mismatch_or_a_bad_archive_installs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("home");
    let mut quiet = |_: &str| {};
    let good = server_zip("1.0.0");
    let r = release("1.0.0", &good);
    let tampered = put(tmp.path(), "tampered.zip", &server_zip("6.6.6"));
    let e = install(r, &root, "darwin-aarch64", false, Some(&tampered), &mut quiet).err().unwrap();
    assert!(e.to_string().contains("SHA-256 mismatch"), "{e}");
    // Pinned to an archive without the launcher: the digest matches, the content does not
    let empty = zip(&[("README", b"hi", FILE, false)]);
    let r2 = release("1.0.0", &empty);
    let file = put(tmp.path(), "empty.zip", &empty);
    assert!(install(r2, &root, "darwin-aarch64", false, Some(&file), &mut quiet).err().unwrap().to_string().contains("no agy_acp_server.par"));
    // No current, no version directory, no stage left behind; the lock file stays, unlocked
    assert_eq!(listing(&ANTIGRAVITY.dir(&root)), ["install.lock"]);
    assert!(install(r, &root, "linux-riscv64", false, Some(&tampered), &mut quiet).is_err());
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

  /// Run by `an_installer_holds_the_lock_until_it_dies` in a child process: takes the lock and holds it
  #[test]
  #[ignore = "helper process of an_installer_holds_the_lock_until_it_dies"]
  fn lock_holder_process() {
    let Ok(path) = std::env::var("ACPIRA_TEST_LOCK") else { return };
    let _held = InstallLock::take(Path::new(&path)).unwrap();
    std::fs::write(format!("{path}.ready"), "").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
  }

  // Two installers are two processes: the second is turned away while the first runs, and a holder that is killed
  // releases the lock with its death (no stale file to clean up)
  #[cfg(unix)]
  #[test]
  fn an_installer_holds_the_lock_until_it_dies() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("install.lock");
    let mut holder = std::process::Command::new(std::env::current_exe().unwrap())
      .args(["--exact", "acp::agents::native_release::tests::lock_holder_process", "--ignored", "--nocapture"])
      .env("ACPIRA_TEST_LOCK", &path)
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn()
      .unwrap();
    let ready = tmp.path().join("install.lock.ready");
    let t0 = std::time::Instant::now();
    while !ready.exists() {
      assert!(t0.elapsed() < std::time::Duration::from_secs(20), "the holder never took the lock");
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(InstallLock::take(&path).err().unwrap().to_string().contains("another install"));
    holder.kill().unwrap();
    holder.wait().unwrap();
    let _taken = InstallLock::take(&path).unwrap();
    assert!(path.exists());
  }

  #[test]
  fn a_failed_same_version_reinstall_keeps_the_installed_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("home");
    let mut quiet = |_: &str| {};
    let bytes = server_zip("1.0.0");
    let r = release("1.0.0", &bytes);
    let file = put(tmp.path(), "a.zip", &bytes);
    install(r, &root, "darwin-aarch64", false, Some(&file), &mut quiet).unwrap();
    // `current` turned into a directory: the rename onto it fails after the new copy took the version's place
    let dir = ANTIGRAVITY.dir(&root);
    std::fs::remove_file(dir.join("current")).unwrap();
    std::fs::create_dir_all(dir.join("current/blocker")).unwrap();
    assert!(install(r, &root, "darwin-aarch64", true, Some(&file), &mut quiet).is_err());
    assert_eq!(std::fs::read_to_string(dir.join("1.0.0/agy_acp_server.par")).unwrap(), "#!/bin/sh\necho 1.0.0\n");
    assert!(!listing(&dir).iter().any(|n| n.starts_with(".old-") || n.starts_with(".stage-")), "{:?}", listing(&dir));
  }

  #[test]
  fn versions_are_plain_directory_names() {
    assert!(valid_version("1.2.1"));
    assert!(!valid_version(".."));
    assert!(!valid_version("1.2/../../x"));
    assert!(!valid_version(""));
  }
}
