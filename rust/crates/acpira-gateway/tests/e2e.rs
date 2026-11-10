//! The gateway against the real engine (acpira-host's persistent server, in process, as the engine runs it) and the
//! fake ACP agent, with a phone played by an endpoint on loopback: pairing, the session list, a turn, a resend
//! with the same clientId, a refused message, a blob, and revocation. No model calls. Skipped without the repository's
//! node_modules

#![allow(clippy::unwrap_used, clippy::panic, clippy::string_slice, reason = "test code")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use acpira_gateway::engine::Link;
use acpira_gateway::gateway::{Gateway, Opts};
use acpira_host::sidecar::server::{Engine, ServerOpts};
use acpira_net::endpoint::{self, ALPN_LINK, Options, Relays};
use acpira_net::frame::FrameKind;
use acpira_net::link::{self, BlobRequest, EnvelopeRx, EnvelopeTx, close};
use acpira_net::pair::{self, DeviceKind, PairReply};
use acpira_net::ticket::PairTicket;
use acpira_net::SecretKey;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// A short private directory: a Unix socket path must fit sun_path, which a deep temporary directory may not
fn short_tmp() -> tempfile::TempDir {
  let base = if cfg!(unix) { PathBuf::from("/tmp") } else { std::env::temp_dir() };
  tempfile::Builder::new().prefix("acgw").tempdir_in(base).unwrap_or_else(|_| tempfile::tempdir().unwrap())
}

