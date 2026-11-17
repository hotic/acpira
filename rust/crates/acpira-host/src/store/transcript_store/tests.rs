use acpira_shared::transcript::{CategoryOp, SessionCategories};

use super::*;
use crate::store::db::migration;
use crate::store::record::RecordParts;

#[test]
fn category_reorder_stays_inside_the_project_and_ops_on_missing_ids_do_nothing() {
  let mut list = SessionCategories::default();
  let create = |id: &str, cwd: &str| CategoryOp::Create { id: id.into(), name: id.into(), cwd: cwd.into() };
  for op in [create("a1", "/a"), create("b1", "/b"), create("a2", "/a"), create("a3", "/a"), create("a1", "/b")] {
    apply_category_op(&mut list, &op);
  }
  let ids = |l: &SessionCategories| l.categories.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
  // A repeated create keeps the first one
  assert_eq!(ids(&list), ["a1", "b1", "a2", "a3"]);
  assert_eq!(list.categories[0].cwd, "/a");
  // No anchor: after the project's last category
  apply_category_op(&mut list, &CategoryOp::Reorder { id: "a1".into(), before: None });
  assert_eq!(ids(&list), ["b1", "a2", "a3", "a1"]);
  // An anchor of another project is not followed
  apply_category_op(&mut list, &CategoryOp::Reorder { id: "a1".into(), before: Some("b1".into()) });
  assert_eq!(ids(&list), ["b1", "a2", "a3", "a1"]);
  apply_category_op(&mut list, &CategoryOp::Reorder { id: "a1".into(), before: Some("a2".into()) });
  assert_eq!(ids(&list), ["b1", "a1", "a2", "a3"]);
  let before = list.clone();
  apply_category_op(&mut list, &CategoryOp::Rename { id: "gone".into(), name: "x".into() });
  apply_category_op(&mut list, &CategoryOp::Reorder { id: "gone".into(), before: None });
  assert_eq!(list, before);
}

#[test]
fn projects_keep_their_order_and_seeding_happens_once() {
  let mut list = SessionCategories::default();
  let seed = |cwds: &[&str]| CategoryOp::SeedProjects { cwds: cwds.iter().map(|c| c.to_string()).collect() };
  apply_category_op(&mut list, &seed(&["/b", "/a", "/b", ""]));
  assert_eq!(list.projects.as_deref(), Some(&["/b".to_string(), "/a".to_string()][..]));
  // A second seed (another window, or a later connect) leaves the order alone
  apply_category_op(&mut list, &seed(&["/z"]));
  let add = |c: &str| CategoryOp::AddProject { cwd: c.into() };
  for op in [add("/c"), add("/a"), add("")] {
    apply_category_op(&mut list, &op);
  }
  let order = |l: &SessionCategories| l.projects.clone().unwrap_or_default();
  assert_eq!(order(&list), ["/b", "/a", "/c"]);
  apply_category_op(&mut list, &CategoryOp::ReorderProject { cwd: "/c".into(), before: Some("/b".into()) });
  assert_eq!(order(&list), ["/c", "/b", "/a"]);
  apply_category_op(&mut list, &CategoryOp::ReorderProject { cwd: "/c".into(), before: None });
  assert_eq!(order(&list), ["/b", "/a", "/c"]);
  apply_category_op(&mut list, &CategoryOp::ReorderProject { cwd: "/gone".into(), before: None });
  apply_category_op(&mut list, &CategoryOp::RemoveProject { cwd: "/a".into() });
  assert_eq!(order(&list), ["/b", "/c"]);
  assert_eq!(list.removed_projects, ["/a"]);
  // Removing every folder keeps an empty list, so nothing seeds it again
  for c in ["/b", "/c", "/b"] {
    apply_category_op(&mut list, &CategoryOp::RemoveProject { cwd: c.into() });
  }
  apply_category_op(&mut list, &seed(&["/a"]));
  assert_eq!(list.projects, Some(vec![]));
  assert_eq!(list.removed_projects, ["/a", "/b", "/c"]);
  // Adding a removed folder again forgets the removal
  apply_category_op(&mut list, &add("/b"));
  assert_eq!(order(&list), ["/b"]);
  assert_eq!(list.removed_projects, ["/a", "/c"]);
}

