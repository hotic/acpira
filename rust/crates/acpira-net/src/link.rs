//! The link between a paired device and a computer's gateway (ALPN `acpira/link/1`). The device opens one envelope
//! stream for the sidecar envelopes plus the link's own control frames, one short stream per blob, so an image never
//! holds up a patch, and one per tunnelled connection (`tunnel.rs`). Every stream starts with a byte naming its kind.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use crate::frame::{Codec, FrameKind, FrameReader, FrameWriter};

pub const STREAM_ENVELOPES: u8 = b'E';
pub const STREAM_BLOB: u8 = b'B';
/// A connection to a loopback port on a project folder's machine (`tunnel.rs`); only gateways that list
/// `tunnel::FEATURE` in `Welcome` take it
pub const STREAM_TCP: u8 = b'T';
/// A blob larger than this is refused: transcripts hold screenshots and pasted images, not files
pub const MAX_BLOB: u64 = 64 << 20;

/// The link's own messages, next to the envelopes on the same stream
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Control {
  /// The gateway's first frame once the device is admitted. `protocol` is the engine's `SIDECAR_PROTOCOL_VERSION`:
  /// a device on another one says "update Acpira" instead of sending envelopes the engine would misread
  Welcome {
    name: String,
    version: String,
    protocol: i64,
    /// What this gateway takes beyond the envelope and blob streams (`tunnel::FEATURE`); none from one that predates it
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
  },
  /// Either side probes the other; the answer echoes `n`
  Ping { n: u64 },
  Pong { n: u64 },
  /// How the computer can be reached now, for the device to dial next time
  Addresses {
    #[serde(default)]
    relay: Option<String>,
    #[serde(default)]
    addrs: Vec<String>,
  },
  /// The gateway did not pass a message on: `what` is its type (`setSetting`, `openFile`, …), which only the computer
  /// itself may send (acpira-shared `devices::device_allowed`)
  Denied { what: String },
  /// Device to gateway: where this device's notifications go, or none to stop them. A gateway that predates it reads
  /// an unknown control frame and ignores it
  Push { target: Option<acpira_shared::devices::PushTarget> },
}

/// Why the gateway closed a link: the QUIC application close code, which the device reads to say what happened
pub mod close {
  /// Not a paired device (or a stranger on the link ALPN)
  pub const UNKNOWN: u32 = 1;
  /// The computer removed this device
  pub const REVOKED: u32 = 2;
  /// Remote access was turned off, or the gateway is stopping
  pub const SHUTDOWN: u32 = 3;
  /// The device read its view too slowly; reconnecting starts from whole views
  pub const BEHIND: u32 = 4;
  /// The engine is gone (it failed to start, or ended)
  pub const ENGINE: u32 = 5;
}

/// The close code of a link the other side closed, if it closed it on purpose
pub fn close_code(e: &iroh::endpoint::ConnectionError) -> Option<u32> {
  match e {
    iroh::endpoint::ConnectionError::ApplicationClosed(c) => u32::try_from(c.error_code.into_inner()).ok(),
    _ => None,
  }
}

pub type EnvelopeTx = FrameWriter<SendStream>;
pub type EnvelopeRx = FrameReader<RecvStream>;

/// Device side: open the envelope stream. The gateway answers with its codec byte, then `Welcome`
pub async fn open_envelopes(conn: &Connection) -> Result<(EnvelopeTx, EnvelopeRx)> {
  let (mut send, recv) = conn.open_bi().await.context("open stream")?;
  send.write_all(&[STREAM_ENVELOPES]).await?;
  let tx = FrameWriter::start(send, Codec::Deflate).await?;
  let rx = FrameReader::start(recv).await?;
  Ok((tx, rx))
}