/// test/fake-agent.ts as one ES module node runs directly (acpira-desktop tests/engine.rs builds it the same way)
fn fake_agent() -> Option<PathBuf> {
  // Not a `\\?\` path on Windows: node and esbuild cannot open one
  let root = acpira_host::platform::paths::canonical_for_cli(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")).ok()?;
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
    .is_ok_and(|s| s.success());
  // esbuild is there: a failed bundle is a failure, not a skip
  assert!(ok, "esbuild could not bundle {}", source.display());
  Some(out)
}

/// The gateway's way into `engine`, as acpira-host's `devices::host` gives it
fn link(engine: &Arc<Engine>) -> Link {
  let e = engine.clone();
  Arc::new(move || e.link())
}

/// The phone's view of the link: envelopes in, kept in order, plus the control frames
struct Phone {
  tx: EnvelopeTx,
  seen: Arc<parking_lot::Mutex<Vec<(FrameKind, Value)>>>,
  #[cfg_attr(not(unix), allow(dead_code, reason = "used by the Unix-only terminal tests"))]
  reader: tokio::task::JoinHandle<()>,
}

impl Phone {
  fn start(tx: EnvelopeTx, mut rx: EnvelopeRx) -> Phone {
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let s = seen.clone();
    let reader = tokio::spawn(async move {
      while let Ok(Some((kind, body))) = rx.next().await {
        s.lock().push((kind, serde_json::from_slice(&body).unwrap()));
      }
    });
    Phone { tx, seen, reader }
  }

  async fn send(&mut self, v: Value) {
    self.tx.send(FrameKind::Envelope, v.to_string().as_bytes()).await.unwrap();
  }

  async fn msg(&mut self, view: &str, m: Value) {
    self.send(json!({ "type": "webviewMessage", "viewId": view, "message": m })).await;
  }

  #[cfg(unix)]
  async fn boot(&mut self) {
    self
      .send(json!({
        "type": "hello", "protocolVersion": acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION, "requestId": "terminal-hello",
        "client": { "name": "iPhone", "version": "test", "capabilities": [] }, "env": {}, "settings": {},
      }))
      .await;
    self.wait(0, "helloOk", |_, v| v["type"] == "helloOk").await;
    self.send(json!({ "type": "attachView", "viewId": "V", "host": "editor", "initial": { "empty": true } })).await;
    self.msg("V", json!({ "type": "ready", "patches": false })).await;
    self.wait(0, "init", |_, v| v["type"] == "hostMessage" && v["message"]["type"] == "init").await;
  }

  #[cfg(unix)]
  async fn terminal_text(&self, id: &str, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
      let text: String = self
        .seen
        .lock()
        .iter()
        .filter(|(k, v)| *k == FrameKind::Envelope && v["type"] == "hostMessage" && v["message"]["id"] == id)
        .filter_map(|(_, v)| match v["message"]["type"].as_str() {
          Some("terminalOutput") => v["message"]["data"].as_str(),
          Some("terminalState") => v["message"]["replay"].as_str(),
          _ => None,
        })
        .collect();
      if text.contains(needle) {
        return text;
      }
      assert!(Instant::now() < deadline, "timed out waiting for terminal output: {needle}");
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
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
    // The agents and the default every shell shares (written by the shells in real life)
    std::fs::write(home.path().join("agents.json"), json!({ "fake": { "name": "Fake", "command": "node", "args": [fake.to_string_lossy()] } }).to_string()).unwrap();
    std::fs::write(home.path().join("settings.json"), json!({ "defaultAgent": "fake" }).to_string()).unwrap();

    let engine = Engine::persistent(ServerOpts { version: "test".into(), home: home.path().to_path_buf(), log: Arc::new(|_: &str| {}), ignore_client_agents: false, bridge_exe: None });

    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: link(&engine),
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      ssh: None,
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();

    // Pairing from the link the settings page would show
    let ticket = PairTicket::decode(&g.open_pairing().link).unwrap();
    let phone = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![], bind: Some("127.0.0.1:0".parse().unwrap()), bypass: None }).await.unwrap();
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
    let got = link::fetch_blob(&conn, &BlobRequest { session_id: id.clone(), name: "shot.png".into(), offset: None }, Duration::from_secs(5)).await.unwrap();
    assert_eq!(got.len(), 5000);
    for (sid, name) in [(id.as_str(), "../../agents.json"), ("..", "settings.json"), (id.as_str(), ".hidden")] {
      let r = link::fetch_blob(&conn, &BlobRequest { session_id: sid.into(), name: name.into(), offset: None }, Duration::from_secs(5)).await;
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

    // The terminal runs POSIX sh commands; the link it ends on is the one revoked below
    #[cfg(unix)]
    let conn = {
      // A terminal's state, input, cwd, output and resize all cross the real paired link with the client's original id.
      let tid = format!("phone:{cwd}");
      let open = || json!({ "type": "terminalOpen", "id": tid, "cwd": cwd, "cols": 91, "rows": 25 });
      p.msg("V", json!({ "type": "terminalOpen", "id": "missing-cwd" })).await;
      p.wait(0, "a renderable refusal", |_, v| v["message"]["id"] == "missing-cwd" && v["message"]["error"].is_string()).await;
      p.msg("V", json!({ "type": "terminalOpen", "id": "bad-cwd", "cwd": project.path().join("missing") })).await;
      p.wait(0, "a failed start", |_, v| v["message"]["id"] == "bad-cwd" && v["message"]["error"].is_string()).await;
      p.msg("V", open()).await;
      p.wait(0, "terminal state and title", |_, v| v["message"]["id"] == tid && v["message"]["title"].is_string()).await;
      p.msg("V", json!({ "type": "terminalInput", "id": tid, "data": "ACPIRA_TERMINAL_OWNER=first; printf 'owner:%s\\n' \"$ACPIRA_TERMINAL_OWNER\"; pwd; stty size\r" })).await;
      p.terminal_text(&tid, "owner:first").await;
      p.terminal_text(&tid, &project.path().canonicalize().unwrap().to_string_lossy()).await;
      let text = p.terminal_text(&tid, "25 91").await;
      p.msg("V", json!({ "type": "terminalAck", "id": tid, "chars": text.encode_utf16().count() })).await;
      p.msg("V", json!({ "type": "terminalResize", "id": tid, "cols": 103, "rows": 32 })).await;
      p.msg("V", json!({ "type": "terminalInput", "id": tid, "data": "stty size\r" })).await;
      p.terminal_text(&tid, "32 103").await;

      // A second key with the same display name and terminal id gets a different shell, and cannot address the first's
      // engine id by passing its prefix back as a client id.
      let other = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![], bind: Some("127.0.0.1:0".parse().unwrap()), bypass: None }).await.unwrap();
      let ticket2 = PairTicket::decode(&g.open_pairing().link).unwrap();
      let paired = pair::pair_with(&other, &ticket2, "Spark 的 iPhone", DeviceKind::Phone, "1.0", Duration::from_secs(10)).await.unwrap();
      assert!(matches!(paired, PairReply::Paired { .. }));
      let conn2 = endpoint::dial(&other, ticket2.endpoint_addr(), ALPN_LINK, Duration::from_secs(10)).await.unwrap();
      let (tx, rx) = link::open_envelopes(&conn2).await.unwrap();
      let mut p2 = Phone::start(tx, rx);
      p2.boot().await;
      p2.msg("V", open()).await;
      let (_, state2) = p2.wait(0, "the second terminal", |_, v| v["message"]["id"] == tid && v["message"]["replay"].is_string()).await;
      assert!(!state2["message"]["replay"].as_str().unwrap().contains("owner:first"));
      p2.msg("V", json!({ "type": "terminalInput", "id": tid, "data": "printf 'owner:%s\\n' \"${ACPIRA_TERMINAL_OWNER:-second}\"\r" })).await;
      p2.terminal_text(&tid, "owner:second").await;
      p2.msg("V", json!({ "type": "terminalKill", "id": format!("device:{}:{tid}", phone.id()) })).await;
      p2.msg("V", json!({ "type": "terminalKill", "id": tid })).await;
      p.msg("V", json!({ "type": "terminalInput", "id": tid, "data": "printf 'still:%s\\n' \"$ACPIRA_TERMINAL_OWNER\"\r" })).await;
      p.terminal_text(&tid, "still:first").await;
      assert!(g.revoke(&other.id().to_string()).unwrap());
      other.close().await;

      // A new connection from the same authenticated key replays its shell, then close/reopen and kill/restart retain
      // their existing meaning without granting access to another client's terminal.
      let conn = endpoint::dial(&phone, ticket.endpoint_addr(), ALPN_LINK, Duration::from_secs(10)).await.unwrap();
      let (tx, rx) = link::open_envelopes(&conn).await.unwrap();
      p = Phone::start(tx, rx);
      p.boot().await;
      p.msg("V", open()).await;
      p.terminal_text(&tid, "owner:first").await;
      p.msg("V", json!({ "type": "terminalClose", "id": tid })).await;
      let next = p.seen.lock().len();
      p.msg("V", open()).await;
      p.wait(next, "replay after detaching", |_, v| v["message"]["id"] == tid && v["message"]["replay"].as_str().is_some_and(|s| s.contains("owner:first"))).await;
      p.msg("V", json!({ "type": "terminalKill", "id": tid })).await;
      let next = p.seen.lock().len();
      p.msg("V", open()).await;
      let (_, fresh) = p.wait(next, "a fresh terminal", |_, v| v["message"]["id"] == tid && v["message"]["replay"].is_string()).await;
      assert!(!fresh["message"]["replay"].as_str().unwrap().contains("owner:first"));
      p.msg("V", json!({ "type": "terminalInput", "id": tid, "data": "printf 'owner:%s\\n' \"${ACPIRA_TERMINAL_OWNER:-fresh}\"; exit 7\r" })).await;
      p.terminal_text(&tid, "owner:fresh").await;
      p.wait(next, "terminal exit", |_, v| v["message"]["id"] == tid && v["message"]["exit"]["code"] == 7).await;
      p.msg("V", json!({ "type": "terminalKill", "id": tid })).await;
      conn
    };

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
    engine.stop("test over").await;
    let _ = std::fs::remove_file(Path::new(&fake));
  });
}

