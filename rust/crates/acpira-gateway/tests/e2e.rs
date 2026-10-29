//! The gateway against the real engine (acpira-host's persistent server, in process, on a Unix socket) and the fake ACP
//! agent, with a phone played by an endpoint on loopback: pairing, the session list, a turn, a resend with the same
//! clientId, a refused message, a blob, and revocation. No model calls. Skipped without the repository's node_modules

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::string_slice, reason = "test code")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use acpira_gateway::engine::EngineSpec;
use acpira_gateway::gateway::{Gateway, Opts};
use acpira_host::sidecar::server::{Engine, ServerOpts};
use acpira_net::endpoint::{self, ALPN_LINK, Options, Relays};
use acpira_net::frame::FrameKind;
use acpira_net::link::{self, BlobRequest, EnvelopeRx, EnvelopeTx, close};
use acpira_net::pair::{self, DeviceKind, PairReply};
use acpira_net::ticket::PairTicket;
use acpira_net::SecretKey;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

/// test/fake-agent.ts as one ES module node runs directly (acpira-desktop tests/engine.rs builds it the same way)
fn fake_agent() -> Option<PathBuf> {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().ok()?;
  let esbuild = root.join("node_modules/esbuild/bin/esbuild");
  let source = root.join("test/fake-agent.ts");
  if !esbuild.is_file() || !source.is_file() {
    eprintln!("skipped: {} is missing (run `pnpm install` in the repository root)", esbuild.display());
    return None;
  }
  let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("gateway-fake-agent.{}.mjs", std::process::id()));
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

async fn serve_engine(engine: Arc<Engine>, listener: UnixListener) {
  loop {
    let Ok((stream, _)) = listener.accept().await else { return };
    let (read, mut write) = stream.into_split();
    let (line_tx, line_rx) = mpsc::unbounded_channel::<String>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
      while let Some(line) = out_rx.recv().await {
        if write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
          break;
        }
      }
    });
    tokio::spawn(async move {
      let mut lines = BufReader::new(read).lines();
      while let Ok(Some(l)) = lines.next_line().await {
        if line_tx.send(l).is_err() {
          break;
        }
      }
    });
    let e = engine.clone();
    tokio::spawn(async move { e.serve(out_tx, line_rx).await });
  }
}

/// The phone's view of the link: envelopes in, kept in order, plus the control frames
struct Phone {
  tx: EnvelopeTx,
  seen: Arc<parking_lot::Mutex<Vec<(FrameKind, Value)>>>,
}

impl Phone {
  fn start(tx: EnvelopeTx, mut rx: EnvelopeRx) -> Phone {
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let s = seen.clone();
    tokio::spawn(async move {
      while let Ok(Some((kind, body))) = rx.next().await {
        s.lock().push((kind, serde_json::from_slice(&body).unwrap()));
      }
    });
    Phone { tx, seen }
  }

  async fn send(&mut self, v: Value) {
    self.tx.send(FrameKind::Envelope, v.to_string().as_bytes()).await.unwrap();
  }

  async fn msg(&mut self, view: &str, m: Value) {
    self.send(json!({ "type": "webviewMessage", "viewId": view, "message": m })).await;
  }

  /// The first frame from `from` on that `pred` accepts, and the index after it
  async fn wait(&self, from: usize, what: &str, pred: impl Fn(FrameKind, &Value) -> bool) -> (usize, Value) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
      {
        let seen = self.seen.lock();
        for (i, (k, v)) in seen.iter().enumerate().skip(from) {
          if pred(*k, v) {
            return (i + 1, v.clone());
          }
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}; got {:?}", seen.iter().map(|(_, v)| v["type"].clone()).collect::<Vec<_>>());
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
  }

  /// The newest whole session view pushed to `view` that `pred` accepts
  async fn session(&self, from: usize, pred: impl Fn(&Value) -> bool) -> (usize, Value) {
    let (at, v) = self
      .wait(from, "a session view", |k, v| {
        k == FrameKind::Envelope && v["type"] == "hostMessage" && session_of(v).is_some_and(|s| pred(&s))
      })
      .await;
    (at, session_of(&v).unwrap())
  }
}

fn session_of(env: &Value) -> Option<Value> {
  let m = &env["message"];
  let s = match m["type"].as_str()? {
    "session" => m["session"].clone(),
    "init" => m["state"]["active"].clone(),
    _ => return None,
  };
  s.is_object().then_some(s)
}

