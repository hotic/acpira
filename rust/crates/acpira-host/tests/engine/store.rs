//! test/TranscriptStore.test.ts: records, the shared index, trash, blobs and debounced writes

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use acpira_host::store::record::SessionRecord;
use acpira_host::store::transcript_store::{TranscriptStore, is_session_id};
use acpira_shared::transcript::SessionSummary;

fn record(id: &str, title: &str) -> Arc<SessionRecord> {
  let now = "2026-01-01T00:00:00.000Z";
  Arc::new(serde_json::from_value(json!({ "id": id, "agent": "fake", "cwd": "/tmp", "title": title, "createdAt": now, "updatedAt": now,
    "turns": [], "controls": { "modes": [], "options": [] }, "commands": [] })).unwrap())
}

fn summary(id: &str, title: &str) -> SessionSummary {
  record(id, title).summary()
}

type Logs = Arc<Mutex<Vec<String>>>;

fn store_at(dir: &Path, logs: &Logs) -> Arc<TranscriptStore> {
  let l = logs.clone();
  TranscriptStore::new(dir.to_path_buf(), Arc::new(move |x: &str| l.lock().unwrap().push(x.to_owned())), None)
}

fn fixture() -> (tempfile::TempDir, Logs, Arc<TranscriptStore>) {
  let dir = tempfile::tempdir().unwrap();
  let logs: Logs = Default::default();
  let store = store_at(dir.path(), &logs);
  (dir, logs, store)
}

fn title_of(path: &Path) -> Value {
  serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap()).unwrap()["title"].clone()
}

fn ids(list: &[SessionSummary]) -> Vec<String> {
  list.iter().map(|s| s.id.clone()).collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
  v.sort();
  v
}

fn own(ids: &[&str]) -> HashSet<String> {
  ids.iter().map(|x| x.to_string()).collect()
}

