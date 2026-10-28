//! The connection manager against a fake `ssh`: a script that runs the remote command under a temp "host" home and
//! logs what kind of call it was (probe, upload, start). The helper binary it deploys is a small wrapper that execs this
//! build's `acpira`, so digests and uploads stay cheap while the channel is the real helper

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use acpira_host::remote::connections::{Connections, ConnectionsOpts};
use acpira_shared::remote::{ConnectFailure, ConnectStep, ConnectionOp, ConnectionState, ConnectionView, RunMode};

const VERSION: &str = "9.9.9-test";
const ALIAS: &str = "gen1";

/// A fake host: `fake-ssh` runs remote commands with `host/` as HOME; `rootmap` (when present) is the helper's
/// ACPIRA_REMOTE_ROOT_MAP, so a path the engine reads here instead of through the helper does not exist
pub(crate) struct Lab {
  dir: tempfile::TempDir,
}

impl Lab {
  pub(crate) fn new() -> Lab {
    let lab = Lab { dir: tempfile::tempdir().unwrap() };
    std::fs::create_dir_all(lab.host_home()).unwrap();
    lab.set_machine("machine-a");
    lab.set_binary("one");
    let d = lab.path().display();
    let script = format!(
      r#"#!/bin/sh
D='{d}'
for a; do last="$a"; done
case " $* " in *" -G "*) printf 'hostname 10.0.0.9\nuser hotic\nport 2222\n'; exit 0;; esac
case "$last" in *"gzip -dc"*) kind=upload;; *ACPIRA-PROBE*) kind=probe;; *"remote --stdio"*) kind=start;; *) kind=other;; esac
echo "$kind" >> "$D/calls.log"
if [ -f "$D/fail" ]; then cat "$D/fail" >&2; exit 255; fi
HOME="$D/host"; ACPIRA_REMOTE_MACHINE_ID=$(cat "$D/machine"); export HOME ACPIRA_REMOTE_MACHINE_ID
if [ -f "$D/rootmap" ]; then ACPIRA_REMOTE_ROOT_MAP=$(cat "$D/rootmap"); export ACPIRA_REMOTE_ROOT_MAP; fi
if [ "$kind" = upload ] && [ -f "$D/corrupt" ]; then printf 'not the binary' | gzip -c | /bin/sh -c "$last"; exit $?; fi
exec /bin/sh -c "$last"
"#
    );
    write_exec(&lab.ssh(), &script);
    lab
  }

  pub(crate) fn path(&self) -> &Path {
    self.dir.path()
  }

  pub(crate) fn host_home(&self) -> PathBuf {
    self.path().join("host")
  }

  fn ssh(&self) -> PathBuf {
    self.path().join("fake-ssh")
  }

  fn binary(&self) -> PathBuf {
    self.path().join("helper-bin")
  }

  /// A new "build": the wrapper with another marker, so another digest
  fn set_binary(&self, marker: &str) {
    write_exec(&self.binary(), &format!("#!/bin/sh\n# build {marker}\nexec '{}' \"$@\" --no-login-path\n", env!("CARGO_BIN_EXE_acpira")));
  }

  /// Map the virtual root `virt` (an absolute path that does not exist here) to `real` for the helper
  pub(crate) fn map_root(&self, virt: &str, real: &Path) {
    std::fs::write(self.path().join("rootmap"), format!("{virt}={}", real.display())).unwrap();
  }

  fn set_machine(&self, id: &str) {
    std::fs::write(self.path().join("machine"), id).unwrap();
  }

  pub(crate) fn calls(&self) -> Vec<String> {
    std::fs::read_to_string(self.path().join("calls.log")).unwrap_or_default().lines().map(str::to_owned).collect()
  }

  fn builds(&self) -> Vec<String> {
    let dir = self.host_home().join(".acpira/remote/bin");
    let mut v: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
  }

  /// An engine with its own ACPIRA_HOME (`name`) reaching the lab's host
  pub(crate) fn engine(&self, name: &str) -> Arc<Connections> {
    let bin = self.binary();
    self.engine_with(name, Some(bin), fetch_releases_default())
  }

  /// An engine that carries `binary` for every platform (None: no build at all) and fetches builds from `releases`
  fn engine_with(&self, name: &str, binary: Option<PathBuf>, releases: String) -> Arc<Connections> {
    let root = self.path().join(name);
    std::fs::create_dir_all(&root).unwrap();
    let opts = ConnectionsOpts {
      ssh: self.ssh(),
      binary: Arc::new(move |_platform: &str| binary.clone()),
      releases,
      version: VERSION.into(),
      retry_base: Duration::from_millis(50),
      retry_max: Duration::from_millis(200),
      ..ConnectionsOpts::default()
    };
    Connections::new(&root, &self.path().join("local-home"), Arc::new(|_: &str| {}), None, opts)
  }
}

