//! Pairing (ALPN `acpira/pair/1`). The device dials the EndpointId from the ticket, so the handshake has already proven
//! the computer; inside that encrypted connection the device hands over the ticket's one-time secret, and the computer
//! records the device's EndpointId, which the same handshake proved. A window holds one secret, valid for a few minutes
//! and used once: a second scan of the same QR code, or a guess, gets nothing.
//!
//! The same window also shows a 6-digit code for a device that found the computer on its network (Bonjour) instead of
//! scanning. The EndpointId it dials then came from an unauthenticated broadcast, so the code has to prove both sides:
//! SPAKE2 over the code with both EndpointIds as its identities, then each side proves the key over the connection's TLS
//! exporter, the device first. A wrong guess costs one of the window's few tries and teaches nothing offline; a computer
//! that does not know the code cannot prove it, and the device refuses it.

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};

use iroh::EndpointAddr;
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::endpoint::{ALPN_PAIR, dial};
use crate::frame::{Codec, FrameKind, FrameReader, FrameWriter};
use crate::ticket::PairTicket;

/// How long a pairing window stays open
pub const WINDOW: Duration = Duration::from_secs(5 * 60);
/// Wrong secrets or codes a window takes before it closes
const MAX_FAILURES: u32 = 5;
/// The first byte of a pairing stream: the ticket's secret, or the code exchange
const STREAM_SECRET: u8 = b'P';
const STREAM_CODE: u8 = b'C';
/// TLS exporter label the code confirmations are bound to
const EXPORTER_LABEL: &[u8] = b"acpira pair code 1";

pub use acpira_shared::devices::DeviceKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
  /// The ticket's secret, base64url (empty in the code exchange)
  #[serde(default)]
  pub secret: String,
  /// What the device calls itself ("老板的 iPhone")
  pub name: String,
  pub kind: DeviceKind,
  /// Its Acpira version, shown in the device list
  #[serde(default)]
  pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PairReply {
  Paired {
    /// The computer's name
    name: String,
    #[serde(default)]
    relay: Option<String>,
    #[serde(default)]
    addrs: Vec<String>,
  },
  Refused { reason: Refusal },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
  /// No pairing window is open on the computer
  Closed,
  Expired,
  /// The code was used already (scanned twice, or by someone else first)
  Used,
  Wrong,
}

impl std::fmt::Display for Refusal {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(match self {
      Self::Closed => "pairing is not open on that computer",
      Self::Expired => "the pairing code expired",
      Self::Used => "the pairing code was already used",
      Self::Wrong => "the pairing code is wrong",
    })
  }
}

/// The computer's open pairing window: one secret and one code, a deadline, used once by either
pub struct PairingWindow {
  secret: [u8; 32],
  code: String,
  until: Instant,
  used: bool,
  failures: u32,
}

impl PairingWindow {
  pub fn open(ttl: Duration) -> Self {
    Self { secret: random_secret(), code: random_code(), until: Instant::now() + ttl, used: false, failures: 0 }
  }

  pub fn secret(&self) -> [u8; 32] {
    self.secret
  }

  /// The 6 digits the pairing card shows
  pub fn code(&self) -> &str {
    &self.code
  }

  pub fn expires_in(&self) -> Duration {
    self.until.saturating_duration_since(Instant::now())
  }

  /// Whether the window still takes a pairing
  pub fn is_open(&self) -> bool {
    !self.used && self.failures < MAX_FAILURES && Instant::now() < self.until
  }

  /// Why the window takes no pairing now, if it does not
  pub fn check(&self) -> Result<(), Refusal> {
    if self.used {
      return Err(Refusal::Used);
    }
    if Instant::now() >= self.until || self.failures >= MAX_FAILURES {
      return Err(Refusal::Expired);
    }
    Ok(())
  }

  /// Check a device's secret; a match uses the window up
  pub fn redeem(&mut self, secret: &str) -> Result<(), Refusal> {
    let given = URL_SAFE_NO_PAD.decode(secret).unwrap_or_default();
    self.settle(constant_eq(&given, &self.secret))
  }

