//! A loopback port on an SSH host as a byte stream, over the helper's `tcp` op (which takes loopback targets only and
//! checks that on the host). Reads are the call's `data` chunks; writes go out as its stdin frames, unbounded like every
//! stdin frame on the channel. Dropping the stream cancels the call, so the host closes its end at once: a browser
//! panel's proxy and a device's tunnel stream never leave a helper connection behind.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::client::{Call, Event, RemoteClient, RemoteError};

pub struct TcpCall {
  call: Call,
  /// A chunk not yet read whole, and how far into it
  pending: Vec<u8>,
  at: usize,
  /// The call ended: reads answer the end from now on
  ended: Option<Result<(), String>>,
  shut: bool,
}

/// Connects to `host:port` on the host (`localhost`, `127.0.0.1` or `::1`); fails with the helper's answer
pub async fn connect(client: &RemoteClient, host: &str, port: u16) -> Result<TcpCall, RemoteError> {
  let mut call = client.request("tcp", serde_json::json!({ "host": host, "port": port }));
  match call.next().await {
    Some(Event::Chunk { stream, .. }) if stream == "open" => Ok(TcpCall { call, pending: Vec::new(), at: 0, ended: None, shut: false }),
    Some(Event::Done(Err(e))) => Err(e),
    _ => Err(RemoteError::Disconnected(client.closed_reason().unwrap_or_else(|| "the connection ended".into()))),
  }
}

impl AsyncRead for TcpCall {
  fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
    let me = &mut *self;
    loop {
      if me.at < me.pending.len() {
        let n = buf.remaining().min(me.pending.len() - me.at);
        buf.put_slice(&me.pending[me.at..me.at + n]);
        me.at += n;
        return Poll::Ready(Ok(()));
      }
      if let Some(end) = &me.ended {
        return Poll::Ready(end.clone().map_err(io::Error::other));
      }
      match std::task::ready!(me.call.poll_next(cx)) {
        Some(Event::Chunk { stream, bytes }) if stream == "data" => {
          me.pending = bytes;
          me.at = 0;
        }
        Some(Event::Chunk { .. }) => {}
        Some(Event::Done(Ok(_))) => me.ended = Some(Ok(())),
        Some(Event::Done(Err(e))) => me.ended = Some(Err(e.to_string())),
        None => me.ended = Some(Err("the connection to the host ended".into())),
      }
    }
  }
}

impl AsyncWrite for TcpCall {
  fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
    if self.shut || self.ended.is_some() {
      return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
    }
    self.call.stdin(buf);
    Poll::Ready(Ok(buf.len()))
  }

  fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    Poll::Ready(Ok(()))
  }

  /// Shuts the host connection's write side; its answer keeps coming
  fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    if !self.shut {
      self.shut = true;
      self.call.stdin_eof();
    }
    Poll::Ready(Ok(()))
  }
}
