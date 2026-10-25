//! Putting the helper on a host. One ssh command probes (platform, machine id, home, the installed copy's digest); when
//! the copy is missing or differs, a second one takes the gzipped binary on stdin, checks its digest on the host and
//! renames it into place. Each build has its own folder, `~/.acpira/remote/bin/<version>-<digest>/`, so two machines
//! deploying at once only race to rename identical bytes, and a host serving an older engine elsewhere keeps its copy.
//! The host never downloads anything: gen1 sits in a server room with no promise of reaching GitHub

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use tokio::io::AsyncWriteExt as _;

use super::ssh::{self, Host};

/// Marks the probe's lines in whatever the login shell printed around them
const PROBE: &str = "ACPIRA-PROBE ";
const DEPLOY: &str = "ACPIRA-DEPLOY ";
/// The digest prefix a build's folder is named with
const DIGEST_DIR: usize = 16;

/// A helper binary ready to ship: where it is, its digest, and the gzipped copy uploads stream
#[derive(Debug, Clone)]
pub struct Artifact {
  pub platform: String,
  pub path: PathBuf,
  pub digest: String,
  pub gz: PathBuf,
  pub gz_len: u64,
}

/// What the probe found on the host
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
  /// `linux-x86_64`, `linux-arm64`, `mac-arm64`, `mac-x86_64`, or the raw `uname -sm` when unknown
  pub platform: String,
  pub machine_id: String,
  pub home: String,
  /// The digest prefixes of this version's builds installed on the host (their folder names after the version)
  pub installed: Vec<String>,
}

impl Probe {
  /// Whether the build with `digest` is installed
  pub fn has(&self, digest: &str) -> bool {
    self.installed.iter().any(|d| digest.get(..DIGEST_DIR) == Some(d.as_str()))
  }
}

/// The folder, relative to the host's home, a build lives in
pub fn build_dir(version: &str, digest: &str) -> String {
  format!(".acpira/remote/bin/{version}-{}", &digest[..digest.len().min(DIGEST_DIR)])
}

/// The probe script (POSIX sh). It cannot know the digest to look for before it knows the platform, so it lists this
/// version's installed builds by folder; a build folder only ever holds a binary its upload verified
pub fn probe_script(version: &str) -> String {
  format!(
    r#"printf '{PROBE}platform=%s %s\n' "$(uname -s)" "$(uname -m)"
mid="${{ACPIRA_REMOTE_MACHINE_ID:-}}"
[ -n "$mid" ] || mid=$(cat /etc/machine-id 2>/dev/null || cat /var/lib/dbus/machine-id 2>/dev/null)
[ -n "$mid" ] || mid=$(/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice 2>/dev/null | sed -n 's/.*"IOPlatformUUID" = "\(.*\)"/\1/p')
printf '{PROBE}machine=%s\n' "$mid"
printf '{PROBE}home=%s\n' "$HOME"
for f in "$HOME"/.acpira/remote/bin/{version}-*/acpira; do
  [ -f "$f" ] && [ ! -L "$f" ] || continue
  d=${{f%/acpira}}; printf '{PROBE}installed=%s\n' "${{d##*/{version}-}}"
done
printf '{PROBE}end\n'
"#
  )
}

/// The upload script: stdin is the gzipped binary. Refuses a build folder that is a symlink or not the user's own
pub fn upload_script(version: &str, digest: &str) -> String {
  let dir = build_dir(version, digest);
  format!(
    r#"set -u
umask 077
D="$HOME/{dir}"
mkdir -p "$D" || {{ printf '{DEPLOY}error=cannot create %s\n' "$D"; exit 1; }}
if [ -L "$D" ] || [ ! -O "$D" ]; then printf '{DEPLOY}error=%s is not a folder of this user\n' "$D"; exit 1; fi
T="$D/.upload.$$"
trap 'rm -f "$T"' EXIT
gzip -dc > "$T" || {{ printf '{DEPLOY}error=the upload broke off\n'; exit 1; }}
if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$T"); else s=$(shasum -a 256 "$T"); fi
s="${{s%% *}}"
[ "$s" = "{digest}" ] || {{ printf '{DEPLOY}error=digest %s after the upload\n' "$s"; exit 1; }}
chmod 700 "$T" && mv -f "$T" "$D/acpira" || {{ printf '{DEPLOY}error=cannot put the binary in place\n'; exit 1; }}
trap - EXIT
printf '{DEPLOY}ok\n'
"#
  )
}

