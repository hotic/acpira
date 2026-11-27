//! The loopback hub and the roster: listing personas, the sidekick pick, the browser hand-off, list_changed

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn the_hub_lists_enabled_personas_and_forwards_an_ask_to_the_session_its_grant_names() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let token_of = |env: Value| {
    let names: Vec<&str> = env.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    // Only the way back and the grant travel in the env: session, thread and depth stay with the hub
    assert_eq!(names, ["ACPIRA_RELAY", "ACPIRA_RELAY_TOKEN"]);
    env[1]["value"].as_str().unwrap().to_owned()
  };
  let root = token_of(hub.env(&s.0, None, 0));
  let deepest = token_of(hub.env(&s.0, Some("thr"), acpira_host::relay::MAX_DEPTH));
  // One grant per session, thread and depth: a later session request carries the same entry, which a CLI that
  // compares entries (agy) would otherwise register as one more server
  assert_eq!(token_of(hub.env(&s.0, None, 0)), root);
  assert_eq!(token_of(hub.env(&s.0, Some("thr"), acpira_host::relay::MAX_DEPTH)), deepest);
  assert_ne!(root, deepest);
  let addr = hub.addr().to_owned();
  let replies = tokio::task::spawn_blocking(move || {
    let req = |token: &str, op| HubRequest { token: token.into(), op };
    let mut out = vec![];
    call(&addr, &req(&root, HubOp::List), |r| {
      out.push(r);
      false
    })
    .unwrap();
    call(&addr, &req(&root, HubOp::Ask(ask("fake-review", "relay: via hub"))), |r| {
      let done = !matches!(r, HubReply::Progress(_));
      out.push(r);
      !done
    })
    .unwrap();
    // A grant at the depth limit cannot summon, whatever the request says
    call(&addr, &req(&deepest, HubOp::Ask(ask("fake-review", "relay: deeper"))), |r| {
      out.push(r);
      false
    })
    .unwrap();
    // An unknown token gets no answer at all
    let mut refused = vec![];
    call(&addr, &req("nope", HubOp::List), |r| {
      refused.push(r);
      true
    })
    .unwrap();
    assert!(refused.is_empty());
    out
  })
  .await
  .unwrap();
  let HubReply::Roster { personas, groups } = &replies[0] else { panic!("{replies:?}") };
  assert_eq!((personas.len(), groups.len()), (2, 0));
  let Some(HubReply::Done(text)) = replies.iter().rev().nth(1) else { panic!("{replies:?}") };
  assert!(text.starts_with("reply to relay: via hub (round 1, mode plan)"), "{text}");
  let Some(HubReply::Error(text)) = replies.last() else { panic!("{replies:?}") };
  assert!(text.contains("levels deep"), "{text}");
}

