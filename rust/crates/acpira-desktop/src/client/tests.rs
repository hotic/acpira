//! The client against a scripted engine on a Unix socket: handshake order, the outbox, reconnects, generations, preview
//! leases and platform requests (test/sidecarClient.test.ts covers the TypeScript client the same way)

use super::*;
use serde_json::json;
use tokio::net::UnixListener;

struct View {
  id: String,
  initial: Option<Initial>,
  messages: Mutex<Vec<HostMessage>>,
  states: Mutex<Vec<SidecarState>>,
}

impl View {
  fn new(id: &str, initial: Option<Initial>) -> Arc<View> {
    Arc::new(View { id: id.into(), initial, messages: Mutex::new(vec![]), states: Mutex::new(vec![]) })
  }

  fn kinds(&self) -> Vec<String> {
    self.messages.lock().iter().map(|m| m.kind.clone()).collect()
  }
}

impl ShellView for View {
  fn view_id(&self) -> &str {
    &self.id
  }
  fn host(&self) -> &str {
    "editor"
  }
  fn initial(&self) -> Option<Initial> {
    self.initial.clone()
  }
  fn blob_base(&self) -> Option<String> {
    Some(format!("acpira-app://localhost/blobs/{}", self.id))
  }
  fn on_host_message(&self, m: HostMessage) {
    self.messages.lock().push(m);
  }
  fn on_state(&self, state: SidecarState, _: Option<&str>) {
    self.states.lock().push(state);
  }
}

/// One connection as the engine sees it
struct Wire {
  lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
  write: tokio::net::unix::OwnedWriteHalf,
}

impl Wire {
  async fn next(&mut self) -> Value {
    let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
      .await
      .expect("an envelope in time")
      .unwrap()
      .expect("the connection is open");
    serde_json::from_str(&line).unwrap()
  }

  /// Nothing more arrives within a moment
  async fn quiet(&mut self) {
    if let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_millis(150), self.lines.next_line()).await {
      panic!("unexpected envelope: {line}");
    }
  }

  async fn send(&mut self, v: Value) {
    self.write.write_all(format!("{v}\n").as_bytes()).await.unwrap();
  }

  async fn hello_ok(&mut self) -> Value {
    let hello = self.next().await;
    assert_eq!(hello["type"], "hello");
    let id = hello["requestId"].clone();
    self.send(json!({ "type": "helloOk", "requestId": id, "protocolVersion": 1, "sidecar": { "version": "t", "pid": 1 }, "sessionsDir": "/s", "features": ["preview"] })).await;
    hello
  }
}

struct Engine {
  listener: UnixListener,
  _dir: tempfile::TempDir,
}