fn fetch_releases_default() -> String {
  ConnectionsOpts::default().releases
}

fn write_exec(path: &Path, text: &str) {
  use std::os::unix::fs::PermissionsExt;
  std::fs::write(path, text).unwrap();
  std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn view(c: &Connections) -> ConnectionView {
  c.views().into_iter().find(|v| v.alias == ALIAS).expect("the host is listed")
}

async fn until(c: &Connections, what: &str, pred: impl Fn(&ConnectionView) -> bool) -> ConnectionView {
  let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
  loop {
    let v = view(c);
    if pred(&v) {
      return v;
    }
    assert!(tokio::time::Instant::now() < deadline, "waiting for {what}, last {v:?}");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

async fn connected(c: &Connections) -> ConnectionView {
  until(c, "connected", |v| v.state == ConnectionState::Connected).await
}

fn helper_pid(c: &Connections) -> i32 {
  c.client(ALIAS).expect("a channel").hello().pid as i32
}

fn alive(pid: i32) -> bool {
  // SAFETY: signal 0 only checks the process exists
  unsafe { libc::kill(pid, 0) == 0 }
}

async fn gone(pid: i32) -> bool {
  for _ in 0..200 {
    if !alive(pid) {
      return true;
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
  }
  false
}

async fn add(c: &Connections) {
  c.op(ConnectionOp::Add { alias: ALIAS.into(), mode: RunMode::Local }).await.unwrap();
}

#[tokio::test]
async fn the_first_connection_deploys_and_later_ones_reuse_the_build() {
  let lab = Lab::new();
  let c = lab.engine("engine");
  add(&c).await;
  let v = connected(&c).await;
  assert_eq!(v.address.as_deref(), Some("hotic@10.0.0.9:2222"));
  assert_eq!(v.remote_version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
  assert_eq!(lab.calls(), ["probe", "upload", "start"]);
  let builds = lab.builds();
  assert_eq!(builds.len(), 1);
  assert!(builds[0].starts_with(&format!("{VERSION}-")), "{builds:?}");
  let installed = lab.host_home().join(".acpira/remote/bin").join(&builds[0]).join("acpira");
  use std::os::unix::fs::PermissionsExt;
  assert_eq!(std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777, 0o700);

  // The folder picker lists the host's home by default
  let (path, entries) = c.browse(ALIAS, None).await.unwrap();
  assert_eq!(path, lab.host_home().to_string_lossy());
  assert!(entries.iter().any(|e| e.name == ".acpira" && e.dir), "{entries:?}");

  // The machine id is kept from the first connection
  let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(lab.path().join("engine/connections.json")).unwrap()).unwrap();
  assert_eq!(saved["hosts"][0]["alias"], ALIAS);
  assert_eq!(saved["hosts"][0]["machineId"], "machine-a");

  let pid = helper_pid(&c);
  c.op(ConnectionOp::Disconnect { alias: ALIAS.into() }).await.unwrap();
  assert_eq!(view(&c).state, ConnectionState::Off);
  assert!(gone(pid).await, "the helper ends with its channel");

  // On demand (what a session does): probe and start, no upload
  c.ensure(ALIAS, Duration::from_secs(30)).await.unwrap();
  assert_eq!(lab.calls(), ["probe", "upload", "start", "probe", "start"]);

  // Another engine on this machine reads the same list
  let other = lab.engine("engine");
  assert_eq!(view(&other).state, ConnectionState::Off);
  drop(other);
  // The runtime disposes it on its way out (a running driver holds the manager, so dropping is not enough)
  let pid = helper_pid(&c);
  c.dispose();
  assert!(gone(pid).await, "the helper ends with the engine");
}

#[tokio::test]
async fn a_new_build_goes_beside_the_old_one() {
  let lab = Lab::new();
  let c = lab.engine("engine");
  add(&c).await;
  connected(&c).await;
  c.op(ConnectionOp::Disconnect { alias: ALIAS.into() }).await.unwrap();
  lab.set_binary("two");
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: false }).await.unwrap();
  connected(&c).await;
  assert_eq!(lab.calls().iter().filter(|k| *k == "upload").count(), 2);
  assert_eq!(lab.builds().len(), 2, "an older engine elsewhere keeps its build");
}

#[tokio::test]
async fn a_corrupted_upload_never_lands_and_is_retried() {
  let lab = Lab::new();
  std::fs::write(lab.path().join("corrupt"), "").unwrap();
  let c = lab.engine("engine");
  add(&c).await;
  let v = until(&c, "the upload error", |v| v.state == ConnectionState::Error).await;
  let e = v.error.unwrap();
  assert_eq!((e.kind, e.step), (ConnectFailure::Other, ConnectStep::Upload));
  assert!(e.message.contains("digest"), "{}", e.message);
  let bin = lab.host_home().join(".acpira/remote/bin");
  let placed = lab.builds().iter().any(|b| bin.join(b).join("acpira").exists());
  assert!(!placed, "nothing unverified is put in place");
  // Transient: the driver keeps trying, and the next clean upload connects
  std::fs::remove_file(lab.path().join("corrupt")).unwrap();
  connected(&c).await;
  assert!(lab.calls().iter().filter(|k| *k == "upload").count() >= 2);
}

#[tokio::test]
async fn an_installed_copy_that_does_not_start_is_uploaded_again() {
  let lab = Lab::new();
  let c = lab.engine("engine");
  add(&c).await;
  connected(&c).await;
  c.op(ConnectionOp::Disconnect { alias: ALIAS.into() }).await.unwrap();
  let installed = lab.host_home().join(".acpira/remote/bin").join(&lab.builds()[0]).join("acpira");
  write_exec(&installed, "#!/bin/sh\necho broken >&2\nexit 1\n");
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: false }).await.unwrap();
  connected(&c).await;
  assert_eq!(lab.calls()[3..], ["probe", "start", "probe", "upload", "start"]);
}

