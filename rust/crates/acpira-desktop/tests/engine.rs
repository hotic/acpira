//! The client against the real engine (acpira-host's persistent server, in process, on a Unix socket) and the fake ACP
//! agent: a session runs a round trip, the connection drops, the client reconnects and the view comes back on the same
//! session. Skipped without the repository's node_modules (the fake agent is bundled with esbuild)

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acpira_desktop::client::{ClientOpts, HelloPayload, ShellView, SidecarClient, SidecarState};
use acpira_desktop::engine::EngineEndpoint;
use acpira_desktop::locator::SidecarCommand;
use acpira_desktop::wire::{HostMessage, Initial};
use acpira_host::sidecar::server::{Engine, ServerOpts};
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

/// test/fake-agent.ts as one ES module node runs directly (acpira-host tests/engine/support.rs builds it the same way)
fn fake_agent() -> Option<PathBuf> {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().ok()?;
  let esbuild = root.join("node_modules/esbuild/bin/esbuild");
  let source = root.join("test/fake-agent.ts");
  if !esbuild.is_file() || !source.is_file() {
    eprintln!("skipped: {} is missing (run `pnpm install` in the repository root)", esbuild.display());
    return None;
  }
  let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("desktop-fake-agent.{}.mjs", std::process::id()));
  let script = std::fs::read(&esbuild).is_ok_and(|b| b.starts_with(b"#!"));
  let mut cmd = if script {
    let mut c = std::process::Command::new("node");
    c.arg(&esbuild);
    c
  } else {
    std::process::Command::new(&esbuild)
  };
  let ok = cmd
    .arg(&source)
    .args(["--bundle", "--platform=node", "--format=esm", "--log-level=warning"])
    .arg(format!("--outfile={}", out.display()))
    .status()
    .ok()?
    .success();
  ok.then_some(out)
}

struct View {
  messages: Mutex<Vec<HostMessage>>,
}

impl ShellView for View {
  fn view_id(&self) -> &str {
    "V"
  }
  fn host(&self) -> &str {
    "editor"
  }
  fn initial(&self) -> Option<Initial> {
    Some(Initial::MostRecent)
  }
  fn blob_base(&self) -> Option<String> {
    None
  }
  fn on_host_message(&self, m: HostMessage) {
    self.messages.lock().unwrap().push(m);
  }
  fn on_state(&self, _: SidecarState, _: Option<&str>) {}
}

impl View {
  /// The newest session view (full pushes and init) the predicate accepts, from message `from` on
  async fn session(&self, from: usize, pred: impl Fn(&Value) -> bool) -> (usize, Value) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
      {
        let ms = self.messages.lock().unwrap();
        for (i, m) in ms.iter().enumerate().skip(from) {
          let v: Value = serde_json::from_str(m.raw.get()).unwrap();
          let s = match m.kind.as_str() {
            "session" => v["session"].clone(),
            "init" => v["state"]["active"].clone(),
            _ => continue,
          };
          if s.is_object() && pred(&s) {
            return (i + 1, s);
          }
        }
        assert!(Instant::now() < deadline, "timed out; kinds: {:?}", ms.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>());
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
  }
}

/// Accepts connections for the engine; each connection's socket halves live in tasks the test can abort to drop it
async fn accept(engine: Arc<Engine>, listener: UnixListener, conns: Arc<Mutex<Vec<Vec<tokio::task::JoinHandle<()>>>>>) {
  loop {
    let Ok((stream, _)) = listener.accept().await else { return };
    let (read, mut write) = stream.into_split();
    let (line_tx, line_rx) = mpsc::unbounded_channel::<String>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
      while let Some(line) = out_rx.recv().await {
        if write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
          break;
        }
      }
    });
    let reader = tokio::spawn(async move {
      let mut lines = BufReader::new(read).lines();
      while let Ok(Some(l)) = lines.next_line().await {
        if line_tx.send(l).is_err() {
          break;
        }
      }
    });
    let e = engine.clone();
    tokio::spawn(async move {
      e.serve(out_tx, line_rx).await;
    });
    conns.lock().unwrap().push(vec![writer, reader]);
  }
}

