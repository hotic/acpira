//! A project shell (`ShellEnv.projects`, the desktop app): no single workspace, sessions in any registered folder

use super::*;

fn projects_of(m: &Mgr) -> Value {
  serde_json::to_value(m.m.categories()).unwrap()["projects"].clone()
}

fn removed_of(m: &Mgr) -> Value {
  serde_json::to_value(m.m.categories()).unwrap()["removedProjects"].clone()
}

fn cwd_of(m: &Mgr) -> Value {
  m.active().map(|s| s["cwd"].clone()).unwrap_or(Value::Null)
}

fn folder(root: &Path, name: &str) -> String {
  let p = root.join(name);
  std::fs::create_dir_all(&p).unwrap();
  p.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_shell_starts_nothing_without_a_project_and_new_sessions_follow_the_view() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let home = folder(dir.path(), "home");
  let (p1, p2) = (folder(dir.path(), "p1"), folder(dir.path(), "p2"));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake).cwd(&home).projects());
  m.init().await;

  // A fresh install: no session in the home directory, the page is told to show its empty state
  m.m.ensure_active_for(&m.v).await;
  assert_eq!(m.active_id(), None);
  assert_eq!(m.m.folder_for(Some(&m.v), None), None);
  assert!(m.session_ids().is_empty());
  assert!(projects_of(&m).is_null(), "nothing to seed from");
  m.handle(json!({ "type": "newSession" })).await;
  assert_eq!(m.active_id(), None, "a new session needs a project");

  // Open Folder: the page registers the folder, then starts a session there
  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": p1 })).await;
  m.handle(json!({ "type": "newSession", "cwd": p1 })).await;
  assert_eq!(cwd_of(&m), p1.as_str());
  let empty = m.active_id().unwrap();
  // Switching an empty session to another project replaces it instead of leaving a blank entry; the folder joins the list
  m.handle(json!({ "type": "newSession", "cwd": p2 })).await;
  assert_eq!(cwd_of(&m), p2.as_str());
  assert!(!m.session_ids().contains(&empty));
  assert_eq!(projects_of(&m), json!([p1, p2]));

  // Without a folder a new session opens where the shown session is, and the settings page / @ search mean it too
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let in_p2 = m.active_id().unwrap();
  m.handle(json!({ "type": "newSession" })).await;
  assert_eq!(cwd_of(&m), p2.as_str());
  assert_ne!(m.active_id().unwrap(), in_p2);
  // A second window that shows nothing yet means the newest session's folder
  let other = m.m.attach(None);
  assert_eq!(m.m.folder_for(Some(&other), None).as_deref(), Some(p2.as_str()));

  // "New session in this category" starts in the category's project
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c1", "name": "c1", "cwd": p1 })).await;
  m.handle(json!({ "type": "newSession", "category": "c1" })).await;
  assert_eq!(cwd_of(&m), p1.as_str());
  let filed = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "filed" })).await;
  until(|| m.sessions().iter().any(|s| s["id"] == filed.as_str() && s["category"] == "c1"), 2000).await;

  // A folder that is gone is refused with its path
  let gone = dir.path().join("gone").to_string_lossy().into_owned();
  m.handle(json!({ "type": "newSession", "cwd": gone })).await;
  assert_eq!(cwd_of(&m), p1.as_str());
  assert!(m.toasts().iter().any(|t| t.contains(&gone)), "{:?}", m.toasts());

  // Reordering and removal edit the list only; sessions stay, and the removal is remembered so the page stops listing
  // the folder for them
  m.handle(json!({ "type": "categoryOp", "op": "reorderProject", "cwd": p2, "before": p1 })).await;
  assert_eq!(projects_of(&m), json!([p2, p1]));
  m.handle(json!({ "type": "categoryOp", "op": "removeProject", "cwd": p2 })).await;
  assert_eq!(projects_of(&m), json!([p1]));
  assert_eq!(removed_of(&m), json!([p2]));
  assert!(m.session_ids().contains(&in_p2));
  // A session started there brings the folder back
  m.handle(json!({ "type": "newSession", "cwd": p2 })).await;
  assert_eq!(cwd_of(&m), p2.as_str());
  assert_eq!(projects_of(&m), json!([p1, p2]));
  assert!(removed_of(&m).is_null(), "{:?}", removed_of(&m));
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_the_last_session_of_a_project_shell_with_no_project_left_shows_the_empty_state() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let home = folder(dir.path(), "home");
  let p1 = folder(dir.path(), "p1");
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake).cwd(&home).projects());
  m.init().await;
  m.handle(json!({ "type": "newSession", "cwd": p1 })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let id = m.active_id().unwrap();
  m.handle(json!({ "type": "categoryOp", "op": "removeProject", "cwd": p1 })).await;
  m.events.lock().unwrap().clear();
  m.handle(json!({ "type": "deleteSession", "id": id })).await;
  assert_eq!(m.active_id(), None);
  assert!(m.events.lock().unwrap().iter().any(|e| e["type"] == "noSession"), "{:?}", m.events.lock().unwrap());
  assert!(m.session_ids().is_empty(), "no session started in the home directory");
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_shell_seeds_the_project_order_from_the_sessions_newest_first() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let sessions = dir.path().join("sessions");
  let (a, b) = (folder(dir.path(), "a"), folder(dir.path(), "b"));
  // An IDE window ran sessions in two folders (b last) and never wrote a project list
  let ide = Mgr::new(&sessions, Opts::fake(&fake).cwd(&a));
  ide.init().await;
  for cwd in [&a, &b] {
    ide.handle(json!({ "type": "newSession", "cwd": cwd })).await;
    ide.handle(json!({ "type": "send", "text": "hi" })).await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
  ide.m.refresh_index().await;
  assert!(projects_of(&ide).is_null(), "an IDE shell never registers projects");
  ide.dispose().await;

  let desk = Mgr::new(&sessions, Opts::fake(&fake).cwd(dir.path().to_str().unwrap()).projects());
  desk.init().await;
  assert_eq!(projects_of(&desk), json!([b, a]));
  desk.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_shell_starts_new_sessions_in_its_workspace_whatever_session_it_shows() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let (w, other) = (folder(dir.path(), "w"), folder(dir.path(), "other"));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake).cwd(&w));
  m.init().await;
  m.handle(json!({ "type": "newSession", "cwd": other })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  assert_eq!(cwd_of(&m), other.as_str());
  m.handle(json!({ "type": "newSession" })).await;
  assert_eq!(cwd_of(&m), w.as_str());
  assert_eq!(m.m.folder_for(Some(&m.v), None).as_deref(), Some(w.as_str()));
  m.dispose().await;
}

