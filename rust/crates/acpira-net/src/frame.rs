//! Frames on a QUIC stream: `[kind u8][len u32 BE][payload]`, after one codec byte the writer sends first.
//!
//! The deflate codec keeps one compression context for the whole stream and flushes after every frame. The engine's
//! patches resend the whole growing tail block of a streaming reply every batch, so consecutive frames repeat most of
//! each other: measured on replayed replies, one streaming session drops from 60-400 KB/s to 1.4-4 KB/s, without
//! touching the patch protocol. Pure Rust (miniz_oxide), so the phone build needs no C toolchain.

use std::io;

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A frame larger than this ends the stream: a whole session view is a few MB at most, images go through blob streams
pub const MAX_FRAME: usize = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Codec {
  Plain = 0,
  Deflate = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
  /// A sidecar envelope (`packages/shared/sidecar.ts`), JSON
  Envelope = 0,
  /// A message of the link itself (`link::Control`), JSON
  Control = 1,
}

impl FrameKind {
  fn from_u8(b: u8) -> io::Result<Self> {
    match b {
      0 => Ok(Self::Envelope),
      1 => Ok(Self::Control),
      _ => Err(invalid(format!("unknown frame kind {b}"))),
    }
  }
}

fn invalid(msg: impl Into<String>) -> io::Error {
  io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub struct FrameWriter<W> {
  w: W,
  z: Option<Compress>,
  plain: Vec<u8>,
  out: Vec<u8>,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
  pub async fn start(mut w: W, codec: Codec) -> io::Result<Self> {
    w.write_all(&[codec as u8]).await?;
    let z = (codec == Codec::Deflate).then(|| Compress::new(Compression::new(6), false));
    Ok(Self { w, z, plain: Vec::new(), out: Vec::new() })
  }

  pub async fn send(&mut self, kind: FrameKind, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME {
      return Err(invalid(format!("frame of {} bytes", payload.len())));
    }
    self.plain.clear();
    self.plain.push(kind as u8);
    self.plain.extend_from_slice(&u32::try_from(payload.len()).map_err(|_| invalid("frame too large"))?.to_be_bytes());
    self.plain.extend_from_slice(payload);
    match &mut self.z {
      None => self.w.write_all(&self.plain).await?,
      Some(z) => {
        self.out.clear();
        deflate_all(z, &self.plain, &mut self.out)?;
        self.w.write_all(&self.out).await?;
      }
    }
    self.w.flush().await
  }

  pub fn into_inner(self) -> W {
    self.w
  }

  pub fn get_mut(&mut self) -> &mut W {
    &mut self.w
  }
}

/// Compress all of `input` and sync-flush, so the reader can decode this frame without waiting for the next
fn deflate_all(z: &mut Compress, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
  let mut pos = 0;
  loop {
    if out.capacity() - out.len() < 1024 {
      out.reserve(out.capacity().max(4096));
    }
    let before = z.total_in();
    z.compress_vec(&input[pos..], out, FlushCompress::Sync).map_err(|e| invalid(e.to_string()))?;
    pos += usize::try_from(z.total_in() - before).map_err(|_| invalid("deflate overflow"))?;
    // Done once every input byte went in and the flush did not fill the buffer (a full one may hold more)
    if pos == input.len() && out.len() < out.capacity() {
      return Ok(());
    }
  }
}

pub struct FrameReader<R> {
  r: R,
  z: Option<Decompress>,
  /// Decoded bytes not yet taken as frames
  plain: Vec<u8>,
  raw: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
  pub async fn start(mut r: R) -> io::Result<Self> {
    let codec = r.read_u8().await?;
    let z = match codec {
      0 => None,
      1 => Some(Decompress::new(false)),
      c => return Err(invalid(format!("unknown codec {c}"))),
    };
    Ok(Self { r, z, plain: Vec::new(), raw: vec![0; 16 * 1024] })
  }

  /// The next frame; `None` when the stream ended cleanly between frames
  pub async fn next(&mut self) -> io::Result<Option<(FrameKind, Vec<u8>)>> {
    loop {
      if let Some(f) = self.take()? {
        return Ok(Some(f));
      }
      let n = self.r.read(&mut self.raw).await?;
      if n == 0 {
        return if self.plain.is_empty() { Ok(None) } else { Err(io::ErrorKind::UnexpectedEof.into()) };
      }
      match &mut self.z {
        None => self.plain.extend_from_slice(&self.raw[..n]),
        Some(z) => inflate_all(z, &self.raw[..n], &mut self.plain)?,
      }
    }
  }

  fn take(&mut self) -> io::Result<Option<(FrameKind, Vec<u8>)>> {
    let Some(head) = self.plain.get(..5) else { return Ok(None) };
    let kind = FrameKind::from_u8(head[0])?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_FRAME {
      return Err(invalid(format!("frame of {len} bytes")));
    }
    if self.plain.len() < 5 + len {
      return Ok(None);
    }
    let payload = self.plain[5..5 + len].to_vec();
    self.plain.drain(..5 + len);
    Ok(Some((kind, payload)))
  }

  pub fn get_mut(&mut self) -> &mut R {
    &mut self.r
  }

  /// What the reader pulled in past the last frame: with the plain codec, the start of raw bytes that follow the frames
  /// on the stream (a blob body)
  pub fn take_buffered(&mut self) -> Vec<u8> {
    std::mem::take(&mut self.plain)
  }
}

fn inflate_all(z: &mut Decompress, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
  let mut pos = 0;
  loop {
    if out.capacity() - out.len() < 4096 {
      out.reserve(out.capacity().max(16 * 1024));
    }
    let before = z.total_in();
    let status = z.decompress_vec(&input[pos..], out, FlushDecompress::Sync).map_err(|e| invalid(e.to_string()))?;
    pos += usize::try_from(z.total_in() - before).map_err(|_| invalid("inflate overflow"))?;
    if status == Status::StreamEnd {
      return Err(invalid("deflate stream ended"));
    }
    if pos == input.len() && out.len() < out.capacity() {
      return Ok(());
    }
    if out.len() > 2 * MAX_FRAME {
      return Err(invalid("inflated past any frame"));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  async fn round_trip(codec: Codec, frames: &[(FrameKind, Vec<u8>)]) -> usize {
    let (a, b) = tokio::io::duplex(1 << 20);
    let mut w = FrameWriter::start(a, codec).await.unwrap();
    for (k, p) in frames {
      w.send(*k, p).await.unwrap();
    }
    drop(w);
    let mut r = FrameReader::start(b).await.unwrap();
    for (k, p) in frames {
      let (k2, p2) = r.next().await.unwrap().unwrap();
      assert_eq!((k2, &p2), (*k, p));
    }
    assert!(r.next().await.unwrap().is_none());
    frames.len()
  }

  #[tokio::test]
  async fn frames_survive_both_codecs_and_repeats_compress() {
    let text = "Streaming reply line with **markdown** and `code`.\n".repeat(40);
    // A growing tail block resent every batch, like the engine's patches
    let frames: Vec<(FrameKind, Vec<u8>)> = (1..200)
      .map(|n| (FrameKind::Envelope, format!(r#"{{"type":"sessionPatch","text":{:?}}}"#, text.get(..n * 10).unwrap()).into_bytes()))
      .chain([(FrameKind::Control, br#"{"type":"ping","n":1}"#.to_vec()), (FrameKind::Envelope, vec![]), (FrameKind::Envelope, vec![7u8; 300_000])])
      .collect();
    assert_eq!(round_trip(Codec::Plain, &frames).await, frames.len());
    assert_eq!(round_trip(Codec::Deflate, &frames).await, frames.len());

    let raw: usize = frames.iter().map(|(_, p)| p.len() + 5).sum();
    let mut sink = Vec::new();
    let mut w = FrameWriter::start(&mut sink, Codec::Deflate).await.unwrap();
    for (k, p) in &frames {
      w.send(*k, p).await.unwrap();
    }
    assert!(sink.len() * 20 < raw, "{} of {raw}", sink.len());
  }

  #[tokio::test]
  async fn a_cut_stream_is_an_error_not_a_clean_end() {
    let mut buf = Vec::new();
    let mut w = FrameWriter::start(&mut buf, Codec::Plain).await.unwrap();
    w.send(FrameKind::Envelope, b"hello").await.unwrap();
    buf.truncate(buf.len() - 2);
    let mut r = FrameReader::start(&buf[..]).await.unwrap();
    assert!(r.next().await.is_err());
  }
}
