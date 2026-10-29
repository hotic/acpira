//! acpira-mobile against an in-process gateway bridged to the real engine (acpira-host's persistent server, in process,
//! on a Unix socket), everything on loopback with relays off: pairing by link and by code, hello → helloOk through the
//! bridge, a blob, a probe, and how each way a link ends reaches Swift. No agent runs and nothing leaves the machine

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

use std::path::PathBuf;
use std::sync::{Arc, Once};
use std::time::Duration;

use acpira_gateway::engine::EngineSpec;
use acpira_gateway::gateway::{Gateway, Opts};
use acpira_host::sidecar::server::{Engine, ServerOpts};
use acpira_mobile::{ComputerAddr, DeviceType, Link, LinkEvent, LinkFailure, LinkPath, Node, PairFailure, RelayChoice, decode_link, generate_key, protocol_version};
use acpira_net::endpoint::Relays;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

const LOOPBACK: &str = "127.0.0.1:0";

/// Before any test reads the environment: every test calls this first, and `Once` holds the others until it is set
fn quiet_engine() {
  static ENV: Once = Once::new();
  // SAFETY: every test of this binary waits here before it starts a thread or reads the environment
  ENV.call_once(|| unsafe {
    std::env::set_var("ACPIRA_CATALOG_REFRESH", "0");
    std::env::set_var("ACPIRA_LOGIN_PATH", "0");
  });
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

/// A gateway on loopback and, when `with_engine`, the engine it bridges to; dropped in order by `stop`
struct Desk {
  g: Arc<Gateway>,
  engine: Option<(Arc<Engine>, tokio::task::JoinHandle<()>)>,
  _home: tempfile::TempDir,
  _run: tempfile::TempDir,
}

impl Desk {
  async fn start(with_engine: bool) -> Desk {
    let home = tempfile::tempdir().unwrap();
    // Short: the engine's socket lives under it
    let run = tempfile::Builder::new().prefix("acmb").tempdir_in("/tmp").unwrap();
    std::fs::write(home.path().join("agents.json"), "{}").unwrap();
    let socket = run.path().join("e.sock");
    let engine = with_engine.then(|| {
      let e = Engine::persistent(ServerOpts { version: "test".into(), home: home.path().to_path_buf(), log: Arc::new(|_: &str| {}), ignore_client_agents: false, bridge_exe: None });
      let acceptor = tokio::spawn(serve_engine(e.clone(), UnixListener::bind(&socket).unwrap()));
      (e, acceptor)
    });
    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: EngineSpec { binary: "/nonexistent/acpira".into(), home: home.path().to_path_buf(), explicit_home: true, env: Default::default(), socket: Some(socket) },
      relays: Some(Relays::Off),
      bind: Some(LOOPBACK.parse().unwrap()),
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();
    Desk { g, engine, _home: home, _run: run }
  }

  /// Where the Bonjour record would say the computer is
  fn addr(&self) -> ComputerAddr {
    ComputerAddr { id: self.g.endpoint().id().to_string(), relay: None, addrs: self.g.endpoint().bound_sockets().iter().map(ToString::to_string).collect() }
  }

  async fn stop(self) {
    self.g.stop().await;
    if let Some((e, acceptor)) = self.engine {
      acceptor.abort();
      e.stop("test over").await;
    }
  }
}

async fn phone() -> Arc<Node> {
  Node::start_bound(generate_key().try_into().unwrap(), RelayChoice::Off, Some(LOOPBACK.parse().unwrap())).await.unwrap()
}

/// The next envelope whose `type` is `ty`, skipping everything else
async fn envelope(link: &Link, ty: &str) -> Value {
  let wait = async {
    loop {
      match link.recv().await {
        Ok(Some(LinkEvent::Envelope { line })) => {
          let v: Value = serde_json::from_str(&line).unwrap();
          if v["type"] == ty {
            return v;
          }
        }
        Ok(Some(_)) => {}
        other => panic!("link ended waiting for {ty}: {other:?}"),
      }
    }
  };
  tokio::time::timeout(Duration::from_secs(20), wait).await.unwrap_or_else(|_| panic!("no {ty}"))
}

/// How the link ends, skipping what is still queued before it
async fn end_of(link: &Link) -> Result<Option<LinkEvent>, LinkFailure> {
  let wait = async {
    loop {
      match link.recv().await {
        Ok(Some(_)) => {}
        end => return end,
      }
    }
  };
  tokio::time::timeout(Duration::from_secs(10), wait).await.expect("the link did not end")
}

fn hello() -> String {
  json!({
    "type": "hello", "protocolVersion": protocol_version(), "requestId": "h1",
    "client": { "name": "iPhone", "version": "1.0", "capabilities": ["toast", "notify"] },
    "env": { "hostLanguage": "zh-CN" }, "settings": {},
  })
  .to_string()
}

#[test]
fn pairs_by_link_reaches_the_engine_and_learns_it_was_revoked() {
  quiet_engine();
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let desk = Desk::start(true).await;
    let link = desk.g.open_pairing().link;
    let named = decode_link(link.clone()).unwrap();
    assert_eq!((named.id.as_str(), named.name.as_str()), (desk.addr().id.as_str(), desk.g.name().as_str()));
    assert_eq!(decode_link("acpira://pair/nope".into()), Err(PairFailure::InvalidLink));

    let node = phone().await;
    let paired = node.pair_link(link.clone(), "Spark 的 iPhone".into(), DeviceType::Phone, "1.0".into(), 10_000).await.unwrap();
    assert_eq!(paired.id, desk.addr().id);
    assert!(!paired.addrs.is_empty(), "somewhere to dial next time");
    assert_eq!(desk.g.status().devices[0].device.id, node.id());
    // The window took one pairing
    assert_eq!(node.pair_link(link, "again".into(), DeviceType::Phone, "1.0".into(), 10_000).await, Err(PairFailure::Closed));

    let to = ComputerAddr { id: paired.id.clone(), relay: paired.relay.clone(), addrs: paired.addrs.clone() };
    // A protocol this gateway does not speak is told apart before any envelope
    match node.connect(to.clone(), protocol_version() + 1, 10_000).await {
      Err(LinkFailure::Incompatible { message }) => assert!(message.contains("update Acpira on the computer"), "{message}"),
      other => panic!("{:?}", other.map(|_| ())),
    }

    let l = node.connect(to.clone(), protocol_version(), 10_000).await.unwrap();
    assert_eq!(l.welcome().protocol_version, protocol_version());
    assert_eq!(l.welcome().name, desk.g.name());
    l.send(hello()).await.unwrap();
    let ok = envelope(&l, "helloOk").await;
    let sessions_dir = PathBuf::from(ok["sessionsDir"].as_str().unwrap());
    assert!(desk.g.status().devices[0].online);
    assert!(l.ping(3_000).await.unwrap() < 3_000);
    assert_eq!(l.path(), Some(LinkPath::Lan), "loopback counts as the LAN");

    // A session's image over the link; nothing outside the sessions directory
    std::fs::create_dir_all(sessions_dir.join("s1")).unwrap();
    std::fs::write(sessions_dir.join("s1/shot.png"), vec![9u8; 70_000]).unwrap();
    assert_eq!(l.fetch_blob("s1".into(), "shot.png".into(), 5_000).await.unwrap().len(), 70_000);
    assert!(l.fetch_blob("s1".into(), "../../agents.json".into(), 5_000).await.is_err());

    // What only the computer may send is turned back with its name
    l.send(json!({ "type": "webviewMessage", "viewId": "V", "message": { "type": "setSetting", "key": "proxy", "value": "http://x" } }).to_string()).await.unwrap();
    let denied = tokio::time::timeout(Duration::from_secs(10), async {
      loop {
        if let Ok(Some(LinkEvent::Denied { what })) = l.recv().await {
          return what;
        }
      }
    })
    .await
    .unwrap();
    assert_eq!(denied, "setSetting");

    // Removed on the computer: the live link ends as revoked, and so does the next dial
    assert!(desk.g.revoke(&node.id()).unwrap());
    assert_eq!(end_of(&l).await, Err(LinkFailure::Revoked));
    assert_eq!(l.recv().await, Err(LinkFailure::Revoked), "the end stays");
    assert!(matches!(l.send(hello()).await, Err(LinkFailure::Revoked)));
    // Path events end with the link, after the ones still queued
    tokio::time::timeout(Duration::from_secs(5), async { while l.next_path().await.is_some() {} }).await.expect("path events end with the link");
    match node.connect(to, protocol_version(), 10_000).await {
      Err(e) => assert_eq!(e, LinkFailure::Revoked),
      Ok(_) => panic!("admitted after revocation"),
    }

    node.close().await;
    desk.stop().await;
  });
}

