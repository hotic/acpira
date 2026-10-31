//! Remote run's wire (`remote/peer.rs`): this engine reaches the host's engine through the helper's `engine` op as a
//! peer, the host starts that engine when none runs and keeps it past the connection, and a second link finds the same
//! one. The host is this machine behind the fake ssh, with its home under the test's directory

#![cfg(unix)]

use std::time::Duration;

use acpira_host::remote::peer::{PeerHello, PeerLink};
use acpira_shared::remote::ConnectionOp;
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
    // `--`: the pattern starts with `--home`, which BSD pkill otherwise reads as an option and kills nothing
    let _ = std::process::Command::new("pkill").args(["-TERM", "-f", "--", &self.0]).status();
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

fn list_record(id: &str, cwd: &str, title: &str, pinned: bool) -> Value {
  json!({ "id": id, "agent": "fake", "cwd": cwd, "title": title, "pinned": pinned,
    "createdAt": "2026-10-01T00:00:00.000Z", "updatedAt": "2026-10-01T00:00:00.000Z",
    "turns": [{ "role": "user", "text": "conversation needle" }],
    "controls": { "modes": [], "options": [] }, "commands": [] })
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_list_scopes_filter_the_wire_and_leave_other_views_independent() {
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let host = lab.host_home().join(".acpira/sessions");
  std::fs::create_dir_all(&host).unwrap();
  for (id, cwd, pinned) in [("s-visible", "/visible", false), ("s-hidden", "/hidden", true), ("s-nested", "ssh://other/nested", false)] {
    std::fs::write(host.join(format!("{id}.json")), serde_json::to_vec(&list_record(id, cwd, id, pinned)).unwrap()).unwrap();
  }
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let client = connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();
  let mut link = PeerLink::connect(&client, None, ME).await.unwrap();
  for (view, initial, expected) in [
    ("P", json!({ "sessionList": { "folders": ["/visible"] } }), vec!["s-visible"]),
    ("E", json!({ "sessionList": { "folders": [] } }), vec![]),
    ("S", json!({ "sessionList": { "folders": [] }, "session": "s-hidden" }), vec![]),
    ("Q", json!({ "empty": true }), vec!["s-hidden", "s-nested", "s-visible"]),
  ] {
    link.send(&json!({ "type": "attachView", "viewId": view, "host": "sidebar", "initial": initial }));
    link.send(&json!({ "type": "webviewMessage", "viewId": view, "message": { "type": "ready" } }));
    let init = next_of(&mut link, |v| v["viewId"] == view && v["message"]["type"] == "init").await;
    let mut ids: Vec<_> = init["message"]["state"]["sessions"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, expected, "initial wire list for {view}");
    let after = next_of(&mut link, |v| v["viewId"] == view && v["message"]["type"] == "sessions").await;
    assert_eq!(after["message"]["sessions"], init["message"]["state"]["sessions"], "post-init list stays scoped even after a pre-init push");
  }
  link.send(&json!({ "type": "webviewMessage", "viewId": "Q", "message": { "type": "renameSession", "id": "s-hidden", "title": "Changed hidden" } }));
  loop {
    let e = next_of(&mut link, |v| v["message"]["type"] == "sessions").await;
    assert_ne!(e["viewId"], "P", "hidden updates must not resend an unchanged scoped list: {e}");
    assert_ne!(e["viewId"], "E", "an empty scoped list must stay silent: {e}");
    if e["viewId"] == "S" {
      assert_eq!(e["message"]["sessions"], json!([]), "a transcript view must not resend the full list");
    }
    if e["message"]["sessions"].as_array().unwrap().iter().any(|s| s["title"] == "Changed hidden") {
      break;
    }
  }
  // A search crosses the list scope once, returning only matching local summaries, never another host's list.
  link.send(&json!({ "type": "webviewMessage", "viewId": "P", "message": { "type": "searchSessions", "query": "needle", "seq": 42 } }));
  let hits = next_of(&mut link, |v| v["message"]["type"] == "sessionHits").await;
  let hits = hits["message"]["hits"].as_array().unwrap();
  assert_eq!(hits.len(), 2, "nested host results cannot recurse");
  assert!(hits.iter().all(|h| h["summary"]["id"] == h["id"] && h["snippet"].as_str().unwrap().contains("needle")));
  link.send(&json!({ "type": "setSessionList", "viewId": "P", "scope": { "folders": ["/hidden"] } }));
  let changed = next_of(&mut link, |v| v["viewId"] == "P" && v["message"]["type"] == "sessions").await;
  assert_eq!(changed["message"]["sessions"].as_array().unwrap().len(), 1);
  assert_eq!(changed["message"]["sessions"][0]["id"], "s-hidden");
  drop(link);
  connections.dispose();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_subscriptions_prune_caches_and_query_hidden_sessions_on_demand() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let home = lab.host_home().join(".acpira");
  let host = home.join("sessions");
  std::fs::create_dir_all(&host).unwrap();
  std::fs::write(home.join("agents.json"), serde_json::to_vec(&fake.setting(json!({}))).unwrap()).unwrap();
  let work = lab.path().join("hostfs/work");
  let hidden = lab.path().join("hostfs/hidden");
  std::fs::create_dir_all(&work).unwrap();
  std::fs::create_dir_all(&hidden).unwrap();
  for (id, path, title) in [("s-visible", &work, "Visible"), ("s-hidden", &hidden, "Hidden pin"), ("s-link", &hidden, "Link only")] {
    std::fs::write(host.join(format!("{id}.json")), serde_json::to_vec(&list_record(id, &path.to_string_lossy(), title, true)).unwrap())
      .unwrap();
  }
  let folder = format!("ssh://gen1{}", work.display());
  let hidden_folder = format!("ssh://gen1{}", hidden.display());
  let dir = tempfile::tempdir().unwrap();
  std::fs::write(
    dir.path().join("categories.json"),
    serde_json::to_vec(&json!({ "categories": [], "collapsedProjects": [], "projects": [folder] })).unwrap(),
  )
  .unwrap();
  let cache = dir.path().join("lease-home/run/remote/gen1.sessions.json");
  std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
  std::fs::write(
    &cache,
    serde_json::to_vec(&json!([
      { "id": "s-hidden", "cwd": hidden_folder, "title": "Old full-list cache", "agent": "fake", "updatedAt": "2026-10-01T00:00:00.000Z" }
    ]))
    .unwrap(),
  )
  .unwrap();
  let cached = || serde_json::from_slice::<Value>(&std::fs::read(&cache).unwrap()).unwrap();
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let mut opts = crate::session_manager::Opts::fake(&fake).projects();
  opts.remote = Some(connections.clone());
  let m = crate::session_manager::Mgr::new(dir.path(), opts.clone());
  m.init().await;
  assert!(m.sessions().iter().all(|s| s["id"] != "s-hidden"), "legacy cache is narrowed before exposure");
  wait(&m, "the subscribed project", 30_000, || m.session_ids() == vec!["s-visible"]).await;
  assert_eq!(cached().as_array().unwrap().len(), 1);
  m.handle(json!({ "type": "pinSession", "id": "s-hidden", "pinned": false })).await;
  wait(&m, "a hidden result's pin routed to its owner", 15_000, || {
    serde_json::from_slice::<Value>(&std::fs::read(host.join("s-hidden.json")).unwrap()).unwrap()["pinned"] != true
  })
  .await;
  // Link-only opens without ever searching or registering its folder, and without retaining its siblings.
  m.handle(json!({ "type": "selectSession", "id": "s-link" })).await;
  wait(&m, "a hidden session opened by id", 30_000, || m.active().is_some_and(|v| v["id"] == "s-link" && v["cwd"] == hidden_folder)).await;
  assert_eq!(m.m.folder_for(Some(&m.v), None).as_deref(), Some(hidden_folder.as_str()), "file actions follow the opened view, not the project list");
  assert_eq!(m.session_ids(), vec!["s-visible"]);
  for query in ["Hidden pin", "needle", "Fake"] {
    let hits = m.m.search_sessions_for(&m.v, query).await;
    assert!(
      hits.iter().any(|h| h.id == "s-hidden" && h.summary.as_ref().is_some_and(|s| s.cwd == hidden_folder)),
      "remote title/content/agent search: {query}"
    );
    assert_eq!(m.session_ids(), vec!["s-visible"], "search results never enter the subscription");
    assert_eq!(cached().as_array().unwrap().len(), 1, "search results never enter the disk cache");
  }
  assert!(m.m.search_sessions_for(&m.v, "").await.is_empty());
  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": hidden_folder })).await;
  wait(&m, "the newly added project", 15_000, || m.sessions().len() == 3).await;
  assert_eq!(cached().as_array().unwrap().len(), 3);
  m.handle(json!({ "type": "categoryOp", "op": "removeProject", "cwd": hidden_folder })).await;
  assert_eq!(m.session_ids(), vec!["s-visible"], "removal prunes memory immediately");
  assert_eq!(cached().as_array().unwrap().len(), 1, "removal prunes the offline cache immediately");
  m.dispose().await;
  let restarted = crate::session_manager::Mgr::new(dir.path(), opts);
  restarted.init().await;
  wait(&restarted, "the scoped reconnect", 30_000, || {
    !restarted.m.host_engines().is_empty() && restarted.session_ids() == vec!["s-visible"]
  })
  .await;
  assert_eq!(cached().as_array().unwrap().len(), 1);
  restarted.dispose().await;
  connections.dispose();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_reaches_the_hosts_engine_which_outlives_the_connection() {
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
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

/// Remote run end to end on this engine's side: an agent `sshLocalAgents` does not name runs in the host's engine, whose sessions join the local list as
/// `ssh://` folders, a new session in a folder there starts on the host and is shown here, and a rename goes there
#[tokio::test(flavor = "multi_thread")]
async fn a_cli_agent_in_a_host_folder_runs_in_the_hosts_engine_and_lists_here() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
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
  assert_eq!(m.m.session_cwd(&id).as_deref(), Some(folder.as_str()));
  // Started there, by the host's own engine: no record of it here
  assert!(!dir.path().join(format!("{id}.json")).exists());

  m.handle(json!({ "type": "renameSession", "id": id, "title": "On the host" })).await;
  wait(&m, "the rename", 15_000, || m.sessions().iter().any(|s| s["id"] == id.as_str() && s["title"] == "On the host")).await;

  // A prompt from here runs there; its turns come back through the host's patches, rebuilt into this engine's views
  m.handle(json!({ "type": "send", "sessionId": id, "text": "hello from here" })).await;
  wait(&m, "the prompt's turns", 30_000, || {
    m.active().is_some_and(|v| v["turns"].as_array().is_some_and(|t| t.len() >= 2) && v["running"] == false)
  })
  .await;
  let view = m.active().unwrap();
  assert!(view["turns"][0].to_string().contains("hello from here"), "{view}");
  assert_eq!(view["cwd"], folder.as_str());
  assert!(!m.logs().iter().any(|l| l.contains("out of step")), "every patch from the host applied: {:?}", m.logs());

  // A deletion completes through the list view, so leaving the shown view cannot swallow its receipt. Undo still routes
  // to the host after the id has left its list (and the shown view was detached here).
  m.handle(json!({ "type": "deleteSession", "id": id, "requestId": "host-delete" })).await;
  wait(&m, "the delete receipt", 15_000, || {
    m.events.lock().unwrap().iter().any(|e| e == &json!({ "type": "deleteSessionResult", "requestId": "host-delete", "undo": true }))
  })
  .await;
  wait(&m, "the removed host entry", 15_000, || !m.session_ids().contains(&id)).await;
  m.handle(json!({ "type": "deselectSession", "id": id })).await;
  m.handle(json!({ "type": "restoreSession", "id": id })).await;
  wait(&m, "undo on the host", 15_000, || m.sessions().iter().any(|s| s["id"] == id.as_str() && s["title"] == "On the host")).await;
  assert!(!dir.path().join(format!("{id}.json")).exists(), "undo must not create a local copy");

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
async fn sessions_driven_here_and_the_hosts_own_list_once() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
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
  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": folder })).await;
  wait(&m, "the host's list under local run", 30_000, || m.sessions().iter().any(|s| s["id"] == "s-host" && s["cwd"] == folder.as_str()))
    .await;
  let mine: Vec<Value> = m.sessions().into_iter().filter(|s| s["id"] == "s-mine").collect();
  assert_eq!(mine.len(), 1, "listed once");
  assert_eq!(mine[0]["title"], "Driven here", "this machine's record wins over the host's copy");

  // Acted on here: a rename lands in this machine's record, not in the host's copy through the host's engine
  m.handle(json!({ "type": "renameSession", "id": "s-mine", "title": "Renamed here" })).await;
  let title = || serde_json::from_slice::<Value>(&std::fs::read(dir.path().join("s-mine.json")).unwrap()).unwrap()["title"].clone();
  wait(&m, "the rename in this machine's record", 15_000, || title() == "Renamed here").await;

  // The host's own session opens through the host's engine
  m.handle(json!({ "type": "selectSession", "id": "s-host" })).await;
  wait(&m, "the host's session on screen", 30_000, || {
    m.active().is_some_and(|v| v["title"] == "The host's own" && v["cwd"] == folder.as_str())
  })
  .await;
  m.dispose().await;
  connections.dispose();
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_agents_replaces_an_empty_host_session() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let mut agents = fake.setting(json!({}));
  agents.as_object_mut().unwrap().extend(fake.setting_as("fake2", json!({})).as_object().unwrap().clone());
  let home = lab.host_home().join(".acpira");
  std::fs::create_dir_all(&home).unwrap();
  std::fs::write(home.join("agents.json"), serde_json::to_vec(&agents).unwrap()).unwrap();
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let work = lab.path().join("hostfs/work");
  std::fs::create_dir_all(&work).unwrap();
  let folder = format!("ssh://gen1{}", work.display());
  let dir = tempfile::tempdir().unwrap();
  let mut opts = crate::session_manager::Opts::with_agents(agents, "fake").projects();
  opts.remote = Some(connections.clone());
  let m = crate::session_manager::Mgr::new(dir.path(), opts);
  m.init().await;

  let mut previous = None;
  for agent in ["fake", "fake2", "fake", "fake2"] {
    m.handle(json!({ "type": "newSession", "agent": agent, "cwd": folder })).await;
    wait(&m, "the selected agent on the host", 30_000, || m.active().is_some_and(|v| v["agent"] == agent && v["status"] == "ready")).await;
    let id = m.active_id().unwrap();
    wait(&m, "the new host entry", 15_000, || m.session_ids().contains(&id)).await;
    assert_eq!(m.session_ids(), [id.clone()], "switching agents must not accumulate empty sessions");
    if let Some(old) = previous {
      assert!(!home.join("sessions").join(format!("{old}.json")).exists(), "the empty record is removed on the host");
    }
    previous = Some(id);
  }

  // Another page showing the empty session keeps it; changing this page's agent must leave that page alone.
  let shared = m.active_id().unwrap();
  let other = m.m.attach(Some(acpira_shared::sidecar::InitialView::Id(shared.clone())));
  m.handle(json!({ "type": "newSession", "agent": "fake", "cwd": folder })).await;
  wait(&m, "a separate session for this page", 30_000, || m.active().is_some_and(|v| v["agent"] == "fake" && v["status"] == "ready")).await;
  assert_eq!(other.active_id().as_deref(), Some(shared.as_str()));
  assert!(m.session_ids().contains(&shared));
  m.m.detach(&other);

  // Once a prompt has been sent, starting over preserves its transcript.
  let used = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "sessionId": used, "text": "keep this conversation" })).await;
  wait(&m, "the completed fake reply", 30_000, || {
    m.active().is_some_and(|v| v["turns"].as_array().is_some_and(|t| t.len() >= 2) && v["running"] == false)
  })
  .await;
  m.handle(json!({ "type": "newSession", "agent": "fake2", "cwd": folder })).await;
  wait(&m, "the next agent", 30_000, || m.active().is_some_and(|v| v["agent"] == "fake2" && v["status"] == "ready")).await;
  assert!(m.session_ids().contains(&used));
  assert!(home.join("sessions").join(format!("{used}.json")).exists());
  assert!(!m.logs().iter().any(|l| l.contains("out of step")), "{:?}", m.logs());
  m.dispose().await;
  connections.dispose();
}