#[test]
fn sections_hold_each_project_once_and_give_them_back_when_deleted() {
  let mut list = SessionCategories::default();
  let op = |o: CategoryOp, l: &mut SessionCategories| apply_category_op(l, &o);
  let file = |cwd: &str, section: Option<&str>| CategoryOp::SetProjectSection { cwd: cwd.into(), section: section.map(Into::into), before: None };
  let create = |id: &str, name: &str, project: Option<&str>| CategoryOp::CreateSection { id: id.into(), name: name.into(), project: project.map(Into::into) };
  op(create("w", " Work ", None), &mut list);
  // Made from a project's menu: the project goes in with it
  op(create("p", "", Some("/z")), &mut list);
  // A second create with a taken id (two windows) keeps the first
  op(create("w", "Other", None), &mut list);
  assert_eq!(list.sections.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Work", crate::i18n::t("session.section.untitled").as_str()]);
  for o in [file("/a", Some("w")), file("/b", Some("w")), file("/a", Some("p")), file("/c", Some("gone")), file("", Some("w"))] {
    op(o, &mut list);
  }
  let held = |l: &SessionCategories, id: &str| l.sections.iter().find(|s| s.id == id).map(|s| s.projects.clone()).unwrap_or_default();
  // One section per project: filing /a under p took it out of w; a section that is gone files nothing
  assert_eq!(held(&list, "w"), ["/b"]);
  assert_eq!(held(&list, "p"), ["/z", "/a"]);
  // A drag places it: before a project of the section, re-placed within its own one, appended past a gone anchor
  let place = |cwd: &str, section: &str, before: &str| CategoryOp::SetProjectSection { cwd: cwd.into(), section: Some(section.into()), before: Some(before.into()) };
  op(place("/b", "p", "/a"), &mut list);
  assert_eq!(held(&list, "p"), ["/z", "/b", "/a"]);
  op(place("/a", "p", "/z"), &mut list);
  assert_eq!(held(&list, "p"), ["/a", "/z", "/b"]);
  op(place("/b", "w", "/gone"), &mut list);
  assert_eq!(held(&list, "w"), ["/b"]);
  assert_eq!(held(&list, "p"), ["/a", "/z"]);
  op(CategoryOp::ReorderSection { id: "p".into(), before: Some("w".into()) }, &mut list);
  op(CategoryOp::CollapseSection { id: "w".into(), collapsed: true }, &mut list);
  op(CategoryOp::RenameSection { id: "p".into(), name: "Personal".into() }, &mut list);
  assert_eq!(list.sections.iter().map(|s| (s.name.as_str(), s.collapsed)).collect::<Vec<_>>(), [("Personal", None), ("Work", Some(true))]);
  op(file("/b", None), &mut list);
  assert!(held(&list, "w").is_empty());
  // Off the list is out of its section; deleting a section leaves its projects under Projects
  op(CategoryOp::RemoveProject { cwd: "/a".into() }, &mut list);
  assert_eq!(held(&list, "p"), ["/z"]);
  op(CategoryOp::DeleteSection { id: "w".into() }, &mut list);
  assert_eq!(list.sections.len(), 1);
  // The file keeps no empty sections key for lists that never had one
  let fresh = serde_json::to_value(SessionCategories::default()).unwrap();
  assert!(fresh.get("sections").is_none(), "{fresh}");
}

fn record(id: &str, updated: &str) -> SessionRecord {
  serde_json::from_value(serde_json::json!({ "id": id, "agent": "grok", "cwd": "/w", "title": "t", "createdAt": updated, "updatedAt": updated, "turns": [], "controls": { "modes": [], "options": [] }, "commands": [] })).unwrap()
}

fn store(dir: &Path) -> Arc<TranscriptStore> {
  TranscriptStore::with_timing(dir.to_path_buf(), Arc::new(|_| {}), None, Duration::from_millis(20), Duration::from_millis(60))
}

/// A store whose database keeps compatibility files: a machine that had sessions before the database
fn stubbed(dir: &Path) -> Arc<TranscriptStore> {
  drop(SessionDb::open_stubbed(dir).unwrap());
  store(dir)
}

#[tokio::test]
async fn an_unavailable_database_keeps_the_file_backend_usable() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir(dir.path().join(crate::store::db::DB_FILE)).unwrap();
  let messages = Arc::new(parking_lot::Mutex::new(Vec::new()));
  let logged = messages.clone();
  let s = TranscriptStore::with_timing(
    dir.path().to_path_buf(),
    Arc::new(move |message| logged.lock().push(message.to_owned())),
    None,
    Duration::from_millis(20),
    Duration::from_millis(60),
  );
  assert!(s.database().is_none());
  s.flush(Arc::new(record("files", "2026-01-01T00:00:00.000Z"))).await.unwrap();
  assert_eq!(s.load("files").await.unwrap().id, "files");
  s.load_index().await.unwrap();
  assert!(dir.path().join("index.json").exists(), "the fallback still maintains the legacy index");
  assert!(messages.lock().iter().any(|message| message.contains("using record files")));
}