#[test]
fn pairs_by_code_and_ends_as_offline_when_the_gateway_stops() {
  quiet_engine();
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let desk = Desk::start(false).await;
    let node = phone().await;
    let code = desk.g.open_pairing().code;
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert_eq!(node.pair_code(desk.addr(), wrong.into(), "iPad".into(), DeviceType::Tablet, "1.0".into(), 10_000).await, Err(PairFailure::Wrong));
    let spaced = format!("{} {}", code.get(..3).unwrap(), code.get(3..).unwrap());
    let paired = node.pair_code(desk.addr(), spaced, "iPad".into(), DeviceType::Tablet, "1.0".into(), 10_000).await.unwrap();
    assert_eq!(paired.id, desk.addr().id);
    assert_eq!(paired.name, desk.g.name());

    // Remote access turned off while connected
    let l = node.connect(desk.addr(), protocol_version(), 10_000).await.unwrap();
    desk.stop().await;
    assert_eq!(end_of(&l).await, Err(LinkFailure::Offline));

    // Closed from this side: a clean end
    node.close().await;
  });
}

#[test]
fn a_computer_nobody_answers_for_is_offline() {
  quiet_engine();
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let node = phone().await;
    let stranger = Node::start_bound(generate_key().try_into().unwrap(), RelayChoice::Off, None).await.unwrap().id();
    // Nowhere to dial at all, and an address where nothing listens
    for addrs in [vec![], vec!["127.0.0.1:9".to_owned()]] {
      let t = std::time::Instant::now();
      match node.connect(ComputerAddr { id: stranger.clone(), relay: None, addrs }, protocol_version(), 1_500).await {
        Err(e) => assert_eq!(e, LinkFailure::Offline),
        Ok(_) => panic!("connected to nobody"),
      }
      assert!(t.elapsed() < Duration::from_secs(5), "the deadline holds: {:?}", t.elapsed());
    }
    assert_eq!(
      node.pair_code(ComputerAddr { id: stranger, relay: None, addrs: vec!["127.0.0.1:9".into()] }, "123456".into(), "x".into(), DeviceType::Phone, "1".into(), 1_500).await,
      Err(PairFailure::Unreachable)
    );
    assert!(matches!(node.connect(ComputerAddr { id: "nope".into(), relay: None, addrs: vec![] }, 1, 1_000).await, Err(LinkFailure::Other { .. })));
    assert!(matches!(acpira_mobile::start_node(vec![1, 2, 3], RelayChoice::Off).await, Err(acpira_mobile::NodeFailure::BadKey)));
    node.close().await;
  });
}

#[test]
fn a_closed_link_ends_cleanly() {
  quiet_engine();
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let desk = Desk::start(false).await;
    let node = phone().await;
    node.pair_link(desk.g.open_pairing().link, "p".into(), DeviceType::Phone, "1".into(), 10_000).await.unwrap();
    let l = node.connect(desk.addr(), protocol_version(), 10_000).await.unwrap();
    l.close();
    assert_eq!(end_of(&l).await, Ok(None));
    // And so do the links of a node closed under them (the app went to the background)
    let l = node.connect(desk.addr(), protocol_version(), 10_000).await.unwrap();
    node.close().await;
    assert_eq!(end_of(&l).await, Ok(None));
    desk.stop().await;
  });
}