impl Engine {
  fn new() -> (Engine, EngineEndpoint) {
    let dir = tempfile::Builder::new().prefix("acd").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("e.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let ep = EngineEndpoint { socket, log: dir.path().join("e.log") };
    (Engine { listener, _dir: dir }, ep)
  }

  async fn accept(&self) -> Wire {
    let (stream, _) = tokio::time::timeout(Duration::from_secs(5), self.listener.accept()).await.expect("a connection in time").unwrap();
    let (read, write) = stream.into_split();
    Wire { lines: BufReader::new(read).lines(), write }
  }
}

type Requests = Arc<Mutex<Vec<PlatformRequest>>>;

fn client(ep: Option<EngineEndpoint>, commands: Vec<SidecarCommand>) -> (SidecarClient, Requests, Arc<Mutex<Vec<String>>>) {
  let requests: Requests = Arc::default();
  let logs = Arc::new(Mutex::new(Vec::<String>::new()));
  let (r, l) = (requests.clone(), logs.clone());
  let mut opts = ClientOpts::new(
    move || commands.clone(),
    || HelloPayload {
      client: json!({ "name": "t", "version": "0", "capabilities": ["toast"] }),
      env: json!({ "hostLanguage": "en" }),
      settings: Map::new(),
    },
    move |req| {
      r.lock().push(req.clone());
      Box::pin(async move {
        match req {
          PlatformRequest::OpenExternal { url } if url.contains("fail") => Err("refused".to_owned()),
          _ => Ok(json!({ "ok": true })),
        }
      })
    },
    Arc::new(move |line: &str| l.lock().push(line.to_owned())),
  );
  opts.backoff = Box::new(|_| Duration::from_millis(20));
  if let Some(ep) = ep {
    opts.engine = Box::new(move |_| Some(ep.clone()));
  }
  (SidecarClient::new(opts, tokio::runtime::Handle::current()), requests, logs)
}

fn raw(v: Value) -> Box<RawValue> {
  RawValue::from_string(v.to_string()).unwrap()
}

async fn until(what: &str, f: impl Fn() -> bool) {
  let deadline = Instant::now() + Duration::from_secs(5);
  while !f() {
    assert!(Instant::now() < deadline, "timed out waiting for {what}");
    tokio::time::sleep(Duration::from_millis(5)).await;
  }
}

fn nowhere() -> Vec<SidecarCommand> {
  vec![SidecarCommand::binary("/nonexistent/acpira")]
}

#[tokio::test]
async fn queues_until_the_handshake_then_sends_events_attaches_and_the_rest_in_that_order() {
  let (engine, ep) = Engine::new();
  let (c, _, _) = client(Some(ep), nowhere());
  let v = View::new("V", Some(Initial::MostRecent));
  c.attach(v.clone());
  c.send("V", raw(json!({ "type": "ready" })));
  c.event(json!({ "type": "windowFocus" }));
  c.start();
  let mut w = engine.accept().await;
  let hello = w.hello_ok().await;
  assert_eq!(hello["client"]["name"], "t");
  assert_eq!(hello["protocolVersion"], 1);
  assert_eq!(w.next().await, json!({ "type": "platformEvent", "event": { "type": "windowFocus" } }));
  assert_eq!(
    w.next().await,
    json!({ "type": "attachView", "viewId": "V", "host": "editor", "initial": { "mostRecent": true }, "blobBase": "acpira-app://localhost/blobs/V" })
  );
  assert_eq!(w.next().await, json!({ "type": "webviewMessage", "viewId": "V", "message": { "type": "ready" } }));
  until("ready", || c.current().0 == SidecarState::Ready).await;
  assert_eq!(*v.states.lock(), [SidecarState::Stopped, SidecarState::Starting, SidecarState::Ready]);
  assert_eq!(c.sessions_dir().as_deref(), Some("/s"));
  c.dispose().await;
}

#[tokio::test]
async fn a_reconnect_reattaches_on_the_last_session_and_replays_only_what_was_never_sent() {
  let (engine, ep) = Engine::new();
  let (c, _, _) = client(Some(ep), nowhere());
  let v = View::new("V", Some(Initial::MostRecent));
  c.attach(v.clone());
  c.start();
  let mut w = engine.accept().await;
  w.hello_ok().await;
  assert_eq!(w.next().await["type"], "attachView");
  // Host messages reach the view verbatim; the shown session is remembered
  let push = json!({ "type": "session", "session": { "id": "S1", "turns": [] } });
  w.send(json!({ "type": "hostMessage", "viewId": "V", "message": push })).await;
  until("the session push", || v.kinds() == ["session"]).await;
  assert_eq!(serde_json::from_str::<Value>(v.messages.lock()[0].raw.get()).unwrap(), push);
  c.send("V", raw(json!({ "type": "sent-before" })));
  assert_eq!(w.next().await["message"]["type"], "sent-before");

  // The connection drops: the next one gets hello, the view on S1, and only what was queued meanwhile
  drop(w);
  until("starting again", || c.current().0 == SidecarState::Starting).await;
  c.send("V", raw(json!({ "type": "queued" })));
  let mut w = engine.accept().await;
  w.hello_ok().await;
  assert_eq!(w.next().await["initial"], "S1");
  assert_eq!(w.next().await["message"]["type"], "queued");
  w.quiet().await;
  c.dispose().await;
}

#[tokio::test]
async fn a_view_closed_before_the_handshake_takes_its_queued_envelopes_with_it() {
  let (engine, ep) = Engine::new();
  let (c, _, _) = client(Some(ep), nowhere());
  let v = View::new("V", None);
  let w2 = View::new("W", Some(Initial::Session("S9".into())));
  c.attach(v.clone());
  c.attach(w2.clone());
  c.send("V", raw(json!({ "type": "ready" })));
  c.send("W", raw(json!({ "type": "ready" })));
  c.detach("V");
  // A message for a view that is gone goes nowhere
  c.send("V", raw(json!({ "type": "late" })));
  c.start();
  let mut w = engine.accept().await;
  w.hello_ok().await;
  assert_eq!(
    w.next().await,
    json!({ "type": "attachView", "viewId": "W", "host": "editor", "initial": "S9", "blobBase": "acpira-app://localhost/blobs/W" })
  );
  assert_eq!(w.next().await, json!({ "type": "webviewMessage", "viewId": "W", "message": { "type": "ready" } }));
  w.quiet().await;
  // After the handshake a close is an envelope of its own
  c.detach("W");
  assert_eq!(w.next().await, json!({ "type": "detachView", "viewId": "W" }));
  c.dispose().await;
}

#[tokio::test]
async fn output_of_a_replaced_connection_never_reaches_the_views() {
  let (engine, ep) = Engine::new();
  let (c, _, _) = client(Some(ep), nowhere());
  let v = View::new("V", None);
  c.attach(v.clone());
  c.start();
  let mut w = engine.accept().await;
  w.hello_ok().await;
  w.next().await;
  let old = c.current_generation();
  drop(w);
  let mut w = engine.accept().await;
  assert!(c.current_generation() > old);
  c.on_line_for_test(old, &json!({ "type": "hostMessage", "viewId": "V", "message": { "type": "toast" } }).to_string());
  c.on_line_for_test(old, &json!({ "type": "helloOk", "requestId": "h1", "protocolVersion": 1, "sessionsDir": "/old" }).to_string());
  assert!(v.kinds().is_empty());
  assert_ne!(c.current().0, SidecarState::Ready);
  w.hello_ok().await;
  until("ready on the new connection", || c.current().0 == SidecarState::Ready).await;
  c.dispose().await;
}

#[tokio::test]
async fn preview_answers_for_a_superseded_open_are_dropped_and_leases_are_asked_again_after_a_reconnect() {
  let (engine, ep) = Engine::new();
  let (c, _, _) = client(Some(ep), nowhere());
  c.start();
  let mut w = engine.accept().await;
  w.hello_ok().await;
  let answers = Arc::new(Mutex::new(Vec::<PreviewAnswer>::new()));
  let lease = |url: &str| {
    let a = answers.clone();
    PreviewLease {
      panel_id: "P".into(),
      url: url.into(),
      picker: "/picker.js".into(),
      folder: None,
      on_opened: Box::new(move |r| a.lock().push(r)),
    }
  };
  let _first = c.preview(lease("http://127.0.0.1:3000/"));
  let p1 = w.next().await;
  assert_eq!(p1["type"], "previewOpen");
  let second = c.preview(lease("http://127.0.0.1:3000/b"));
  let p2 = w.next().await;
  // The first open's answer comes late: the panel shows another page now
  w.send(json!({ "type": "previewOpened", "requestId": p1["requestId"], "url": "http://127.0.0.1:9/", "token": "t1" })).await;
  w.send(json!({ "type": "previewOpened", "requestId": p2["requestId"], "url": "http://127.0.0.1:9/b", "token": "t2" })).await;
  until("one answer", || !answers.lock().is_empty()).await;
  tokio::time::sleep(Duration::from_millis(50)).await;
  assert_eq!(*answers.lock(), [PreviewAnswer::Opened { url: "http://127.0.0.1:9/b".into(), token: "t2".into() }]);

  drop(w);
  let mut w = engine.accept().await;
  w.hello_ok().await;
  let again = w.next().await;
  assert_eq!((again["type"].as_str(), again["url"].as_str()), (Some("previewOpen"), Some("http://127.0.0.1:3000/b")));
  w.send(json!({ "type": "previewOpened", "requestId": again["requestId"], "error": "refused" })).await;
  until("the refusal", || answers.lock().len() == 2).await;
  assert_eq!(answers.lock()[1], PreviewAnswer::Error("refused".into()));
  second.release();
  assert_eq!(w.next().await, json!({ "type": "previewClose", "panelId": "P" }));
  c.dispose().await;
}

#[tokio::test]
async fn platform_requests_are_answered_and_an_unknown_method_is_refused() {
  let (engine, ep) = Engine::new();
  let (c, requests, _) = client(Some(ep), nowhere());
  c.start();
  let mut w = engine.accept().await;
  w.hello_ok().await;
  w.send(json!({ "type": "platformRequest", "requestId": "r1", "request": { "method": "openExternal", "url": "https://x" } })).await;
  assert_eq!(w.next().await, json!({ "type": "platformResponse", "requestId": "r1", "result": { "ok": true } }));
  w.send(json!({ "type": "platformRequest", "requestId": "r2", "request": { "method": "openExternal", "url": "https://fail" } })).await;
  assert_eq!(w.next().await, json!({ "type": "platformResponse", "requestId": "r2", "error": "refused" }));
  w.send(json!({ "type": "platformRequest", "requestId": "r3", "request": { "method": "teleport" } })).await;
  assert_eq!(
    w.next().await,
    json!({ "type": "platformResponse", "requestId": "r3", "error": "teleport is not supported by the desktop app" })
  );
  // A notification gets no answer
  w.send(json!({ "type": "platformRequest", "request": { "method": "toast", "level": "info", "text": "hi" } })).await;
  w.quiet().await;
  assert_eq!(requests.lock().len(), 3);
  c.dispose().await;
}

#[tokio::test]
async fn an_engine_shutting_down_is_left_for_the_next_one() {
  let (engine, ep) = Engine::new();
  let (c, _, _) = client(Some(ep), nowhere());
  c.start();
  let mut w = engine.accept().await;
  let hello = w.next().await;
  w.send(json!({ "type": "helloReject", "requestId": hello["requestId"], "protocolVersion": 1, "reason": "engine shutting down" })).await;
  let mut w = engine.accept().await;
  w.hello_ok().await;
  until("ready", || c.current().0 == SidecarState::Ready).await;
  c.dispose().await;
}

#[tokio::test]
async fn falls_through_the_candidates_gives_up_after_repeated_exits_and_starts_over_on_retry() {
  let dies =
    SidecarCommand { command: "/bin/sh".into(), attached_args: vec![], args: vec!["-c".into(), "exit 3".into()], env: Default::default(), label: "dies".into() };
  let (c, _, logs) = client(None, vec![SidecarCommand::binary("/nonexistent/acpira"), dies]);
  c.start();
  until("failed", || c.current().0 == SidecarState::Failed).await;
  let (_, detail) = c.current();
  assert!(detail.unwrap_or_default().contains("keeps exiting"));
  let l = logs.lock().join("\n");
  assert!(l.contains("could not start (binary /nonexistent/acpira)"), "{l}");
  assert!(l.contains("sidecar exited (3)"), "{l}");
  drop(l);
  c.retry();
  until("starting again", || c.current().0 == SidecarState::Starting).await;
  c.dispose().await;
}

#[tokio::test]
async fn speaks_over_stdio_and_shuts_the_child_down_on_dispose() {
  let dir = tempfile::tempdir().unwrap();
  let marker = dir.path().join("got");
  // A one-line engine: answers hello, then records what it is told until its input closes
  let script = format!(
    "read line; echo '{{\"type\":\"helloOk\",\"requestId\":\"h1\",\"protocolVersion\":1,\"sidecar\":{{\"version\":\"sh\",\"pid\":0}},\"sessionsDir\":\"/s\"}}'; cat > '{}'",
    marker.display()
  );
  let sh = SidecarCommand { command: "/bin/sh".into(), attached_args: vec![], args: vec!["-c".into(), script], env: Default::default(), label: "sh".into() };
  let (c, _, _) = client(None, vec![sh]);
  c.start();
  until("ready", || c.current().0 == SidecarState::Ready).await;
  c.dispose().await;
  until("the child's record", || std::fs::read_to_string(&marker).is_ok_and(|s| s.contains("\"shutdown\""))).await;
}
