//! Remote run's wire (`remote/peer.rs`): this engine reaches the host's engine through the helper's `engine` op as a
//! peer, the host starts that engine when none runs and keeps it past the connection, and a second link finds the same
//! one. The host is this machine behind the fake ssh, with its home under the test's directory

#![cfg(unix)]

use std::time::Duration;

use acpira_host::remote::peer::{PeerHello, PeerLink};
use acpira_shared::remote::{ConnectionOp, RunMode};
use serde_json::{Value, json};

use crate::remote_connections::Lab;

/// Stops every engine started for a test's fake host when the test ends, a failing one too: they run detached from
/// the helper and would outlive the test. Found by their `--home`, the fake host's data root, so an engine a retry
/// started after the test last looked goes too
struct StopEngines(String);

impl StopEngines {
  fn for_lab(lab: &Lab) -> StopEngines {
    StopEngines(format!("--home {}", lab.path().join("host").join(".acpira").display()))
  }
}

impl Drop for StopEngines {
  fn drop(&mut self) {
    let _ = std::process::Command::new("pkill").args(["-TERM", "-f", &self.0]).status();
  }
}

/// `until` that names what it waited for and shows the manager's log when it gives up
async fn wait(m: &crate::session_manager::Mgr, what: &str, ms: u64, mut pred: impl FnMut() -> bool) {
  let t0 = std::time::Instant::now();
  while !pred() {
    assert!(t0.elapsed() < Duration::from_millis(ms), "timed out waiting for {what}; active: {:?}\nlog: {:#?}", m.active(), m.logs());
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

const ME: PeerHello<'static> = PeerHello { version: "test", capabilities: &["toast"], host_language: "en" };

async fn next_of(link: &mut PeerLink, pred: impl Fn(&Value) -> bool) -> Value {
  tokio::time::timeout(Duration::from_secs(30), async {
    loop {
      let v = link.recv().await.expect("the host's engine closed the connection");
      if pred(&v) {
        return v;
      }
    }
  })
  .await
  .expect("no such envelope from the host's engine")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_reaches_the_hosts_engine_which_outlives_the_connection() {
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Remote }).await.unwrap();
  let client = connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();

  let mut link = PeerLink::connect(&client, None, ME).await.expect("connected to the host's engine");
  let pid = link.hello_ok["sidecar"]["pid"].as_u64().expect("the engine's pid") as i32;
  assert_ne!(pid as u32, std::process::id());
  link.send(&json!({ "type": "attachView", "viewId": "P", "host": "sidebar", "initial": { "mostRecent": true } }));
  link.send(&json!({ "type": "webviewMessage", "viewId": "P", "message": { "type": "ready" } }));
  let init = next_of(&mut link, |v| v["type"] == "hostMessage" && v["message"]["type"] == "init").await;
  assert_eq!(init["viewId"], "P");
  drop(link);

  // The engine stays for the next connection, which finds it rather than starting another
  tokio::time::sleep(Duration::from_millis(300)).await;
  // SAFETY: kill with signal 0 only checks that the process exists
  assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the host's engine outlives the connection");
  let again = PeerLink::connect(&client, None, ME).await.expect("connected again");
  assert_eq!(again.hello_ok["sidecar"]["pid"].as_u64(), Some(pid as u64));
  drop(again);

  // SAFETY: SIGTERM to the engine this test started; it runs its normal teardown
  unsafe { libc::kill(pid, libc::SIGTERM) };
  let gone = async {
    // SAFETY: as above, existence only
    while unsafe { libc::kill(pid, 0) } == 0 {
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
  };
  tokio::time::timeout(Duration::from_secs(10), gone).await.expect("the host's engine ended on SIGTERM");
  connections.dispose();
}

/// Remote run end to end on this engine's side: a host in remote run has its engine's sessions in the local list as
/// `ssh://` folders, a new session in a folder there starts on the host and is shown here, and a rename goes there
#[tokio::test(flavor = "multi_thread")]
async fn a_host_in_remote_run_has_its_sessions_in_the_local_list() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Remote }).await.unwrap();
  let work = lab.path().join("hostfs").join("work");
  std::fs::create_dir_all(&work).unwrap();
  let folder = format!("ssh://gen1{}", work.display());
  let dir = tempfile::tempdir().unwrap();
  let mut opts = crate::session_manager::Opts::fake(&fake).projects();
  opts.remote = Some(connections.clone());
  let m = crate::session_manager::Mgr::new(dir.path(), opts);
  m.init().await;

  m.handle(json!({ "type": "newSession", "cwd": folder })).await;
  wait(&m, "the new session on the host", 30_000, || m.active().is_some_and(|v| v["cwd"] == folder.as_str())).await;
  let id = m.active_id().unwrap();
  wait(&m, "the host's list", 15_000, || m.sessions().iter().any(|s| s["id"] == id.as_str())).await;
  let listed = m.sessions().into_iter().find(|s| s["id"] == id.as_str()).unwrap();
  assert_eq!(listed["cwd"], folder.as_str());
  // Started there, by the host's own engine: no record of it here
  assert!(!dir.path().join(format!("{id}.json")).exists());

  m.handle(json!({ "type": "renameSession", "id": id, "title": "On the host" })).await;
  wait(&m, "the rename", 15_000, || m.sessions().iter().any(|s| s["id"] == id.as_str() && s["title"] == "On the host")).await;

  // A prompt from here runs there; its turns come back through the host's patches, rebuilt into this engine's views
  m.handle(json!({ "type": "send", "sessionId": id, "text": "hello from here" })).await;
  wait(&m, "the prompt's turns", 30_000, || m.active().is_some_and(|v| v["turns"].as_array().is_some_and(|t| t.len() >= 2) && v["running"] == false)).await;
  let view = m.active().unwrap();
  assert!(view["turns"][0].to_string().contains("hello from here"), "{view}");
  assert_eq!(view["cwd"], folder.as_str());
  assert!(!m.logs().iter().any(|l| l.contains("out of step")), "every patch from the host applied: {:?}", m.logs());

  // The host's engine goes away: the list keeps what it last said
  let engines = m.m.host_engines();
  assert_eq!(engines.len(), 1, "{engines:?}");
  for (_, pid) in &engines {
    // SAFETY: SIGTERM to the host engine this test started
    unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
  }
  wait(&m, "the host's engine to go", 15_000, || m.m.host_engines().is_empty()).await;
  assert!(m.sessions().iter().any(|s| s["id"] == id.as_str() && s["title"] == "On the host"), "the cached list stays");
  m.dispose().await;
  for (_, pid) in engines {
    // SAFETY: SIGTERM to the host engine this test started
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    let gone = async {
      // SAFETY: existence only
      while unsafe { libc::kill(pid as i32, 0) } == 0 {
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
    };
    tokio::time::timeout(Duration::from_secs(10), gone).await.expect("the host's engine ended on SIGTERM");
  }
  connections.dispose();
}

