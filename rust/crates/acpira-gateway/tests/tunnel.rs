//! Tunnel streams against the gateway alone, with a device played by a raw endpoint on loopback (relays off), so the
//! gateway's own rules are what is tested, not the device's: loopback targets only, a folder naming its machine, an SSH
//! host that is not up, the stream cap, and every pipe ending with the link. SSH hosts are a fake `SshDial` here; the
//! engine's real one is tested against the fake ssh in acpira-host (tests/engine/remote_workspace.rs)

#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use acpira_gateway::gateway::{Gateway, Opts};
use acpira_gateway::tunnel::{Dialing, Pipe, SshDial};
use acpira_net::endpoint::{self, ALPN_LINK, Options, Relays};
use acpira_net::iroh::endpoint::Connection;
use acpira_net::pair::{self, DeviceKind};
use acpira_net::tunnel::{self, MAX_STREAMS, TcpCode, TcpFailure, TcpRequest};
use acpira_net::{Endpoint, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// SSH hosts: `up` is connected and its loopback is this machine's; every other alias is off. Keeps the hosts asked for
#[derive(Default)]
struct FakeHosts {
  asked: parking_lot::Mutex<Vec<String>>,
}

impl SshDial for FakeHosts {
  fn up(&self, alias: &str) -> bool {
    alias == "up"
  }

  fn dial(&self, _alias: &str, host: String, port: u16) -> Dialing {
    self.asked.lock().push(host);
    Box::pin(async move {
      let s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.map_err(|e| TcpFailure::new(TcpCode::Refused, e.to_string()))?;
      Ok(Box::new(s) as Box<dyn Pipe>)
    })
  }
}

/// A server on this machine's loopback that echoes what it reads and ends its side once the client ended its own;
/// `open` counts the connections it still holds
struct Echo {
  port: u16,
  open: Arc<AtomicUsize>,
}

async fn echo() -> Echo {
  let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let port = l.local_addr().unwrap().port();
  let open = Arc::new(AtomicUsize::new(0));
  let held = open.clone();
  tokio::spawn(async move {
    while let Ok((mut s, _)) = l.accept().await {
      held.fetch_add(1, Ordering::SeqCst);
      let held = held.clone();
      tokio::spawn(async move {
        let mut buf = vec![0u8; 16 << 10];
        loop {
          match s.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
              if s.write_all(&buf[..n]).await.is_err() {
                break;
              }
            }
          }
        }
        let _ = s.shutdown().await;
        held.fetch_sub(1, Ordering::SeqCst);
      });
    }
  });
  Echo { port, open }
}

async fn until(what: &str, f: impl Fn() -> bool) {
  let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
  while !f() {
    assert!(tokio::time::Instant::now() < deadline, "{what}");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

/// A gateway on loopback and a device paired with it, linked
async fn linked(hosts: Arc<FakeHosts>) -> (Arc<Gateway>, Endpoint, Connection, tempfile::TempDir) {
  let home = tempfile::tempdir().unwrap();
  let g = Gateway::start(Opts {
    home: home.path().to_path_buf(),
    engine: acpira_gateway::engine::none(),
    relays: Some(Relays::Off),
    bind: Some("127.0.0.1:0".parse().unwrap()),
    ssh: Some(hosts),
    log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
  })
  .await
  .unwrap();
  let at = acpira_net::EndpointAddr::from_parts(g.endpoint().id(), g.endpoint().bound_sockets().into_iter().map(acpira_net::iroh::TransportAddr::Ip));
  let phone = endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns: vec![], bind: Some("127.0.0.1:0".parse().unwrap()), bypass: None }).await.unwrap();
  let code = g.open_pairing().code;
  pair::pair_with_code(&phone, at.clone(), &code, "phone", DeviceKind::Phone, "1.0", Duration::from_secs(10)).await.unwrap();
  let conn = endpoint::dial(&phone, at, ALPN_LINK, Duration::from_secs(10)).await.unwrap();
  (g, phone, conn, home)
}

fn here() -> String {
  std::env::temp_dir().join("web").to_string_lossy().into_owned()
}

async fn open(conn: &Connection, folder: &str, host: &str, port: u16) -> Result<tunnel::Opened, TcpFailure> {
  tunnel::open(conn, &TcpRequest { folder: folder.into(), host: host.into(), port }, Duration::from_secs(10)).await.unwrap()
}

