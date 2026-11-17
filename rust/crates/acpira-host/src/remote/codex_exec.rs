//! Codex under local run in a folder on an SSH host (docs/dev/agent-quirks-codex.md). The CLI runs here, on this
//! machine's login, and its own shell and file tools run on the host in `codex exec-server` there:
//! `CODEX_EXEC_SERVER_URL` makes that server the thread's only environment, so nothing of the turn touches this
//! machine's disk.
//!
//! Three parts. The host gets the same codex build this machine's adapter runs, under `~/.acpira/codex/<version>/`
//! (the host never downloads anything: the platform's build comes from this machine's install or its npm, then over
//! ssh). Each session's server is started through the host's helper, listening on the host's loopback behind a
//! capability token. And a loopback port here carries each connection to it over the helper's `tcp` op: the URL the
//! CLI gets names that port with a secret path, and the token is added to the WebSocket upgrade on the way, so neither
//! a process of another user here nor one on the host can use the server

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::client::{Call, ClientToken, Event, RemoteClient};
use super::connections::Connections;
use super::ssh;
use crate::acp::agents::adapter_info;
use crate::acp::agents::registry::{AgentDef, NativeLayout};

/// The variable that names the server to the CLI (codex 0.166.0: any value but `none` replaces the local environment)
pub const ENV: &str = "CODEX_EXEC_SERVER_URL";
/// Where the builds live on a host, relative to its home
const HOST_DIR: &str = ".acpira/codex";
const DEPLOY: &str = "ACPIRA-CODEX ";
const CONNECT_WAIT: Duration = Duration::from_secs(60);
/// How long the server may take to print the address it listens on
const START_WAIT: Duration = Duration::from_secs(30);
/// How long npm may take to download a platform's build (about 170 MB)
const FETCH_WAIT: Duration = Duration::from_secs(30 * 60);
/// How long the server has to end the commands it runs when the helper stops it (codex 0.166.0: on SIGTERM it ends
/// them and exits within half a second; SIGKILL leaves them running, each being a session of its own)
const STOP_GRACE: Duration = Duration::from_millis(1500);
/// The upgrade request's head is a few hundred bytes
const MAX_HEAD: usize = 16 * 1024;

/// The codex build this machine's adapter runs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCodex {
  pub version: String,
  /// The installed `@openai/codex` package; None when `CODEX_PATH` names a binary of its own
  pub package: Option<PathBuf>,
}

/// The adapter's variable for a codex binary of the user's own
const OVERRIDE_ENV: &str = "CODEX_PATH";

/// The build the adapter at `binary` runs: the binary `CODEX_PATH` names (in the agent's own env or the engine's),
/// else the engine package beside the adapter
pub async fn local_codex(binary: &str, def: &AgentDef) -> Result<LocalCodex> {
  let over = def.env.as_ref().and_then(|e| e.get(OVERRIDE_ENV).cloned()).or_else(|| std::env::var(OVERRIDE_ENV).ok()).filter(|v| !v.is_empty());
  if let Some(over) = over {
    let mut cmd = crate::acp::agents::launch::command(&over, &["--version".to_owned()]);
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(15), cmd.output()).await.map_err(|_| anyhow!("{over} --version did not answer"))??;
    let version = parse_version(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| anyhow!("{over} --version gave no version"))?;
    return Ok(LocalCodex { version, package: None });
  }
  let version = adapter_info::read_adapter_info(binary, def)
    .await
    .and_then(|i| i.engine)
    .and_then(|e| e.version)
    .ok_or_else(|| anyhow!("the codex this machine's {} runs could not be found", def.name))?;
  Ok(LocalCodex { version, package: adapter_info::engine_dir(binary, def).await })
}

/// `codex-cli 0.166.0` → `0.166.0`
fn parse_version(text: &str) -> Option<String> {
  text.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.')).map(str::to_owned)
}

/// A host's platform in node's names, as the codex packages are named (`linux` / `darwin`, `x64` / `arm64`)
fn node_target(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
  let platform = match os {
    "linux" => "linux",
    "macos" => "darwin",
    _ => return None,
  };
  let arch = match arch {
    "x86_64" => "x64",
    "aarch64" => "arm64",
    _ => return None,
  };
  Some((platform, arch))
}

/// The files of a build's `vendor/<triple>/` folder the server needs: the binary, its package description (which names
/// the two folders beside it), the bundled ripgrep and the Linux sandbox helper. The rest (voice, code mode) is the
/// local CLI's
fn wanted(rel: &str) -> bool {
  matches!(rel, "bin/codex" | "codex-package.json" | "codex-resources/bwrap") || rel.strip_prefix("codex-path/").is_some_and(|f| !f.is_empty() && !f.contains('/'))
}