/// The command line that starts the installed helper
pub fn start_script(version: &str, digest: &str) -> String {
  format!(r#"exec "$HOME/{}/acpira" remote --stdio --digest {}"#, build_dir(version, digest), ssh::sh_quote(digest))
}

/// `uname -sm` in Acpira's platform names (the `dist/sidecar` folders)
pub fn platform_of(uname: &str) -> String {
  let mut parts = uname.split_whitespace();
  let os = parts.next().unwrap_or("");
  let arch = parts.next().unwrap_or("");
  let os = match os {
    "Linux" => "linux",
    "Darwin" => "mac",
    _ => return uname.trim().to_owned(),
  };
  let arch = match arch {
    "x86_64" | "amd64" => "x86_64",
    "aarch64" | "arm64" => "arm64",
    _ => return uname.trim().to_owned(),
  };
  format!("{os}-{arch}")
}

/// This machine's platform name
pub fn local_platform() -> String {
  let os = if cfg!(target_os = "macos") { "mac" } else if cfg!(target_os = "linux") { "linux" } else { std::env::consts::OS };
  let arch = match std::env::consts::ARCH {
    "aarch64" => "arm64",
    a => a,
  };
  format!("{os}-{arch}")
}

pub fn parse_probe(stdout: &str) -> Option<Probe> {
  let mut p = Probe::default();
  let mut ended = false;
  for line in stdout.lines() {
    let Some(rest) = line.trim_end().strip_prefix(PROBE) else { continue };
    match rest.split_once('=') {
      Some(("platform", v)) => p.platform = platform_of(v),
      Some(("machine", v)) => p.machine_id = v.trim().to_owned(),
      Some(("home", v)) => p.home = v.to_owned(),
      Some(("installed", v)) if !v.trim().is_empty() => p.installed.push(v.trim().to_owned()),
      _ if rest == "end" => ended = true,
      _ => {}
    }
  }
  ended.then_some(p)
}

/// Where this machine keeps the helper for `platform`: `ACPIRA_REMOTE_BIN` (any platform; tests), then
/// `ACPIRA_REMOTE_BIN_<PLATFORM>`, then `remote/<platform>/acpira` beside the engine (the packaged apps), then the
/// checkout's `dist/sidecar/<platform>/acpira`, and for this machine's own platform the engine binary itself
pub fn local_binary(platform: &str) -> Option<PathBuf> {
  if let Some(p) = std::env::var_os("ACPIRA_REMOTE_BIN") {
    return Some(PathBuf::from(p));
  }
  let key = format!("ACPIRA_REMOTE_BIN_{}", platform.to_ascii_uppercase().replace('-', "_"));
  if let Some(p) = std::env::var_os(key) {
    return Some(PathBuf::from(p));
  }
  let exe = std::env::current_exe().ok()?;
  let dir = exe.parent()?;
  let beside = dir.join("remote").join(platform).join("acpira");
  if beside.is_file() {
    return Some(beside);
  }
  // rust/target/<profile>/acpira → <checkout>/dist/sidecar
  if let Some(checkout) = dir.parent().and_then(Path::parent).and_then(Path::parent) {
    let dist = checkout.join("dist").join("sidecar").join(platform).join("acpira");
    if dist.is_file() {
      return Some(dist);
    }
  }
  (platform == local_platform()).then_some(exe)
}

/// Digest and gzip a binary, once per (path, size, mtime): the gzipped copy is cached under `cache`
pub async fn prepare(platform: &str, path: &Path, cache: &Path) -> Result<Artifact> {
  type Key = (PathBuf, u64, Option<std::time::SystemTime>);
  static MEMO: std::sync::LazyLock<parking_lot::Mutex<Vec<(Key, Artifact)>>> = std::sync::LazyLock::new(Default::default);
  let meta = tokio::fs::metadata(path).await.with_context(|| format!("no helper binary at {}", path.display()))?;
  let key: Key = (path.to_path_buf(), meta.len(), meta.modified().ok());
  if let Some((_, a)) = MEMO.lock().iter().find(|(k, _)| *k == key)
    && a.gz.exists()
  {
    return Ok(a.clone());
  }
  let (path2, cache2, platform2) = (path.to_path_buf(), cache.to_path_buf(), platform.to_owned());
  let artifact = tokio::task::spawn_blocking(move || -> Result<Artifact> {
    let bytes = std::fs::read(&path2)?;
    let digest = crate::util::sha256_hex(&bytes);
    std::fs::create_dir_all(&cache2)?;
    let gz = cache2.join(format!("{platform2}-{}.gz", &digest[..DIGEST_DIR]));
    if !gz.exists() {
      let tmp = cache2.join(format!(".{}.tmp", crate::util::random_hex(6)));
      let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&tmp)?, flate2::Compression::new(6));
      std::io::Write::write_all(&mut enc, &bytes)?;
      enc.finish()?.sync_all()?;
      std::fs::rename(&tmp, &gz)?;
    }
    let gz_len = std::fs::metadata(&gz)?.len();
    Ok(Artifact { platform: platform2, path: path2, digest, gz, gz_len })
  })
  .await??;
  let mut memo = MEMO.lock();
  memo.retain(|(k, _)| k.0 != key.0);
  memo.push((key, artifact.clone()));
  Ok(artifact)
}

