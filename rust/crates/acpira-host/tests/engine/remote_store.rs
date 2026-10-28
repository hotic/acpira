//! One session history per host (`remote/host_store.rs`): a record this engine keeps for a host folder gets a copy in the
//! host's own store through the real helper, with the folder as the host names it and no category; a record written
//! while the host was away is copied when it connects, with its blobs; a deletion here removes the copy there, even one
//! made while the host was away and this engine restarted since; a copy the host updated later, or deleted and still
//! holds in its trash, is not replaced

#![cfg(unix)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acpira_host::remote::host_store::HostMirror;
use acpira_host::store::record::SessionRecord;
use acpira_host::store::transcript_store::TranscriptStore;
use acpira_shared::remote::{ConnectionOp, RunMode};
use serde_json::{Value, json};

use crate::remote_connections::Lab;

fn record(id: &str, cwd: &str, updated_at: &str) -> Arc<SessionRecord> {
  Arc::new(
    serde_json::from_value(json!({ "id": id, "agent": "acpira", "cwd": cwd, "title": id, "category": "c1", "createdAt": updated_at,
      "updatedAt": updated_at, "turns": [], "controls": { "modes": [], "options": [] }, "commands": [] }))
    .unwrap(),
  )
}

fn host_sessions(lab: &Lab) -> PathBuf {
  lab.host_home().join(".acpira").join("sessions")
}

fn read(path: &std::path::Path) -> Option<Value> {
  serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// What the gone hook was told: the host, and the session or None for "read the list again"
type GoneLog = Arc<Mutex<Vec<(String, Option<String>)>>>;

async fn until(what: &str, mut pred: impl FnMut() -> bool) {
  let t0 = std::time::Instant::now();
  while !pred() {
    assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn records_in_a_host_folder_have_their_copy_in_the_hosts_store() {
  let lab = Lab::new();
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
  // `add` connects at once: stop it, so the first record is written while the host is away
  connections.op(ConnectionOp::Disconnect { alias: "gen1".into() }).await.unwrap();
  let local = lab.path().join("engine").join("sessions");
  let store = TranscriptStore::new(local.clone(), Arc::new(|_: &str| {}), None);

  // Written before the host is reached, with a blob: copied once it connects (the comparison reads the index)
  let (blob, _) = store.save_blob("s-early", ".png", b"picture").await.unwrap();
  store.flush(record("s-early", "ssh://gen1/home/u/app", "2026-10-01T00:00:00.000Z")).await.unwrap();
  store.flush(record("s-here", "/work/x", "2026-10-01T00:00:00.000Z")).await.unwrap();
  store.sync_index(&[], &HashSet::new()).await.unwrap();

  let mirror = HostMirror::with_batch(connections.clone(), store.clone(), &lab.path().join("engine"), Arc::new(|_: &str| {}), Duration::from_millis(50));
  let gone: GoneLog = Default::default();
  let g = gone.clone();
  mirror.set_gone_hook(Arc::new(move |alias: &str, id: Option<&str>| g.lock().unwrap().push((alias.to_owned(), id.map(str::to_owned)))));
  mirror.start();
  connections.op(ConnectionOp::Connect { alias: "gen1".into(), accept_new_machine: false, fetch_build: false }).await.unwrap();

  let early = host_sessions(&lab).join("s-early.json");
  until("the early record's copy", || early.exists()).await;
  let copy = read(&early).unwrap();
  assert_eq!(copy["cwd"], "/home/u/app", "the host's own path");
  assert!(copy.get("category").is_none(), "categories are this machine's list");
  until("its blob", || host_sessions(&lab).join("s-early").join(&blob).exists()).await;
  assert!(!host_sessions(&lab).join("s-here.json").exists(), "a local folder's record never leaves");

  // Written while connected: copied with the next batch
  store.flush(record("s-live", "ssh://gen1/home/u/app", "2026-10-02T00:00:00.000Z")).await.unwrap();
  let live = host_sessions(&lab).join("s-live.json");
  until("the live record's copy", || live.exists()).await;

  // Updated later on the host: an older write here does not replace it
  let mut newer = read(&live).unwrap();
  newer["updatedAt"] = json!("2026-10-09T00:00:00.000Z");
  newer["title"] = json!("continued on the host");
  std::fs::write(&live, serde_json::to_vec(&newer).unwrap()).unwrap();
  store.flush(record("s-live", "ssh://gen1/home/u/app", "2026-10-03T00:00:00.000Z")).await.unwrap();
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert_eq!(read(&live).unwrap()["title"], "continued on the host");

  // Deleted here: the host's list drops it at once, then the copy and its blobs go
  store.trash("s-early").await.unwrap();
  assert!(gone.lock().unwrap().contains(&("gen1".into(), Some("s-early".into()))));
  until("the early copy removed", || !early.exists() && !host_sessions(&lab).join("s-early").exists()).await;
  until("the host's list asked for again", || gone.lock().unwrap().contains(&("gen1".into(), None))).await;

  // Deleted on the host and still in its trash: not brought back
  let trash = host_sessions(&lab).join("trash");
  std::fs::create_dir_all(&trash).unwrap();
  std::fs::rename(&live, trash.join("s-live.json")).unwrap();
  store.flush(record("s-live", "ssh://gen1/home/u/app", "2026-10-10T00:00:00.000Z")).await.unwrap();
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert!(!live.exists(), "a deletion on the host stands while its undo window lasts");

  // Deleted while the host is away, and this engine restarted before it is back: the deletion still reaches it
  store.flush(record("s-away", "ssh://gen1/home/u/app", "2026-10-11T00:00:00.000Z")).await.unwrap();
  let away = host_sessions(&lab).join("s-away.json");
  until("the away record's copy", || away.exists()).await;
  connections.op(ConnectionOp::Disconnect { alias: "gen1".into() }).await.unwrap();
  store.trash("s-away").await.unwrap();
  let tombs = lab.path().join("engine").join("run").join("remote").join("gone.json");
  let filed = |id: &str| read(&tombs).is_some_and(|t| t["gen1"].as_array().is_some_and(|ids| ids.contains(&json!(id))));
  until("the deletion filed", || filed("s-away")).await;
  mirror.stop();
  let mirror = HostMirror::with_batch(connections.clone(), store.clone(), &lab.path().join("engine"), Arc::new(|_: &str| {}), Duration::from_millis(50));
  mirror.start();
  connections.op(ConnectionOp::Connect { alias: "gen1".into(), accept_new_machine: false, fetch_build: false }).await.unwrap();
  until("the away copy removed", || !away.exists()).await;
  until("the deletion unfiled", || !filed("s-away")).await;

  mirror.stop();
  connections.dispose();
}
