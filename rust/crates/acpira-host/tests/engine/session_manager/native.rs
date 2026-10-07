//! Listing and importing the agents' native sessions

use super::*;

// "Import from <agent>": session/list on a throwaway process marks the ids this window already holds; importing a
// foreign one opens a record whose transcript the session/load replay fills
#[tokio::test(flavor = "multi_thread")]
async fn native_sessions_list_marks_imported_ones_import_replays_and_re_import_selects_the_record() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })), "fake"));
  m.init().await;
  // (a) a session this manager runs is listed with localId pointing back at its record
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let own_id = m.active_id().unwrap();
  let own = m.m.list_native_sessions("fake").await.unwrap().into_iter().find(|s| s.local_id.as_deref() == Some(own_id.as_str())).expect("own session listed");
  // The fake stores the canonical cwd (codex-acp canonicalizes thread cwd the same way)
  assert_eq!(own.cwd, std::fs::canonicalize("/tmp").unwrap().to_string_lossy());
  assert!(own.title.as_deref().is_some_and(|t| t.starts_with("Fake ")));
  assert!(own.updated_at.is_some());
  // (b) a native session another process owns is listed bare; importing it replays the native history
  let other = {
    let deps = SessionDeps {
      registry: Arc::new(AgentRegistry::new(&fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })))),
      log: Arc::new(|_: &str| {}),
      on_change: Arc::new(|_, _| {}),
      blobs: m.store.clone(),
      notify: None,
      accounts: None,
      compaction: None,
      plan_auto_approve: None,
      pool: None,
      model_shapes: None,
      shared_mcp: None,
      host_mcp: None,
      claim: None,
    };
    let s = AcpSession::fresh("fake", "/tmp", deps, None);
    s.start().await;
    s.prompt("hi".into(), vec![], false, None, None).await;
    let id = s.to_record().acp_session_id.unwrap();
    s.dispose();
    id
  };
  let target = m.m.list_native_sessions("fake").await.unwrap().into_iter().find(|s| s.session_id == other).expect("foreign listed");
  assert!(target.local_id.is_none());
  let count = m.sessions().len();
  let viewer = m.m.attach(None);
  m.m.import_native_session(&viewer, "fake", &target.session_id, &target.cwd, target.title.as_deref(), target.updated_at.as_deref()).await;
  until(|| m.sessions().len() == count + 1, 2000).await;
  let imported = viewer.active_id().unwrap();
  let view = m.view_of(&imported).unwrap();
  assert_eq!(view["status"], "ready");
  assert!(crate::acp_session::agent_blocks(&view).iter().any(|b| b["markdown"].as_str().is_some_and(|t| t.contains("NATIVE_REPLAY"))));
  assert_eq!(m.sessions().iter().find(|s| s["id"] == imported.as_str()).unwrap()["acpSessionId"], other.as_str());
  let t0 = std::time::Instant::now();
  let record = loop {
    if let Some(r) = m.store.load(&imported).await.filter(|r| !r.import_pending) {
      break r;
    }
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  };
  assert_eq!(record.acp_session_id.as_deref(), Some(other.as_str()));
  assert_eq!(v(&record.imported_from), json!({ "sessionId": other }));
  // (c) importing the same native id again lands the viewer on the existing record instead of making a second one
  m.m.import_native_session(&viewer, "fake", &target.session_id, &target.cwd, target.title.as_deref(), target.updated_at.as_deref()).await;
  assert_eq!(m.sessions().len(), count + 1);
  assert_eq!(viewer.active_id().as_deref(), Some(imported.as_str()));
  m.dispose().await;
}

// codex-acp stores the canonicalized thread cwd and filters each page by string compare, so a project reached
// through a symlink produced only empty pages with cursors; the host pages through and retries with the realpath
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn native_sessions_of_a_symlinked_project_still_list() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let target = dir.path().join("proj-real");
  std::fs::create_dir(&target).unwrap();
  let link = dir.path().join("proj-link");
  std::os::unix::fs::symlink(&target, &link).unwrap();
  let other_dir = dir.path().join("other");
  std::fs::create_dir(&other_dir).unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let agents = fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_LIST_PAGE": "1" } }));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(agents, "fake").cwd(link.to_str().unwrap()));
  m.init().await;
  let mk = |cwd: PathBuf| {
    let (fake_ref, native, store) = (&fake, native.clone(), m.store.clone());
    async move {
      let deps = SessionDeps {
        registry: Arc::new(AgentRegistry::new(&fake_ref.setting(json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_LIST_PAGE": "1" } })))),
        log: Arc::new(|_: &str| {}),
        on_change: Arc::new(|_, _| {}),
        blobs: store,
        notify: None,
        accounts: None,
        compaction: None,
        plan_auto_approve: None,
        pool: None,
        model_shapes: None,
        shared_mcp: None,
        host_mcp: None,
        claim: None,
      };
      let s = AcpSession::fresh("fake", cwd.to_str().unwrap(), deps, None);
      s.start().await;
      let id = s.to_record().acp_session_id.unwrap();
      s.dispose();
      id
    }
  };
  let target_id = mk(link.clone()).await;
  let foreign = [mk(other_dir.clone()).await, mk(other_dir.clone()).await];
  // Deterministic order: the foreign sessions lead (filtered-out pages), the project's trails on the last page
  let stamp = |id: &str, secs: i64| {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    std::fs::File::options().write(true).open(native.join(format!("{id}.json"))).unwrap().set_modified(t).unwrap();
  };
  stamp(&target_id, 1);
  for (i, id) in foreign.iter().enumerate() {
    stamp(id, 2 + i as i64);
  }
  let listed = m.m.list_native_sessions("fake").await.unwrap();
  assert_eq!(listed.iter().map(|s| s.session_id.clone()).collect::<Vec<_>>(), [target_id]);
  assert_eq!(listed[0].cwd, std::fs::canonicalize(&link).unwrap().to_string_lossy());
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_native_sessions_of_an_agent_without_the_capability_is_unsupported() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  // no FAKE_SESSION_DIR: the fixture advertises no list capability
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake));
  m.init().await;
  let err = m.m.list_native_sessions("fake").await.expect_err("unsupported");
  assert!(err.to_string().contains("does not list its sessions"), "{err}");
  m.dispose().await;
}