/// The persona ids the hub lists for a grant
async fn listed(hub: &Arc<RelayHub>, token: String) -> Vec<Value> {
  let addr = hub.addr().to_owned();
  tokio::task::spawn_blocking(move || {
    let mut out = None;
    call(&addr, &HubRequest { token, op: HubOp::List }, |r| {
      out = Some(r);
      false
    })
    .unwrap();
    match out {
      Some(HubReply::Roster { personas, .. }) => personas.iter().map(v).collect(),
      other => panic!("{other:?}"),
    }
  })
  .await
  .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_built_in_agents_sidekick_is_listed_first_for_the_root_and_keeps_one_thread() {
  let fake = fake_or_skip!();
  // The fake offers the built-in agent's sidekick select, since the engine declares `acpira/sidekick`
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SIDEKICK": "1" } }));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let token = |caller: Option<&str>, depth: u32| hub.env(&s.0, caller, depth)[1]["value"].as_str().unwrap().to_owned();
  let ids = |list: &[Value]| list.iter().map(|p| p["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();

  assert_eq!(ids(&listed(&hub, token(None, 0)).await), ["fake-review", "fake-worker"], "none picked yet");
  s.0.set_config("sidekick".into(), "m2".into()).await.unwrap();
  let root = listed(&hub, token(None, 0)).await;
  assert_eq!(ids(&root), ["sidekick", "fake-review", "fake-worker"]);
  expect_match(&root[0], json!({ "name": "Sidekick", "agent": "fake", "model": "m2", "mode": "inherit", "duty": "executor" }));
  assert!(root[0]["brief"].as_str().unwrap().starts_with("You are the sidekick of another agent, the lead,"));
  // The root's own: a summoned child does not get it
  assert_eq!(ids(&listed(&hub, token(Some("thr"), 1)).await), ["fake-review", "fake-worker"]);

  // Summoned by its id in the root's own CLI on the picked model and effort; the next task continues its thread, at the
  // effort picked since
  s.0.set_config("sidekick_effort".into(), "low".into()).await.unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("sidekick", "relay: rename the helper")).await);
  assert!(ok, "{text}");
  let thread = thread_of(&text);
  s.0.set_config("sidekick_effort".into(), "high".into()).await.unwrap();
  // The sidekick takes the root's access whatever the call asks: a consult call to it still runs as work here
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { mode: Some("consult".into()), ..ask("sidekick", "relay: now the tests") }).await);
  assert!(ok && text.contains("(round 2,"), "{text}");
  assert_eq!(thread_of(&text), thread);
  let list = nodes(&s);
  expect_match(&list[0], json!({ "role": "Sidekick", "model": "Model 2", "harness": { "agent": "fake", "persona": "sidekick", "thread": thread, "round": 1, "effort": "Low" } }));
  expect_match(&list[1], json!({ "harness": { "thread": thread, "round": 2, "effort": "High", "mode": "work" } }));
  assert_eq!(list[0]["peer"]["sessionId"], list[1]["peer"]["sessionId"]);

  // Another pick: the same conversation goes on with the new model
  s.0.set_config("sidekick".into(), "m1".into()).await.unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("Sidekick", "relay: and the docs")).await);
  assert!(ok && thread_of(&text) == thread, "{text}");
  expect_match(&nodes(&s)[2], json!({ "model": "Model 1", "harness": { "thread": thread, "round": 3 } }));

  // None: neither a new task nor its thread reaches it any more
  s.0.set_config("sidekick".into(), "none".into()).await.unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("sidekick", "relay: more")).await);
  assert!(!ok && text.contains("No subagent named"), "{text}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: more") }).await);
  assert!(!ok && text.contains("no sidekick now"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_without_a_sidekick_select_gets_the_hosts_whose_pick_names_the_cli_to_run_in() {
  use acpira_host::acp::session::host_sidekick::SidekickSource;
  use acpira_shared::transcript::{ConfigControl, SessionOption};
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  let option = |id: &str, name: &str| SessionOption { id: id.into(), name: name.into(), ..Default::default() };
  let model = ConfigControl { id: "model".into(), name: "Model".into(), category: Some("model".into()), options: vec![option("m1", "Model 1"), option("m2", "Model 2")], ..Default::default() };
  h.deps.policy = Arc::new(crate::support::TestPolicy { sidekick: vec![SidekickSource { agent: "fake".into(), name: "Fake CLI".into(), model, shapes: None }], ..Default::default() });
  // The select goes only to a CLI that is handed Acpira's MCP server, which `ask_agent` lives in
  h.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new("acpira")));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let token = hub.env(&s.0, None, 0)[1]["value"].as_str().unwrap().to_owned();
  let ids = |list: &[Value]| list.iter().map(|p| p["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  let select = |s: &Disposing| view(&s.0)["controls"]["options"].as_array().unwrap().iter().find(|o| o["category"] == "_sidekick").cloned().expect("the host's select");

  let shown = select(&s);
  expect_match(&shown, json!({ "id": "host_sidekick", "value": "none" }));
  expect_match(&shown["options"][2], json!({ "id": "fake/m2", "name": "Model 2", "description": "Fake CLI" }));
  assert_eq!(ids(&listed(&hub, token.clone()).await), ["fake-review", "fake-worker"], "none picked yet");

  // The pick stays here (the fake would refuse a config id it does not offer) and survives the agent's next answer
  s.0.set_config("host_sidekick".into(), "fake/m2".into()).await.unwrap();
  s.0.set_config("model".into(), "m2".into()).await.unwrap();
  assert_eq!(select(&s)["value"], "fake/m2");
  let root = listed(&hub, token).await;
  assert_eq!(ids(&root), ["sidekick", "fake-review", "fake-worker"]);
  expect_match(&root[0], json!({ "name": "Sidekick", "agent": "fake", "model": "m2", "mode": "inherit", "duty": "executor" }));

  // The lead hears of it ahead of the user's next prompt, once
  prompt(&s.0, "echo-text alpha").await;
  let reply = |s: &Disposing| last_turn(&view(&s.0))["blocks"].as_array().and_then(|b| b.last()).and_then(|b| b["markdown"].as_str()).unwrap_or_default().to_owned();
  let said = reply(&s);
  assert!(said.contains("<sidekick>") && said.contains("Model 2 in Fake CLI") && said.contains("agent \"sidekick\""), "{said}");
  prompt(&s.0, "echo-text beta").await;
  assert!(!reply(&s).contains("<sidekick>"), "told once");

  let (ok, text) = last_text(&run(&s, &roster, 0, ask("sidekick", "relay: rename the helper")).await);
  assert!(ok, "{text}");
  expect_match(&nodes(&s)[0], json!({ "role": "Sidekick", "model": "Model 2", "harness": { "agent": "fake", "persona": "sidekick", "round": 1 } }));

  s.0.set_config("host_sidekick".into(), "none".into()).await.unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("sidekick", "relay: more")).await);
  assert!(!ok && text.contains("No subagent named"), "{text}");
  prompt(&s.0, "echo-text gamma").await;
  assert!(reply(&s).contains("You no longer have a sidekick"), "{}", reply(&s));
}