/// A build packed for upload: the gzipped tar and the binary's sha256, which the host checks after unpacking
#[derive(Debug, Clone)]
struct Artifact {
  tgz: PathBuf,
  len: u64,
  digest: String,
}

struct Hashing<R> {
  inner: R,
  hash: sha2::Sha256,
}

impl<R: Read> Read for Hashing<R> {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    use sha2::Digest as _;
    let n = self.inner.read(buf)?;
    self.hash.update(&buf[..n]);
    Ok(n)
  }
}

fn hex(bytes: &[u8]) -> String {
  bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Packs the wanted files of `entries` (path relative to the vendor folder, mode, size, content) into `to`; the
/// binary's digest, or an error when there was none
fn pack<'a>(to: &Path, mut each: impl FnMut(&mut dyn FnMut(&str, u32, u64, &mut dyn Read) -> Result<()>) -> Result<()> + 'a) -> Result<String> {
  use sha2::Digest as _;
  let file = std::fs::File::create(to)?;
  // The binary is 320 MB; the fastest level already halves it and the link is rarely the slow part
  let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(file, flate2::Compression::fast()));
  let mut digest = None;
  each(&mut |rel, mode, size, content| {
    if !wanted(rel) {
      return Ok(());
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(if mode & 0o111 != 0 { 0o755 } else { 0o644 });
    header.set_mtime(0);
    let mut hashing = Hashing { inner: content, hash: sha2::Sha256::new() };
    tar.append_data(&mut header, rel, &mut hashing)?;
    if rel == "bin/codex" {
      digest = Some(hex(&hashing.hash.finalize()));
    }
    Ok(())
  })?;
  tar.into_inner()?.finish()?.sync_all()?;
  digest.ok_or_else(|| anyhow!("the build holds no bin/codex"))
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
  use std::os::unix::fs::PermissionsExt;
  meta.permissions().mode()
}

#[cfg(not(unix))]
fn mode_of(_meta: &std::fs::Metadata) -> u32 {
  0
}

/// Packs a build from an installed package's `vendor/<triple>/` folder
fn pack_dir(vendor: &Path, to: &Path) -> Result<String> {
  pack(to, |add| {
    let mut files = vec!["bin/codex".to_owned(), "codex-package.json".to_owned(), "codex-resources/bwrap".to_owned()];
    if let Ok(dir) = std::fs::read_dir(vendor.join("codex-path")) {
      files.extend(dir.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| format!("codex-path/{}", e.file_name().to_string_lossy())));
    }
    for rel in files {
      let path = vendor.join(&rel);
      let Ok(meta) = std::fs::metadata(&path) else { continue };
      // The binary keeps its mode even where this machine has none to read (Windows)
      let mode = if rel == "codex-package.json" { 0o644 } else { mode_of(&meta) | 0o111 };
      add(&rel, mode, meta.len(), &mut std::fs::File::open(&path)?)?;
    }
    Ok(())
  })
}

/// Packs a build from the platform package's npm tarball (`package/vendor/<triple>/…`)
fn pack_npm(tarball: &Path, to: &Path) -> Result<String> {
  pack(to, |add| {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(tarball)?));
    for entry in archive.entries()? {
      let mut entry = entry?;
      if !entry.header().entry_type().is_file() {
        continue;
      }
      let path = entry.path()?.to_string_lossy().replace('\\', "/");
      // package/vendor/<triple>/<rel>
      let mut parts = path.splitn(4, '/');
      let (Some("package"), Some("vendor"), Some(_), Some(rel)) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
      let (rel, mode, size) = (rel.to_owned(), entry.header().mode().unwrap_or(0o644), entry.size());
      add(&rel, mode, size, &mut entry)?;
    }
    Ok(())
  })
}

/// The installed build for `(platform, arch)`, when this machine has it: the engine package's own `vendor/`, or the
/// platform package on its resolution chain
async fn installed_vendor(package: &Path, platform: &str, arch: &str) -> Option<PathBuf> {
  let c = adapter_info::native_candidates(NativeLayout::CodexVendor, platform, arch)?;
  // `…/bin/codex` → the vendor folder two levels up
  let vendor_of = |binary: PathBuf| binary.parent().and_then(Path::parent).map(Path::to_path_buf);
  let is_file = |p: PathBuf| async move { tokio::fs::metadata(&p).await.is_ok_and(|m| m.is_file()).then_some(p) };
  if let Some(rel) = &c.in_engine
    && let Some(hit) = is_file(package.join(rel)).await
  {
    return vendor_of(hit);
  }
  let mut dir = package.to_path_buf();
  for _ in 0..12 {
    for rel in &c.in_node_modules {
      if let Some(hit) = is_file(dir.join("node_modules").join(rel)).await {
        return vendor_of(hit);
      }
    }
    let up = dir.parent()?.to_path_buf();
    if up == dir {
      return None;
    }
    dir = up;
  }
  None
}