  /// A code exchange ended: proven uses the window up, anything else counts as a wrong try
  pub fn try_code(&mut self, proven: bool) -> Result<(), Refusal> {
    self.settle(proven)
  }

  fn settle(&mut self, right: bool) -> Result<(), Refusal> {
    self.check()?;
    if !right {
      self.failures += 1;
      return Err(Refusal::Wrong);
    }
    self.used = true;
    Ok(())
  }
}

/// The computer's side of pairing, which `answer` asks
pub trait Desk {
  /// This computer's EndpointId, bound into the code exchange
  fn id(&self) -> EndpointId;
  /// A scanned QR code or a pasted link: check the secret against the open window, admit the device on a match
  fn redeem(&self, device: EndpointId, req: &PairRequest) -> PairReply;
  /// The open window's code, or why no window takes a pairing
  fn code(&self) -> Result<String, Refusal>;
  /// A code exchange ended (`proven`: the device showed it knows the code): admit it, or count the wrong try
  fn try_code(&self, device: EndpointId, req: &PairRequest, proven: bool) -> PairReply;
}

/// The device's pairing could not be finished because the computer did not prove it knows the code: it answered from a
/// broadcast with someone else's identity, or the exchange was tampered with
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeNotProven;

impl std::fmt::Display for CodeNotProven {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("the computer could not prove it knows the pairing code")
  }
}

impl std::error::Error for CodeNotProven {}

/// Device → computer: who it is and its SPAKE2 message
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeOpen {
  #[serde(flatten)]
  req: PairRequest,
  spake: String,
}

/// Computer → device: its SPAKE2 message, or why there is no window to try
#[derive(Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "camelCase")]
enum CodeStep {
  Spake { spake: String },
  Refused { reason: Refusal },
}

/// Device → computer: proof of the key
#[derive(Serialize, Deserialize)]
struct CodeConfirm {
  confirm: String,
}

/// Computer → device: the answer, and on success the computer's own proof
#[derive(Serialize, Deserialize)]
struct CodeDone {
  reply: PairReply,
  #[serde(default)]
  confirm: Option<String>,
}

pub fn random_secret() -> [u8; 32] {
  use ring::rand::SecureRandom;
  let mut b = [0u8; 32];
  // The system generator does not fail on the platforms Acpira runs on; a zero secret would still only open a window
  // that closes in minutes
  let _ = ring::rand::SystemRandom::new().fill(&mut b);
  b
}

/// Six uniform digits
fn random_code() -> String {
  use ring::rand::SecureRandom;
  let rng = ring::rand::SystemRandom::new();
  loop {
    let mut b = [0u8; 4];
    let _ = rng.fill(&mut b);
    let n = u32::from_be_bytes(b);
    // Below the largest multiple of a million, so every code is equally likely
    if n < 4_294_000_000 {
      return format!("{:06}", n % 1_000_000);
    }
  }
}

/// One side's proof of the SPAKE2 key, bound to this connection's TLS session
fn proof(key: &[u8], side: &[u8], exporter: &[u8; 32]) -> Vec<u8> {
  let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
  ring::hmac::sign(&k, &[side, exporter.as_slice()].concat()).as_ref().to_vec()
}

fn proof_ok(key: &[u8], side: &[u8], exporter: &[u8; 32], given: &str) -> bool {
  let Ok(given) = URL_SAFE_NO_PAD.decode(given) else { return false };
  let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
  ring::hmac::verify(&k, &[side, exporter.as_slice()].concat(), &given).is_ok()
}

fn exporter(conn: &Connection) -> Result<[u8; 32]> {
  let mut out = [0u8; 32];
  conn.export_keying_material(&mut out, EXPORTER_LABEL, b"").map_err(|e| anyhow!("no TLS exporter: {e:?}"))?;
  Ok(out)
}

fn spake_password(code: &str) -> Password {
  // Spaces and dashes a person types between the digit groups do not count
  Password::new(code.chars().filter(char::is_ascii_digit).collect::<String>())
}

