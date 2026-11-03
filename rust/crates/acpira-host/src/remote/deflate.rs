//! Deflate for a stream a call carries (the host engine's connection, `helper/streams.rs`): one compression context for
//! the whole stream, sync-flushed after every piece, so the reader decodes each piece as it lands without waiting for
//! the next. Session views are JSON that repeats itself (and every patch resends the growing tail block), so this
//! shrinks what crosses SSH several times over; the same scheme as the paired link (`acpira_net::frame`)

use std::io;

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

/// What the `open` chunk of a compressed stream says, instead of `1`
pub const OPEN_DEFLATE: &[u8] = b"deflate";

fn invalid(msg: impl Into<String>) -> io::Error {
  io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub struct Deflater(Compress);

impl Default for Deflater {
  fn default() -> Self {
    // Level 3: most of level 6's gain on JSON at a fraction of its time, since a host may push a view of many MB
    Self(Compress::new(Compression::new(3), false))
  }
}

impl Deflater {
  /// All of `input` compressed onto `out`, then a sync flush
  pub fn pack(&mut self, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    let mut pos = 0;
    loop {
      if out.capacity() - out.len() < 1024 {
        out.reserve(out.capacity().max(4096));
      }
      let before = self.0.total_in();
      self.0.compress_vec(&input[pos..], out, FlushCompress::Sync).map_err(|e| invalid(e.to_string()))?;
      pos += usize::try_from(self.0.total_in() - before).map_err(|_| invalid("deflate overflow"))?;
      // Done once every input byte went in and the flush did not fill the buffer (a full one may hold more)
      if pos == input.len() && out.len() < out.capacity() {
        return Ok(());
      }
    }
  }
}

pub struct Inflater(Decompress);

impl Default for Inflater {
  fn default() -> Self {
    Self(Decompress::new(false))
  }
}

impl Inflater {
  /// Everything `input` decodes to, appended to `out`
  pub fn unpack(&mut self, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    let mut pos = 0;
    loop {
      if out.capacity() - out.len() < 4096 {
        out.reserve(out.capacity().max(16 * 1024));
      }
      let before = self.0.total_in();
      let status = self.0.decompress_vec(&input[pos..], out, FlushDecompress::Sync).map_err(|e| invalid(e.to_string()))?;
      pos += usize::try_from(self.0.total_in() - before).map_err(|_| invalid("inflate overflow"))?;
      if status == Status::StreamEnd {
        return Err(invalid("deflate stream ended"));
      }
      if pos == input.len() && out.len() < out.capacity() {
        return Ok(());
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pieces_decode_as_they_land_and_repetition_shrinks() {
    let line = br#"{"type":"sessionPatch","patch":{"id":"s1","turns":[{"role":"agent","blocks":[{"type":"text","text":"hello"}]}]}}"#;
    let (mut d, mut i) = (Deflater::default(), Inflater::default());
    let (mut wire, mut plain) = (0, vec![]);
    for _ in 0..50 {
      let mut packed = vec![];
      d.pack(line, &mut packed).unwrap();
      wire += packed.len();
      let before = plain.len();
      i.unpack(&packed, &mut plain).unwrap();
      // A piece is whole on its own: the reader never waits for the next one
      assert_eq!(&plain[before..], line);
    }
    assert!(wire * 5 < line.len() * 50, "{wire} bytes for {}", line.len() * 50);
  }
}