#[tokio::test]
async fn save_debounces_and_index_reconciles() {
  let dir = tempfile::tempdir().unwrap();
  let s = stubbed(dir.path());
  s.save(Arc::new(record("a", "2026-01-01T00:00:00.000Z")));
  s.save(Arc::new(record("b", "2026-01-02T00:00:00.000Z")));
  assert!(s.load("a").await.is_some(), "pending record is served from memory");
  // The debounce is 20 ms; the first save also opens the database, which a loaded machine takes its time over
  for _ in 0..200 {
    if dir.path().join("a.json").exists() && dir.path().join("b.json").exists() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
  assert!(dir.path().join("a.json").exists());
  let idx = s.load_index().await.unwrap();
  assert_eq!(idx.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
  assert!(!dir.path().join("index.json").exists(), "the SQLite catalogue replaces index.json");
  s.trash("a").await.unwrap();
  assert_eq!(s.load_index().await.unwrap().len(), 1);
  s.restore("a").await.unwrap();
  assert_eq!(s.load_index().await.unwrap().len(), 2);
  // Deleted by another window: never written back
  std::fs::remove_file(dir.path().join("b.json")).unwrap();
  s.flush(Arc::new(record("b", "2026-01-03T00:00:00.000Z"))).await.unwrap();
  assert!(!dir.path().join("b.json").exists());
}

#[tokio::test]
async fn turns_an_older_engine_added_to_a_migration_stub_are_recovered() {
  let dir = tempfile::tempdir().unwrap();
  let s = stubbed(dir.path());
  let mut stored = record("legacy", "2026-01-01T00:00:00.000Z");
  stored.turns = serde_json::from_value(serde_json::json!([{ "role": "user", "text": "before migration" }])).unwrap();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let path = dir.path().join("legacy.json");
  let mut stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
  stub.turns.push(serde_json::from_value(serde_json::json!({ "role": "user", "text": "added by old engine" })).unwrap());
  stub.subagents = Some(vec![serde_json::from_value(serde_json::json!({ "id": "old-child", "turnIndex": 1, "visibility": "session", "state": "completed", "stateSource": "agent", "announcedAt": 1, "turns": [] })).unwrap()]);
  std::fs::write(&path, serde_json::to_vec(&stub).unwrap()).unwrap();

  let recovered = s.load("legacy").await.unwrap();
  assert_eq!(recovered.turns.len(), 2);
  assert_eq!(recovered.turns[1].as_user().unwrap().text, "added by old engine");
  assert_eq!(recovered.subagents.as_ref().unwrap()[0].core.id, "old-child");
  s.flush(Arc::new(recovered.clone())).await.unwrap();
  assert_eq!(s.database().unwrap().load_record("legacy").unwrap().unwrap().turns, recovered.turns);
  let stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
  assert_eq!(stub.turns.len(), 1, "the next save puts the compatibility placeholder back");
}

#[tokio::test]
async fn reverting_writes_a_complete_file_before_dropping_database_rows() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("revert", "2026-01-01T00:00:00.000Z");
  stored.turns = serde_json::from_value(serde_json::json!([{ "role": "user", "text": "complete on disk" }])).unwrap();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  assert!(s.revert_to_file("revert").await.unwrap());
  let file: SessionRecord = serde_json::from_slice(&std::fs::read(dir.path().join("revert.json")).unwrap()).unwrap();
  assert_eq!(file, stored);
  assert_eq!(s.database().unwrap().body_location("revert").unwrap(), Some(BodyLocation::File));
  assert_eq!(s.load("revert").await.unwrap(), stored);
  assert!(!s.revert_to_file("revert").await.unwrap());
}

#[tokio::test]
async fn a_quarantined_database_falls_back_to_the_migration_archive() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("quarantine", "2026-01-01T00:00:00.000Z");
  stored.turns = serde_json::from_value(serde_json::json!([{ "role": "user", "text": "archived" }])).unwrap();
  // A record from before the database: its first save here archives it
  std::fs::write(dir.path().join("quarantine.json"), serde_json::to_vec(&stored).unwrap()).unwrap();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  s.maintain_database().await.unwrap();
  s.quarantine_database().await.unwrap();
  assert!(s.database().is_none());
  assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("sessions.db.corrupt-")));
  assert_eq!(s.load("quarantine").await.unwrap(), stored);
}

#[tokio::test]
async fn saving_migrates_then_only_refreshes_the_small_compatibility_stub() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("migrate", "2026-01-01T00:00:00.000Z");
  stored.turns = serde_json::from_value(serde_json::json!([
    { "role": "user", "text": "hello" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "kept in SQLite" }] }
  ]))
  .unwrap();
  let path = dir.path().join("migrate.json");
  std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
  s.flush(Arc::new(stored.clone())).await.unwrap();

  let stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
  assert!(stub.turns.iter().any(migration::is_placeholder));
  let archive = dir.path().join("migrate").join(migration::MIGRATED_FILE);
  let archived = zstd::stream::decode_all(std::fs::File::open(&archive).unwrap()).unwrap();
  assert_eq!(serde_json::from_slice::<SessionRecord>(&archived).unwrap(), stored);
  assert_eq!(s.load("migrate").await.unwrap(), stored);
  assert!(s.database().unwrap().catalog().unwrap().iter().any(|entry| entry.summary.id == "migrate" && entry.body == BodyLocation::Database));

  let archive_before = std::fs::read(&archive).unwrap();
  stored.title = "new title".into();
  stored.updated_at = "2026-01-02T00:00:00.000Z".into();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  assert_eq!(std::fs::read(&archive).unwrap(), archive_before, "later saves never rewrite the pre-migration archive");
  let stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
  assert_eq!(stub.title, "new title");
  assert!(stub.turns.iter().any(migration::is_placeholder));
}