/// Chat authentication belongs to the engine running the agent; settings actions without a session stay local.
#[tokio::test(flavor = "multi_thread")]
async fn host_account_metadata_and_actions_never_use_the_local_account_store() {
  use acpira_host::accounts::account_manager::AccountManager;
  use acpira_host::accounts::account_store::{AccountStore, FileVault};
  use std::sync::Arc;

  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let home = lab.host_home().join(".acpira");
  std::fs::create_dir_all(&home).unwrap();
  let auth_log = lab.path().join("auth.log");
  let agents = fake.setting(json!({ "env": { "FAKE_TERMINAL_AUTH": auth_log, "FAKE_FLAG": "agent" } }));
  std::fs::write(home.join("agents.json"), serde_json::to_vec(&agents).unwrap()).unwrap();
  let account = |label: &str| json!([{ "id": "same-id", "agent": "fake", "label": label, "addedAt": "2026-01-01T00:00:00.000Z" }]);
  std::fs::write(home.join("accounts.json"), serde_json::to_vec(&account("host@example.test")).unwrap()).unwrap();

  let dir = tempfile::tempdir().unwrap();
  std::fs::write(dir.path().join("accounts.json"), serde_json::to_vec(&account("local@example.test")).unwrap()).unwrap();
  let log: acpira_host::store::transcript_store::LogFn = Arc::new(|_| {});
  let store = Arc::new(AccountStore::new(
    dir.path().join("accounts.json"),
    Arc::new(FileVault::new(dir.path().join("secrets.json"), log.clone())),
    log.clone(),
  ));
  store.load().await.unwrap();
  let accounts = AccountManager::new(store, vec![], log, Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}));
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let work = lab.path().join("hostfs/work");
  std::fs::create_dir_all(&work).unwrap();
  let mut opts = crate::session_manager::Opts::with_agents(agents, "fake").projects();
  opts.accounts = Some(accounts.clone());
  opts.remote = Some(connections.clone());
  let m = crate::session_manager::Mgr::new(&dir.path().join("sessions"), opts);
  m.init().await;
  m.handle(json!({ "type": "newSession", "agent": "fake", "cwd": format!("ssh://gen1{}", work.display()) })).await;
  wait(&m, "host account metadata", 30_000, || m.active().is_some_and(|v| v["remoteAuth"]["accounts"][0]["label"] == "host@example.test")).await;
  let id = m.active_id().unwrap();
  assert_eq!(accounts.list()[0].label, "local@example.test");
  wait(&m, "the host's terminal auth method", 15_000, || m.active().is_some_and(|v| v["status"] == "auth_required")).await;
  m.handle(json!({ "type": "login", "sessionId": id, "methodId": "term-login" })).await;
  // A peer cannot yet display the host's terminal. The fallback names the host and never starts a local login.
  wait(&m, "the host terminal fallback", 15_000, || {
    m.toasts.lock().unwrap().iter().any(|t| t.starts_with("gen1:") && t.contains("FAKE_LOGIN=1") && t.contains("--login"))
  })
  .await;
  assert!(m.terminals.lock().unwrap().is_empty(), "a host login must not run in a local terminal");
  assert!(!auth_log.exists(), "terminal auth must not call ACP authenticate");

  // Fake has no account provider: the host's error still returns through remoteAuth, not the local progress list.
  for (kind, via) in [("addAccount", Some("import")), ("unlockCredentials", None)] {
    let mut msg = json!({ "type": kind, "sessionId": id, "agent": "fake" });
    if let Some(via) = via { msg["via"] = json!(via); }
    m.handle(msg).await;
    let via = via.unwrap_or("unlock");
    wait(&m, "host authentication feedback", 15_000, || {
      m.active().is_some_and(|v| v["remoteAuth"]["action"]["via"] == via && v["remoteAuth"]["action"]["status"] == "error")
    })
    .await;
    assert!(m.m.account_actions().is_empty(), "host progress must not replace local settings progress");
  }
  m.handle(json!({ "type": "refreshQuota", "sessionId": id, "agent": "fake" })).await;
  m.handle(json!({ "type": "removeAccount", "sessionId": id, "id": "same-id" })).await;
  wait(&m, "host account removed", 15_000, || {
    m.active().is_some_and(|v| v["remoteAuth"]["accounts"].as_array().is_some_and(Vec::is_empty))
  })
  .await;
  assert_eq!(accounts.list()[0].label, "local@example.test", "identical ids on two hosts must stay isolated");
  assert!(m.events.lock().unwrap().iter().filter(|e| e["type"] == "accounts").all(|e| !e.to_string().contains("host@example.test")));

  // Settings omit sessionId even while a host session is shown.
  m.handle(json!({ "type": "removeAccount", "id": "same-id" })).await;
  assert!(accounts.list().is_empty());
  m.dispose().await;
  connections.dispose();
}

