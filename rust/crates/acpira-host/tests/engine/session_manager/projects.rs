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
  connections.op(acpira_shared::remote::ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let mut opts = Opts::fake(&fake).cwd(&home).projects();
  // The fake agent runs here like the built-in agent would (`sshLocalAgents`), so the folder checks are this engine's
  opts.ssh_local_agents.lock().unwrap().push("fake".into());
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

  // There: the session takes the canonical folder, and an agent that cannot run its tools on a host refuses to start here
  m.handle(json!({ "type": "newSession", "cwd": "ssh://gen1/acpira-virtual-host/work/" })).await;
  assert_eq!(cwd_of(&m), json!(canonical));
  until(|| m.active().is_some_and(|v| v["error"].as_str().is_some_and(|e| e.contains("not added"))), 5000).await;
  connections.dispose();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_runs_in_a_folder_of_its_own_that_is_no_project_and_an_unprompted_one_takes_it_along() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let home = folder(dir.path(), "home");
  let p1 = folder(dir.path(), "p1");
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake).cwd(&home).projects());
  m.init().await;
  let chat_cwd = |m: &Mgr| cwd_of(m).as_str().unwrap_or_default().to_owned();

  // No project needed: a fresh folder directly under the chats root
  m.handle(json!({ "type": "newSession", "chat": true })).await;
  let first = chat_cwd(&m);
  assert!(m.m.is_chat(&first), "{first}");
  assert_eq!(Path::new(&first).parent(), Some(m.m.chat_root().as_path()));
  assert!(Path::new(&first).is_dir());
  // Started over as another chat (another agent on the start page): the empty one's folder goes with it
  m.handle(json!({ "type": "newSession", "chat": true })).await;
  let second = chat_cwd(&m);
  assert_ne!(first, second);
  assert!(!Path::new(&first).exists());

  // Prompted, it stays a chat: the project list never takes its folder, and a window showing nothing does not mean it
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| m.sessions().iter().any(|s| s["cwd"] == second.as_str()), 2000).await;
  assert!(projects_of(&m).is_null(), "{}", projects_of(&m));
  let other = m.m.attach(None);
  assert_eq!(m.m.folder_for(Some(&other), None), None);

  // An empty chat switched to a project on the start page leaves no folder behind; a prompted chat keeps its own
  m.handle(json!({ "type": "newSession", "chat": true })).await;
  let third = chat_cwd(&m);
  m.handle(json!({ "type": "newSession", "cwd": p1 })).await;
  assert_eq!(cwd_of(&m), p1.as_str());
  assert!(!Path::new(&third).exists());
  assert!(Path::new(&second).is_dir());
  assert_eq!(projects_of(&m), json!([p1]));
}

/// A file the page shows from its path in a folder on an SSH host: read there once into the blob store, the copy reused
/// while the file is unchanged, read again once it changed
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_host_file_the_page_shows_is_read_once_until_it_changes() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let lab = crate::remote_connections::Lab::new();
  let real = lab.path().join("hostfs");
  std::fs::create_dir_all(real.join("work")).unwrap();
  lab.map_root("/acpira-virtual-host", &real);
  let connections = lab.engine("engine");
  connections.op(acpira_shared::remote::ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let mut opts = Opts::fake(&fake).cwd(&folder(dir.path(), "home"));
  opts.remote = Some(connections.clone());
  let m = Mgr::new(&dir.path().join("sessions"), opts);
  let ws = acpira_host::workspace::Workspace::open("ssh://gen1/acpira-virtual-host/work", Some(&connections)).await.unwrap();
  let shot = Path::new("/acpira-virtual-host/work/shot.png");
  std::fs::write(real.join("work/shot.png"), b"\x89PNG first").unwrap();

  let first = m.m.stage_file("s1", &ws, shot).await.expect("read from the host");
  assert_eq!(std::fs::read(m.m.blob_path("s1", &first).unwrap()).unwrap(), b"\x89PNG first");
  // Unchanged on the host: the same blob, without another read (the file is unreadable now; its stat still answers)
  use std::os::unix::fs::PermissionsExt;
  std::fs::set_permissions(real.join("work/shot.png"), std::fs::Permissions::from_mode(0o000)).unwrap();
  assert_eq!(m.m.stage_file("s1", &ws, shot).await.as_deref(), Some(first.as_str()));
  std::fs::set_permissions(real.join("work/shot.png"), std::fs::Permissions::from_mode(0o644)).unwrap();
  // Rewritten: read again (the size differs, so a coarse mtime cannot hide it)
  std::fs::write(real.join("work/shot.png"), b"\x89PNG second, longer").unwrap();
  let second = m.m.stage_file("s1", &ws, shot).await.expect("read again");
  assert_ne!(first, second);
  assert_eq!(std::fs::read(m.m.blob_path("s1", &second).unwrap()).unwrap(), b"\x89PNG second, longer");
  // Not a file the page shows, or gone: nothing
  std::fs::write(real.join("work/notes.txt"), b"text").unwrap();
  assert_eq!(m.m.stage_file("s1", &ws, Path::new("/acpira-virtual-host/work/notes.txt")).await, None);
  assert_eq!(m.m.stage_file("s1", &ws, Path::new("/acpira-virtual-host/work/gone.png")).await, None);
  connections.dispose();
}