#[tokio::test]
async fn two_engines_deploying_at_once_both_connect() {
  let lab = Lab::new();
  let (a, b) = (lab.engine("a"), lab.engine("b"));
  tokio::join!(add(&a), add(&b));
  tokio::join!(connected(&a), connected(&b));
  assert_eq!(lab.builds().len(), 1);
  assert_ne!(helper_pid(&a), helper_pid(&b));
}

#[tokio::test]
async fn another_machine_behind_the_alias_is_refused_until_confirmed() {
  let lab = Lab::new();
  let c = lab.engine("engine");
  add(&c).await;
  connected(&c).await;
  c.op(ConnectionOp::Disconnect { alias: ALIAS.into() }).await.unwrap();
  lab.set_machine("machine-b");
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: false }).await.unwrap();
  let v = until(&c, "the machine check", |v| v.state == ConnectionState::Error).await;
  assert_eq!(v.error.as_ref().map(|e| e.kind), Some(ConnectFailure::MachineChanged));
  // Final: no retry behind the user's back
  let calls = lab.calls().len();
  tokio::time::sleep(Duration::from_millis(400)).await;
  assert_eq!(lab.calls().len(), calls);
  assert!(c.client(ALIAS).is_none());

  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: true, fetch_build: false }).await.unwrap();
  connected(&c).await;
  let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(lab.path().join("engine/connections.json")).unwrap()).unwrap();
  assert_eq!(saved["hosts"][0]["machineId"], "machine-b");
}

#[tokio::test]
async fn a_dropped_channel_reconnects() {
  let lab = Lab::new();
  let c = lab.engine("engine");
  add(&c).await;
  connected(&c).await;
  let first = helper_pid(&c);
  // SAFETY: kill has no memory preconditions; the pid is the helper this test started
  unsafe { libc::kill(first, libc::SIGKILL) };
  until(&c, "a new channel", |_| c.client(ALIAS).is_some_and(|cl| cl.hello().pid as i32 != first && cl.closed_reason().is_none())).await;
  assert_eq!(view(&c).state, ConnectionState::Connected);
}