fn raw(v: Value) -> Box<RawValue> {
  RawValue::from_string(v.to_string()).unwrap()
}

async fn until(what: &str, f: impl Fn() -> bool) {
  let deadline = Instant::now() + Duration::from_secs(20);
  while !f() {
    assert!(Instant::now() < deadline, "timed out waiting for {what}");
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
}

#[test]
fn a_session_comes_back_on_the_same_engine_after_the_connection_drops() {
  let Some(fake) = fake_agent() else { return };
  // SAFETY: set before the runtime and any other thread of this test binary start
  unsafe {
    std::env::set_var("ACPIRA_CATALOG_REFRESH", "0");
    std::env::set_var("ACPIRA_LOGIN_PATH", "0");
  }
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let run = tempfile::Builder::new().prefix("acde").tempdir_in("/tmp").unwrap();
    let socket = run.path().join("e.sock");
    let engine = Engine::persistent(ServerOpts {
      version: "test".into(),
      home: home.path().to_path_buf(),
      log: Arc::new(|_: &str| {}),
      ignore_client_agents: false,
      bridge_exe: None,
    });
    let conns = Arc::new(Mutex::new(Vec::new()));
    let acceptor = tokio::spawn(accept(engine.clone(), UnixListener::bind(&socket).unwrap(), conns.clone()));

    let ep = EngineEndpoint { socket: socket.clone(), log: run.path().join("e.log") };
    let cwd_s = cwd.path().to_string_lossy().into_owned();
    let mut settings = Map::new();
    settings.insert("defaultAgent".into(), json!("fake"));
    settings.insert("agents".into(), json!({ "fake": { "name": "Fake", "command": "node", "args": [fake.to_string_lossy()] } }));
    let states = Arc::new(Mutex::new(Vec::<SidecarState>::new()));
    let st = states.clone();
    let mut opts = ClientOpts::new(
      || vec![SidecarCommand::binary("/nonexistent/acpira")],
      move || HelloPayload {
        client: json!({ "name": "desktop-test", "version": "0", "capabilities": [] }),
        env: json!({ "hostLanguage": "en", "cwd": cwd_s }),
        settings: settings.clone(),
      },
      |_| Box::pin(async { Ok(Value::Null) }),
      Arc::new(|_: &str| {}),
    );
    opts.engine = Box::new(move |_| Some(ep.clone()));
    opts.backoff = Box::new(|_| Duration::from_millis(50));
    opts.on_state = Box::new(move |s, _| st.lock().unwrap().push(s));
    let client = SidecarClient::new(opts, tokio::runtime::Handle::current());
    let view = Arc::new(View { messages: Mutex::new(vec![]) });
    client.attach(view.clone());
    client.send("V", raw(json!({ "type": "ready" })));
    client.start();

    let (at, s) = view.session(0, |s| s["status"] == "ready").await;
    let id = s["id"].as_str().unwrap().to_owned();
    client.send("V", raw(json!({ "type": "send", "sessionId": id, "text": "hello there" })));
    let (at, _) = view.session(at, |s| s["running"] == false && s["turns"].as_array().is_some_and(|t| t.len() >= 2)).await;

    // The engine side drops the connection: the client reconnects and the view comes back on the session it showed
    for task in conns.lock().unwrap()[0].drain(..) {
      task.abort();
    }
    until("a second connection", || conns.lock().unwrap().len() == 2).await;
    until("ready again", || states.lock().unwrap().iter().filter(|s| **s == SidecarState::Ready).count() == 2).await;
    client.send("V", raw(json!({ "type": "ready" })));
    let (_, back) = view.session(at, |s| s["id"] == id.as_str()).await;
    assert!(back["turns"].as_array().is_some_and(|t| t.len() >= 2), "the round trip is still there: {back}");

    client.dispose().await;
    acceptor.abort();
    engine.stop("test over").await;
    let _ = std::fs::remove_file(Path::new(&fake));
  });
}