/// ssh's failure in a form the connection manager can classify: the transport's stderr and our own words
#[derive(Debug)]
pub struct SshFailure {
  pub stderr: String,
  pub message: String,
  /// ssh itself could not be started: there is none on PATH
  pub no_ssh: bool,
}

impl std::fmt::Display for SshFailure {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.message)
  }
}

impl std::error::Error for SshFailure {}

/// Run the probe. Verbose, so a locked key can be told apart in the error
pub async fn probe(host: &Host, version: &str) -> Result<Probe, SshFailure> {
  let mut cmd = host.command(&probe_script(version), true);
  cmd.stdin(std::process::Stdio::null());
  let out = tokio::time::timeout(std::time::Duration::from_secs(45), cmd.output())
    .await
    .map_err(|_| SshFailure { stderr: String::new(), message: "the host did not answer within 45 s".into(), no_ssh: false })?
    .map_err(|e| SshFailure { stderr: e.to_string(), message: format!("cannot run ssh: {e}"), no_ssh: e.kind() == std::io::ErrorKind::NotFound })?;
  let stdout = String::from_utf8_lossy(&out.stdout);
  let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
  match parse_probe(&stdout) {
    Some(p) if out.status.success() => Ok(p),
    _ => {
      let (_, line) = ssh::classify(&stderr);
      let message = if line.is_empty() { format!("ssh ended with {}", out.status) } else { line };
      Err(SshFailure { stderr, message, no_ssh: false })
    }
  }
}

/// Upload `artifact`, reporting `(sent, total)` gzipped bytes as they go
pub async fn upload(host: &Host, version: &str, artifact: &Artifact, progress: Arc<dyn Fn(u64, u64) + Send + Sync>) -> Result<()> {
  let mut cmd = host.command(&upload_script(version, &artifact.digest), false);
  cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
  let mut child = cmd.spawn().context("cannot run ssh")?;
  let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
  let gz = artifact.gz.clone();
  let total = artifact.gz_len;
  let feed = tokio::spawn(async move {
    use tokio::io::AsyncReadExt as _;
    let mut file = tokio::fs::File::open(&gz).await?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut sent = 0u64;
    loop {
      let n = file.read(&mut buf).await?;
      if n == 0 {
        break;
      }
      stdin.write_all(&buf[..n]).await?;
      sent += n as u64;
      progress(sent, total);
    }
    stdin.shutdown().await?;
    anyhow::Ok(())
  });
  let out = child.wait_with_output().await.context("ssh failed")?;
  let fed = feed.await.map_err(|e| anyhow!("{e}"))?;
  let stdout = String::from_utf8_lossy(&out.stdout);
  if let Some(line) = stdout.lines().rev().find_map(|l| l.strip_prefix(DEPLOY)) {
    if line == "ok" {
      return Ok(());
    }
    if let Some(err) = line.strip_prefix("error=") {
      return Err(anyhow!("deploying to the host failed: {err}"));
    }
  }
  let stderr = String::from_utf8_lossy(&out.stderr);
  let (_, shown) = ssh::classify(&stderr);
  fed.and(Err(anyhow!(if shown.is_empty() { format!("the upload ended with {}", out.status) } else { shown })))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn platforms_and_probe_output() {
    assert_eq!(platform_of("Linux x86_64"), "linux-x86_64");
    assert_eq!(platform_of("Linux aarch64"), "linux-arm64");
    assert_eq!(platform_of("Darwin arm64"), "mac-arm64");
    assert_eq!(platform_of("FreeBSD amd64"), "FreeBSD amd64");
    let out = "Last login: x\nACPIRA-PROBE platform=Linux x86_64\nACPIRA-PROBE machine=237b\nACPIRA-PROBE home=/home/hotic\nACPIRA-PROBE installed=abc\nACPIRA-PROBE installed=def\nACPIRA-PROBE end\n";
    assert_eq!(
      parse_probe(out),
      Some(Probe { platform: "linux-x86_64".into(), machine_id: "237b".into(), home: "/home/hotic".into(), installed: vec!["abc".into(), "def".into()] })
    );
    // Cut short (the connection dropped mid-way): no probe
    assert_eq!(parse_probe("ACPIRA-PROBE platform=Linux x86_64\n"), None);
    assert_eq!(build_dir("1.10.8", "0123456789abcdef0123"), ".acpira/remote/bin/1.10.8-0123456789abcdef");
  }

  // The scripts run on the SSH host; Windows has no /bin/sh to parse them with
  #[cfg(unix)]
  #[test]
  fn the_scripts_run_under_sh() {
    // Syntax only: `sh -n` parses without running
    for script in [probe_script("1.0.0"), upload_script("1.0.0", "ab"), start_script("1.0.0", "ab")] {
      let out = std::process::Command::new("/bin/sh").arg("-n").arg("-c").arg(&script).output().unwrap();
      assert!(out.status.success(), "{script}\n{}", String::from_utf8_lossy(&out.stderr));
    }
  }
}