#[tokio::test]
async fn database_bodies_share_the_normal_record_read_path() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("database", "2026-01-01T00:00:00.000Z");
  stored.turns = serde_json::from_value(serde_json::json!([
    { "role": "user", "text": "hello" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "from SQLite" }] }
  ]))
  .unwrap();
  s.database().unwrap().save_record(&stored).unwrap();
  std::fs::write(dir.path().join("database.json"), migration::stub(&RecordParts::of(stored.clone()).head, &[]).unwrap()).unwrap();
  assert_eq!(s.load("database").await.unwrap(), stored);
  assert_eq!(s.load_controls("database").await.unwrap(), stored.controls);
  assert_eq!(s.load_commands("database").await.unwrap(), stored.commands);
  assert_eq!(s.load_index().await.unwrap().iter().map(|summary| summary.id.as_str()).collect::<Vec<_>>(), ["database"]);
}

#[tokio::test]
async fn migrated_sessions_keep_their_undo_window_in_the_catalogue() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  s.flush(Arc::new(record("undo", "2026-01-01T00:00:00.000Z"))).await.unwrap();
  s.trash("undo").await.unwrap();
  assert!(s.load_index().await.unwrap().is_empty());
  assert!(s.database().unwrap().catalog().unwrap()[0].trashed_at.is_some());
  s.restore("undo").await.unwrap();
  assert_eq!(s.load_index().await.unwrap()[0].id, "undo");
  assert_eq!(s.database().unwrap().catalog().unwrap()[0].trashed_at, None);
  s.trash("undo").await.unwrap();
  s.sweep_trash(Duration::ZERO).await;
  assert!(s.database().unwrap().catalog().unwrap().is_empty());
}

