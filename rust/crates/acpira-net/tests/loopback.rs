//! Two endpoints in one process on loopback, relays off: pairing, then the link's envelope stream, control frames and a
//! blob, the way the gateway and a phone use them.

#![allow(clippy::unwrap_used, clippy::panic, clippy::string_slice, reason = "test code")]

use std::time::Duration;

use acpira_net::endpoint::{self, ALPN_LINK, ALPN_PAIR, Options, Relays};
use acpira_net::frame::FrameKind;
use acpira_net::link::{self, BlobRequest, Control, Incoming};
use acpira_net::pair::{self, DeviceKind, PairReply, PairingWindow, Refusal};
use acpira_net::path::PathKind;
use acpira_net::ticket::PairTicket;
use acpira_net::{EndpointAddr, EndpointId, SecretKey};

type Window = std::sync::Arc<parking_lot::Mutex<PairingWindow>>;
type Trusted = std::sync::Arc<parking_lot::Mutex<Vec<EndpointId>>>;

/// A computer's pairing desk over one window: admits a device into `trusted`
struct TestDesk {
  id: EndpointId,
  window: Window,
  trusted: Trusted,
  /// An impostor: answers with this code and claims every exchange succeeded
  impostor: Option<String>,
}

impl TestDesk {
  fn admit(&self, device: EndpointId, r: Result<(), Refusal>) -> PairReply {
    match r {
      Ok(()) => {
        self.trusted.lock().push(device);
        PairReply::Paired { name: "mini".into(), relay: None, addrs: vec![] }
      }
      Err(reason) => PairReply::Refused { reason },
    }
  }
}

impl pair::Desk for TestDesk {
  fn id(&self) -> EndpointId {
    self.id
  }
  fn redeem(&self, device: EndpointId, req: &pair::PairRequest) -> PairReply {
    let r = self.window.lock().redeem(&req.secret);
    self.admit(device, r)
  }
  fn code(&self) -> Result<String, Refusal> {
    if let Some(c) = &self.impostor {
      return Ok(c.clone());
    }
    let w = self.window.lock();
    w.check().map(|()| w.code().to_owned())
  }
  fn try_code(&self, device: EndpointId, _req: &pair::PairRequest, proven: bool) -> PairReply {
    let r = if self.impostor.is_some() { Ok(()) } else { self.window.lock().try_code(proven) };
    self.admit(device, r)
  }
}

/// Answer pairings on `computer` until aborted
fn serve_pairing(computer: acpira_net::Endpoint, window: Window, trusted: Trusted, impostor: Option<String>) -> tokio::task::JoinHandle<()> {
  let desk = std::sync::Arc::new(TestDesk { id: computer.id(), window, trusted, impostor });
  tokio::spawn(async move {
    while let Some(incoming) = computer.accept().await {
      let Ok(conn) = incoming.await else { continue };
      let desk = desk.clone();
      tokio::spawn(async move {
        let _ = pair::answer(&conn, &*desk).await;
      });
    }
  })
}

