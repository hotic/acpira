//! Tunnel streams (`link::STREAM_TCP`): a paired device reaches a dev server on the machine a project folder lives on,
//! the computer itself or one of its SSH hosts. The device opens one stream per connection and sends one request frame
//! naming the folder, a loopback host and a port; the gateway answers one header frame (empty, or why not), and the
//! stream is then a raw byte pipe to that port until either side ends it. Port 0 asks only whether the folder's machine
//! can be reached now.
//!
//! Only loopback targets are taken (`loopback`): this is a window onto the user's own dev servers, never a proxy into
//! the computer's network. The gateway decides it; the device's own check spares a round trip and nothing more.

use std::io;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::frame::{Codec, FrameKind, FrameReader, FrameWriter};
use crate::link::STREAM_TCP;

/// What `Control::Welcome` lists in `features` when the gateway takes tunnel streams. A gateway that predates them
/// stops serving the whole link on an unknown stream kind, so a device checks this before it opens one
pub const FEATURE: &str = "tcp";
/// Tunnel streams one device may hold open at once. QUIC allows 100 bidirectional streams per connection: the rest stay
/// for the envelope stream and blobs
pub const MAX_STREAMS: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TcpRequest {
  /// Whose machine: a local path (the computer) or `ssh://<alias>/<path>` as the session list carries it
  pub folder: String,
  /// `localhost`, `*.localhost`, a 127.0.0.0/8 address or `::1`
  pub host: String,
  /// 0: only check that the folder's machine can be reached now
  pub port: u16,
}

/// Why the gateway did not open a tunnel stream
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TcpCode {
  /// Not a loopback target, or not a folder
  Denied,
  /// The folder's SSH host is not connected on the computer
  Offline,
  /// Nothing answered on that port
  Refused,
  /// The device holds `MAX_STREAMS` already
  Busy,
  /// Anything else; a code a newer gateway sends lands here
  #[serde(other)]
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpFailure {
  pub code: TcpCode,
  /// For the device to show as it is
  pub message: String,
}

impl TcpFailure {
  pub fn new(code: TcpCode, message: impl Into<String>) -> Self {
    TcpFailure { code, message: message.into() }
  }
}

impl std::fmt::Display for TcpFailure {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.message)
  }
}

impl std::error::Error for TcpFailure {}

/// The gateway's answer: empty when the pipe is open
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TcpHeader {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  error: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  code: Option<TcpCode>,
}

/// A loopback target, as the gateway dials it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loopback {
  /// `localhost` or a name under it: whichever of 127.0.0.1 and ::1 answers
  Name,
  Ip(IpAddr),
}

/// `host` when it names this machine's loopback: `localhost`, `*.localhost` (RFC 6761), 127.0.0.0/8 or `::1`
/// (bracketed or not); None for anything else, 0.0.0.0 and v4-mapped addresses included
pub fn loopback(host: &str) -> Option<Loopback> {
  let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
  let bracketed = bare.len() != host.len();
  if let Ok(ip) = bare.parse::<IpAddr>() {
    let ok = match ip {
      IpAddr::V4(v4) => !bracketed && v4.is_loopback(),
      IpAddr::V6(v6) => v6.is_loopback(),
    };
    return ok.then_some(Loopback::Ip(ip));
  }
  if bracketed {
    return None;
  }
  let name = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
  let label = |l: &str| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
  let under = name.strip_suffix(".localhost").is_some_and(|sub| sub.split('.').all(label));
  (name == "localhost" || under).then_some(Loopback::Name)
}

/// Gateway side: the request on a stream whose kind byte was read, and what came after it (nothing, from this crate's
/// device side, which waits for the header before it sends)
pub async fn read_request(recv: RecvStream) -> Result<(TcpRequest, RecvStream, Vec<u8>)> {
  let mut rx = FrameReader::start(recv).await?;
  let (_, body) = rx.next().await?.ok_or_else(|| anyhow!("tunnel stream without a request"))?;
  let req = serde_json::from_slice(&body)?;
  let first = rx.take_buffered();
  Ok((req, rx.into_inner(), first))
}

/// Gateway side: the header, then the stream is the pipe (or ends, on a failure or a check)
pub async fn answer(send: SendStream, r: std::result::Result<(), &TcpFailure>) -> io::Result<SendStream> {
  let mut tx = FrameWriter::start(send, Codec::Plain).await?;
  let header = match r {
    Ok(()) => TcpHeader::default(),
    Err(f) => TcpHeader { error: Some(f.message.clone()), code: Some(f.code) },
  };
  tx.send(FrameKind::Control, &serde_json::to_vec(&header).unwrap_or_default()).await?;
  Ok(tx.into_inner())
}

/// An open tunnel stream on the device: the pipe's two halves and the bytes that came with the header
#[derive(Debug)]
pub struct Opened {
  pub send: SendStream,
  pub recv: RecvStream,
  pub first: Vec<u8>,
}