#[tokio::test]
async fn ssh_failures_only_the_user_can_settle_stop_the_driver() {
  let lab = Lab::new();
  std::fs::write(lab.path().join("fail"), "Host key verification failed.\n").unwrap();
  let c = lab.engine("engine");
  add(&c).await;
  let v = until(&c, "the host key error", |v| v.state == ConnectionState::Error).await;
  let e = v.error.unwrap();
  assert_eq!((e.kind, e.step), (ConnectFailure::HostKey, ConnectStep::Ssh));
  tokio::time::sleep(Duration::from_millis(400)).await;
  assert_eq!(lab.calls(), ["probe"]);
  // A session asking for the host tries once more, then reports the same failure
  let err = c.ensure(ALIAS, Duration::from_secs(30)).await.err().expect("no channel");
  assert!(err.to_string().contains("Host key verification failed"), "{err}");
  assert_eq!(lab.calls(), ["probe", "probe"]);
}

#[tokio::test]
async fn background_work_never_undoes_a_disconnect_or_retries_what_the_user_must_settle() {
  let lab = Lab::new();
  let c = lab.engine("engine");
  add(&c).await;
  connected(&c).await;
  c.op(ConnectionOp::Disconnect { alias: ALIAS.into() }).await.unwrap();
  let calls = lab.calls().len();
  assert!(c.follow(ALIAS, Duration::from_millis(300)).await.is_err(), "the host stays off");
  assert_eq!(lab.calls().len(), calls);
  // Something the user does (opening a folder there) connects it again
  c.ensure(ALIAS, Duration::from_secs(30)).await.expect("connected again");

  std::fs::write(lab.path().join("fail"), "Host key verification failed.\n").unwrap();
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: false }).await.unwrap();
  until(&c, "the host key error", |v| v.state == ConnectionState::Error).await;
  let calls = lab.calls().len();
  assert!(c.follow(ALIAS, Duration::from_millis(300)).await.is_err());
  assert_eq!(lab.calls().len(), calls, "no new attempt");
}

#[tokio::test]
async fn a_network_failure_retries_with_a_backoff() {
  let lab = Lab::new();
  std::fs::write(lab.path().join("fail"), "ssh: connect to host 10.0.0.9 port 2222: Connection refused\n").unwrap();
  let c = lab.engine("engine");
  add(&c).await;
  let v = until(&c, "the network error", |v| v.state == ConnectionState::Error).await;
  assert_eq!(v.error.map(|e| e.kind), Some(ConnectFailure::Network));
  until(&c, "a few retries", |_| lab.calls().len() >= 3).await;
  std::fs::remove_file(lab.path().join("fail")).unwrap();
  c.kick();
  connected(&c).await;
  // Turning the host off ends the channel and keeps it off
  let pid = helper_pid(&c);
  c.op(ConnectionOp::SetDisabled { alias: ALIAS.into(), disabled: true }).await.unwrap();
  assert!(gone(pid).await);
  let v = view(&c);
  assert!(v.disabled && v.state == ConnectionState::Off, "{v:?}");
  assert!(c.ensure(ALIAS, Duration::from_secs(1)).await.is_err());
}

/// A stored (uncompressed) zip of (name, data, unix mode) entries
fn stored_zip(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
  fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
      c ^= u32::from(b);
      for _ in 0..8 {
        c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
      }
    }
    !c
  }
  let (mut out, mut central) = (Vec::new(), Vec::new());
  for (name, data, mode) in entries {
    let offset = out.len() as u32;
    let (crc, len, n) = (crc32(data), data.len() as u32, name.len() as u16);
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(data);
    central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    // Made by unix (3), version 2.0
    central.extend_from_slice(&[20, 3, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&len.to_le_bytes());
    central.extend_from_slice(&len.to_le_bytes());
    central.extend_from_slice(&n.to_le_bytes());
    central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    central.extend_from_slice(&(mode << 16).to_le_bytes());
    central.extend_from_slice(&offset.to_le_bytes());
    central.extend_from_slice(name.as_bytes());
  }
  let (cd_offset, cd_len, count) = (out.len() as u32, central.len() as u32, entries.len() as u16);
  out.extend_from_slice(&central);
  out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
  out.extend_from_slice(&[0, 0, 0, 0]);
  out.extend_from_slice(&count.to_le_bytes());
  out.extend_from_slice(&count.to_le_bytes());
  out.extend_from_slice(&cd_len.to_le_bytes());
  out.extend_from_slice(&cd_offset.to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes());
  out
}