/// Downloads the platform package with this machine's npm (its mirror and proxy; npm checks the registry's integrity)
async fn npm_pack(version: &str, platform: &str, arch: &str, into: &Path) -> Result<PathBuf> {
  use crate::acp::agents::{launch, registry};
  let os = crate::platform::command::Os::current();
  let npm = registry::resolve_command("npm", &[], os, &launch::ProcessEnv).await.ok_or_else(|| anyhow!("npm is needed to fetch codex {version} for {platform}-{arch}, and none was found"))?;
  let spec = format!("@openai/codex@{version}-{platform}-{arch}");
  let mut cmd = launch::command(&npm, &["pack".into(), spec.clone(), "--pack-destination".into(), into.to_string_lossy().into_owned()]);
  cmd.env("npm_config_update_notifier", "false").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
  let out = tokio::time::timeout(FETCH_WAIT, cmd.output()).await.map_err(|_| anyhow!("npm pack {spec} did not finish"))?.context("cannot run npm")?;
  if !out.status.success() {
    let err = String::from_utf8_lossy(&out.stderr);
    bail!("npm pack {spec} failed: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim());
  }
  let mut found = tokio::fs::read_dir(into).await?;
  while let Some(e) = found.next_entry().await? {
    if e.file_name().to_string_lossy().ends_with(".tgz") {
      return Ok(e.path());
    }
  }
  bail!("npm pack {spec} left no tarball")
}

/// The build for a host's platform, packed once per version under `cache`
async fn artifact(cache: &Path, codex: &LocalCodex, platform: &str, arch: &str, log: &(dyn Fn(&str) + Send + Sync)) -> Result<Artifact> {
  let tgz = cache.join(format!("codex-{}-{platform}-{arch}.tgz", codex.version));
  let sum = cache.join(format!("codex-{}-{platform}-{arch}.sha256", codex.version));
  let cached = |tgz: &Path, sum: &Path| {
    let digest = std::fs::read_to_string(sum).ok().map(|s| s.trim().to_owned()).filter(|d| d.len() == 64)?;
    Some(Artifact { tgz: tgz.to_path_buf(), len: std::fs::metadata(tgz).ok()?.len(), digest })
  };
  if let Some(a) = cached(&tgz, &sum) {
    return Ok(a);
  }
  tokio::fs::create_dir_all(cache).await?;
  let tmp = cache.join(format!(".codex-{}.tmp", crate::util::random_hex(6)));
  let vendor = match &codex.package {
    Some(p) => installed_vendor(p, platform, arch).await,
    None => None,
  };
  let packed = match vendor {
    Some(vendor) => {
      let tmp = tmp.clone();
      tokio::task::spawn_blocking(move || pack_dir(&vendor, &tmp)).await?
    }
    None => {
      log(&format!("remote: fetching codex {} for {platform}-{arch} with npm", codex.version));
      let dir = cache.join(format!(".npm-{}", crate::util::random_hex(6)));
      tokio::fs::create_dir_all(&dir).await?;
      let fetched = npm_pack(&codex.version, platform, arch, &dir).await;
      let packed = match fetched {
        Ok(tarball) => {
          let tmp = tmp.clone();
          tokio::task::spawn_blocking(move || pack_npm(&tarball, &tmp)).await?
        }
        Err(e) => Err(e),
      };
      let _ = tokio::fs::remove_dir_all(&dir).await;
      packed
    }
  };
  let digest = match packed {
    Ok(d) => d,
    Err(e) => {
      let _ = tokio::fs::remove_file(&tmp).await;
      return Err(e);
    }
  };
  // The digest lands first: a tarball without one is packed again
  tokio::fs::write(&sum, &digest).await?;
  tokio::fs::rename(&tmp, &tgz).await?;
  cached(&tgz, &sum).ok_or_else(|| anyhow!("the packed build could not be read back"))
}

/// The folder a version lives in on a host whose home is `home`
fn host_dir(home: &str, version: &str) -> String {
  format!("{}/{HOST_DIR}/{version}", home.trim_end_matches('/'))
}

/// A version as a folder name: digits, letters, dots and dashes only, so it can stand in a script unquoted
fn safe_version(version: &str) -> bool {
  !version.is_empty() && !version.starts_with('.') && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The upload script (POSIX sh): stdin is the gzipped tar of `len` bytes. It is kept whole first and checked (its
/// length, then gzip's own checksum), since a pipe into tar hides a stream that broke off after the last file tar
/// wanted. Then the build is unpacked beside its place, its binary checked, and renamed in unless another engine's
/// upload landed meanwhile; other versions no process runs are removed, since each is a third of a gigabyte (one
/// whose server is down is put back before that server's next start)
fn upload_script(version: &str, digest: &str, len: u64) -> String {
  format!(
    r#"set -u
umask 077
B="$HOME/{HOST_DIR}"
mkdir -p "$B" || {{ printf '{DEPLOY}error=cannot create %s\n' "$B"; exit 1; }}
if [ -L "$B" ] || [ ! -O "$B" ]; then printf '{DEPLOY}error=%s is not a folder of this user\n' "$B"; exit 1; fi
D="$B/{version}"
T="$B/.upload.$$"
rm -rf "$T"
mkdir "$T" || {{ printf '{DEPLOY}error=cannot create %s\n' "$T"; exit 1; }}
trap 'rm -rf "$T" "$T.tgz"' EXIT
cat > "$T.tgz" || {{ printf '{DEPLOY}error=cannot keep the upload\n'; exit 1; }}
n=$(wc -c < "$T.tgz")
[ "$((n))" = "{len}" ] && gzip -t "$T.tgz" 2>/dev/null || {{ printf '{DEPLOY}error=the upload broke off\n'; exit 1; }}
gzip -dc "$T.tgz" | tar -xf - -C "$T" || {{ printf '{DEPLOY}error=the upload cannot be unpacked\n'; exit 1; }}
rm -f "$T.tgz"
if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$T/bin/codex"); else s=$(shasum -a 256 "$T/bin/codex"); fi
s="${{s%% *}}"
[ "$s" = "{digest}" ] || {{ printf '{DEPLOY}error=digest %s after the upload\n' "$s"; exit 1; }}
: > "$T/.ok"
if [ ! -f "$D/.ok" ]; then
  rm -rf "$D"
  mv "$T" "$D" || {{ printf '{DEPLOY}error=cannot put the build in place\n'; exit 1; }}
fi
for o in "$B"/*; do
  [ -d "$o" ] && [ "$o" != "$D" ] || continue
  pgrep -f "$o/bin/codex" >/dev/null 2>&1 || rm -rf "$o"
done
printf '{DEPLOY}ok\n'
"#
  )
}

/// Uploads the build, reporting gzipped bytes sent
async fn upload(host: &ssh::Host, version: &str, artifact: &Artifact, progress: impl Fn(u64, u64) + Send + 'static) -> Result<()> {
  let mut cmd = host.command(&upload_script(version, &artifact.digest, artifact.len), false);
  cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
  let mut child = cmd.spawn().context("cannot run ssh")?;
  let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
  let (tgz, total) = (artifact.tgz.clone(), artifact.len);
  let feed = tokio::spawn(async move {
    let mut file = tokio::fs::File::open(&tgz).await?;
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
      bail!("putting codex on the host failed: {err}");
    }
  }
  let stderr = String::from_utf8_lossy(&out.stderr);
  let (_, shown) = ssh::classify(&stderr);
  fed.and(Err(anyhow!(if shown.is_empty() { format!("the upload ended with {}", out.status) } else { shown })))
}

/// The upgrade request as the host's server takes it: the secret path replaced, the capability token added, any
/// `Authorization` the client sent dropped. None when the head is not a GET of `/<secret>`
fn rewrite_head(head: &str, secret: &str, token: &str) -> Option<String> {
  let mut lines = head.split("\r\n");
  let mut request = lines.next()?.splitn(3, ' ');
  let (method, target, version) = (request.next()?, request.next()?, request.next()?);
  if method != "GET" || target.strip_prefix('/') != Some(secret) {
    return None;
  }
  let mut out = format!("GET / {version}\r\nAuthorization: Bearer {token}\r\n");
  for line in lines.filter(|l| !l.is_empty()) {
    if line.split(':').next().is_some_and(|k| k.trim().eq_ignore_ascii_case("authorization")) {
      continue;
    }
    out.push_str(line);
    out.push_str("\r\n");
  }
  out.push_str("\r\n");
  Some(out)
}

/// The port in the line the server prints once it listens (`ws://127.0.0.1:<port>`)
fn listening_port(line: &str) -> Option<u16> {
  line.trim().strip_prefix("ws://127.0.0.1:")?.trim_end_matches('/').parse().ok()
}

/// A server running on a host: the helper channel it was started over, where it listens there, its token, and the
/// task holding its request. Dropped, the task goes and the request with it, which has the helper stop the server
struct Server {
  on: ClientToken,
  port: u16,
  token: String,
  life: tokio::task::AbortHandle,
}

impl Drop for Server {
  fn drop(&mut self) {
    self.life.abort();
  }
}

/// One session's way to its own server on a host: the loopback port here and the secret its URL carries. The server
/// is the session's alone because ending it is the only way to end the commands a stopped turn left there
struct Tunnel {
  alias: String,
  codex: LocalCodex,
  port: u16,
  secret: String,
  /// One start at a time
  starting: tokio::sync::Mutex<()>,
  server: parking_lot::Mutex<Option<Server>>,
  /// Set, under `server`'s lock, when the session let go of the tunnel: a start still under way keeps no server
  closed: std::sync::atomic::AtomicBool,
  connections: Weak<Connections>,
}

impl Tunnel {
  fn url(&self) -> String {
    format!("ws://127.0.0.1:{}/{}", self.port, self.secret)
  }

  fn running(&self, client: &RemoteClient) -> Option<(u16, String)> {
    self.server.lock().as_ref().filter(|s| s.on.is(client)).map(|s| (s.port, s.token.clone()))
  }

  /// The server on the host, started when none runs on the channel that is up now (a new channel means a new helper:
  /// the old one took the server it started down with it)
  async fn server(self: &Arc<Self>, client: &RemoteClient) -> Result<(u16, String)> {
    use std::sync::atomic::Ordering;
    let _one = self.starting.lock().await;
    if let Some(s) = self.running(client) {
      return Ok(s);
    }
    let connections = self.connections.upgrade().filter(|_| !self.closed.load(Ordering::Relaxed)).ok_or_else(|| anyhow!("the session's tunnel is closed"))?;
    // Before every start, not only the session's first: another machine's deploy removes a version no process runs,
    // which is this one while its server is down
    connections.codex.deploy(&connections, &self.alias, client, &self.codex).await?;
    let token = crate::util::random_hex(32);
    let binary = format!("{}/bin/codex", host_dir(&client.hello().home, &self.codex.version));
    let argv = [binary.as_str(), "exec-server", "--listen", "ws://127.0.0.1:0", "--ws-auth", "capability-token", "--ws-token-sha256", &crate::util::sha256_hex(token.as_bytes())];
    let mut call = client.request("exec", json!({ "argv": argv, "graceMs": STOP_GRACE.as_millis() as u64 }));
    let port = tokio::time::timeout(START_WAIT, first_port(&mut call)).await.map_err(|_| anyhow!("codex exec-server on {} did not say where it listens", self.alias))??;
    // Registered under the lock the watcher takes to clear it, so a server that ends at once is never left in the slot
    let mut slot = self.server.lock();
    if self.closed.load(Ordering::Relaxed) {
      // Dropping the request stops the server
      bail!("the session's tunnel is closed");
    }
    // The request is the server's life: this task keeps it until the server ends
    let me = Arc::downgrade(self);
    let (alias, mine) = (self.alias.clone(), token.clone());
    let life = tokio::spawn(async move {
      let mut tail = String::new();
      while let Some(ev) = call.next().await {
        match ev {
          Event::Chunk { stream, bytes } if stream == "err" => {
            tail.push_str(&String::from_utf8_lossy(&bytes));
            if tail.len() > 4096 {
              let mut cut = tail.len() - 2048;
              while !tail.is_char_boundary(cut) {
                cut += 1;
              }
              tail.drain(..cut);
            }
          }
          Event::Done(_) => break,
          _ => {}
        }
      }
      let Some(me) = me.upgrade() else { return };
      let ended = {
        let mut slot = me.server.lock();
        if slot.as_ref().is_some_and(|s| s.token == mine) { slot.take() } else { None }
      };
      if ended.is_some()
        && let Some(c) = me.connections.upgrade()
      {
        let last = tail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
        c.log_line(&format!("remote: codex exec-server on {alias} ended{}", if last.is_empty() { String::new() } else { format!(": {last}") }));
      }
    });
    *slot = Some(Server { on: client.token(), port, token: token.clone(), life: life.abort_handle() });
    Ok((port, token))
  }

  /// One connection of the CLI: its upgrade checked and rewritten, then bytes both ways until either side ends
  async fn serve(self: Arc<Self>, mut sock: tokio::net::TcpStream) -> Result<()> {
    let mut buf = Vec::with_capacity(1024);
    let end = loop {
      if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        break i;
      }
      if buf.len() > MAX_HEAD {
        bail!("an upgrade request over {MAX_HEAD} bytes");
      }
      let mut chunk = [0u8; 2048];
      let n = tokio::time::timeout(Duration::from_secs(10), sock.read(&mut chunk)).await.map_err(|_| anyhow!("no upgrade request came"))??;
      if n == 0 {
        bail!("closed before the upgrade request");
      }
      buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    // Checked before anything is asked of the host: a stranger's connection starts nothing there
    if rewrite_head(&head, &self.secret, "").is_none() {
      let _ = sock.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
      bail!("not this tunnel's URL");
    }
    let connections = self.connections.upgrade().ok_or_else(|| anyhow!("the engine is shutting down"))?;
    let up = async {
      let client = connections.ensure(&self.alias, CONNECT_WAIT).await?;
      let (port, token) = self.server(&client).await?;
      let head = rewrite_head(&head, &self.secret, &token).ok_or_else(|| anyhow!("not this tunnel's URL"))?;
      let mut up = super::tcp::connect(&client, "127.0.0.1", port).await.map_err(|e| anyhow!("{e}"))?;
      up.write_all(head.as_bytes()).await?;
      up.write_all(&buf[end + 4..]).await?;
      anyhow::Ok(up)
    };
    let mut up = match up.await {
      Ok(up) => up,
      Err(e) => {
        let _ = sock.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return Err(e);
      }
    };
    let _ = tokio::io::copy_bidirectional(&mut sock, &mut up).await;
    Ok(())
  }
}

/// Reads the server's output up to the line that names its port; its stderr explains an early end
async fn first_port(call: &mut Call) -> Result<u16> {
  let (mut out, mut err) = (String::new(), String::new());
  loop {
    match call.next().await {
      Some(Event::Chunk { stream, bytes }) if stream == "out" => {
        out.push_str(&String::from_utf8_lossy(&bytes));
        if let Some(port) = out.lines().find_map(listening_port) {
          return Ok(port);
        }
      }
      Some(Event::Chunk { stream, bytes }) if stream == "err" => err.push_str(&String::from_utf8_lossy(&bytes)),
      Some(Event::Chunk { .. }) => {}
      Some(Event::Done(r)) => {
        let why = match r {
          Err(e) => e.to_string(),
          Ok(_) => err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it ended at once").trim().to_owned(),
        };
        bail!("codex exec-server could not start on the host: {why}");
      }
      None => bail!("the connection to the host ended while codex exec-server was starting"),
    }
  }
}

/// A session's tunnel. Dropped, its port closes and its server stops
pub struct ExecTunnel {
  tunnel: Arc<Tunnel>,
  accept: tokio::task::AbortHandle,
}

impl ExecTunnel {
  /// What `ENV` is set to for the session's CLI
  pub fn url(&self) -> String {
    self.tunnel.url()
  }

  /// Ends the commands the server still runs, by ending the server: codex 0.166.0 sends it no `process/terminate` when
  /// a turn is cancelled (only when the CLI exits), so a stopped turn's command would run on. The CLI reconnects by
  /// itself, and that connection starts the next server
  pub fn stop_commands(&self) {
    self.tunnel.server.lock().take();
  }
}

impl Drop for ExecTunnel {
  fn drop(&mut self) {
    self.accept.abort();
    // Under the slot's lock, so a start that is under way either sees this or has its server taken here
    let mut slot = self.tunnel.server.lock();
    self.tunnel.closed.store(true, std::sync::atomic::Ordering::Relaxed);
    slot.take();
  }
}

/// Opens the sessions' tunnels
#[derive(Default)]
pub struct Tunnels {
  /// One deploy at a time: two sessions opening together upload once
  deploying: tokio::sync::Mutex<()>,
}

impl Tunnels {
  /// A tunnel from this machine to a `codex exec-server` of its own on `alias`, for the build `codex`: a port opened
  /// here, the build put on the host when it is missing and the server started
  pub async fn open(&self, connections: &Arc<Connections>, alias: &str, codex: &LocalCodex) -> Result<ExecTunnel> {
    if !safe_version(&codex.version) {
      bail!("codex version {:?} cannot name a folder", codex.version);
    }
    let client = connections.ensure(alias, CONNECT_WAIT).await.map_err(|e| anyhow!("{alias} cannot be reached: {e:#}"))?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.context("cannot open a loopback port for the codex tunnel")?;
    let tunnel = Arc::new(Tunnel {
      alias: alias.to_owned(),
      codex: codex.clone(),
      port: listener.local_addr()?.port(),
      secret: crate::util::random_hex(16),
      starting: Default::default(),
      server: Default::default(),
      closed: Default::default(),
      connections: Arc::downgrade(connections),
    });
    let weak = Arc::downgrade(&tunnel);
    let accept = tokio::spawn(async move {
      while let Ok((sock, _)) = listener.accept().await {
        let Some(t) = weak.upgrade() else { return };
        tokio::spawn(async move {
          let (alias, connections) = (t.alias.clone(), t.connections.clone());
          if let Err(e) = t.serve(sock).await
            && let Some(c) = connections.upgrade()
          {
            c.log_line(&format!("remote: a codex connection to {alias} failed: {e:#}"));
          }
        });
      }
    })
    .abort_handle();
    let open = ExecTunnel { tunnel, accept };
    // Started now (the build put on the host first when it is missing), so a server that cannot run fails the
    // session's start with its reason instead of every tool call
    open.tunnel.server(&client).await?;
    Ok(open)
  }

  /// Puts the build on the host unless it is there (`.ok` is written last, so a broken upload never counts)
  async fn deploy(&self, connections: &Arc<Connections>, alias: &str, client: &RemoteClient, codex: &LocalCodex) -> Result<()> {
    let _one = self.deploying.lock().await;
    let hello = client.hello();
    let marker = format!("{}/.ok", host_dir(&hello.home, &codex.version));
    if client.call("stat", json!({ "path": marker })).await.is_ok() {
      return Ok(());
    }
    let (platform, arch) = node_target(&hello.os, &hello.arch).ok_or_else(|| anyhow!("codex has no build for {alias} ({} {})", hello.os, hello.arch))?;
    let log = connections.clone();
    let artifact = artifact(&connections.cache_dir().join("codex"), codex, platform, arch, &move |line| log.log_line(line)).await?;
    connections.log_line(&format!("remote: putting codex {} on {alias} ({} MB)", codex.version, artifact.len / (1024 * 1024)));
    let (log, to) = (connections.clone(), alias.to_owned());
    let step = std::sync::atomic::AtomicU64::new(0);
    upload(&connections.host(alias), &codex.version, &artifact, move |sent, total| {
      // A line per quarter
      let quarter = (sent * 4).checked_div(total).unwrap_or(4);
      if step.swap(quarter, std::sync::atomic::Ordering::Relaxed) != quarter && quarter > 0 {
        log.log_line(&format!("remote: codex to {to}: {}%", quarter * 25));
      }
    })
    .await?;
    connections.log_line(&format!("remote: codex {} is on {alias}", codex.version));
    Ok(())
  }
}

/// A value for a tool row or a log: never the secret path
pub fn redact(url: &str) -> Value {
  json!(url.rsplit_once('/').map(|(base, _)| format!("{base}/…")).unwrap_or_default())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_upgrade_is_taken_only_at_the_secret_path_and_gets_the_token() {
    let head = "GET /s3cret HTTP/1.1\r\nHost: 127.0.0.1:4000\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nauthorization: Bearer mine\r\nSec-WebSocket-Key: abc==";
    let out = rewrite_head(head, "s3cret", "tok").unwrap();
    assert_eq!(out, "GET / HTTP/1.1\r\nAuthorization: Bearer tok\r\nHost: 127.0.0.1:4000\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: abc==\r\n\r\n");
    assert_eq!(rewrite_head("GET / HTTP/1.1\r\nHost: x", "s3cret", "tok"), None);
    assert_eq!(rewrite_head("GET /s3cret/more HTTP/1.1\r\nHost: x", "s3cret", "tok"), None);
    assert_eq!(rewrite_head("POST /s3cret HTTP/1.1\r\nHost: x", "s3cret", "tok"), None);
    assert_eq!(redact("ws://127.0.0.1:4000/s3cret"), json!("ws://127.0.0.1:4000/…"));
  }

  #[test]
  fn names_and_lines() {
    assert_eq!(listening_port("ws://127.0.0.1:47613\n"), Some(47613));
    assert_eq!(listening_port("warning: something"), None);
    assert_eq!(parse_version("codex-cli 0.166.0\n").as_deref(), Some("0.166.0"));
    assert_eq!(node_target("linux", "x86_64"), Some(("linux", "x64")));
    assert_eq!(node_target("macos", "aarch64"), Some(("darwin", "arm64")));
    assert_eq!(node_target("windows", "x86_64"), None);
    assert_eq!(host_dir("/home/u/", "0.166.0"), "/home/u/.acpira/codex/0.166.0");
    assert!(safe_version("0.166.0") && safe_version("0.167.0-alpha.3"));
    assert!(!safe_version("../x") && !safe_version("1.0; rm -rf ~") && !safe_version(""));
    assert!(wanted("bin/codex") && wanted("codex-path/rg") && wanted("codex-resources/bwrap") && wanted("codex-package.json"));
    assert!(!wanted("bin/codex-code-mode-host") && !wanted("codex-resources/voice/bin/codex-voice-host") && !wanted("codex-path/"));
  }

  // The script runs on the SSH host; Windows has no /bin/sh to parse it with
  #[cfg(unix)]
  #[test]
  fn the_upload_script_runs_under_sh() {
    let out = std::process::Command::new("/bin/sh").arg("-n").arg("-c").arg(upload_script("0.166.0", "ab", 12)).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
  }

  /// A stream that broke off is never taken for a build, wherever it broke; the whole one lands
  #[cfg(unix)]
  #[test]
  fn a_broken_upload_leaves_no_build() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let npm = dir.path().join("codex.tgz");
    npm_tarball(&npm);
    let packed = dir.path().join("a.tgz");
    let digest = pack_npm(&npm, &packed).unwrap();
    let bytes = std::fs::read(&packed).unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let run = |sent: &[u8], len: usize| {
      let mut sh = Command::new("/bin/sh");
      sh.arg("-c").arg(upload_script("1.2.3", &digest, len as u64)).env("HOME", &home);
      let mut child = sh.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
      child.stdin.take().unwrap().write_all(sent).unwrap();
      String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).into_owned()
    };
    let builds = home.join(HOST_DIR);
    // Shorter than announced, then a stream of the announced length that gzip's checksum does not pass: tar alone
    // takes both, the cut falling after the last file
    let cut = &bytes[..bytes.len() - 5];
    assert!(run(cut, bytes.len()).contains("error=the upload broke off"));
    assert!(run(cut, cut.len()).contains("error=the upload broke off"));
    assert_eq!(std::fs::read_dir(&builds).unwrap().count(), 0, "nothing is left of a failed upload");
    assert!(run(&bytes, bytes.len()).contains(&format!("{DEPLOY}ok")));
    let names: Vec<String> = std::fs::read_dir(&builds).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["1.2.3"]);
    assert!(builds.join("1.2.3/.ok").is_file() && builds.join("1.2.3/bin/codex").is_file());
  }

  /// A platform package as npm packs it, with more in it than the server needs
  fn npm_tarball(to: &Path) {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(std::fs::File::create(to).unwrap(), flate2::Compression::fast()));
    for (path, body, mode) in [
      ("package/package.json", "{}", 0o644),
      ("package/vendor/x86_64-unknown-linux-musl/bin/codex", "#!/bin/sh\necho codex\n", 0o755),
      ("package/vendor/x86_64-unknown-linux-musl/bin/codex-code-mode-host", "host", 0o755),
      ("package/vendor/x86_64-unknown-linux-musl/codex-path/rg", "rg", 0o755),
      ("package/vendor/x86_64-unknown-linux-musl/codex-resources/bwrap", "bwrap", 0o755),
      ("package/vendor/x86_64-unknown-linux-musl/codex-resources/voice/lib/libopus.so.0", "opus", 0o644),
      ("package/vendor/x86_64-unknown-linux-musl/codex-package.json", "{\"layoutVersion\":1}", 0o644),
    ] {
      let mut header = tar::Header::new_gnu();
      header.set_size(body.len() as u64);
      header.set_mode(mode);
      tar.append_data(&mut header, path, body.as_bytes()).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
  }

  fn entries(tgz: &Path) -> Vec<(String, u32)> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(tgz).unwrap()));
    let mut out: Vec<(String, u32)> = archive.entries().unwrap().map(|e| e.unwrap()).map(|e| (e.path().unwrap().to_string_lossy().into_owned(), e.header().mode().unwrap())).collect();
    out.sort();
    out
  }

  #[test]
  fn a_build_is_packed_from_npm_or_from_an_install_with_only_what_the_server_needs() {
    let dir = tempfile::tempdir().unwrap();
    let npm = dir.path().join("codex.tgz");
    npm_tarball(&npm);
    let packed = dir.path().join("a.tgz");
    let digest = pack_npm(&npm, &packed).unwrap();
    assert_eq!(digest, crate::util::sha256_hex(b"#!/bin/sh\necho codex\n"));
    let want = vec![("bin/codex".to_owned(), 0o755), ("codex-package.json".to_owned(), 0o644), ("codex-path/rg".to_owned(), 0o755), ("codex-resources/bwrap".to_owned(), 0o755)];
    assert_eq!(entries(&packed), want);

    // The same build unpacked, as an installed package holds it
    let vendor = dir.path().join("vendor");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(&npm).unwrap()));
    for entry in archive.entries().unwrap() {
      let mut entry = entry.unwrap();
      let path = entry.path().unwrap().to_string_lossy().into_owned();
      let Some(rel) = path.strip_prefix("package/vendor/x86_64-unknown-linux-musl/") else { continue };
      let to = vendor.join(rel);
      std::fs::create_dir_all(to.parent().unwrap()).unwrap();
      entry.unpack(&to).unwrap();
    }
    let again = dir.path().join("b.tgz");
    assert_eq!(pack_dir(&vendor, &again).unwrap(), digest);
    assert_eq!(entries(&again), want);
    // A folder without the binary is no build
    assert!(pack_dir(dir.path(), &dir.path().join("c.tgz")).is_err());
  }
}