// The terminal runs a POSIX sh script
#[cfg(unix)]
#[test]
fn replacing_an_envelope_stream_keeps_terminal_acks_flowing() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // Wait in the same pty across replacement, then emit 330 KB: well above the terminal's ACK high-water mark.
    let line = "0123456789012345678901234567890123456789012345678901234567890123";
    std::fs::write(
      project.path().join("emit.sh"),
      format!(
        r#"stty -echo opost onlcr
printf 'swap-ready\n'
read -r start
printf '\036'
i=0
while [ "$i" -lt 5000 ]; do
  printf '%s\n' '{line}'
  i=$((i + 1))
done
printf '\037'
"#
      ),
    )
    .unwrap();
    let engine = Engine::persistent(ServerOpts {
      version: "test".into(),
      home: home.path().to_path_buf(),
      log: Arc::new(|_: &str| {}),
      ignore_client_agents: false,
      bridge_exe: None,
    });
    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: link(&engine),
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      ssh: None,
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();
    let ticket = PairTicket::decode(&g.open_pairing().link).unwrap();
    let phone = endpoint::bind(Options {
      secret: SecretKey::generate(),
      relays: Relays::Off,
      alpns: vec![],
      bind: Some("127.0.0.1:0".parse().unwrap()),
      bypass: None,
    })
    .await
    .unwrap();
    let paired = pair::pair_with(&phone, &ticket, "Stream replacement", DeviceKind::Phone, "test", Duration::from_secs(10)).await.unwrap();
    assert!(matches!(paired, PairReply::Paired { .. }));
    let conn = endpoint::dial(&phone, ticket.endpoint_addr(), ALPN_LINK, Duration::from_secs(10)).await.unwrap();
    let (tx, rx) = link::open_envelopes(&conn).await.unwrap();
    let mut old = Phone::start(tx, rx);
    old.boot().await;
    let id = "stream-swap";
    let open = || json!({ "type": "terminalOpen", "id": id, "cwd": project.path(), "cols": 80, "rows": 24 });
    old.msg("V", open()).await;
    old.wait(0, "the original terminal", |_, v| v["message"]["id"] == id && v["message"]["replay"].is_string()).await;
    old.msg("V", json!({ "type": "terminalInput", "id": id, "data": "/bin/sh emit.sh\r" })).await;
    old.terminal_text(id, "swap-ready\r\n").await;

    // Keep both halves of the old stream alive. Only the replacement viewer acknowledges the output.
    let (tx, rx) = link::open_envelopes(&conn).await.unwrap();
    let mut p = Phone::start(tx, rx);
    p.boot().await;
    p.msg("V", open()).await;
    let (mut next, _) = p
      .wait(0, "the same terminal's replay", |_, v| {
        v["message"]["id"] == id && v["message"]["replay"].as_str().is_some_and(|s| s.contains("swap-ready\r\n"))
      })
      .await;
    p.msg("V", json!({ "type": "terminalInput", "id": id, "data": "go\r" })).await;
    let mut text = String::new();
    // Less than the terminal's ten-second ACK valve, so a leaked viewer cannot pass through its timeout recovery.
    let output = tokio::time::timeout(Duration::from_secs(5), async {
      loop {
        let (at, v) =
          p.wait(next, "acknowledged terminal output", |_, v| v["message"]["type"] == "terminalOutput" && v["message"]["id"] == id).await;
        next = at;
        let data = v["message"]["data"].as_str().unwrap();
        text.push_str(data);
        p.msg("V", json!({ "type": "terminalAck", "id": id, "chars": data.encode_utf16().count() })).await;
        if text.contains('\u{1f}') {
          break;
        }
      }
    })
    .await;
    let wires = engine.wire_count();
    let old_closed = tokio::time::timeout(Duration::from_secs(2), &mut old.reader).await;
    p.msg("V", json!({ "type": "terminalKill", "id": id })).await;
    phone.close().await;
    g.stop().await;
    engine.stop("test over").await;

    assert!(
      output.is_ok(),
      "terminal output stalled after {} bytes despite ACKs from the new viewer; {wires} engine wires remain",
      text.len()
    );
    let payload = text.split_once('\u{1e}').and_then(|(_, rest)| rest.split_once('\u{1f}')).expect("complete output markers").0;
    assert_eq!(payload, format!("{line}\r\n").repeat(5000), "the replacement must receive the whole 330 KB output in order");
    assert_eq!(wires, 1, "the replaced stream must release its engine wire and terminal viewer");
    assert!(matches!(old_closed, Ok(Ok(()))), "the replaced stream's writer must close");
  });
}