/// Device side: open a tunnel stream, within `within` for the stream and the header. The outer error is the link (or a
/// gateway that never answered); the inner one the gateway's refusal. A check (port 0) ends the stream once answered
pub async fn open(conn: &Connection, req: &TcpRequest, within: Duration) -> Result<std::result::Result<Opened, TcpFailure>> {
  tokio::time::timeout(within, async {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[STREAM_TCP]).await?;
    let mut tx = FrameWriter::start(send, Codec::Plain).await?;
    tx.send(FrameKind::Control, &serde_json::to_vec(req)?).await?;
    let send = tx.into_inner();
    let mut rx = FrameReader::start(recv).await?;
    let (_, head) = rx.next().await?.ok_or_else(|| anyhow!("the computer ended the tunnel stream unanswered"))?;
    let head: TcpHeader = serde_json::from_slice(&head)?;
    if let Some(message) = head.error {
      return Ok(Err(TcpFailure { code: head.code.unwrap_or(TcpCode::Failed), message }));
    }
    let first = rx.take_buffered();
    Ok(Ok(Opened { send, recv: rx.into_inner(), first }))
  })
  .await
  .map_err(|_| anyhow!("the computer did not answer the tunnel stream in time"))?
}

/// What a finished pipe moved: bytes into the socket and out of it
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Moved {
  pub to_socket: u64,
  pub from_socket: u64,
}

/// Pipes a tunnel stream and a socket both ways. A direction that ends cleanly passes the end on (a half close) and the
/// other keeps going; a failure either way ends both, resetting the stream so the far side never reads a cut-off
/// answer as complete. `first`: stream bytes already read, written to the socket before the rest
pub async fn splice<S: AsyncRead + AsyncWrite + Send>(sock: S, mut send: SendStream, mut recv: RecvStream, first: Vec<u8>) -> Moved {
  let (mut sr, mut sw) = tokio::io::split(sock);
  let mut moved = Moved::default();
  let ok = {
    let (to_socket, from_socket) = (&mut moved.to_socket, &mut moved.from_socket);
    let send = &mut send;
    let recv = &mut recv;
    let down = async move {
      if !first.is_empty() {
        sw.write_all(&first).await?;
        *to_socket += first.len() as u64;
      }
      pump(recv, &mut sw, to_socket).await?;
      sw.shutdown().await
    };
    let up = async move {
      pump(&mut sr, send, from_socket).await?;
      send.finish().map_err(io::Error::other)
    };
    tokio::try_join!(down, up).is_ok()
  };
  if !ok {
    let _ = send.reset(0u32.into());
    let _ = recv.stop(0u32.into());
  }
  moved
}

async fn pump(r: &mut (impl AsyncRead + Unpin), w: &mut (impl AsyncWrite + Unpin), n: &mut u64) -> io::Result<()> {
  let mut buf = vec![0u8; 32 << 10];
  loop {
    let k = r.read(&mut buf).await?;
    if k == 0 {
      return Ok(());
    }
    w.write_all(&buf[..k]).await?;
    *n += k as u64;
  }
}

#[cfg(test)]
mod tests {
  #![allow(clippy::unwrap_used, reason = "test code")]

  use super::*;

  #[test]
  fn only_loopback_targets_are_taken() {
    for host in ["localhost", "LocalHost", "localhost.", "app.localhost", "a-b.c.localhost", "127.0.0.1", "127.8.9.10", "::1", "[::1]"] {
      assert!(loopback(host).is_some(), "{host}");
    }
    assert_eq!(loopback("localhost"), Some(Loopback::Name));
    assert_eq!(loopback("[::1]"), Some(Loopback::Ip("::1".parse().unwrap())));
    for host in [
      "", "0.0.0.0", "::", "10.0.0.2", "192.168.1.1", "example.com", "localhost.example.com", "notlocalhost", ".localhost", "a..localhost",
      "a b.localhost", "::ffff:127.0.0.1", "[127.0.0.1]", "[localhost]", "fe80::1", "169.254.1.1",
    ] {
      assert_eq!(loopback(host), None, "{host}");
    }
  }

  #[test]
  fn the_header_round_trips_and_reads_unknown_codes_as_failed() {
    let ok: TcpHeader = serde_json::from_str("{}").unwrap();
    assert_eq!(ok, TcpHeader::default());
    assert_eq!(serde_json::to_string(&TcpHeader::default()).unwrap(), "{}");
    let refused = TcpHeader { error: Some("nothing on 5173".into()), code: Some(TcpCode::Refused) };
    let text = serde_json::to_string(&refused).unwrap();
    assert_eq!(text, r#"{"error":"nothing on 5173","code":"refused"}"#);
    assert_eq!(serde_json::from_str::<TcpHeader>(&text).unwrap(), refused);
    let newer: TcpHeader = serde_json::from_str(r#"{"error":"x","code":"somethingNew"}"#).unwrap();
    assert_eq!(newer.code, Some(TcpCode::Failed));
    let req = TcpRequest { folder: "ssh://gen1/home/a".into(), host: "localhost".into(), port: 5173 };
    let text = serde_json::to_string(&req).unwrap();
    assert_eq!(text, r#"{"folder":"ssh://gen1/home/a","host":"localhost","port":5173}"#);
    assert_eq!(serde_json::from_str::<TcpRequest>(&text).unwrap(), req);
  }
}