const SIDE_DEVICE: &[u8] = b"device";
const SIDE_COMPUTER: &[u8] = b"computer";

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
  a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Device side: pair with the computer a ticket names
pub async fn pair_with(ep: &Endpoint, ticket: &PairTicket, name: &str, kind: DeviceKind, version: &str, within: Duration) -> Result<PairReply> {
  let conn = dial(ep, ticket.endpoint_addr(), ALPN_PAIR, within).await?;
  let req = PairRequest { secret: URL_SAFE_NO_PAD.encode(ticket.secret), name: name.to_owned(), kind, version: version.to_owned() };
  let reply = tokio::time::timeout(within, async {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[STREAM_SECRET]).await?;
    let mut tx = FrameWriter::start(send, Codec::Plain).await?;
    tx.send(FrameKind::Control, &serde_json::to_vec(&req)?).await?;
    let mut rx = FrameReader::start(recv).await?;
    let (_, body) = rx.next().await?.ok_or_else(|| anyhow!("the computer closed the pairing"))?;
    anyhow::Ok(serde_json::from_slice::<PairReply>(&body)?)
  })
  .await
  .map_err(|_| anyhow!("the computer did not answer"))??;
  conn.close(0u32.into(), b"paired");
  Ok(reply)
}

/// Device side: pair with a computer found on this network, by the code its pairing card shows. `computer` is where
/// the broadcast said it is; the code exchange, not the broadcast, proves it. Errors with `CodeNotProven` when the
/// computer cannot prove the code
pub async fn pair_with_code(ep: &Endpoint, computer: EndpointAddr, code: &str, name: &str, kind: DeviceKind, version: &str, within: Duration) -> Result<PairReply> {
  let (me, them) = (ep.id(), computer.id);
  let conn = dial(ep, computer, ALPN_PAIR, within).await?;
  let reply = tokio::time::timeout(within, async {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[STREAM_CODE]).await?;
    let mut tx = FrameWriter::start(send, Codec::Plain).await?;
    let (state, msg) = Spake2::<Ed25519Group>::start_a(&spake_password(code), &Identity::new(me.as_bytes()), &Identity::new(them.as_bytes()));
    let req = PairRequest { secret: String::new(), name: name.to_owned(), kind, version: version.to_owned() };
    tx.send(FrameKind::Control, &serde_json::to_vec(&CodeOpen { req, spake: URL_SAFE_NO_PAD.encode(msg) })?).await?;
    let mut rx = FrameReader::start(recv).await?;
    let theirs = match next_json::<CodeStep, _>(&mut rx).await? {
      CodeStep::Spake { spake } => URL_SAFE_NO_PAD.decode(spake)?,
      CodeStep::Refused { reason } => return anyhow::Ok(PairReply::Refused { reason }),
    };
    // A malformed answer is a computer that does not hold up its side
    let key = state.finish(&theirs).map_err(|_| CodeNotProven)?;
    let ex = exporter(&conn)?;
    tx.send(FrameKind::Control, &serde_json::to_vec(&CodeConfirm { confirm: URL_SAFE_NO_PAD.encode(proof(&key, SIDE_DEVICE, &ex)) })?).await?;
    let done = next_json::<CodeDone, _>(&mut rx).await?;
    if matches!(done.reply, PairReply::Paired { .. }) && !done.confirm.as_deref().is_some_and(|c| proof_ok(&key, SIDE_COMPUTER, &ex, c)) {
      return Err(CodeNotProven.into());
    }
    anyhow::Ok(done.reply)
  })
  .await
  .map_err(|_| anyhow!("the computer did not answer"))??;
  conn.close(0u32.into(), b"paired");
  Ok(reply)
}

async fn next_json<T: serde::de::DeserializeOwned, R: tokio::io::AsyncRead + Unpin>(rx: &mut FrameReader<R>) -> Result<T> {
  let (_, body) = rx.next().await?.ok_or_else(|| anyhow!("the other side closed the pairing"))?;
  Ok(serde_json::from_slice(&body)?)
}