fn turns(s: &Value) -> usize {
  s["turns"].as_array().map_or(0, Vec::len)
}

#[test]
fn a_phone_pairs_drives_a_session_and_is_revoked() {
  let Some(fake) = fake_agent() else { return };
  // SAFETY: set before the runtime and any other thread of this test binary start
  unsafe {
    std::env::set_var("ACPIRA_CATALOG_REFRESH", "0");
    std::env::set_var("ACPIRA_LOGIN_PATH", "0");
  }
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let run = tempfile::Builder::new().prefix("acgw").tempdir_in("/tmp").unwrap();
    // The agents and the default every shell shares (written by the shells in real life)
    std::fs::write(home.path().join("agents.json"), json!({ "fake": { "name": "Fake", "command": "node", "args": [fake.to_string_lossy()] } }).to_string()).unwrap();
    std::fs::write(home.path().join("settings.json"), json!({ "defaultAgent": "fake" }).to_string()).unwrap();

    let socket = run.path().join("e.sock");
    let engine = Engine::persistent(ServerOpts { version: "test".into(), home: home.path().to_path_buf(), log: Arc::new(|_: &str| {}), ignore_client_agents: false, bridge_exe: None });
    let acceptor = tokio::spawn(serve_engine(engine.clone(), UnixListener::bind(&socket).unwrap()));

    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: EngineSpec { binary: "/nonexistent/acpira".into(), home: home.path().to_path_buf(), explicit_home: true, env: Default::default(), socket: Some(socket.clone()) },
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();

    // Pairing from the link the settings page would show
    let ticket = PairTicket::decode(&g.open_pairing().link).unwrap();
    let phone = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![], bind: Some("127.0.0.1:0".parse().unwrap()) }).await.unwrap();
    let r = pair::pair_with(&phone, &ticket, "Spark 的 iPhone", DeviceKind::Phone, "1.0", Duration::from_secs(10)).await.unwrap();
    assert!(matches!(&r, PairReply::Paired { .. }), "{r:?}");
    let st = g.status();
    assert!(st.pairing.is_none(), "a used window closes");
    assert_eq!(st.devices.len(), 1);
    assert_eq!(st.devices[0].device.id, phone.id().to_string());

    // The link: welcome, then the device's hello goes to the engine as a peer's
    let conn = endpoint::dial(&phone, ticket.endpoint_addr(), ALPN_LINK, Duration::from_secs(10)).await.unwrap();
    let (tx, rx) = link::open_envelopes(&conn).await.unwrap();
    let mut p = Phone::start(tx, rx);
    let (at, w) = p.wait(0, "welcome", |k, _| k == FrameKind::Control).await;
    assert_eq!(w["type"], "welcome");
    p.send(json!({
      "type": "hello", "protocolVersion": acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION, "requestId": "h1",
      "client": { "name": "iPhone", "version": "1.0", "capabilities": ["openResolvedFile", "writeSetting"], "role": "window" },
      "env": { "hostLanguage": "zh-CN", "cwd": "/" }, "settings": { "defaultAgent": "not-this" },
    }))
    .await;
    let (at, ok) = p.wait(at, "helloOk", |_, v| v["type"] == "helloOk").await;
    let sessions_dir = PathBuf::from(ok["sessionsDir"].as_str().unwrap());
    assert!(g.status().devices[0].online);

    // A chat view, a new session in the project, a turn
    p.send(json!({ "type": "attachView", "viewId": "V", "host": "editor", "initial": { "empty": true } })).await;
    p.msg("V", json!({ "type": "ready", "patches": false })).await;
    let cwd = project.path().to_string_lossy().into_owned();
    p.msg("V", json!({ "type": "newSession", "agent": "fake", "cwd": cwd })).await;
    let (at, s) = p.session(at, |s| s["status"] == "ready").await;
    let id = s["id"].as_str().unwrap().to_owned();
    let send = |cid: &str| json!({ "type": "send", "sessionId": id, "text": "hello from the phone", "clientId": cid });
    p.msg("V", send("c1")).await;
    let (at, _) = p.session(at, |s| s["running"] == false && turns(s) == 2).await;
    // The same clientId again (a resend after a lost link) runs nothing; a new one runs
    p.msg("V", send("c1")).await;
    p.msg("V", send("c2")).await;
    let (at, s) = p.session(at, |s| s["running"] == false && turns(s) >= 4).await;
    assert_eq!(turns(&s), 4, "{s}");
    // Settled: a duplicate that did run would have queued a third turn by now
    tokio::time::sleep(Duration::from_millis(800)).await;
    let last = p.seen.lock().iter().rev().find_map(|(_, v)| session_of(v)).unwrap();
    assert_eq!((turns(&last), last["running"].clone()), (4, json!(false)));

    // What only the computer may send never reaches the engine
    p.msg("V", json!({ "type": "setSetting", "key": "proxy", "value": "http://x" })).await;
    let (at, d) = p.wait(at, "denied", |k, v| k == FrameKind::Control && v["type"] == "denied").await;
    assert_eq!(d["what"], "setSetting");
    p.msg("V", json!({ "type": "imageBlob", "sessionId": id, "path": "/etc/hosts", "seq": 1 })).await;
    p.wait(at, "denied", |k, v| k == FrameKind::Control && v["what"] == "imageBlob").await;
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(home.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings["defaultAgent"], "fake");

    // Blobs: a session's own file, nothing outside its directory
    let blob_dir = sessions_dir.join(&id);
    std::fs::create_dir_all(&blob_dir).unwrap();
    std::fs::write(blob_dir.join("shot.png"), vec![7u8; 5000]).unwrap();
    let got = link::fetch_blob(&conn, &BlobRequest { session_id: id.clone(), name: "shot.png".into() }, Duration::from_secs(5)).await.unwrap();
    assert_eq!(got.len(), 5000);
    for (sid, name) in [(id.as_str(), "../../agents.json"), ("..", "settings.json"), (id.as_str(), ".hidden")] {
      let r = link::fetch_blob(&conn, &BlobRequest { session_id: sid.into(), name: name.into() }, Duration::from_secs(5)).await;
      assert!(r.is_err(), "{sid}/{name}");
    }

    // Notifications: the registration the phone sends is kept beside the devices, and goes with the device
    let target = json!({ "token": "ab".repeat(32), "topic": "cc.hotic.acpira", "team": "TEAM", "sandbox": true });
    p.tx.send(FrameKind::Control, json!({ "type": "push", "target": target }).to_string().as_bytes()).await.unwrap();
    let registrations = || std::fs::read_to_string(home.path().join("devices").join("push.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let deadline = Instant::now() + Duration::from_secs(5);
    while registrations().is_none_or(|r| r[phone.id().to_string()] != target) {
      assert!(Instant::now() < deadline, "the registration was not kept: {:?}", registrations());
      tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Revoked: the link drops at once, and the next dial is turned away
    assert!(g.revoke(&phone.id().to_string()).unwrap());
    assert_eq!(registrations(), Some(json!({})));
    let why = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await.unwrap();
    assert_eq!(link::close_code(&why), Some(close::REVOKED));
    let again = endpoint::dial(&phone, ticket.endpoint_addr(), ALPN_LINK, Duration::from_secs(10)).await.unwrap();
    let why = tokio::time::timeout(Duration::from_secs(5), again.closed()).await.unwrap();
    assert_eq!(link::close_code(&why), Some(close::UNKNOWN));
    assert!(g.status().devices.is_empty());

    phone.close().await;
    g.stop().await;
    acceptor.abort();
    engine.stop("test over").await;
    let _ = std::fs::remove_file(Path::new(&fake));
  });
}