/// A release server on loopback: `GET <path>` answers the matching file, anything else 404; the paths asked are kept
fn serve(files: Vec<(String, Vec<u8>)>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
  use std::io::{BufRead, BufReader, Write};
  let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
  let log = asked.clone();
  std::thread::spawn(move || {
    for stream in listener.incoming().flatten() {
      let mut reader = BufReader::new(stream.try_clone().unwrap());
      let mut line = String::new();
      if reader.read_line(&mut line).is_err() {
        continue;
      }
      // Drain the headers
      let mut h = String::new();
      while reader.read_line(&mut h).is_ok_and(|n| n > 2) {
        h.clear();
      }
      let path = line.split_whitespace().nth(1).unwrap_or_default().to_owned();
      log.lock().unwrap().push(path.clone());
      let mut stream = stream;
      match files.iter().find(|(p, _)| *p == path) {
        Some((_, body)) => {
          let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
          let _ = stream.write_all(body);
        }
        None => {
          let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
      }
    }
  });
  (base, asked)
}

#[tokio::test]
async fn a_missing_platform_build_is_fetched_from_the_release_when_asked() {
  use sha2::{Digest, Sha256};
  let lab = Lab::new();
  let platform = acpira_host::remote::deploy::local_platform();
  let wrapper = std::fs::read(lab.binary()).unwrap();
  let inner = format!("acpira/sidecar/bin/{platform}/acpira");
  let zip = stored_zip(&[("acpira/plugin.txt", b"other files of the archive", 0o100644), (&inner, &wrapper, 0o100755)]);
  let name = format!("acpira-{VERSION}-{platform}.zip");
  let digest: String = Sha256::digest(&zip).iter().map(|b| format!("{b:02x}")).collect();
  let sums = format!("{}  acpira-{VERSION}-other.zip\n{digest}  {name}\n", "0".repeat(64));
  let (base, asked) = serve(vec![(format!("/v{VERSION}/SHA256SUMS"), sums.into_bytes()), (format!("/v{VERSION}/{name}"), zip)]);
  let c = lab.engine_with("engine", None, base.clone());
  add(&c).await;

  // No build here: the driver stops on `unsupported` at the probe and downloads nothing on its own
  let v = until(&c, "the unsupported error", |v| v.state == ConnectionState::Error).await;
  let e = v.error.unwrap();
  assert_eq!((e.kind, e.step), (ConnectFailure::Unsupported, ConnectStep::Probe));
  assert!(asked.lock().unwrap().is_empty());

  // Asked to: fetch, check, unpack, then the usual upload and start
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: true }).await.unwrap();
  connected(&c).await;
  assert_eq!(*asked.lock().unwrap(), [format!("/v{VERSION}/SHA256SUMS"), format!("/v{VERSION}/{name}")]);
  let fetched = lab.path().join("engine/run/remote/builds").join(VERSION).join(&platform).join("acpira");
  use std::os::unix::fs::PermissionsExt;
  assert_eq!(std::fs::metadata(&fetched).unwrap().permissions().mode() & 0o777, 0o755);
  assert_eq!(lab.calls(), ["probe", "probe", "upload", "start"]);

  // The fetched build is found from now on without asking again
  c.op(ConnectionOp::Disconnect { alias: ALIAS.into() }).await.unwrap();
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: false }).await.unwrap();
  connected(&c).await;
  assert_eq!(asked.lock().unwrap().len(), 2);
  c.dispose();

  // A release without the build: a final `unsupported` at the fetch step, which names what is missing
  let (base, _) = serve(vec![(format!("/v{VERSION}/SHA256SUMS"), format!("{}  acpira-{VERSION}-other.zip\n", "0".repeat(64)).into_bytes())]);
  let lab = Lab::new();
  let c = lab.engine_with("engine", None, base);
  add(&c).await;
  until(&c, "the unsupported error", |v| v.state == ConnectionState::Error).await;
  c.op(ConnectionOp::Connect { alias: ALIAS.into(), accept_new_machine: false, fetch_build: true }).await.unwrap();
  let v = until(&c, "the fetch error", |v| v.error.as_ref().is_some_and(|e| e.step == ConnectStep::Fetch)).await;
  let e = v.error.unwrap();
  assert_eq!(e.kind, ConnectFailure::Unsupported);
  assert!(e.message.contains(&name), "{}", e.message);
}