/// The settings page's side (acpira-host `devices`): it follows the gateway's control socket and drives it
#[test]
fn the_settings_page_follows_the_gateway() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    // Short: the control socket lives under it
    let home = short_tmp();
    let user_home = tempfile::tempdir().unwrap();
    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: acpira_gateway::engine::none(),
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      ssh: None,
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();
    let (on, _on) = mpsc::unbounded_channel();
    let control = tokio::spawn(acpira_gateway::control::serve(g.clone(), acpira_gateway::control::bind(&g.paths.socket).await.unwrap(), on));

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
    // No engine binary given, so nothing to install the login service from
    assert!(!v.available && v.unavailable.as_deref() == Some("component"));
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

    // Stopping the control channel (`Hosted::stop`) ends its subscriptions, so a page finds the next gateway instead of
    // following this one forever
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (r, mut w) = tokio::io::split(acpira_shared::local_ipc::connect(&g.paths.socket).await.unwrap());
    let mut req = serde_json::to_string(&acpira_shared::devices::GatewayRequest::Watch).unwrap();
    req.push('\n');
    w.write_all(req.as_bytes()).await.unwrap();
    let mut lines = tokio::io::BufReader::new(r).lines();
    assert!(lines.next_line().await.unwrap().is_some(), "the first status");
    ra.dispose();
    control.abort();
    let end = tokio::time::timeout(Duration::from_secs(2), async {
      while let Ok(Some(_)) = lines.next_line().await {}
    });
    assert!(end.await.is_ok(), "a watch outlived its control channel");
    g.stop().await;
  });
}