/// The settings page's side (acpira-host `devices`): it follows the gateway's control socket and drives it
#[test]
fn the_settings_page_follows_the_gateway() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    // Short: the control socket lives under it
    let home = tempfile::Builder::new().prefix("acgw").tempdir_in("/tmp").unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: EngineSpec { binary: "/nonexistent/acpira".into(), home: home.path().to_path_buf(), explicit_home: true, env: Default::default(), socket: None },
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();
    let stop = Arc::new(tokio::sync::Notify::new());
    let control = tokio::spawn(acpira_gateway::control::serve(g.clone(), acpira_gateway::control::bind(&g.paths.socket).await.unwrap(), stop));

    let ra = acpira_host::devices::RemoteAccess::new(home.path(), user_home.path(), None, Arc::new(|l: &str| eprintln!("engine: {l}")));
    let (tx, mut views) = mpsc::unbounded_channel();
    ra.subscribe(Arc::new(move |v| {
      let _ = tx.send(v.clone());
    }));
    let mut next = async |what: &str, pred: &dyn Fn(&acpira_shared::devices::RemoteAccessView) -> bool| {
      let deadline = Instant::now() + Duration::from_secs(5);
      loop {
        let v = tokio::time::timeout_at(deadline.into(), views.recv()).await.unwrap_or_else(|_| panic!("no view: {what}")).unwrap();
        if pred(&v) {
          return v;
        }
      }
    };

    let v = next("the gateway's report", &|v| v.gateway.is_some()).await;
    assert!(!v.available && v.unavailable.as_deref() == Some(if cfg!(target_os = "macos") { "component" } else { "platform" }));
    assert!(!v.enabled);
    assert_eq!(v.gateway.as_ref().unwrap().id, g.endpoint().id().to_string());

    ra.op(acpira_shared::devices::RemoteAccessOp::Pair).await.unwrap();
    let v = next("a pairing window", &|v| v.gateway.as_ref().is_some_and(|g| g.pairing.is_some())).await;
    assert!(v.gateway.unwrap().pairing.unwrap().link.starts_with("acpira://pair/"));
    ra.op(acpira_shared::devices::RemoteAccessOp::CancelPair).await.unwrap();
    next("the window closed", &|v| v.gateway.as_ref().is_some_and(|g| g.pairing.is_none())).await;

    assert!(g.keep_awake());
    ra.op(acpira_shared::devices::RemoteAccessOp::SetKeepAwake { on: false }).await.unwrap();
    assert!(!g.keep_awake(), "applied without a restart");
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&g.paths.config).unwrap()).unwrap();
    assert_eq!(cfg["keepAwake"], false);
    assert_eq!(ra.view().keep_awake, Some(false));

    ra.op(acpira_shared::devices::RemoteAccessOp::SetName { name: "  Studio  ".into() }).await.unwrap();
    next("the new name", &|v| v.gateway.as_ref().is_some_and(|g| g.name == "Studio")).await;
    ra.op(acpira_shared::devices::RemoteAccessOp::SetName { name: " ".into() }).await.unwrap();
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&g.paths.config).unwrap()).unwrap();
    assert!(cfg.get("name").is_none(), "blank goes back to the computer's own name");
    assert_ne!(g.name(), "Studio");

    ra.dispose();
    control.abort();
    g.stop().await;
  });
}

