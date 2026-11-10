//! acpira-mobile's tunnels against an in-process gateway on loopback with relays off (no engine): a webview's way in
//! through the SOCKS proxy and through a forward, to a server on the computer's loopback, the targets refused on the
//! device, the reasons a page shows, the tunnel ending with its link, and a computer whose Acpira predates tunnels

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use acpira_gateway::gateway::{Gateway, Opts};
use acpira_mobile::{ComputerAddr, DeviceType, Link, LinkFailure, Node, RelayChoice, generate_key, protocol_version};
use acpira_net::endpoint::{self, ALPN_LINK, Options, Relays};
use acpira_net::frame::FrameKind;
use acpira_net::link::{self, Control, Incoming};
use acpira_net::SecretKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const LOOPBACK: &str = "127.0.0.1:0";

async fn phone() -> Arc<Node> {
  Node::start_bound(generate_key().try_into().unwrap(), RelayChoice::Off, Some(LOOPBACK.parse().unwrap())).await.unwrap()
}

/// A gateway with no engine and a phone linked to it
async fn linked() -> (Arc<Gateway>, Arc<Node>, Arc<Link>, tempfile::TempDir) {
  let home = tempfile::tempdir().unwrap();
  let g = Gateway::start(Opts {
    home: home.path().to_path_buf(),
    engine: acpira_gateway::engine::none(),
    relays: Some(Relays::Off),
    bind: Some(LOOPBACK.parse().unwrap()),
    ssh: None,
    log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
  })
  .await
  .unwrap();
  let node = phone().await;
  node.pair_link(g.open_pairing().link, "p".into(), DeviceType::Phone, "1".into(), 10_000).await.unwrap();
  let to = ComputerAddr { id: g.endpoint().id().to_string(), relay: None, addrs: g.endpoint().bound_sockets().iter().map(ToString::to_string).collect() };
  let l = node.connect(to, protocol_version(), 10_000).await.unwrap();
  (g, node, l, home)
}

/// Echoes what it reads and ends its side once the client ended its own; `open` counts the connections it holds
async fn echo() -> (u16, Arc<AtomicUsize>) {
  let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let port = l.local_addr().unwrap().port();
  let open = Arc::new(AtomicUsize::new(0));
  let held = open.clone();
  tokio::spawn(async move {
    while let Ok((mut s, _)) = l.accept().await {
      held.fetch_add(1, Ordering::SeqCst);
      let held = held.clone();
      tokio::spawn(async move {
        let (mut r, mut w) = s.split();
        let _ = tokio::io::copy(&mut r, &mut w).await;
        let _ = w.shutdown().await;
        held.fetch_sub(1, Ordering::SeqCst);
      });
    }
  });
  (port, open)
}