/// The engine binary the ignored round trips install: `cargo build --bin acpira` first
fn built_engine() -> PathBuf {
  let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
  let exe = target.join("debug").join(if cfg!(windows) { "acpira.exe" } else { "acpira" });
  assert!(exe.is_file(), "build the engine first ({})", exe.display());
  exe
}

/// The switch for real: the engine copies itself, writes the LaunchAgent and loads it, the engine launchd starts runs
/// the gateway, a relay change restarts the gateway in that same engine, and turning it off leaves the engine running
/// with its gateway stopped. The plist sits in this user's LaunchAgents under a label of the temporary data root's
/// own, since the engine looks for it there; its relays are off. By hand: `cargo build --bin acpira`, then
/// `cargo test -p acpira-gateway --test e2e -- --ignored launch_agent`
#[cfg(target_os = "macos")]
#[test]
#[ignore = "loads a real LaunchAgent"]
fn launch_agent_round_trip() {
  use acpira_host::devices::{RemoteAccess, launch_agent, stage};
  use acpira_shared::devices::RemoteAccessOp as O;
  let exe = built_engine();
  let user_home = PathBuf::from(std::env::var_os("HOME").unwrap());
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let root = tempfile::Builder::new().prefix("acgw").tempdir_in("/tmp").unwrap();
    // The engine resolves the data root as given; /tmp is /private/tmp on macOS
    let root_path = root.path().canonicalize().unwrap();
    std::fs::create_dir_all(root_path.join("devices")).unwrap();
    std::fs::write(root_path.join("devices/gateway.json"), json!({ "relays": { "mode": "off" } }).to_string()).unwrap();
    let label = launch_agent::label(&root_path, &user_home);
    // Unloaded, its plist and its engine gone however the test ends
    struct Unload(String, PathBuf, PathBuf);
    impl Drop for Unload {
      fn drop(&mut self) {
        // SAFETY: getuid has no preconditions
        let uid = unsafe { libc::getuid() };
        let _ = std::process::Command::new("/bin/launchctl").args(["bootout", &format!("gui/{uid}/{}", self.0)]).output();
        let _ = std::fs::remove_file(&self.1);
        let _ = std::process::Command::new("pkill").args(["-f", &format!("serve .*--home {}", self.2.display())]).status();
      }
    }
    let _unload = Unload(label.clone(), launch_agent::plist_path(&label, &user_home), root_path.clone());

    let ra = RemoteAccess::new(&root_path, &user_home, Some(exe), Arc::new(|l: &str| eprintln!("engine: {l}")));
    let (tx, mut views) = mpsc::unbounded_channel();
    ra.subscribe(Arc::new(move |v| {
      let _ = tx.send(v.clone());
    }));
    let mut next = async |what: &str, pred: &dyn Fn(&acpira_shared::devices::RemoteAccessView) -> bool| {
      let deadline = Instant::now() + Duration::from_secs(60);
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
    let first = ra.view().gateway.expect("running once the op returns");
    assert!(ra.view().enabled);
    let engine = pid().expect("launchd runs the engine");
    assert_eq!(first.engine_pid.map(|p| p.to_string()), Some(engine.clone()), "the gateway runs in the engine launchd started");
    assert!(stage::exe(&root_path).is_file(), "the service's copy");
    // Turning it on again (another window, the desktop card opening) leaves the running engine alone
    ra.op(O::Enable).await.unwrap();
    assert_eq!(pid().as_deref(), Some(engine.as_str()), "not restarted");

    // A relay change restarts the gateway in the same engine: same identity, same process
    ra.op(O::SetRelays { relays: Relays::Off }).await.unwrap();
    let again = next("back after the relay change", &|v| v.gateway.is_some()).await.gateway.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.engine_pid, first.engine_pid);

    // Unloaded behind the engine's back (its engine ends with it): the next engine start loads it again
    let out = std::process::Command::new("/bin/launchctl").args(["bootout", &target]).output().unwrap();
    assert!(out.status.success());
    next("gone after the bootout", &|v| v.gateway.is_none()).await;
    ra.refresh_installed().await;
    next("loaded again", &|v| v.gateway.is_some()).await;

    // Off: the gateway stops and the plist goes, the engine (which may serve windows) runs on
    let engine = pid();
    ra.op(O::Disable).await.unwrap();
    let v = ra.view();
    assert!(!v.enabled && v.gateway.is_none());
    assert!(!launch_agent::plist_path(&label, &user_home).exists(), "the plist is removed");
    assert!(!stage::exe(&root_path).exists(), "the copy is removed");
    assert_eq!(pid(), engine, "the engine keeps running");
    ra.dispose();
  });
}