/// The switch for real: the engine copies the binaries, writes the LaunchAgent and loads it, the gateway launchd starts
/// answers, a relay change restarts it, and turning it off unloads it. Its plist stays in a temporary home (launchd
/// loads it from anywhere), its relays are off, and the data root is temporary. By hand, after a build of `acpira`:
/// `cargo test -p acpira-gateway --test e2e -- --ignored launch_agent`
#[cfg(target_os = "macos")]
#[test]
#[ignore = "loads a real LaunchAgent"]
fn launch_agent_round_trip() {
  use acpira_host::devices::{RemoteAccess, launch_agent};
  use acpira_shared::devices::RemoteAccessOp as O;
  let gateway = PathBuf::from(env!("CARGO_BIN_EXE_acpira-gateway"));
  let exe = gateway.with_file_name("acpira");
  assert!(exe.is_file(), "build the engine first ({})", exe.display());
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let root = tempfile::Builder::new().prefix("acgw").tempdir_in("/tmp").unwrap();
    let user_home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("devices")).unwrap();
    std::fs::write(root.path().join("devices/gateway.json"), json!({ "relays": { "mode": "off" } }).to_string()).unwrap();
    let label = launch_agent::label(root.path(), user_home.path());
    // Unloaded however the test ends
    struct Unload(String);
    impl Drop for Unload {
      fn drop(&mut self) {
        // SAFETY: getuid has no preconditions
        let uid = unsafe { libc::getuid() };
        let _ = std::process::Command::new("/bin/launchctl").args(["bootout", &format!("gui/{uid}/{}", self.0)]).output();
      }
    }
    let _unload = Unload(label.clone());

    let ra = RemoteAccess::new(root.path(), user_home.path(), Some(exe), Arc::new(|l: &str| eprintln!("engine: {l}")));
    let (tx, mut views) = mpsc::unbounded_channel();
    ra.subscribe(Arc::new(move |v| {
      let _ = tx.send(v.clone());
    }));
    let mut next = async |what: &str, pred: &dyn Fn(&acpira_shared::devices::RemoteAccessView) -> bool| {
      let deadline = Instant::now() + Duration::from_secs(20);
      loop {
        let v = tokio::time::timeout_at(deadline.into(), views.recv()).await.unwrap_or_else(|_| panic!("no view: {what}")).unwrap();
        if pred(&v) {
          return v;
        }
      }
    };
    assert!(ra.view().available);

    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() };
    let target = format!("gui/{uid}/{label}");
    let pid = || {
      let out = std::process::Command::new("/bin/launchctl").args(["print", &target]).output().unwrap();
      String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.trim().strip_prefix("pid = ").map(str::to_owned))
    };

    ra.op(O::Enable).await.unwrap();
    // The op ends on a gateway that answers, never on a view that reads as stopped while it starts
    assert!(ra.view().gateway.is_some(), "running once the op returns");
    let v = next("the LaunchAgent's gateway", &|v| v.gateway.is_some()).await;
    assert!(v.enabled);
    let first = v.gateway.unwrap();
    // Turning it on again (another window, the desktop card opening) leaves the running gateway alone
    let before = pid();
    assert!(before.is_some());
    ra.op(O::Enable).await.unwrap();
    assert_eq!(pid(), before, "not restarted");

    // Unloaded behind the engine's back: the next engine start loads it again
    let out = std::process::Command::new("/bin/launchctl").args(["bootout", &target]).output().unwrap();
    assert!(out.status.success());
    next("gone after the bootout", &|v| v.gateway.is_none()).await;
    ra.refresh_installed().await;
    next("loaded again", &|v| v.gateway.is_some()).await;
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(root.path().join("devices/gateway.json")).unwrap()).unwrap();
    let engine = PathBuf::from(cfg["engine"].as_str().unwrap());
    assert!(engine.starts_with(root.path().join("devices/bin")) && engine.is_file(), "{}", engine.display());

    // A relay change restarts it: same identity, a new process
    ra.op(O::SetRelays { relays: Relays::Off }).await.unwrap();
    next("gone while it restarts", &|v| v.gateway.is_none()).await;
    let again = next("back", &|v| v.gateway.is_some()).await.gateway.unwrap();
    assert_eq!(again.id, first.id);

    ra.op(O::Disable).await.unwrap();
    let v = ra.view();
    assert!(!v.enabled && v.gateway.is_none());
    assert!(!root.path().join("devices/bin").read_dir().unwrap().any(|_| true), "copies removed");
    let loaded = std::process::Command::new("/bin/launchctl").args(["print", &target]).output().unwrap();
    assert!(!loaded.status.success(), "unloaded");
    ra.dispose();
  });
}