// One engine serves several windows: each viewer reads its own window's workspace, project mode and session scope
// (`ViewerShell`), so two VS Code windows on different folders and the desktop app neither share a folder nor flip each
// other's project mode
#[tokio::test(flavor = "multi_thread")]
async fn viewers_of_different_windows_keep_their_own_workspace_scope_and_project_mode() {
  use acpira_host::session_manager::ViewerShell;
  use acpira_shared::sidecar::InitialView;
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let (a, b, home) = (folder(dir.path(), "a"), folder(dir.path(), "b"), folder(dir.path(), "home"));
  // The engine-wide facts are window A's: a workspace shell scoped to its folder
  let opts = Opts::fake(&fake).cwd(&a);
  *opts.scope.lock().unwrap() = "workspace".into();
  let m = Mgr::new(&dir.path().join("sessions"), opts);
  m.init().await;
  let shell = |cwd: &str, projects: bool| {
    let cwd = cwd.to_owned();
    let scope = if projects { "all" } else { "workspace" };
    Some(ViewerShell { cwd: Arc::new(move || cwd.clone()), projects: Arc::new(move || projects), scope: Arc::new(move || scope.into()) })
  };
  let va = m.m.attach_in(None, shell(&a, false));
  let vb = m.m.attach_in(None, shell(&b, false));
  let desk = m.m.attach_in(None, shell(&home, true));

  // Each window starts its sessions in its own workspace
  m.handle_on(&va, json!({ "type": "newSession" })).await;
  m.handle_on(&va, json!({ "type": "send", "text": "in a" })).await;
  let sa = va.active_id().unwrap();
  m.handle_on(&vb, json!({ "type": "newSession" })).await;
  m.handle_on(&vb, json!({ "type": "send", "text": "in b" })).await;
  let sb = vb.active_id().unwrap();
  assert_eq!(m.active_of(&va).unwrap()["cwd"], a.as_str());
  assert_eq!(m.active_of(&vb).unwrap()["cwd"], b.as_str());
  let listed = |id: &str| m.sessions().iter().any(|s| s["id"] == id);
  until(|| listed(&sa) && listed(&sb), 5000).await;

  // "Most recent" opens on the newest session of the window's own workspace, not the engine's
  let most_recent = || Some(InitialView::MostRecent { most_recent: true });
  assert_eq!(m.m.attach_in(most_recent(), shell(&a, false)).active_id(), Some(sa.clone()));
  assert_eq!(m.m.attach_in(most_recent(), shell(&b, false)).active_id(), Some(sb.clone()));

  // The desktop window means the newest session's project; the workspace windows keep theirs
  assert_eq!(m.m.folder_for(Some(&desk), None).as_deref(), Some(b.as_str()));
  assert_eq!(m.m.folder_for(Some(&va), None).as_deref(), Some(a.as_str()));
  // A chat is a project shell's: the desktop window gets a chat folder, a workspace window's new session stays in its folder
  m.handle_on(&desk, json!({ "type": "newSession", "chat": true })).await;
  assert!(m.m.is_chat(m.active_of(&desk).unwrap()["cwd"].as_str().unwrap()));
  m.handle_on(&va, json!({ "type": "newSession", "chat": true })).await;
  assert_eq!(m.active_of(&va).unwrap()["cwd"], a.as_str());
  m.dispose().await;
}

// An engine a workspace window started never seeded the project list at init: the first project shell to attach does
#[tokio::test(flavor = "multi_thread")]
async fn the_first_project_shell_to_attach_seeds_the_project_list() {
  use acpira_host::session_manager::ViewerShell;
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let (a, b) = (folder(dir.path(), "a"), folder(dir.path(), "b"));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake).cwd(&a));
  m.init().await;
  for cwd in [&a, &b] {
    m.handle(json!({ "type": "newSession", "cwd": cwd })).await;
    m.handle(json!({ "type": "send", "text": "hi" })).await;
    let id = m.active_id().unwrap();
    until(|| m.sessions().iter().any(|s| s["id"] == id.as_str()), 5000).await;
  }
  assert!(projects_of(&m).is_null(), "a workspace window seeds nothing");
  let home = dir.path().to_string_lossy().into_owned();
  let desk = ViewerShell { cwd: Arc::new(move || home.clone()), projects: Arc::new(|| true), scope: Arc::new(|| "all".into()) };
  let _v = m.m.attach_in(None, Some(desk));
  until(|| !projects_of(&m).is_null(), 5000).await;
  assert_eq!(projects_of(&m), json!([b, a]));
  m.dispose().await;
}

// A project of several folders: a session in either folder gets the other one as ACP `additionalDirectories` (the
// agent advertises them), and a session started in the further folder does not list it as a project of its own
#[tokio::test(flavor = "multi_thread")]
async fn a_project_of_two_folders_hands_each_session_the_other_folder() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let home = folder(dir.path(), "home");
  let (app, web) = (folder(dir.path(), "app"), folder(dir.path(), "web"));
  let meta_log = dir.path().join("meta.log");
  let opts = Opts::with_agents(fake.setting(json!({ "env": { "FAKE_EXTRA_DIRS": "1", "FAKE_META_LOG": meta_log } })), "fake");
  let m = Mgr::new(&dir.path().join("sessions"), opts.cwd(&home).projects());
  m.init().await;
  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": app })).await;
  m.handle(json!({ "type": "categoryOp", "op": "editProject", "cwd": app, "primary": app, "name": "Both", "extra": [web] })).await;
  let read = || std::fs::read_to_string(&meta_log).unwrap_or_default().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).collect::<Vec<_>>();

  m.handle(json!({ "type": "newSession", "cwd": app })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| !read().is_empty(), 5000).await;
  assert_eq!(read()[0]["dirs"], json!([web]));

  m.handle(json!({ "type": "newSession", "cwd": web })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| read().len() > 1, 5000).await;
  let last = read().pop().unwrap();
  assert_eq!(last["cwd"], json!(web));
  assert_eq!(last["dirs"], json!([app]));
  assert_eq!(projects_of(&m), json!([app]));
  m.dispose().await;
}
