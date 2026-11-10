//! The SOCKS5 side of a tunnel's proxy (RFC 1928): no authentication, CONNECT only, IPv4, IPv6 or domain targets. The
//! webview resolves nothing itself; a domain target arrives as the name it asked for.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Reply codes (RFC 1928 section 6)
pub(crate) mod reply {
  pub const OK: u8 = 0;
  pub const FAILURE: u8 = 1;
  pub const NOT_ALLOWED: u8 = 2;
  pub const HOST_UNREACHABLE: u8 = 4;
  pub const REFUSED: u8 = 5;
  pub const COMMAND: u8 = 7;
  pub const ADDRESS_TYPE: u8 = 8;
}

#[derive(Debug)]
#[allow(dead_code, reason = "the tunnel only ends the connection; the tests read why")]
pub(crate) enum SocksError {
  /// The client went away or sent too little
  Io(io::Error),
  /// Not SOCKS5 (a version byte other than 5)
  Version(u8),
  /// The client offered no method this proxy speaks (no authentication)
  NoMethod,
  /// Answered already with this reply code
  Refused(u8),
}

impl From<io::Error> for SocksError {
  fn from(e: io::Error) -> Self {
    SocksError::Io(e)
  }
}

/// The greeting and the request: answers the method choice, and a reply for a request it refuses; the caller answers
/// an accepted one (`reply`) once the target is open. Returns the target's host and port
pub(crate) async fn handshake(r: &mut (impl AsyncRead + Unpin), w: &mut (impl AsyncWrite + Unpin)) -> Result<(String, u16), SocksError> {
  let ver = r.read_u8().await?;
  if ver != 5 {
    return Err(SocksError::Version(ver));
  }
  let n = r.read_u8().await?;
  let mut methods = vec![0u8; n as usize];
  r.read_exact(&mut methods).await?;
  if !methods.contains(&0) {
    w.write_all(&[5, 0xff]).await?;
    return Err(SocksError::NoMethod);
  }
  w.write_all(&[5, 0]).await?;
  let mut head = [0u8; 4];
  r.read_exact(&mut head).await?;
  let [ver, cmd, _, atyp] = head;
  if ver != 5 {
    return Err(SocksError::Version(ver));
  }
  let host = match atyp {
    1 => {
      let mut a = [0u8; 4];
      r.read_exact(&mut a).await?;
      Ipv4Addr::from(a).to_string()
    }
    3 => {
      let len = r.read_u8().await?;
      let mut name = vec![0u8; len as usize];
      r.read_exact(&mut name).await?;
      match String::from_utf8(name) {
        Ok(name) if !name.is_empty() => name,
        _ => return refuse(w, reply::NOT_ALLOWED).await,
      }
    }
    4 => {
      let mut a = [0u8; 16];
      r.read_exact(&mut a).await?;
      Ipv6Addr::from(a).to_string()
    }
    _ => return refuse(w, reply::ADDRESS_TYPE).await,
  };
  let port = r.read_u16().await?;
  if cmd != 1 {
    return refuse(w, reply::COMMAND).await;
  }
  Ok((host, port))
}

async fn refuse<T>(w: &mut (impl AsyncWrite + Unpin), code: u8) -> Result<T, SocksError> {
  w.write_all(&answer(code)).await?;
  Err(SocksError::Refused(code))
}

/// A reply with an unspecified bound address, all a CONNECT client reads
pub(crate) fn answer(code: u8) -> [u8; 10] {
  [5, code, 0, 1, 0, 0, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
  #![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

  use super::*;

  async fn run(input: &[u8]) -> (Result<(String, u16), SocksError>, Vec<u8>) {
    let mut out = Vec::new();
    let r = handshake(&mut &input[..], &mut out).await;
    (r, out)
  }

  #[tokio::test]
  async fn connect_targets_by_name_and_address() {
    let (r, out) = run(&[5, 2, 2, 0, 5, 1, 0, 3, 9, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't', 0x14, 0x45]).await;
    assert_eq!(r.unwrap(), ("localhost".to_owned(), 5189));
    assert_eq!(out, [5, 0], "only the method: the caller answers the request");
    let (r, _) = run(&[5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1, 0, 80]).await;
    assert_eq!(r.unwrap(), ("127.0.0.1".to_owned(), 80));
    let mut v6 = vec![5, 1, 0, 5, 1, 0, 4];
    v6.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
    v6.extend_from_slice(&[0x1f, 0x90]);
    assert_eq!(run(&v6).await.0.unwrap(), ("::1".to_owned(), 8080));
  }

  #[tokio::test]
  async fn other_versions_methods_commands_and_address_types_are_turned_away() {
    assert!(matches!(run(&[4, 1, 0]).await.0, Err(SocksError::Version(4))));
    let (r, out) = run(&[5, 1, 2]).await;
    assert!(matches!(r, Err(SocksError::NoMethod)));
    assert_eq!(out, [5, 0xff]);
    assert!(matches!(run(&[5, 1, 0, 4, 1, 0, 1, 127, 0, 0, 1, 0, 80]).await.0, Err(SocksError::Version(4))));
    // BIND and UDP ASSOCIATE
    for cmd in [2, 3] {
      let (r, out) = run(&[5, 1, 0, 5, cmd, 0, 1, 127, 0, 0, 1, 0, 80]).await;
      assert!(matches!(r, Err(SocksError::Refused(reply::COMMAND))));
      assert_eq!(out[2..], answer(reply::COMMAND));
    }
    let (r, out) = run(&[5, 1, 0, 5, 1, 0, 9, 1, 2]).await;
    assert!(matches!(r, Err(SocksError::Refused(reply::ADDRESS_TYPE))));
    assert_eq!(out[2..], answer(reply::ADDRESS_TYPE));
    let (r, _) = run(&[5, 1, 0, 5, 1, 0, 3, 0, 0, 80]).await;
    assert!(matches!(r, Err(SocksError::Refused(reply::NOT_ALLOWED))), "an empty name");
  }

  #[tokio::test]
  async fn truncated_input_is_an_error_not_a_hang() {
    let full = [5u8, 1, 0, 5, 1, 0, 3, 9, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't', 0x14, 0x45];
    for cut in 0..full.len() {
      match run(&full[..cut]).await.0 {
        Err(SocksError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "cut at {cut}"),
        other => panic!("cut at {cut}: {other:?}"),
      }
    }
  }
}