/// A host session's images live in the host's store: they come over into this machine's before its view is shown, so a
/// page (or a paired phone, which reads the same directory) loads them as any session's
#[tokio::test(flavor = "multi_thread")]
async fn a_host_sessions_blobs_come_over_before_its_view_shows() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let _stop = StopEngines::for_lab(&lab);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  let work = lab.path().join("hostfs").join("work");
  std::fs::create_dir_all(&work).unwrap();
  let folder = format!("ssh://gen1{}", work.display());
  let pixels = b"\x89PNG\r\n\x1a\nnot really a picture".to_vec();
  let name = acpira_host::store::transcript_store::blob_name(".png", &pixels);
  let host = lab.host_home().join(".acpira").join("sessions");
  std::fs::create_dir_all(host.join("s-pic")).unwrap();
  std::fs::write(host.join("s-pic").join(&name), &pixels).unwrap();
  let recording = b"staged recording".to_vec();
  let video = acpira_host::store::transcript_store::blob_name(".mp4", &recording);
  std::fs::write(host.join("s-pic").join(&video), &recording).unwrap();
  let video_uri = acpira_host::platform::file_url::path_to_file_url(&host.join("s-pic").join(&video).to_string_lossy());
  let at = "2026-10-01T00:00:00.000Z";
  let turns = json!([
    { "role": "user", "text": "draw it", "attachments": [
      { "kind": "image", "name": "shot.png", "blob": name, "mimeType": "image/png" },
      { "kind": "file", "name": "clip.mp4", "uri": video_uri }
    ] },
    { "role": "agent", "blocks": [{ "type": "image", "id": "i1", "blob": name, "mimeType": "image/png" }] },
  ]);
  let record = json!({ "id": "s-pic", "agent": "fake", "cwd": work.display().to_string(), "title": "With a picture", "createdAt": at,
    "updatedAt": at, "turns": turns, "controls": { "modes": [], "options": [] }, "commands": [] });
  std::fs::write(host.join("s-pic.json"), serde_json::to_vec(&record).unwrap()).unwrap();

  let dir = tempfile::tempdir().unwrap();
  let mut opts = crate::session_manager::Opts::fake(&fake).projects();
  opts.remote = Some(connections.clone());
  let m = crate::session_manager::Mgr::new(dir.path(), opts);
  m.init().await;
  m.handle(json!({ "type": "categoryOp", "op": "addProject", "cwd": folder })).await;
  wait(&m, "the host's list", 30_000, || m.sessions().iter().any(|s| s["id"] == "s-pic" && s["cwd"] == folder.as_str())).await;
  let here = dir.path().join("s-pic").join(&name);
  assert!(!here.exists());

  m.handle(json!({ "type": "selectSession", "id": "s-pic" })).await;
  wait(&m, "the host's session on screen", 30_000, || m.active().is_some_and(|v| v["title"] == "With a picture")).await;
  // Held until the blob was here: the first view a page gets can already load it
  assert_eq!(std::fs::read(&here).ok(), Some(pixels), "the blob is in this machine's store once the view shows; log: {:#?}", m.logs());
  assert_eq!(std::fs::read(dir.path().join("s-pic").join(&video)).ok(), Some(recording), "a staged file URI is cached too");
  assert!(m.active().unwrap()["turns"].to_string().contains(&name));
  m.dispose().await;
  connections.dispose();
}