/// The switch for real on Linux and Windows: the engine copies itself, installs the login service (a systemd user unit,
/// or a logon task) and starts it, the engine it starts runs the gateway, an engine killed outright is started again
/// (systemd's restart; on Windows the next engine start), a relay change restarts the gateway in the same engine, and
/// turning it off removes the service and the copy. The service is named after the temporary data root and sits in this
/// user's real service directory, since that is where the service manager looks. By hand: `cargo build --bin acpira`,
/// then `cargo test -p acpira-gateway --test e2e -- --ignored login_service`
#[cfg(any(target_os = "linux", windows))]
#[test]
#[ignore = "installs a real login service"]
fn login_service_round_trip() {
  use acpira_host::devices::{RemoteAccess, stage};
  use acpira_shared::devices::RemoteAccessOp as O;
  let exe = built_engine();
  let user_home = PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap());
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let root = short_tmp();
    std::fs::create_dir_all(root.path().join("devices")).unwrap();
    std::fs::write(root.path().join("devices/gateway.json"), json!({ "relays": { "mode": "off" } }).to_string()).unwrap();
    let ra = RemoteAccess::new(root.path(), &user_home, Some(exe), Arc::new(|l: &str| eprintln!("engine: {l}")));
    let home = root.path().to_string_lossy().into_owned();
    let kill = |home: &str| {
      if cfg!(windows) {
        let script = format!("Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*serve*--home*{home}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}");
        std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).status().unwrap().success()
      } else {
        std::process::Command::new("pkill").args(["-9", "-f", &format!("serve .*--home {home}")]).status().unwrap().success()
      }
    };
    // Removed, and its engine gone, however the test ends
    struct Off(Arc<RemoteAccess>, tokio::runtime::Handle, String);
    impl Drop for Off {
      fn drop(&mut self) {
        let ra = self.0.clone();
        let _ = std::thread::spawn({
          let h = self.1.clone();
          move || h.block_on(async move { ra.op(acpira_shared::devices::RemoteAccessOp::Disable).await })
        })
        .join();
        // The engine goes once idle on its own; a test leaves none behind
        #[cfg(unix)]
        let _ = std::process::Command::new("pkill").args(["-f", &format!("serve .*--home {}", self.2)]).status();
        #[cfg(not(unix))]
        let _ = &self.2;
      }
    }
    let _off = Off(ra.clone(), tokio::runtime::Handle::current(), home.clone());
    let (tx, mut views) = mpsc::unbounded_channel();
    ra.subscribe(Arc::new(move |v| {
      let _ = tx.send(v.clone());
    }));
    let mut next = async |what: &str, pred: &dyn Fn(&acpira_shared::devices::RemoteAccessView) -> bool| {
      let deadline = Instant::now() + Duration::from_secs(60);
      loop {
        let v = tokio::time::timeout_at(deadline.into(), views.recv()).await.unwrap_or_else(|_| panic!("no view: {what}")).unwrap();
        if pred(&v) {
          return v;
        }
      }
    };
    let v = ra.view();
    assert!(v.available, "{v:?}");

    ra.op(O::Enable).await.unwrap();
    let first = ra.view().gateway.expect("running once the op returns");
    assert!(first.engine_pid.is_some(), "the gateway runs in an engine");
    let t = Instant::now();
    ra.op(O::Enable).await.unwrap();
    assert!(t.elapsed() < Duration::from_secs(5), "turning it on again leaves the running engine alone");

    // Killed outright: systemd starts it again after its 10 s; on Windows the next engine start does
    assert!(kill(&home));
    next("gone after the kill", &|v| v.gateway.is_none()).await;
    if cfg!(windows) {
      ra.refresh_installed().await;
    }
    let back = next("started again", &|v| v.gateway.is_some()).await.gateway.unwrap();
    assert_eq!(back.id, first.id);

    // A relay change restarts the gateway in the same engine: same identity, same process
    ra.op(O::SetRelays { relays: Relays::Off }).await.unwrap();
    let again = next("back after the relay change", &|v| v.gateway.is_some()).await.gateway.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.engine_pid, back.engine_pid);

    ra.op(O::Disable).await.unwrap();
    let v = ra.view();
    assert!(!v.enabled && v.gateway.is_none());
    if cfg!(unix) {
      assert!(!stage::exe(root.path()).exists(), "the copy is removed");
    }
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
      engine: acpira_gateway::engine::none(),
      relays: Some(Relays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      ssh: None,
      log: Arc::new(|_: &str| {}),
    })
    .await
    .unwrap();
    let p = g.open_pairing();
    assert_eq!(p.code.len(), 6);
    assert_eq!(g.status().pairing.unwrap().code, p.code, "the settings page shows the same code");

    // Where the Bonjour TXT record would have said the computer is
    let at = acpira_net::EndpointAddr::from_parts(g.endpoint().id(), g.endpoint().bound_sockets().into_iter().map(acpira_net::iroh::TransportAddr::Ip));
    let phone = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![], bind: Some("127.0.0.1:0".parse().unwrap()), bypass: None }).await.unwrap();
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