/// A shell that shows pages: every URL it is asked for, and the forwarded address it answers with
struct FakeBrowser {
  can: bool,
  opened: parking_lot::Mutex<Vec<String>>,
  /// The asking session (id, folder) of each request
  sessions: parking_lot::Mutex<Vec<(String, String)>>,
}

impl HostBrowser for FakeBrowser {
  fn available(&self) -> bool {
    self.can
  }
  fn open<'a>(&'a self, url: &'a str, folder: Option<&'a str>, session: Option<(&'a str, &'a str)>) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<BrowserOpened>> + Send + 'a>> {
    Box::pin(async move {
      self.sessions.lock().extend(session.map(|(id, cwd)| (id.to_owned(), cwd.to_owned())));
      // The second request for one URL finds the tab the first one opened; a folder on an SSH host is noted after it
      let shown = folder.map_or_else(|| url.to_owned(), |f| format!("{url} in {f}"));
      let reused = self.opened.lock().contains(&shown).then_some(BrowserReuse::Revealed);
      self.opened.lock().push(shown);
      if url.contains("unreachable") { anyhow::bail!("it did not answer from the user's machine") }
      Ok(BrowserOpened { url: url.replace(":1/", ":2/"), reused })
    })
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn open_browser_goes_through_the_hub_to_the_shell_once_the_page_is_served_here() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let hub = RelayHub::start(roster(h.dir.path()).await, Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let token = hub.env(&s.0, None, 0)[1]["value"].as_str().unwrap().to_owned();
  let addr = hub.addr().to_owned();
  let one = move |op: HubOp| {
    let (addr, token) = (addr.clone(), token.clone());
    let mut out = None;
    call(&addr, &HubRequest { token, op }, |r| {
      out = Some(r);
      false
    })
    .unwrap();
    out.expect("one reply")
  };
  let ask = |op: HubOp| {
    let one = one.clone();
    tokio::task::spawn_blocking(move || one(op))
  };
  // No shell platform yet: not offered, and a call says why
  let HubReply::Features(f) = ask(HubOp::Features).await.unwrap() else { panic!() };
  assert!(!f.browser);
  let HubReply::Error(text) = ask(HubOp::OpenBrowser { url: "https://example.com/".into() }).await.unwrap() else { panic!() };
  assert!(text.contains("cannot show web pages"), "{text}");

  let browser = Arc::new(FakeBrowser { can: true, opened: Default::default(), sessions: Default::default() });
  hub.set_browser(browser.clone());
  let HubReply::Features(f) = ask(HubOp::Features).await.unwrap() else { panic!() };
  assert!(f.browser);
  // A loopback page nobody serves here is refused before the shell is asked
  let closed = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
  let HubReply::Error(text) = ask(HubOp::OpenBrowser { url: format!("http://localhost:{closed}/") }).await.unwrap() else { panic!() };
  assert!(text.contains(&format!("Nothing is listening on localhost:{closed}")), "{text}");
  assert!(browser.opened.lock().is_empty());
  // Served here: the shell opens it and its forwarded address is reported
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let port = listener.local_addr().unwrap().port();
  tokio::spawn(async move { while listener.accept().await.is_ok() {} });
  let url = format!("http://127.0.0.1:{port}/:1/");
  let HubReply::Done(text) = ask(HubOp::OpenBrowser { url: url.clone() }).await.unwrap() else { panic!() };
  assert!(text.contains(&format!("loads it as http://127.0.0.1:{port}/:2/")), "{text}");
  // The shell is told which session asked, to put the page with that session's tabs
  assert_eq!(*browser.sessions.lock(), [(s.0.id.clone(), "/tmp".to_owned())]);
  // Asked again for the same page: the tab is brought back, and the model is told it was not reloaded
  let HubReply::Done(text) = ask(HubOp::OpenBrowser { url: url.clone() }).await.unwrap() else { panic!() };
  assert!(text.contains("without reloading"), "{text}");
  // The shell's failure (the page does not answer from the user's side) reaches the model
  let HubReply::Error(text) = ask(HubOp::OpenBrowser { url: format!("http://127.0.0.1:{port}/unreachable") }).await.unwrap() else { panic!() };
  assert!(text.contains("did not answer from the user's machine"), "{text}");
  // Other hosts are the shell's to check
  let HubReply::Done(_) = ask(HubOp::OpenBrowser { url: "https://example.com/".into() }).await.unwrap() else { panic!() };
  assert_eq!(browser.opened.lock().len(), 4);
}