/// Bytes there and back through one stream, then both ends close
async fn round_trip(mut o: tunnel::Opened, n: usize) {
  let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
  let writer = {
    let data = data.clone();
    tokio::spawn(async move {
      o.send.write_all(&data).await.unwrap();
      o.send.finish().unwrap();
    })
  };
  let mut got = o.first;
  tokio::time::timeout(Duration::from_secs(20), AsyncReadExt::read_to_end(&mut o.recv, &mut got)).await.unwrap().unwrap();
  writer.await.unwrap();
  assert_eq!(got.len(), data.len());
  assert!(got == data, "the bytes came back as sent");
}

#[test]
fn a_device_reaches_loopback_ports_on_the_folders_machine_and_nothing_else() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let hosts = Arc::new(FakeHosts::default());
    let (g, phone, conn, _home) = linked(hosts.clone()).await;
    let server = echo().await;

    // This computer, by name and by address; an SSH host whose connection is up
    round_trip(open(&conn, &here(), "localhost", server.port).await.unwrap(), 300_000).await;
    round_trip(open(&conn, &here(), "127.0.0.1", server.port).await.unwrap(), 10).await;
    round_trip(open(&conn, "ssh://up/home/a/web", "app.localhost", server.port).await.unwrap(), 70_000).await;
    assert_eq!(*hosts.asked.lock(), ["localhost"], "a name under localhost is the host's own localhost");

    // A check names no port
    assert!(open(&conn, &here(), "localhost", 0).await.is_ok());
    assert!(open(&conn, "ssh://up/x", "localhost", 0).await.is_ok());

    // The gateway's own rules, whatever the device checked
    for host in ["10.0.0.1", "example.com", "0.0.0.0", "192.168.1.1"] {
      assert_eq!(open(&conn, &here(), host, server.port).await.unwrap_err().code, TcpCode::Denied, "{host}");
    }
    assert_eq!(open(&conn, "web", "localhost", server.port).await.unwrap_err().code, TcpCode::Denied, "a relative folder");
    assert_eq!(open(&conn, "ssh://-oProxyCommand=x/a", "localhost", server.port).await.unwrap_err().code, TcpCode::Denied);
    let off = open(&conn, "ssh://down/home/a", "localhost", 0).await.unwrap_err();
    assert_eq!((off.code, off.message.as_str()), (TcpCode::Offline, "down is not connected on the computer"));
    assert_eq!(hosts.asked.lock().len(), 1, "a host that is off is never dialed");

    // Nothing listening there
    let closed = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap().local_addr().unwrap().port();
    assert_eq!(open(&conn, &here(), "localhost", closed).await.unwrap_err().code, TcpCode::Refused);

    // The link carries on through all of it
    round_trip(open(&conn, &here(), "localhost", server.port).await.unwrap(), 1000).await;
    until("every connection closed", || server.open.load(Ordering::SeqCst) == 0).await;
    conn.close(0u32.into(), b"bye");
    phone.close().await;
    g.stop().await;
  });
}

#[test]
fn streams_are_capped_and_end_with_the_link() {
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async move {
    let (g, phone, conn, _home) = linked(Arc::new(FakeHosts::default())).await;
    let server = echo().await;
    let mut held = Vec::new();
    for i in 0..MAX_STREAMS {
      let folder = if i % 2 == 0 { here() } else { "ssh://up/x".into() };
      held.push(open(&conn, &folder, "localhost", server.port).await.unwrap());
    }
    until("every stream reached the server", || server.open.load(Ordering::SeqCst) == MAX_STREAMS).await;
    let busy = open(&conn, &here(), "localhost", server.port).await.unwrap_err();
    assert_eq!(busy.code, TcpCode::Busy, "{busy}");

    // One ends: its place comes free
    let first = held.remove(0);
    round_trip(first, 10).await;
    until("the ended stream's connection closed", || server.open.load(Ordering::SeqCst) == MAX_STREAMS - 1).await;
    let mut again = None;
    let folder = here();
    until_ok(&mut again, || open(&conn, &folder, "localhost", server.port)).await;
    held.push(again.unwrap());

    // The link goes: every pipe ends on the computer, the servers' connections included
    conn.close(0u32.into(), b"bye");
    until("the link's end closed every connection", || server.open.load(Ordering::SeqCst) == 0).await;
    drop(held);
    phone.close().await;
    g.stop().await;
  });
}

/// The gateway reaps an ended stream when the next one comes, so the freed place may take one more try to show
async fn until_ok<F, T>(slot: &mut Option<T>, mut f: impl FnMut() -> F)
where
  F: Future<Output = Result<T, TcpFailure>>,
{
  let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
  loop {
    match f().await {
      Ok(t) => {
        *slot = Some(t);
        return;
      }
      Err(e) => assert!(e.code == TcpCode::Busy && tokio::time::Instant::now() < deadline, "{e}"),
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}