async fn until(what: &str, f: impl Fn() -> bool) {
  let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
  while !f() {
    assert!(tokio::time::Instant::now() < deadline, "{what}");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

/// A SOCKS5 CONNECT through the proxy at `proxy`: the reply code, and the connection when it is 0
async fn socks(proxy: u16, target: &[u8], port: u16) -> (u8, TcpStream) {
  let mut s = TcpStream::connect(("127.0.0.1", proxy)).await.unwrap();
  s.write_all(&[5, 1, 0]).await.unwrap();
  let mut method = [0u8; 2];
  s.read_exact(&mut method).await.unwrap();
  assert_eq!(method, [5, 0]);
  let mut req = vec![5, 1, 0];
  req.extend_from_slice(target);
  req.extend_from_slice(&port.to_be_bytes());
  s.write_all(&req).await.unwrap();
  let mut reply = [0u8; 10];
  tokio::time::timeout(Duration::from_secs(20), s.read_exact(&mut reply)).await.unwrap().unwrap();
  (reply[1], s)
}

fn name(n: &str) -> Vec<u8> {
  let mut v = vec![3, n.len() as u8];
  v.extend_from_slice(n.as_bytes());
  v
}

/// Bytes there and back, then the end
async fn round_trip(mut s: TcpStream, n: usize) {
  let data: Vec<u8> = (0..n).map(|i| (i % 253) as u8).collect();
  let (mut r, mut w) = s.split();
  let send = async {
    w.write_all(&data).await.unwrap();
    w.shutdown().await.unwrap();
  };
  let mut got = Vec::new();
  let read = tokio::time::timeout(Duration::from_secs(20), r.read_to_end(&mut got));
  let ((), read) = tokio::join!(send, read);
  read.unwrap().unwrap();
  assert_eq!(got.len(), data.len());
  assert!(got == data, "the bytes came back as sent");
}

fn here() -> String {
  std::env::temp_dir().join("web").to_string_lossy().into_owned()
}

#[test]
fn a_webview_reaches_a_dev_server_through_the_proxy_and_a_forward() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let (g, node, l, _home) = linked().await;
    let (port, open) = echo().await;
    let t = l.open_tunnel(here()).await.unwrap();

    // The proxy, by name and by address
    let (code, s) = socks(t.port(), &name("localhost"), port).await;
    assert_eq!(code, 0);
    round_trip(s, 300_000).await;
    let (code, s) = socks(t.port(), &[1, 127, 0, 0, 1], port).await;
    assert_eq!(code, 0);
    round_trip(s, 5).await;

    // A forward: the same port when it is free here, else another; asked twice, the same listener
    let local = t.forward(port).unwrap();
    assert_ne!(local, port, "the server holds that port on this machine");
    assert_eq!(t.forward(port).unwrap(), local);
    round_trip(TcpStream::connect(("127.0.0.1", local)).await.unwrap(), 70_000).await;
    let free = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap().local_addr().unwrap().port();
    assert_eq!(t.forward(free).unwrap(), free, "a port free here keeps its number");

    // Refused on the device: nothing leaves for another host
    assert_eq!(socks(t.port(), &name("example.com"), 80).await.0, 2);
    assert_eq!(socks(t.port(), &[1, 10, 0, 0, 1], 80).await.0, 2);
    assert!(t.last_error().unwrap().contains("example.com") || t.last_error().unwrap().contains("10.0.0.1"));
    // Nothing listening there: refused, with the computer's words for the page
    let closed = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap().local_addr().unwrap().port();
    assert_eq!(socks(t.port(), &name("localhost"), closed).await.0, 5);
    assert!(t.last_error().unwrap().contains(&closed.to_string()), "{:?}", t.last_error());

    // A folder the computer cannot reach fails at open, with its reason
    match l.open_tunnel("ssh://nowhere/home/a".into()).await {
      Err(LinkFailure::Other { message }) => assert!(message.contains("nowhere"), "{message}"),
      other => panic!("{:?}", other.map(|_| ())),
    }

    // Closed: the proxy and the forwards stop, and their connections end
    let (code, held) = socks(t.port(), &name("localhost"), port).await;
    assert_eq!(code, 0);
    until("the server holds the connection", || open.load(Ordering::SeqCst) == 1).await;
    t.close();
    until("the server's connection ended with the tunnel", || open.load(Ordering::SeqCst) == 0).await;
    drop(held);
    assert!(TcpStream::connect(("127.0.0.1", t.port())).await.is_err(), "the proxy is gone");
    assert!(TcpStream::connect(("127.0.0.1", local)).await.is_err(), "the forward is gone");
    assert!(matches!(t.forward(1), Err(LinkFailure::Other { .. })), "a closed tunnel forwards nothing");

    // A tunnel ends with its link
    let t = l.open_tunnel(here()).await.unwrap();
    let (code, held) = socks(t.port(), &name("localhost"), port).await;
    assert_eq!(code, 0);
    until("the server holds the connection", || open.load(Ordering::SeqCst) == 1).await;
    l.close();
    until("the server's connection ended with the link", || open.load(Ordering::SeqCst) == 0).await;
    let proxy = t.port();
    let gone = || async move { TcpStream::connect(("127.0.0.1", proxy)).await.is_err() };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !gone().await {
      assert!(tokio::time::Instant::now() < deadline, "the proxy outlived its link");
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(held);
    node.close().await;
    g.stop().await;
  });
}

/// A gateway from before tunnels greets without `features`, and would drop the whole link on a stream kind it does not
/// know: the phone never sends one and says which side to update
#[test]
fn a_computer_that_predates_tunnels_is_told_apart() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let old = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![ALPN_LINK.to_vec()], bind: Some(LOOPBACK.parse().unwrap()), bypass: None })
      .await
      .unwrap();
    let streams = Arc::new(AtomicUsize::new(0));
    let server = {
      let (old, streams) = (old.clone(), streams.clone());
      tokio::spawn(async move {
        let conn = old.accept().await.unwrap().await.unwrap();
        while let Ok(incoming) = link::accept(&conn).await {
          streams.fetch_add(1, Ordering::SeqCst);
          if let Incoming::Envelopes(mut tx, rx) = incoming {
            // A welcome as a gateway before tunnels wrote it: no `features`
            let welcome = br#"{"type":"welcome","name":"old","version":"2.0.0","protocol":PROTO}"#;
            let welcome = String::from_utf8_lossy(welcome).replace("PROTO", &protocol_version().to_string());
            tx.send(FrameKind::Control, welcome.as_bytes()).await.unwrap();
            assert!(serde_json::from_str::<Control>(&welcome).is_ok());
            tokio::spawn(async move {
              let _keep = (tx, rx);
              std::future::pending::<()>().await;
            });
          }
        }
      })
    };
    let node = phone().await;
    let to = ComputerAddr { id: old.id().to_string(), relay: None, addrs: old.bound_sockets().iter().map(ToString::to_string).collect() };
    let l = node.connect(to, protocol_version(), 10_000).await.unwrap();
    match l.open_tunnel(here()).await {
      Err(LinkFailure::Incompatible { message }) => assert!(message.contains("update Acpira on the computer"), "{message}"),
      other => panic!("{:?}", other.map(|_| ())),
    }
    assert_eq!(streams.load(Ordering::SeqCst), 1, "only the envelope stream was opened");
    l.close();
    node.close().await;
    server.abort();
    old.close().await;
  });
}
