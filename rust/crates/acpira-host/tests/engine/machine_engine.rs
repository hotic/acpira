//! `acpira engine` against a stand-in for the machine's running engine: an older one is asked to hand the endpoint over
//! and the real engine starts in its place; a newer one is left alone and named

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use acpira_host::machine_engine::ensure::socket_path;

/// Answers probes as `version`; a drain is recorded, answered, and the socket given up like a real engine does
fn stand_in(socket: &Path, version: &'static str) -> Arc<Mutex<Vec<Value>>> {
  std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
  let listener = tokio::net::UnixListener::bind(socket).unwrap();
  let drains = Arc::new(Mutex::new(vec![]));
  let (seen, path) = (drains.clone(), socket.to_path_buf());
  tokio::spawn(async move {
    loop {
      let Ok((stream, _)) = listener.accept().await else { return };
      let (read, mut write) = stream.into_split();
      let mut lines = BufReader::new(read).lines();
      let Ok(Some(line)) = lines.next_line().await else { continue };
      let msg: Value = serde_json::from_str(&line).unwrap();
      match msg["type"].as_str() {
        Some("probe") => {
          let ok = json!({ "type": "probeOk", "requestId": msg["requestId"], "version": version, "pid": 1, "protocolVersion": 1, "protocolMin": 1, "draining": false });
          write.write_all(format!("{ok}\n").as_bytes()).await.unwrap();
        }
        Some("drain") => {
          seen.lock().unwrap().push(msg.clone());
          write.write_all(format!("{}\n", json!({ "type": "drainOk", "requestId": msg["requestId"] })).as_bytes()).await.unwrap();
          let _ = std::fs::remove_file(&path);
          return;
        }
        _ => {}
      }
    }
  });
  drains
}

async fn ensure(home: &Path) -> Value {
  let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_acpira"))
    .args(["engine", "--home"])
    .arg(home)
    .args(["--idle-grace", "1"])
    .env_remove("ACPIRA_HOME")
    .env("ACPIRA_CATALOG_REFRESH", "0")
    .env("ACPIRA_LOGIN_PATH", "0")
    .output()
    .await
    .unwrap();
  assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
  serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap()
}

/// A short data root: the socket must fit a Unix socket path
fn home() -> (tempfile::TempDir, PathBuf) {
  let dir = tempfile::Builder::new().prefix("me").tempdir_in("/tmp").unwrap();
  let home = dir.path().join("h");
  (dir, home)
}

fn stop(pid: u64) {
  // SAFETY: kill has no memory effects; SIGTERM runs the engine's normal teardown
  unsafe {
    libc::kill(pid as i32, libc::SIGTERM);
  }
}

#[tokio::test]
async fn an_older_engine_hands_over_and_this_build_serves_from_its_landed_copy() {
  let (_dir, home) = home();
  let drains = stand_in(&socket_path(&home), "0.0.1");
  let reached = ensure(&home).await;
  assert_eq!(reached["version"], acpira_shared::version::VERSION);
  assert_eq!(reached["socket"].as_str().map(PathBuf::from), Some(socket_path(&home)));
  let asked = drains.lock().unwrap().clone();
  assert_eq!(asked.len(), 1);
  assert_eq!(asked[0]["version"], acpira_shared::version::VERSION);
  // The engine runs from its copy under the data root, and the next shell reaches the same one
  let pid = reached["pid"].as_u64().unwrap();
  let again = ensure(&home).await;
  assert_eq!(again["pid"].as_u64(), Some(pid));
  let landed: Vec<_> = std::fs::read_dir(home.join("engine")).unwrap().flatten().filter(|e| e.path().is_dir()).collect();
  assert_eq!(landed.len(), 1);
  stop(pid);
}

#[tokio::test]
async fn a_newer_engine_is_left_running() {
  let (_dir, home) = home();
  let drains = stand_in(&socket_path(&home), "999.0.0");
  let reached = ensure(&home).await;
  assert_eq!(reached["version"], "999.0.0");
  assert!(drains.lock().unwrap().is_empty());
}

/// The macOS launcher (machine_engine/launcher.rs) exits as its child did, and a stop signal reaches the child, so
/// launchd and a drain see the engine end through it
#[test]
fn the_launcher_passes_exit_codes_and_stop_signals_through() {
  use std::process::Command;
  let bin = env!("CARGO_BIN_EXE_acpira");
  let code = Command::new(bin).args(["launch", "/bin/sh", "-c", "exit 3"]).status().unwrap();
  assert_eq!(code.code(), Some(3));
  let dir = tempfile::tempdir().unwrap();
  let ready = dir.path().join("ready");
  let script = format!("trap 'exit 7' TERM; touch '{}'; while :; do sleep 0.05; done", ready.display());
  let mut launcher = Command::new(bin).args(["launch", "/bin/sh", "-c", &script]).spawn().unwrap();
  let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
  while !ready.exists() {
    assert!(std::time::Instant::now() < deadline, "the child never started");
    std::thread::sleep(std::time::Duration::from_millis(20));
  }
  // SAFETY: a signal to the launcher this test started
  unsafe { libc::kill(launcher.id() as libc::pid_t, libc::SIGTERM) };
  assert_eq!(launcher.wait().unwrap().code(), Some(7));
}