/// One `acpira mcp` process with a session's relay env, read line by line on a thread
struct McpProcess {
  child: std::process::Child,
  stdin: std::process::ChildStdin,
  lines: std::sync::mpsc::Receiver<Value>,
}

impl McpProcess {
  fn spawn(env: &Value) -> McpProcess {
    use std::io::BufRead;
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_acpira"));
    cmd.arg("mcp").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    for pair in env.as_array().unwrap() {
      cmd.env(pair["name"].as_str().unwrap(), pair["value"].as_str().unwrap());
    }
    let mut child = cmd.spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
      for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Ok(v) = serde_json::from_str::<Value>(&line)
          && tx.send(v).is_err()
        {
          break;
        }
      }
    });
    McpProcess { child, stdin, lines }
  }

  fn send(&mut self, msg: Value) {
    use std::io::Write;
    writeln!(self.stdin, "{msg}").unwrap();
    self.stdin.flush().unwrap();
  }

  /// The next line matching `pick` within `secs`, or None
  fn next(&self, secs: u64, pick: impl Fn(&Value) -> bool) -> Option<Value> {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
      match self.lines.recv_timeout(left) {
        Ok(v) if pick(&v) => return Some(v),
        Ok(_) => {}
        Err(_) => return None,
      }
    }
    None
  }

  fn agents(&mut self, id: u64) -> Vec<String> {
    self.send(json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list" }));
    let list = self.next(10, |v| v["id"] == id).expect("tools/list answered");
    let tools = list["result"]["tools"].as_array().unwrap().clone();
    let ask = tools.iter().find(|t| t["name"] == "ask_agent");
    ask.map(|t| t["inputSchema"]["properties"]["agent"]["enum"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect()).unwrap_or_default()
  }
}

impl Drop for McpProcess {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

fn list_changed(v: &Value) -> bool {
  v["method"] == "notifications/tools/list_changed"
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_sessions_ask_agent_follows_the_roster_through_list_changed() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  let mut mcp = McpProcess::spawn(&hub.env(&s.0, None, 0));
  mcp.send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "claude-code" } } }));
  let init = mcp.next(10, |v| v["id"] == 1).expect("initialize answered");
  assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], true, "{init}");
  assert_eq!(mcp.agents(2), ["fake-review", "fake-worker"]);

  // Nothing changed: no notification
  assert!(mcp.next(3, list_changed).is_none());
  // A persona added on the settings page: told once, and the next listing offers it
  let base = serde_json::to_value(roster.list()).unwrap();
  let mut personas = base.as_array().unwrap().clone();
  personas.push(json!({ "name": "Fresh", "agent": "fake", "mode": "consult" }));
  roster.save(&json!({ "base": base, "personas": personas })).await.unwrap();
  assert!(mcp.next(6, list_changed).is_some(), "no list_changed after a persona was added");
  assert!(mcp.next(3, list_changed).is_none(), "told twice of one change");
  assert_eq!(mcp.agents(3), ["fake-review", "fake-worker", "fresh"]);
  // Removing it again is a change too, back to the first roster
  let now = serde_json::to_value(roster.list()).unwrap();
  roster.save(&json!({ "base": now, "personas": base })).await.unwrap();
  assert!(mcp.next(6, list_changed).is_some(), "no list_changed after a persona was removed");
  assert_eq!(mcp.agents(4), ["fake-review", "fake-worker"]);
}
