//! The helper's wire: one JSON object per line over the ssh channel's stdio, both directions. The helper opens with a
//! handshake line behind `HELLO_PREFIX`; whatever a login shell printed before it is dropped (docs/dev/remote.md).
//!
//! Requests carry an id unique on the connection; their answer is one `res`, optionally after `chunk`s of bulk bytes.
//! Bulk travels under a credit window: the helper keeps at most `BULK_WINDOW` chunk bytes unacknowledged and the
//! client acks each chunk as it lands, so a cancel or a heartbeat never queues behind megabytes already handed to ssh
//! (measured: a ping behind an 8 MiB write took 6 s on gen1). Control frames jump the helper's own queue

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Starts the handshake line; the client ignores every line before one that starts with it
pub const HELLO_PREFIX: &str = "ACPIRA-REMOTE ";
/// Bumped when a frame changes meaning; the deploy keeps one binary per version, so both ends always match
pub const PROTOCOL: u32 = 1;
/// A line longer than this is a protocol error on either side (a request's inline data included)
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Raw bytes per bulk chunk (base64 makes the line about 87 KiB)
pub const CHUNK: usize = 64 * 1024;
/// Unacknowledged bulk bytes the helper may have in flight: about 0.2 s at gen1's slowest measured rate
pub const BULK_WINDOW: usize = 256 * 1024;
/// How often each side pings, and how long either waits for any frame before it gives the other up
pub const PING_EVERY_MS: u64 = 10_000;
pub const DEAD_AFTER_MS: u64 = 35_000;

/// What the helper says first
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
  pub protocol: u32,
  pub version: String,
  /// sha256 of the helper binary, hex; empty when it could not be read
  pub digest: String,
  /// /etc/machine-id on Linux, the hardware UUID on macOS: the identity check against the alias
  pub machine_id: String,
  pub os: String,
  pub arch: String,
  pub home: String,
  pub user: String,
  pub pid: u32,
  /// The login shell's PATH was recovered (`$SHELL -i -l -c`) and commands get it
  pub login_path: bool,
}

/// One frame, either direction
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Frame {
  /// client → helper
  Req { id: u64, op: String, #[serde(default)] a: Value },
  /// client → helper: stop request `id`; its `res` still comes (as `cancelled`), late chunks are dropped
  Cancel { id: u64 },
  /// client → helper: bytes for a running exec's stdin; `eof` closes it
  Stdin { id: u64, #[serde(default)] b: String, #[serde(default)] eof: bool },
  /// client → helper: `n` bulk bytes landed
  Ack { n: usize },
  /// helper → client: part of a request's bulk output; `s` names the stream (`data`, `out`, `err`)
  Chunk { id: u64, s: String, b: String },
  /// helper → client: the request's end
  Res { id: u64, #[serde(default, skip_serializing_if = "Option::is_none")] ok: Option<Value>, #[serde(default, skip_serializing_if = "Option::is_none")] err: Option<WireError> },
  Ping { n: u64 },
  Pong { n: u64 },
}

/// A failed request. `code` is stable (`notFound`, `exists`, `notDir`, `isDir`, `denied`, `tooLarge`, `invalid`,
/// `changed`, `cancelled`, `timeout`, `io`, `unknownOp`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
  pub code: String,
  pub message: String,
}

impl WireError {
  pub fn new(code: &str, message: impl Into<String>) -> WireError {
    WireError { code: code.to_owned(), message: message.into() }
  }

  pub fn io(e: &std::io::Error, what: &str) -> WireError {
    use std::io::ErrorKind as K;
    let code = match e.kind() {
      K::NotFound => "notFound",
      K::AlreadyExists => "exists",
      K::PermissionDenied => "denied",
      K::NotADirectory => "notDir",
      K::IsADirectory => "isDir",
      K::DirectoryNotEmpty => "notEmpty",
      _ => "io",
    };
    WireError::new(code, format!("{what}: {e}"))
  }
}

pub fn b64(bytes: &[u8]) -> String {
  base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn unb64(text: &str) -> Result<Vec<u8>, WireError> {
  base64::engine::general_purpose::STANDARD.decode(text).map_err(|e| WireError::new("invalid", format!("bad base64: {e}")))
}

/// The handshake line (without its newline)
pub fn hello_line(hello: &Hello) -> String {
  format!("{HELLO_PREFIX}{}", serde_json::to_string(hello).unwrap_or_default())
}

/// The hello a line carries, when it is the handshake
pub fn parse_hello(line: &str) -> Option<Hello> {
  serde_json::from_str(line.trim_end().strip_prefix(HELLO_PREFIX)?).ok()
}

/// One line into `buf` (without its newline); 0 at EOF with nothing read. Refuses lines over MAX_LINE
pub async fn read_line<R: tokio::io::AsyncRead + Unpin>(r: &mut tokio::io::BufReader<R>, buf: &mut Vec<u8>) -> std::io::Result<usize> {
  use tokio::io::AsyncBufReadExt as _;
  loop {
    let available = r.fill_buf().await?;
    if available.is_empty() {
      return Ok(buf.len());
    }
    if let Some(i) = available.iter().position(|b| *b == b'\n') {
      buf.extend_from_slice(&available[..i]);
      r.consume(i + 1);
      return Ok(buf.len() + 1);
    }
    let n = available.len();
    buf.extend_from_slice(available);
    r.consume(n);
    if buf.len() > MAX_LINE {
      return Err(std::io::Error::other(format!("a line over {MAX_LINE} bytes")));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn frames_keep_their_short_wire_shape() {
    let req = Frame::Req { id: 7, op: "stat".into(), a: serde_json::json!({ "path": "/x" }) };
    assert_eq!(serde_json::to_string(&req).unwrap(), r#"{"t":"req","id":7,"op":"stat","a":{"path":"/x"}}"#);
    let res = Frame::Res { id: 7, ok: Some(serde_json::json!({})), err: None };
    assert_eq!(serde_json::to_string(&res).unwrap(), r#"{"t":"res","id":7,"ok":{}}"#);
    let back: Frame = serde_json::from_str(r#"{"t":"req","id":1,"op":"info"}"#).unwrap();
    assert_eq!(back, Frame::Req { id: 1, op: "info".into(), a: Value::Null });
  }

  #[test]
  fn the_handshake_survives_noise_in_front() {
    let hello = Hello {
      protocol: PROTOCOL, version: "1.2.3".into(), digest: "ab".into(), machine_id: "m".into(), os: "linux".into(), arch: "x86_64".into(),
      home: "/home/u".into(), user: "u".into(), pid: 9, login_path: true,
    };
    let line = hello_line(&hello);
    assert_eq!(parse_hello(&format!("{line}\r\n")), Some(hello));
    assert_eq!(parse_hello("Welcome to Ubuntu"), None);
  }
}