#[tokio::test]
async fn a_fresh_store_keeps_its_sessions_in_the_database_alone() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("fresh", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["hello"]);
  s.flush(Arc::new(stored.clone())).await.unwrap();
  assert!(s.database().unwrap().stubless());
  assert!(!dir.path().join("fresh.json").exists(), "no compatibility file");
  assert_eq!(s.load("fresh").await.unwrap(), stored);
  assert!(s.has_record("fresh").await);
  let stamp = s.record_stamp("fresh").await.unwrap();
  assert_eq!(s.load_index().await.unwrap().iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["fresh"]);
  assert!(!dir.path().join("index.json").exists());
  assert!(s.search_sources().await.iter().any(|source| source.id == "fresh" && source.in_database && source.stamp.0.is_some()));

  // Another engine on the same store sees it, and sees it change
  let other = store(dir.path());
  assert_eq!(other.load_index().await.unwrap()[0].id, "fresh");
  stored.title = "renamed".into();
  stored.updated_at = "2026-01-02T00:00:00.000Z".into();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  assert_ne!(other.record_stamp("fresh").await.unwrap(), stamp, "a mirror polling it sees the save");
  assert_eq!(other.load_index().await.unwrap()[0].title, "renamed");

  // Trashed by the other engine: hidden everywhere, never written back by this one, back on undo
  std::fs::create_dir_all(dir.path().join("fresh")).unwrap();
  std::fs::write(dir.path().join("fresh").join("blob.png"), b"png").unwrap();
  other.trash("fresh").await.unwrap();
  assert!(dir.path().join(TRASH_DIR).join("fresh").join("blob.png").exists(), "the blob folder waits in the trash");
  assert!(s.load_index().await.unwrap().is_empty());
  assert!(s.load_detailed("fresh").await.unwrap_err().is_missing());
  assert!(!s.has_record("fresh").await && s.record_stamp("fresh").await.is_none());
  s.flush(Arc::new(stored.clone())).await.unwrap();
  assert!(s.load_index().await.unwrap().is_empty(), "a trashed session is not written back");
  assert!(s.copy_stamps(&["fresh".into()]).await.is_empty());
  other.restore("fresh").await.unwrap();
  assert_eq!(s.load_index().await.unwrap()[0].id, "fresh");
  assert!(dir.path().join("fresh").join("blob.png").exists());

  // The undo window closes: the row and the blob folder go together
  s.trash("fresh").await.unwrap();
  s.sweep_trash(Duration::from_secs(3600)).await;
  assert_eq!(s.database().unwrap().catalog().unwrap().len(), 1, "a fresh trash entry is still in its window");
  s.sweep_trash(Duration::ZERO).await;
  assert!(s.database().unwrap().catalog().unwrap().is_empty());
  assert!(!dir.path().join(TRASH_DIR).join("fresh").exists());

  // A whole record an older engine left here moves into the rows on its first save, and its file goes
  let legacy = record("legacy", "2026-01-03T00:00:00.000Z");
  std::fs::write(dir.path().join("legacy.json"), serde_json::to_vec(&legacy).unwrap()).unwrap();
  assert_eq!(s.load_index().await.unwrap()[0].id, "legacy");
  s.flush(Arc::new(s.load("legacy").await.unwrap())).await.unwrap();
  assert!(!dir.path().join("legacy.json").exists());
  assert!(!dir.path().join("legacy").join(migration::MIGRATED_FILE).exists(), "nothing to fall back to, nothing archived");
  assert_eq!(s.load("legacy").await.unwrap(), legacy);
  assert_eq!(s.load_index().await.unwrap()[0].id, "legacy");

  // A trashed record file whose session was restored and saved elsewhere since: the sweep takes the file, never the body
  std::fs::create_dir_all(dir.path().join(TRASH_DIR)).unwrap();
  std::fs::write(dir.path().join(TRASH_DIR).join("legacy.json"), serde_json::to_vec(&legacy).unwrap()).unwrap();
  s.sweep_trash(Duration::ZERO).await;
  assert!(!dir.path().join(TRASH_DIR).join("legacy.json").exists());
  assert_eq!(s.load("legacy").await.unwrap(), legacy);

  // An undo that finds the session swept already has nothing to bring back
  s.trash("legacy").await.unwrap();
  s.sweep_trash(Duration::ZERO).await;
  s.restore("legacy").await.unwrap();
  assert!(s.load_index().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_catalogue_reconciles_records_written_by_an_older_engine() {
  let dir = tempfile::tempdir().unwrap();
  let original = record("a", "2026-01-01T00:00:00.000Z");
  std::fs::write(dir.path().join("a.json"), serde_json::to_vec(&original).unwrap()).unwrap();
  std::fs::write(dir.path().join("index.json"), serde_json::to_string_pretty(&[original.summary()]).unwrap()).unwrap();
  let s = store(dir.path());
  assert_eq!(s.load_index().await.unwrap()[0].title, "t");
  let legacy_index = std::fs::read(dir.path().join("index.json")).unwrap();

  let mut changed = record("a", "2026-01-03T00:00:00.000Z");
  changed.title = "written by an old engine".into();
  std::fs::write(dir.path().join("a.json"), serde_json::to_vec(&changed).unwrap()).unwrap();
  assert_eq!(s.load_index().await.unwrap()[0].title, "written by an old engine");
  assert_eq!(std::fs::read(dir.path().join("index.json")).unwrap(), legacy_index, "the old cache is not rewritten");

  std::fs::write(dir.path().join("b.json"), serde_json::to_vec(&record("b", "2026-01-02T00:00:00.000Z")).unwrap()).unwrap();
  let listed = s.load_index().await.unwrap();
  assert_eq!(listed.iter().map(|summary| summary.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);

  // A second engine can use the stamps and summaries already in SQLite without trusting the stale legacy index.
  let next = store(dir.path());
  let listed = next.load_index().await.unwrap();
  assert_eq!(listed.iter().find(|summary| summary.id == "a").unwrap().title, "written by an old engine");
  assert!(listed.iter().any(|summary| summary.id == "b"));
  let catalogue = next.database().unwrap().catalog().unwrap();
  assert!(catalogue.iter().all(|entry| entry.body == BodyLocation::File && entry.file_size.is_some()));
  assert_eq!(std::fs::read(dir.path().join("index.json")).unwrap(), legacy_index);
}

#[tokio::test]
async fn index_entries_without_a_model_learn_it_from_their_record() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut r = serde_json::to_value(record("a", "2026-01-01T00:00:00.000Z")).unwrap();
  r["controls"]["options"] = serde_json::json!([{ "id": "model", "name": "Model", "value": "opus", "options": [{ "id": "opus", "name": "Opus 5.5" }] }]);
  std::fs::write(dir.path().join("a.json"), r.to_string()).unwrap();
  // An index an older engine wrote: the entry is there, its model is not
  let mut entry = serde_json::to_value(serde_json::from_value::<SessionRecord>(r).unwrap().summary()).unwrap();
  entry.as_object_mut().unwrap().remove("model");
  std::fs::write(dir.path().join("index.json"), serde_json::json!([entry]).to_string()).unwrap();
  assert_eq!(s.load_index().await.unwrap()[0].model, None, "the entry is taken as it is");
  assert!(s.read_models(&["a".into()]).await);
  assert_eq!(s.load_index().await.unwrap()[0].model.as_ref().map(|m| m.name.as_str()), Some("Opus 5.5"));
}

fn builtin(id: &str, acp: &str) -> SessionRecord {
  let mut r = record(id, "2026-01-01T00:00:00.000Z");
  r.agent = SELF_AGENT_ID.into();
  r.acp_session_id = Some(acp.into());
  r
}

/// A data root with `sessions/` for the store and the built-in agent's logs under `agent/sessions/`
fn rooted() -> (tempfile::TempDir, Arc<TranscriptStore>, impl Fn(&str) -> bool) {
  let root = tempfile::tempdir().unwrap();
  let s = store(&root.path().join("sessions"));
  s.set_agent_home(root.path().to_path_buf());
  let home = root.path().to_path_buf();
  let log_exists = move |acp: &str| {
    let [file, dir] = acpira_agent::acp::session_paths(&home, acp);
    assert_eq!(file.exists(), dir.exists(), "log and folder go together");
    file.exists()
  };
  (root, s, log_exists)
}

fn write_log(root: &Path, acp: &str) {
  let [file, dir] = acpira_agent::acp::session_paths(root, acp);
  std::fs::create_dir_all(dir.join("outputs")).unwrap();
  std::fs::write(dir.join("plan.md"), b"plan").unwrap();
  std::fs::write(file, b"{}\n").unwrap();
}

#[tokio::test]
async fn removing_a_builtin_session_for_good_takes_its_log_along() {
  let (root, s, log_exists) = rooted();
  write_log(root.path(), "acp-1");
  s.flush(Arc::new(builtin("a", "acp-1"))).await.unwrap();
  s.load_index().await.unwrap();
  // The undo window keeps it; a restore finds the log untouched
  s.trash("a").await.unwrap();
  s.restore("a").await.unwrap();
  assert!(log_exists("acp-1"));
  s.trash("a").await.unwrap();
  s.load_index().await.unwrap();
  s.remove("a").await;
  assert!(!log_exists("acp-1"));
  assert!(root.path().join("agent/sessions").exists(), "only the session's own files go");
}

#[tokio::test]
async fn a_log_another_record_still_drives_is_kept() {
  let (root, s, log_exists) = rooted();
  write_log(root.path(), "shared");
  s.flush(Arc::new(builtin("a", "shared"))).await.unwrap();
  s.flush(Arc::new(builtin("b", "shared"))).await.unwrap();
  // Another CLI's session whose acp id happens to match is not the built-in agent's log
  write_log(root.path(), "grok-1");
  let mut other = record("g", "2026-01-01T00:00:00.000Z");
  other.acp_session_id = Some("grok-1".into());
  s.flush(Arc::new(other)).await.unwrap();
  s.load_index().await.unwrap();
  s.trash("a").await.unwrap();
  s.load_index().await.unwrap();
  s.remove("a").await;
  assert!(log_exists("shared"), "b is still listed");
  // b in its own undo window still counts
  s.trash("b").await.unwrap();
  s.flush(Arc::new(builtin("c", "shared"))).await.unwrap();
  s.trash("c").await.unwrap();
  s.load_index().await.unwrap();
  s.remove("c").await;
  assert!(log_exists("shared"), "b may still be undone");
  s.remove("b").await;
  assert!(!log_exists("shared"));
  s.remove("g").await;
  assert!(log_exists("grok-1"));
}

#[tokio::test]
async fn a_crashed_hosts_trash_sweep_takes_the_log_along() {
  let (root, s, log_exists) = rooted();
  write_log(root.path(), "acp-2");
  s.flush(Arc::new(builtin("a", "acp-2"))).await.unwrap();
  s.trash("a").await.unwrap();
  // Another host after the crash: no summary in memory, only the trashed record
  let next = store(&root.path().join("sessions"));
  next.set_agent_home(root.path().to_path_buf());
  next.sweep_trash(Duration::from_secs(3600)).await;
  assert!(log_exists("acp-2"), "a fresh trash entry is still in its window");
  next.sweep_trash(Duration::ZERO).await;
  assert!(!log_exists("acp-2"));
  assert!(!root.path().join("sessions/trash/a.json").exists());
}

#[tokio::test]
async fn the_scratch_folder_goes_with_the_last_record_driving_the_session() {
  let (root, s, log_exists) = rooted();
  let project = tempfile::tempdir().unwrap();
  let acp = "abcdef12-0000-0000-0000-000000000001";
  write_log(root.path(), acp);
  let [file, _] = acpira_agent::acp::session_paths(root.path(), acp);
  std::fs::write(&file, format!("{}\n", serde_json::json!({ "type": "session", "id": acp, "cwd": project.path() }))).unwrap();
  let folder = project.path().join(".acpira/tmp/abcdef12");
  acpira_agent::scratch::Scratch::new(Some(folder.clone()), acp, root.path()).ensure().unwrap();
  s.flush(Arc::new(builtin("a", acp))).await.unwrap();
  s.flush(Arc::new(builtin("b", acp))).await.unwrap();
  s.load_index().await.unwrap();
  s.trash("a").await.unwrap();
  s.load_index().await.unwrap();
  s.remove("a").await;
  assert!(folder.exists(), "b still drives the session");
  s.trash("b").await.unwrap();
  s.load_index().await.unwrap();
  s.remove("b").await;
  assert!(!log_exists(acp));
  assert!(!project.path().join(".acpira").exists(), "no empty shell is left in the project");
}

#[tokio::test]
async fn blobs_are_content_named_and_confined() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let (name, path) = s.save_blob("s1", ".png", b"abc").await.unwrap();
  assert_eq!(name, format!("{}.png", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".get(..16).unwrap()));
  assert!(path.exists());
  assert_eq!(s.read_blob("s1", &name).await.unwrap(), b"abc");
  assert!(s.save_blob("../x", ".png", b"a").await.is_err());
  assert!(s.blob_path("s1", "../../etc").is_none());
}
fn turns(texts: &[&str]) -> Vec<acpira_shared::transcript::Turn> {
  serde_json::from_value(texts.iter().map(|text| serde_json::json!({ "role": "user", "text": text })).collect()).unwrap()
}

fn item_rows(s: &TranscriptStore, id: &str) -> i64 {
  let db = rusqlite::Connection::open(s.dir().join(crate::store::db::DB_FILE)).unwrap();
  db.query_row("SELECT count(*) FROM item WHERE session_id = ?1", [id], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn a_reconcile_never_takes_a_database_body() {
  let dir = tempfile::tempdir().unwrap();
  let s = stubbed(dir.path());
  let mut stored = record("kept", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["one", "two"]);
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let rows = item_rows(&s, "kept");
  assert!(rows >= 3);

  // The compatibility file is gone (an older engine deleted it, or it never made it to disk): the session leaves the
  // list, its body stays
  let path = dir.path().join("kept.json");
  let stub = std::fs::read(&path).unwrap();
  std::fs::remove_file(&path).unwrap();
  let next = store(dir.path());
  assert!(next.load_index().await.unwrap().is_empty());
  assert_eq!(item_rows(&next, "kept"), rows);
  assert!(next.database().unwrap().entry("kept").unwrap().unwrap().trashed_at.is_some());
  // Routine upkeep keeps it too while it is not long gone
  next.maintain_database().await.unwrap();
  assert_eq!(item_rows(&next, "kept"), rows);
  // Back again: listed and whole
  std::fs::write(&path, &stub).unwrap();
  assert_eq!(next.load_index().await.unwrap()[0].id, "kept");
  assert_eq!(next.load("kept").await.unwrap(), stored);

  // A row that does not read back (a hash that no longer fits) with the stub touched: still listed, nothing dropped
  rusqlite::Connection::open(dir.path().join(crate::store::db::DB_FILE)).unwrap().execute("UPDATE item SET hash = zeroblob(16) WHERE session_id = 'kept' AND turn = 0", []).unwrap();
  std::fs::write(&path, [stub.as_slice(), b" "].concat()).unwrap();
  let third = store(dir.path());
  assert!(third.load_detailed("kept").await.is_err());
  assert_eq!(third.load_index().await.unwrap()[0].id, "kept");
  assert_eq!(item_rows(&third, "kept"), rows);
}

#[tokio::test]
async fn a_store_without_the_database_never_buries_a_database_body() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("forked.json");
  let mut stored = record("forked", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["before the database"]);
  std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
  {
    let s = store(dir.path());
    let mut live = s.load("forked").await.unwrap();
    live.turns.extend(turns(&["after the database"]));
    live.updated_at = "2026-01-02T00:00:00.000Z".into();
    s.flush(Arc::new(live)).await.unwrap();
  }
  // The database cannot be opened this time
  let db = dir.path().join(crate::store::db::DB_FILE);
  let aside = dir.path().join("aside");
  std::fs::create_dir(&aside).unwrap();
  for suffix in ["", "-wal", "-shm"] {
    let _ = std::fs::rename(dir.path().join(format!("sessions.db{suffix}")), aside.join(format!("sessions.db{suffix}")));
  }
  std::fs::create_dir(&db).unwrap();
  let stub = std::fs::read(&path).unwrap();
  {
    let s = store(dir.path());
    assert!(s.database().is_none());
    // What it can show is the session as it was archived; writing that back is refused, the stub stays
    let mut old = s.load("forked").await.unwrap();
    assert_eq!(old.turns.len(), 1);
    old.turns.extend(turns(&["typed without the database"]));
    old.updated_at = "2026-01-03T00:00:00.000Z".into();
    assert!(s.flush(Arc::new(old)).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), stub);
  }
  std::fs::remove_dir(&db).unwrap();
  for suffix in ["", "-wal", "-shm"] {
    let _ = std::fs::rename(aside.join(format!("sessions.db{suffix}")), dir.path().join(format!("sessions.db{suffix}")));
  }
  let s = store(dir.path());
  let back = s.load("forked").await.unwrap();
  assert_eq!(back.turns.len(), 2);
  assert_eq!(back.turns[1].as_user().unwrap().text, "after the database");
}

#[tokio::test]
async fn a_whole_record_file_newer_than_the_database_body_wins() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("newer", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["in the database"]);
  s.flush(Arc::new(stored.clone())).await.unwrap();
  // A file backend wrote the whole session over the stub later on
  let mut whole = stored.clone();
  whole.turns.extend(turns(&["written as a file"]));
  whole.updated_at = "2026-01-02T00:00:00.000Z".into();
  std::fs::write(dir.path().join("newer.json"), serde_json::to_vec(&whole).unwrap()).unwrap();
  let next = store(dir.path());
  assert_eq!(next.load("newer").await.unwrap(), whole);
  assert_eq!(next.database().unwrap().body_location("newer").unwrap(), Some(BodyLocation::File));
  assert_eq!(item_rows(&next, "newer"), 0);
}

#[tokio::test]
async fn the_stub_follows_lazily_and_the_body_never_reads_stale() {
  let dir = tempfile::tempdir().unwrap();
  drop(SessionDb::open_stubbed(dir.path()).unwrap());
  let s = TranscriptStore::with_timing(dir.path().to_path_buf(), Arc::new(|_| {}), None, Duration::from_millis(5), Duration::from_millis(20));
  let path = dir.path().join("lazy.json");
  let mut stored = record("lazy", "2026-01-01T00:00:00.000Z");
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let first = std::fs::read(&path).unwrap();
  // A debounced save right after: the body has the new title, the stub is left as it is for now
  stored.title = "renamed".into();
  stored.updated_at = "2026-01-02T00:00:00.000Z".into();
  s.save(Arc::new(stored.clone()));
  for _ in 0..200 {
    if s.database().unwrap().entry("lazy").unwrap().is_some_and(|entry| entry.summary.title == "renamed") {
      break;
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
  s.flush_pending().await;
  assert_eq!(std::fs::read(&path).unwrap(), first);
  assert_eq!(s.load("lazy").await.unwrap(), stored);
  assert_eq!(store(dir.path()).load("lazy").await.unwrap(), stored, "another store reads the body, not the old stub");
  assert_eq!(s.load_index().await.unwrap()[0].title, "renamed");
  // A blob joining the folder is named by the very next save: an older engine's cleanup goes by the stub
  let (blob, _) = s.save_blob("lazy", ".png", b"image").await.unwrap();
  s.save(Arc::new(stored.clone()));
  s.flush_pending().await;
  let text = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
  assert!(text.contains(&blob) && text.contains("renamed"));
}

#[tokio::test]
async fn a_session_that_went_back_to_its_file_archives_anew_when_it_migrates_again() {
  let dir = tempfile::tempdir().unwrap();
  let s = store(dir.path());
  let mut stored = record("again", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["first"]);
  std::fs::write(dir.path().join("again.json"), serde_json::to_vec(&stored).unwrap()).unwrap();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let archive = dir.path().join("again").join(migration::MIGRATED_FILE);
  let first = std::fs::read(&archive).unwrap();
  stored.turns = turns(&["first", "second"]);
  stored.updated_at = "2026-01-02T00:00:00.000Z".into();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  assert!(s.revert_to_file("again").await.unwrap());
  assert!(!archive.exists(), "a whole record has nothing older to fall back to");
  // One left behind all the same (a crash right after the file was written) is not taken for this migration's
  std::fs::write(&archive, &first).unwrap();
  stored.turns = turns(&["first", "second", "third"]);
  stored.updated_at = "2026-01-03T00:00:00.000Z".into();
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let archived: SessionRecord = serde_json::from_slice(&zstd::stream::decode_all(std::fs::File::open(&archive).unwrap()).unwrap()).unwrap();
  assert_eq!(archived.turns, turns(&["first", "second"]), "the fallback is the record as it went in this time");
}

#[tokio::test]
async fn a_trash_sweep_leaves_the_body_of_a_session_that_is_listed_again() {
  let dir = tempfile::tempdir().unwrap();
  let s = stubbed(dir.path());
  let mut stored = record("back", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["kept"]);
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let rows = item_rows(&s, "back");
  s.trash("back").await.unwrap();
  // Another engine's undo between the sweep's look at the trash and its removal: the record is listed again
  std::fs::copy(dir.path().join(TRASH_DIR).join("back.json"), dir.path().join("back.json")).unwrap();
  s.sweep_trash(Duration::ZERO).await;
  assert!(!dir.path().join(TRASH_DIR).join("back.json").exists());
  assert_eq!(item_rows(&s, "back"), rows);
  assert_eq!(store(dir.path()).load("back").await.unwrap(), stored);
}

#[tokio::test]
async fn a_save_lists_a_body_a_reconcile_had_hidden() {
  let dir = tempfile::tempdir().unwrap();
  let s = stubbed(dir.path());
  let mut stored = record("hidden", "2026-01-01T00:00:00.000Z");
  s.flush(Arc::new(stored.clone())).await.unwrap();
  s.database().unwrap().set_trashed("hidden", Some(1)).unwrap();
  assert!(s.search_sources().await.iter().all(|source| !source.in_database));
  stored.updated_at = "2026-01-02T00:00:00.000Z".into();
  s.flush(Arc::new(stored)).await.unwrap();
  assert_eq!(s.database().unwrap().entry("hidden").unwrap().unwrap().trashed_at, None);
  assert!(s.search_sources().await.iter().any(|source| source.id == "hidden" && source.in_database));
}

#[tokio::test]
async fn a_list_edit_an_older_engine_made_on_the_stub_is_kept() {
  let dir = tempfile::tempdir().unwrap();
  let s = stubbed(dir.path());
  let mut stored = record("renamed", "2026-01-01T00:00:00.000Z");
  stored.turns = turns(&["kept"]);
  s.flush(Arc::new(stored.clone())).await.unwrap();
  let path = dir.path().join("renamed.json");
  // The older engine reads the stub as a record, edits it and writes it back: `updatedAt` stays as it was
  let mut stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
  stub.title = "renamed by an old engine".into();
  stub.renamed = true;
  stub.pinned = Some(true);
  std::fs::write(&path, serde_json::to_vec(&stub).unwrap()).unwrap();
  let loaded = s.load("renamed").await.unwrap();
  assert_eq!((loaded.title.as_str(), loaded.renamed, loaded.pinned), ("renamed by an old engine", true, Some(true)));
  assert_eq!(loaded.turns, stored.turns);
  assert_eq!(s.load_index().await.unwrap()[0].title, "renamed by an old engine");
  // Saved from there, the body has the edit and the stub is this store's again
  s.flush(Arc::new(loaded)).await.unwrap();
  assert_eq!(s.database().unwrap().load_record("renamed").unwrap().unwrap().title, "renamed by an old engine");
  assert!(!migration::rewritten(&std::fs::read(&path).unwrap()));
}