/// Computer side: answer one pairing on an accepted `acpira/pair/1` connection, a secret or a code exchange, through
/// `desk` (it sees the device's EndpointId as the handshake proved it)
pub async fn answer(conn: &Connection, desk: &impl Desk) -> Result<(PairRequest, PairReply)> {
  let (send, mut recv) = tokio::time::timeout(Duration::from_secs(20), conn.accept_bi()).await.map_err(|_| anyhow!("no pairing request"))??;
  let tag = tokio::io::AsyncReadExt::read_u8(&mut recv).await?;
  let device = conn.remote_id();
  let mut rx = FrameReader::start(recv).await?;
  let mut tx = FrameWriter::start(send, Codec::Plain).await?;
  let (req, reply) = tokio::time::timeout(Duration::from_secs(30), async {
    match tag {
      STREAM_SECRET => {
        let req: PairRequest = next_json(&mut rx).await?;
        let reply = desk.redeem(device, &req);
        tx.send(FrameKind::Control, &serde_json::to_vec(&reply)?).await?;
        anyhow::Ok((req, reply))
      }
      STREAM_CODE => {
        let open: CodeOpen = next_json(&mut rx).await?;
        let code = match desk.code() {
          Ok(c) => c,
          Err(reason) => {
            tx.send(FrameKind::Control, &serde_json::to_vec(&CodeStep::Refused { reason })?).await?;
            return Ok((open.req, PairReply::Refused { reason }));
          }
        };
        let (state, msg) = Spake2::<Ed25519Group>::start_b(&spake_password(&code), &Identity::new(device.as_bytes()), &Identity::new(desk.id().as_bytes()));
        tx.send(FrameKind::Control, &serde_json::to_vec(&CodeStep::Spake { spake: URL_SAFE_NO_PAD.encode(msg) })?).await?;
        let key = URL_SAFE_NO_PAD.decode(&open.spake).ok().and_then(|m| state.finish(&m).ok());
        // The device proves the key first, so a guess is only ever tested here, where it counts against the window
        let confirm: CodeConfirm = next_json(&mut rx).await?;
        let ex = exporter(conn)?;
        let proven = key.as_ref().is_some_and(|k| proof_ok(k, SIDE_DEVICE, &ex, &confirm.confirm));
        let reply = desk.try_code(device, &open.req, proven);
        let mine = match (&reply, &key) {
          (PairReply::Paired { .. }, Some(k)) => Some(URL_SAFE_NO_PAD.encode(proof(k, SIDE_COMPUTER, &ex))),
          _ => None,
        };
        tx.send(FrameKind::Control, &serde_json::to_vec(&CodeDone { reply: reply.clone(), confirm: mine })?).await?;
        Ok((open.req, reply))
      }
      _ => bail!("not a pairing stream"),
    }
  })
  .await
  .map_err(|_| anyhow!("the device stalled the pairing"))??;
  tx.into_inner().finish()?;
  // Let the reply reach the device before the caller drops the connection
  let _ = tokio::time::timeout(Duration::from_secs(3), conn.closed()).await;
  Ok((req, reply))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_window_pays_out_once_and_closes_after_guesses() {
    let mut w = PairingWindow::open(WINDOW);
    let good = URL_SAFE_NO_PAD.encode(w.secret());
    assert_eq!(w.redeem("nope"), Err(Refusal::Wrong));
    assert_eq!(w.redeem(&good), Ok(()));
    assert_eq!(w.redeem(&good), Err(Refusal::Used));
    assert!(!w.is_open());

    let mut w = PairingWindow::open(WINDOW);
    let good = URL_SAFE_NO_PAD.encode(w.secret());
    for _ in 0..MAX_FAILURES {
      assert_eq!(w.redeem("guess"), Err(Refusal::Wrong));
    }
    assert_eq!(w.redeem(&good), Err(Refusal::Expired));

    let mut w = PairingWindow::open(Duration::ZERO);
    assert_eq!(w.redeem(&URL_SAFE_NO_PAD.encode(w.secret())), Err(Refusal::Expired));
  }
}