pub enum Incoming {
  Envelopes(EnvelopeTx, EnvelopeRx),
  Blob(BlobRequest, SendStream),
  /// A tunnel stream, its request still unread (`tunnel::read_request`)
  Tcp(SendStream, RecvStream),
  /// A kind this build does not know: the stream is dropped, the link goes on
  Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobRequest {
  pub session_id: String,
  pub name: String,
  /// Where to start: a download resuming its partial copy. A gateway that predates it sends the whole blob, and its
  /// header then carries no `total`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlobHeader {
  /// The bytes that follow
  #[serde(default, skip_serializing_if = "Option::is_none")]
  size: Option<u64>,
  /// The whole blob's size, set when the answer starts at the request's `offset`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  total: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  error: Option<String>,
}

/// Gateway side: the next stream the device opened, by kind
pub async fn accept(conn: &Connection) -> Result<Incoming> {
  let (send, mut recv) = conn.accept_bi().await.context("accept stream")?;
  match recv.read_u8().await? {
    STREAM_ENVELOPES => {
      let rx = FrameReader::start(recv).await?;
      let tx = FrameWriter::start(send, Codec::Deflate).await?;
      Ok(Incoming::Envelopes(tx, rx))
    }
    STREAM_BLOB => {
      let mut rx = FrameReader::start(recv).await?;
      let (_, payload) = rx.next().await?.ok_or_else(|| anyhow!("blob stream without a request"))?;
      Ok(Incoming::Blob(serde_json::from_slice(&payload)?, send))
    }
    STREAM_TCP => Ok(Incoming::Tcp(send, recv)),
    k => Ok(Incoming::Unknown(k)),
  }
}

/// Gateway side: answer a blob request with its bytes from `offset` on (the whole blob without one), or why not
pub async fn send_blob(send: SendStream, data: std::result::Result<Vec<u8>, String>, offset: Option<u64>) -> Result<()> {
  let mut tx = FrameWriter::start(send, Codec::Plain).await?;
  let from = |d: &[u8]| offset.map_or(0, |o| usize::try_from(o).unwrap_or(usize::MAX).min(d.len()));
  let header = match &data {
    Ok(d) => BlobHeader { size: Some((d.len() - from(d)) as u64), total: offset.map(|_| d.len() as u64), error: None },
    Err(e) => BlobHeader { size: None, total: None, error: Some(e.clone()) },
  };
  tx.send(FrameKind::Control, &serde_json::to_vec(&header)?).await?;
  let mut send = tx.into_inner();
  if let Ok(d) = data {
    send.write_all(&d[from(&d)..]).await?;
  }
  send.finish()?;
  Ok(())
}

/// Device side: fetch one blob of a session (an image the transcript shows)
pub async fn fetch_blob(conn: &Connection, req: &BlobRequest, within: Duration) -> Result<Vec<u8>> {
  tokio::time::timeout(within, async {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[STREAM_BLOB]).await?;
    let mut tx = FrameWriter::start(send, Codec::Plain).await?;
    tx.send(FrameKind::Control, &serde_json::to_vec(req)?).await?;
    tx.into_inner().finish()?;
    let mut rx = FrameReader::start(recv).await?;
    let (_, head) = rx.next().await?.ok_or_else(|| anyhow!("no answer"))?;
    let head: BlobHeader = serde_json::from_slice(&head)?;
    if let Some(e) = head.error {
      bail!("{e}");
    }
    let size = head.size.unwrap_or(0);
    if size > MAX_BLOB {
      bail!("blob of {size} bytes");
    }
    // Bytes the frame reader already pulled past the header are lost to it: the header is sent on its own write and
    // the body follows, but a reader may have buffered both. Take what it holds first
    let mut body = rx.take_buffered();
    rx.get_mut().take(size - body.len() as u64).read_to_end(&mut body).await?;
    if body.len() as u64 != size {
      bail!("blob cut short");
    }
    Ok(body)
  })
  .await
  .map_err(|_| anyhow!("blob timed out"))?
}

/// Device side: download one blob into the file at `path`, resuming after the bytes it already holds, and answer the
/// blob's size. Unlike `fetch_blob` there is no deadline for the whole file, which a large video on a slow path could
/// never meet: only `idle` without a byte ends it, and the partial file stays for the next try. `progress` hears the
/// bytes held and the total as they grow
pub async fn fetch_blob_to(
  conn: &Connection,
  session_id: &str,
  name: &str,
  path: &std::path::Path,
  idle: Duration,
  mut progress: impl FnMut(u64, u64),
) -> Result<u64> {
  use tokio::io::{AsyncSeekExt, AsyncWriteExt};
  let mut file = tokio::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).await.context("open the download")?;
  let mut have = file.metadata().await?.len();
  let req = BlobRequest { session_id: session_id.to_owned(), name: name.to_owned(), offset: Some(have) };
  let quiet = |what: &'static str| move |_| anyhow!("blob stalled ({what})");
  let (mut send, recv) = tokio::time::timeout(idle, conn.open_bi()).await.map_err(quiet("open"))??;
  send.write_all(&[STREAM_BLOB]).await?;
  let mut tx = FrameWriter::start(send, Codec::Plain).await?;
  tx.send(FrameKind::Control, &serde_json::to_vec(&req)?).await?;
  tx.into_inner().finish()?;
  let mut rx = tokio::time::timeout(idle, FrameReader::start(recv)).await.map_err(quiet("header"))??;
  let (_, head) = tokio::time::timeout(idle, rx.next()).await.map_err(quiet("header"))??.ok_or_else(|| anyhow!("no answer"))?;
  let head: BlobHeader = serde_json::from_slice(&head)?;
  if let Some(e) = head.error {
    bail!("{e}");
  }
  let size = head.size.unwrap_or(0);
  let total = match head.total {
    Some(total) => total,
    // An older gateway sent the whole blob: start the file over
    None => {
      have = 0;
      size
    }
  };
  if total > MAX_BLOB || have + size != total {
    bail!("blob of {total} bytes from {have}");
  }
  file.set_len(have).await?;
  file.seek(std::io::SeekFrom::Start(have)).await?;
  progress(have, total);
  // Bytes the frame reader already pulled past the header (as in `fetch_blob`)
  let first = rx.take_buffered();
  let mut left = size;
  if !first.is_empty() {
    let n = (first.len() as u64).min(left);
    file.write_all(&first[..n as usize]).await?;
    have += n;
    left -= n;
    progress(have, total);
  }
  let recv = rx.get_mut();
  let mut buf = vec![0u8; 64 << 10];
  while left > 0 {
    let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
    let n = tokio::time::timeout(idle, AsyncReadExt::read(recv, &mut buf[..want])).await.map_err(quiet("body"))??;
    if n == 0 {
      bail!("blob cut short");
    }
    file.write_all(&buf[..n]).await?;
    have += n as u64;
    left -= n as u64;
    progress(have, total);
  }
  file.flush().await?;
  file.sync_all().await?;
  Ok(total)
}

/// Encode a control frame
pub fn control(c: &Control) -> Vec<u8> {
  serde_json::to_vec(c).unwrap_or_default()
}