/// A phone that found the computer on its network pairs with the code of the open window; no engine involved
#[test]
fn a_phone_on_the_network_pairs_with_the_code() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let home = tempfile::tempdir().unwrap();
    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: EngineSpec { binary: "/nonexistent/acpira".into(), home: home.path().to_path_buf(), explicit_home: true, env: Default::default(), socket: None },
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      log: Arc::new(|_: &str| {}),
    })
    .await
    .unwrap();
    let p = g.open_pairing();
    assert_eq!(p.code.len(), 6);
    assert_eq!(g.status().pairing.unwrap().code, p.code, "the settings page shows the same code");

    // Where the Bonjour TXT record would have said the computer is
    let at = acpira_net::EndpointAddr::from_parts(g.endpoint().id(), g.endpoint().bound_sockets().into_iter().map(acpira_net::iroh::TransportAddr::Ip));
    let phone = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![], bind: Some("127.0.0.1:0".parse().unwrap()) }).await.unwrap();
    let r = pair::pair_with_code(&phone, at, &p.code, "Spark 的 iPhone", DeviceKind::Phone, "1.0", Duration::from_secs(10)).await.unwrap();
    assert!(matches!(&r, PairReply::Paired { .. }), "{r:?}");
    let st = g.status();
    assert!(st.pairing.is_none(), "a used window closes");
    assert_eq!(st.devices.len(), 1);
    assert_eq!(st.devices[0].device.id, phone.id().to_string());
    phone.close().await;
    g.stop().await;
  });
}