#[tokio::test(flavor = "multi_thread")]
async fn under_local_run_the_hosts_sessions_list_too_and_this_machines_own_stay_here() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
  let work = lab.path().join("hostfs").join("work");
  std::fs::create_dir_all(&work).unwrap();
  let folder = format!("ssh://gen1{}", work.display());
  let record = |id: &str, cwd: &str, title: &str| {
    let at = "2026-10-01T00:00:00.000Z";
    serde_json::to_vec(&json!({ "id": id, "agent": "fake", "cwd": cwd, "title": title, "createdAt": at, "updatedAt": at,
      "turns": [], "controls": { "modes": [], "options": [] }, "commands": [] }))
    .unwrap()
  };
  // A session this machine drives in the host folder, its copy in the host's store under another title, and one of the
  // host's own
  let dir = tempfile::tempdir().unwrap();
  std::fs::write(dir.path().join("s-mine.json"), record("s-mine", &folder, "Driven here")).unwrap();
  let host = lab.host_home().join(".acpira").join("sessions");
  std::fs::create_dir_all(&host).unwrap();
  let path = work.display().to_string();
  std::fs::write(host.join("s-mine.json"), record("s-mine", &path, "Copy on the host")).unwrap();
  std::fs::write(host.join("s-host.json"), record("s-host", &path, "The host's own")).unwrap();

  let mut opts = crate::session_manager::Opts::fake(&fake).projects();
  opts.remote = Some(connections.clone());
  let m = crate::session_manager::Mgr::new(dir.path(), opts);
  m.init().await;
  wait(&m, "the host's list under local run", 30_000, || m.sessions().iter().any(|s| s["id"] == "s-host" && s["cwd"] == folder.as_str())).await;
  let mine: Vec<Value> = m.sessions().into_iter().filter(|s| s["id"] == "s-mine").collect();
  assert_eq!(mine.len(), 1, "listed once");
  assert_eq!(mine[0]["title"], "Driven here", "this machine's record wins over the host's copy");

  // Acted on here: a rename lands in this machine's record, not in the host's copy through the host's engine
  m.handle(json!({ "type": "renameSession", "id": "s-mine", "title": "Renamed here" })).await;
  let title = || serde_json::from_slice::<Value>(&std::fs::read(dir.path().join("s-mine.json")).unwrap()).unwrap()["title"].clone();
  wait(&m, "the rename in this machine's record", 15_000, || title() == "Renamed here").await;

  // The host's own session opens through the host's engine
  m.handle(json!({ "type": "selectSession", "id": "s-host" })).await;
  wait(&m, "the host's session on screen", 30_000, || m.active().is_some_and(|v| v["title"] == "The host's own" && v["cwd"] == folder.as_str())).await;
  m.dispose().await;
  connections.dispose();
}