/// `ssh://` folders (docs/dev/remote.md): listed in one spelling, checked on the host, and under local run only the
/// built-in agent starts there
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_on_an_ssh_host_is_a_project_in_one_spelling() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let home = folder(dir.path(), "home");
  let lab = crate::remote_connections::Lab::new();
  let real = lab.path().join("hostfs");
  std::fs::create_dir_all(real.join("work")).unwrap();
  lab.map_root("/acpira-virtual-host", &real);
  let connections = lab.engine("engine");
  connections.op(acpira_shared::remote::ConnectionOp::Add { alias: "gen1".into(), mode: acpira_shared::remote::RunMode::Local }).await.unwrap();
  let mut opts = Opts::fake(&fake).cwd(&home).projects();
  opts.remote = Some(connections.clone());
  let m = Mgr::new(&dir.path().join("sessions"), opts);
  m.init().await;

  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": "ssh://gen1//acpira-virtual-host/./work/" })).await;
  let canonical = "ssh://gen1/acpira-virtual-host/work";
  assert_eq!(projects_of(&m), json!([canonical]));
  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": canonical })).await;
  assert_eq!(projects_of(&m), json!([canonical]), "the same folder once");

  // Missing on the host: refused like a missing local folder
  m.handle(json!({ "type": "newSession", "cwd": "ssh://gen1/acpira-virtual-host/nope" })).await;
  assert!(m.toasts().iter().any(|t| t.contains("nope")), "{:?}", m.toasts());
  assert_eq!(m.active_id(), None);

  // There: the session takes the canonical folder, and an agent whose tools stay here refuses to start in it
  m.handle(json!({ "type": "newSession", "cwd": "ssh://gen1/acpira-virtual-host/work/" })).await;
  assert_eq!(cwd_of(&m), json!(canonical));
  until(|| m.active().is_some_and(|v| v["error"].as_str().is_some_and(|e| e.contains("local run"))), 5000).await;
  connections.dispose();
}