async fn sleep(ms: u64) {
  tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_writes_what_is_still_debounced() {
  let (dir, _, store) = fixture();
  store.save_after(record("a", "first"), Duration::from_secs(10));
  store.save_after(record("b", "second"), Duration::from_secs(10));
  store.dispose().await;
  assert_eq!(title_of(&dir.path().join("a.json")), "first");
  assert_eq!(title_of(&dir.path().join("b.json")), "second");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_debounced_save_that_cannot_reach_the_disk_is_logged_and_reported() {
  let dir = tempfile::tempdir().unwrap();
  // A regular file where the record's directory must go makes the write fail
  std::fs::write(dir.path().join("x.json"), "").unwrap();
  let logs: Logs = Default::default();
  let errors: Logs = Default::default();
  let (l, e) = (logs.clone(), errors.clone());
  let broken = TranscriptStore::new(dir.path().join("x.json"), Arc::new(move |x: &str| l.lock().unwrap().push(x.to_owned())),
    Some(Arc::new(move |id: &str, err: &str| e.lock().unwrap().push(format!("{id}:{err}")))));
  broken.save_after(record("x", "T"), Duration::ZERO);
  sleep(50).await;
  assert!(logs.lock().unwrap().iter().any(|l| l.contains("save failed")));
  assert!(errors.lock().unwrap().iter().any(|e| e.starts_with("x:")));
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_or_malformed_records_load_as_missing_with_a_log_line() {
  let (dir, logs, store) = fixture();
  std::fs::write(dir.path().join("bad.json"), "{ not json").unwrap();
  std::fs::write(dir.path().join("shape.json"), json!({ "id": "shape" }).to_string()).unwrap();
  assert!(store.load("bad").await.is_none());
  assert!(store.load("shape").await.is_none());
  assert!(store.load("absent").await.is_none());
  assert_eq!(logs.lock().unwrap().iter().filter(|l| l.contains("record unreadable")).count(), 2);
  store.flush(record("good", "T")).await.unwrap();
  assert_eq!(store.load("good").await.unwrap().id, "good");
}

#[tokio::test(flavor = "multi_thread")]
async fn rebuilding_a_lost_index_skips_broken_records() {
  let (dir, _, store) = fixture();
  store.flush(record("ok", "T")).await.unwrap();
  std::fs::write(dir.path().join("bad.json"), "nope").unwrap();
  assert_eq!(ids(&store.load_index().await.unwrap()), ["ok"]);
}

// The failure that lost sessions in the wild: every window holds its own copy of the list and used to write it back whole
#[tokio::test(flavor = "multi_thread")]
async fn two_hosts_never_clobber_each_others_sessions_and_an_incomplete_index_heals_from_the_files() {
  let (dir, logs, a) = fixture();
  let b = store_at(dir.path(), &logs);
  a.flush(record("a1", "from A")).await.unwrap();
  let list_a = a.sync_index(&[summary("a1", "from A")], &own(&["a1"])).await.unwrap();
  assert_eq!(ids(&list_a), ["a1"]);
  // B booted earlier with an empty list and never saw a1; its write must not drop a1
  b.flush(record("b1", "from B")).await.unwrap();
  let list_b = b.sync_index(&[summary("b1", "from B")], &own(&["b1"])).await.unwrap();
  assert_eq!(sorted(ids(&list_b)), ["a1", "b1"]);
  // A's next write, still ignorant of b1, keeps it too
  assert_eq!(sorted(ids(&a.sync_index(&list_a, &own(&["a1"])).await.unwrap())), ["a1", "b1"]);
  // An index hand-truncated to nothing (or clobbered by an old build) comes back from the files
  std::fs::write(dir.path().join("index.json"), "[]").unwrap();
  assert_eq!(sorted(a.load_index().await.unwrap().into_iter().map(|s| s.title).collect()), ["from A", "from B"]);
  // An entry whose file is gone drops out
  std::fs::remove_file(dir.path().join("b1.json")).unwrap();
  assert_eq!(ids(&a.load_index().await.unwrap()), ["a1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn own_ids_take_this_hosts_summary_the_rest_the_disks_and_legacy_entries_get_cwd() {
  let (dir, _, store) = fixture();
  store.flush(record("x", "renamed elsewhere")).await.unwrap();
  store.flush(record("y", "mine")).await.unwrap();
  // Disk index: x already renamed by another window, y stale; this host's list has both under their old titles
  let mut stale = serde_json::to_value(summary("y", "old")).unwrap();
  stale.as_object_mut().unwrap().remove("cwd");
  std::fs::write(dir.path().join("index.json"), json!([summary("x", "renamed elsewhere"), stale]).to_string()).unwrap();
  let out = store.sync_index(&[summary("x", "old"), summary("y", "mine")], &own(&["y"])).await.unwrap();
  assert_eq!(out.iter().find(|s| s.id == "x").unwrap().title, "renamed elsewhere");
  let y = out.iter().find(|s| s.id == "y").unwrap();
  assert_eq!((y.title.as_str(), y.cwd.as_str()), ("mine", "/tmp"));
}

#[tokio::test(flavor = "multi_thread")]
async fn trash_moves_record_and_blobs_out_and_back_and_sweep_honours_the_window() {
  let (dir, _, store) = fixture();
  let d = dir.path();
  store.flush(record("t", "T")).await.unwrap();
  store.save_blob("t", ".png", &[1, 2, 3]).await.unwrap();
  store.trash("t").await.unwrap();
  assert!(!d.join("t.json").exists());
  assert!(d.join("trash/t.json").exists());
  assert_eq!(std::fs::read_dir(d.join("trash/t")).unwrap().count(), 1);
  assert!(store.load_index().await.unwrap().is_empty());
  store.sweep_trash(Duration::from_secs(60)).await;
  assert!(d.join("trash/t.json").exists());
  store.restore("t").await;
  assert!(d.join("t.json").exists());
  assert_eq!(std::fs::read_dir(d.join("t")).unwrap().count(), 1);
  assert_eq!(ids(&store.load_index().await.unwrap()), ["t"]);
  store.trash("t").await.unwrap();
  store.sweep_trash(Duration::ZERO).await;
  assert!(!d.join("trash/t.json").exists());
  assert!(!d.join("trash/t").exists());
  store.remove("t").await;
}

// The same session live in two windows: one deletes it, the other's debounced save lands afterwards and used to recreate the record
#[tokio::test(flavor = "multi_thread")]
async fn a_save_of_a_record_another_store_removed_is_dropped_but_first_writes_and_restores_go_through() {
  let (dir, logs, a) = fixture();
  let d = dir.path();
  let b = store_at(d, &logs);
  a.flush(record("s", "v1")).await.unwrap();
  assert_eq!(b.load("s").await.unwrap().title, "v1");
  // B's stale save is pending when A trashes; flushing it must not bring the record back into the live directory
  b.save_after(record("s", "v2 from B"), Duration::from_secs(10));
  a.trash("s").await.unwrap();
  b.dispose().await;
  assert!(!d.join("s.json").exists());
  assert_eq!(title_of(&d.join("trash/s.json")), "v1");
  assert!(logs.lock().unwrap().iter().any(|l| l.contains("not written back")));
  assert!(b.sync_index(&[summary("s", "T")], &own(&["s"])).await.unwrap().is_empty());
  assert!(b.knew("s"));
  // Removed for good: still refused. Undone instead: the file is back, so B's next save applies again
  a.remove("s").await;
  b.flush(record("s", "v3 from B")).await.ok();
  assert!(!d.join("s.json").exists());
  a.flush(record("s", "v1")).await.unwrap();
  a.trash("s").await.unwrap();
  a.restore("s").await;
  b.flush(record("s", "v4 from B")).await.unwrap();
  assert_eq!(title_of(&d.join("s.json")), "v4 from B");
  // A record the store never had on disk is created as usual
  b.flush(record("fresh", "T")).await.unwrap();
  assert!(d.join("fresh.json").exists());
}

// The save and index debounces are equal, so a fresh session's first write is often still in flight when the index reconcile lists the
// directory; the list must wait for it, or the manager reads the brand-new session as deleted by another window
#[tokio::test(flavor = "multi_thread")]
async fn sync_index_waits_for_writes_in_flight_so_a_fresh_record_never_reads_as_deleted() {
  let (dir, _, store) = fixture();
  for i in 0..20 {
    let id = format!("fresh-{i}");
    store.save_after(record(&id, "T"), Duration::ZERO);
    tokio::task::yield_now().await;
    assert!(ids(&store.sync_index(&[summary(&id, "T")], &own(&[&id])).await.unwrap()).contains(&id));
    assert!(dir.path().join(format!("{id}.json")).exists());
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writes_of_one_record_serialize_and_remove_waits_for_a_write_in_flight() {
  let (dir, logs, store) = fixture();
  let (r1, r2, r3) = tokio::join!(store.flush(record("c", "v1")), store.flush(record("c", "v2")), store.flush(record("c", "v3")));
  r1.unwrap(); r2.unwrap(); r3.unwrap();
  assert_eq!(title_of(&dir.path().join("c.json")), "v3");
  assert!(logs.lock().unwrap().is_empty());
  let (w, _) = tokio::join!(store.flush(record("c", "v4")), store.remove("c"));
  w.ok();
  assert!(!dir.path().join("c.json").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_debounced_record_is_readable_and_the_index_write_lands_it_first() {
  let (dir, _, store) = fixture();
  store.save_after(record("p", "pending"), Duration::from_secs(10));
  assert_eq!(store.load("p").await.unwrap().title, "pending");
  assert!(!dir.path().join("p.json").exists());
  assert_eq!(ids(&store.sync_index(&[summary("p", "pending")], &own(&["p"])).await.unwrap()), ["p"]);
  assert!(dir.path().join("p.json").exists());
  store.dispose().await;
  // No temp files are left behind by the atomic writes
  assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
}

#[tokio::test(flavor = "multi_thread")]
async fn path_like_ids_are_rejected_and_nothing_outside_the_store_is_touched() {
  let parent = tempfile::tempdir().unwrap();
  let dir = parent.path().join("sessions");
  std::fs::create_dir(&dir).unwrap();
  let logs: Logs = Default::default();
  let store = store_at(&dir, &logs);
  let marker = parent.path().join("keep");
  std::fs::write(&marker, "keep").unwrap();
  store.save_after(record("..", "T"), Duration::ZERO);
  store.flush(record("/tmp/x", "T")).await.ok();
  store.trash("..").await.ok();
  store.remove("..").await;
  store.remove("/etc/passwd").await;
  assert!(store.load("..").await.is_none());
  assert!(store.load("/tmp/x").await.is_none());
  assert!(marker.exists() && dir.exists());
  assert!(logs.lock().unwrap().iter().any(|l| l.contains("illegal id")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_whose_inner_id_differs_from_its_filename_is_missing() {
  let (dir, _, store) = fixture();
  std::fs::write(dir.path().join("good.json"), serde_json::to_string(&*record("other", "T")).unwrap()).unwrap();
  assert!(store.load("good").await.is_none());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_session_directory_symlinked_out_of_the_store_is_never_followed() {
  let (dir, _, store) = fixture();
  let outside = tempfile::tempdir().unwrap();
  std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
  std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
  store.remove("link").await;
  assert!(outside.path().join("secret.txt").exists());
  assert!(store.save_blob("link", ".txt", &[1]).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_of_saves_still_flushes_within_the_max_wait() {
  let dir = tempfile::tempdir().unwrap();
  let store = TranscriptStore::with_timing(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None, Duration::from_secs(10), Duration::from_millis(80));
  store.save(record("a", "v1"));
  sleep(25).await;
  store.save(record("a", "v2"));
  sleep(25).await;
  store.save(record("a", "v3"));
  sleep(80).await;
  assert_eq!(title_of(&dir.path().join("a.json")), "v3");
  store.dispose().await;
}

#[test]
fn session_ids_are_uuid_like_tokens_never_path_components() {
  assert!(is_session_id("a1b2"));
  assert!(is_session_id("550e8400-e29b-41d4-a716-446655440000"));
  for bad in ["..", "../secrets", "/tmp/x", "index", "trash"] {
    assert!(!is_session_id(bad), "{bad}");
  }
}

fn settings(j: Value) -> acpira_shared::transcript::TurnSettings {
  serde_json::from_value(j).unwrap()
}

// From test/fileLock.test.ts: prefs.json is shared by every window, each writes only the agents it changed
#[tokio::test(flavor = "multi_thread")]
async fn two_windows_on_prefs_json_each_write_only_their_own_agent() {
  let (dir, logs, a) = fixture();
  let b = store_at(dir.path(), &logs);
  let mut prefs_a = a.load_prefs().await;
  let mut prefs_b = b.load_prefs().await;
  prefs_a.last_settings.insert("grok".into(), settings(json!({ "modeId": "plan", "config": { "model": "grok-4.6" } })));
  a.save_prefs(&prefs_a, &["grok".into()]).await.unwrap();
  // b's snapshot predates a's write; writing kimi must not drop grok
  prefs_b.last_settings.insert("kimi".into(), settings(json!({ "modeId": "default", "config": {} })));
  let merged = b.save_prefs(&prefs_b, &["kimi".into()]).await.unwrap();
  assert_eq!(merged.last_settings.keys().cloned().collect::<Vec<_>>(), ["grok", "kimi"]);
  assert_eq!(serde_json::to_value(&a.load_prefs().await.last_settings["kimi"]).unwrap(), json!({ "modeId": "default", "config": {} }));
  // a newer choice for the same agent wins, whatever the other window's stale snapshot said
  prefs_a.last_settings.insert("grok".into(), settings(json!({ "modeId": "default", "config": { "model": "grok-4.5" } })));
  a.save_prefs(&prefs_a, &["grok".into()]).await.unwrap();
  assert_eq!(serde_json::to_value(&b.load_prefs().await.last_settings["grok"]).unwrap(), json!({ "modeId": "default", "config": { "model": "grok-4.5" } }));
}