async fn ep(alpns: Vec<Vec<u8>>) -> acpira_net::Endpoint {
  endpoint::bind(Options { secret: SecretKey::generate(), relays: Relays::Off, alpns, bind: Some("127.0.0.1:0".parse().unwrap()) }).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_pairs_once_then_talks_over_the_link() {
  let computer = ep(vec![ALPN_PAIR.to_vec(), ALPN_LINK.to_vec()]).await;
  let phone = ep(vec![]).await;
  let window = std::sync::Arc::new(parking_lot::Mutex::new(PairingWindow::open(pair::WINDOW)));
  let trusted = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));

  // The computer: pairing answers from the window, links only for trusted ids
  let server = {
    let (computer, window, trusted) = (computer.clone(), window.clone(), trusted.clone());
    tokio::spawn(async move {
      let me = computer.id();
      while let Some(incoming) = computer.accept().await {
        let Ok(conn) = incoming.await else { continue };
        let (window, trusted) = (window.clone(), trusted.clone());
        tokio::spawn(async move {
          if conn.alpn() == ALPN_PAIR {
            let desk = TestDesk { id: me, window, trusted, impostor: None };
            let _ = pair::answer(&conn, &desk).await;
            return;
          }
          if !trusted.lock().contains(&conn.remote_id()) {
            conn.close(1u32.into(), b"unknown");
            return;
          }
          loop {
            match link::accept(&conn).await {
              Ok(Incoming::Envelopes(mut tx, mut rx)) => {
                tokio::spawn(async move {
                  tx.send(FrameKind::Control, &link::control(&Control::Welcome { name: "mini".into(), version: "t".into(), protocol: 1 })).await.unwrap();
                  // Echo envelopes back, answer pings
                  while let Ok(Some((kind, body))) = rx.next().await {
                    let reply = match kind {
                      FrameKind::Control => match serde_json::from_slice::<Control>(&body).unwrap() {
                        Control::Ping { n } => link::control(&Control::Pong { n }),
                        _ => continue,
                      },
                      FrameKind::Envelope => body,
                    };
                    tx.send(kind, &reply).await.unwrap();
                  }
                });
              }
              Ok(Incoming::Blob(req, send)) => {
                let data = if req.name == "a.png" { Ok(vec![42u8; 200_000]) } else { Err("no such blob".to_owned()) };
                link::send_blob(send, data).await.unwrap();
              }
              Err(_) => return,
            }
          }
        });
      }
    })
  };

  let (relay, addrs) = endpoint::reachable_at(&computer);
  assert!(relay.is_none());
  assert!(addrs.is_empty(), "loopback is never worth a ticket: {addrs:?}");
  let bound: Vec<_> = computer.bound_sockets();
  let ticket = PairTicket { id: computer.id(), name: "mini".into(), secret: window.lock().secret(), relay: None, addrs: bound.clone() };
  let ticket = PairTicket::decode(&ticket.link()).unwrap();

  // An unpaired device is turned away on the link
  let addr = EndpointAddr::from_parts(computer.id(), bound.iter().copied().map(iroh::TransportAddr::Ip));
  let conn = endpoint::dial(&phone, addr.clone(), ALPN_LINK, Duration::from_secs(5)).await.unwrap();
  assert!(link::open_envelopes(&conn).await.is_err());

  // Wrong secret, then the right one, then the right one again
  let mut wrong = ticket.clone();
  wrong.secret = [1; 32];
  let r = pair::pair_with(&phone, &wrong, "phone", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert_eq!(r, PairReply::Refused { reason: Refusal::Wrong });
  let r = pair::pair_with(&phone, &ticket, "phone", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert!(matches!(r, PairReply::Paired { .. }), "{r:?}");
  assert_eq!(trusted.lock().as_slice(), &[phone.id()]);
  let other = ep(vec![]).await;
  let r = pair::pair_with(&other, &ticket, "thief", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert_eq!(r, PairReply::Refused { reason: Refusal::Used });

  // Paired: the link carries envelopes and control frames, compressed, and blobs on their own streams
  let conn = endpoint::dial(&phone, addr, ALPN_LINK, Duration::from_secs(5)).await.unwrap();
  let (mut tx, mut rx) = link::open_envelopes(&conn).await.unwrap();
  let (k, body) = rx.next().await.unwrap().unwrap();
  assert_eq!(k, FrameKind::Control);
  assert!(matches!(serde_json::from_slice::<Control>(&body).unwrap(), Control::Welcome { protocol: 1, .. }));
  let env = br#"{"type":"webviewMessage","viewId":"v","msg":{"type":"ready","patches":true}}"#;
  tx.send(FrameKind::Envelope, env).await.unwrap();
  assert_eq!(rx.next().await.unwrap().unwrap(), (FrameKind::Envelope, env.to_vec()));
  tx.send(FrameKind::Control, &link::control(&Control::Ping { n: 7 })).await.unwrap();
  let (_, pong) = rx.next().await.unwrap().unwrap();
  assert_eq!(serde_json::from_slice::<Control>(&pong).unwrap(), Control::Pong { n: 7 });

  let blob = link::fetch_blob(&conn, &BlobRequest { session_id: "s".into(), name: "a.png".into() }, Duration::from_secs(5)).await.unwrap();
  assert_eq!(blob.len(), 200_000);
  let missing = link::fetch_blob(&conn, &BlobRequest { session_id: "s".into(), name: "b.png".into() }, Duration::from_secs(5)).await;
  assert_eq!(missing.unwrap_err().to_string(), "no such blob");
  assert_eq!(endpoint::selected_path(&conn).map(|(k, _)| k), Some(PathKind::Lan));

  server.abort();
  phone.close().await;
  other.close().await;
  computer.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_code_pairs_a_device_that_found_the_computer_on_its_network() {
  let computer = ep(vec![ALPN_PAIR.to_vec()]).await;
  let window: Window = std::sync::Arc::new(parking_lot::Mutex::new(PairingWindow::open(pair::WINDOW)));
  let trusted: Trusted = Default::default();
  let server = serve_pairing(computer.clone(), window.clone(), trusted.clone(), None);
  let addr = EndpointAddr::from_parts(computer.id(), computer.bound_sockets().into_iter().map(iroh::TransportAddr::Ip));
  let code = window.lock().code().to_owned();
  assert!(code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()), "{code}");
  let wrong = if code == "000000" { "000001" } else { "000000" };

  let phone = ep(vec![]).await;
  let r = pair::pair_with_code(&phone, addr.clone(), wrong, "phone", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert_eq!(r, PairReply::Refused { reason: Refusal::Wrong });
  assert!(trusted.lock().is_empty());
  // Typed with a space between the groups, the way the card shows it
  let spaced = format!("{} {}", &code[..3], &code[3..]);
  let r = pair::pair_with_code(&phone, addr.clone(), &spaced, "phone", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert!(matches!(r, PairReply::Paired { .. }), "{r:?}");
  assert_eq!(trusted.lock().as_slice(), &[phone.id()]);
  // The window is used up, for the code and the QR secret alike
  let other = ep(vec![]).await;
  let r = pair::pair_with_code(&other, addr.clone(), &code, "thief", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert_eq!(r, PairReply::Refused { reason: Refusal::Used });

  // Guesses close a fresh window: five wrong tries, then even the right code is refused
  *window.lock() = PairingWindow::open(pair::WINDOW);
  let code = window.lock().code().to_owned();
  let wrong = if code == "000000" { "000001" } else { "000000" };
  for _ in 0..5 {
    let r = pair::pair_with_code(&other, addr.clone(), wrong, "thief", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
    assert_eq!(r, PairReply::Refused { reason: Refusal::Wrong });
  }
  let r = pair::pair_with_code(&other, addr, &code, "thief", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap();
  assert_eq!(r, PairReply::Refused { reason: Refusal::Expired });
  assert_eq!(trusted.lock().len(), 1);

  server.abort();
  phone.close().await;
  other.close().await;
  computer.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_that_does_not_know_the_code_is_refused_by_the_device() {
  // Someone on the network advertised their own EndpointId under the computer's name and claims every code works
  let impostor = ep(vec![ALPN_PAIR.to_vec()]).await;
  let window: Window = std::sync::Arc::new(parking_lot::Mutex::new(PairingWindow::open(pair::WINDOW)));
  let server = serve_pairing(impostor.clone(), window, Default::default(), Some("111111".into()));
  let addr = EndpointAddr::from_parts(impostor.id(), impostor.bound_sockets().into_iter().map(iroh::TransportAddr::Ip));
  let phone = ep(vec![]).await;
  let e = pair::pair_with_code(&phone, addr, "222222", "phone", DeviceKind::Phone, "t", Duration::from_secs(5)).await.unwrap_err();
  assert!(e.downcast_ref::<pair::CodeNotProven>().is_some(), "{e:#}");
  server.abort();
  phone.close().await;
  impostor.close().await;
}
