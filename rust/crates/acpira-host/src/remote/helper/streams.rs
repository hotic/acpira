//! Connections a call carries: a loopback port here, or this host's engine for a peer

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::Out;
use crate::remote::wire::WireError;

// Only the engine connection (a Unix socket) uses this
#[cfg(unix)]
use std::path::PathBuf;

/// A connection to a loopback port here, and only there: the target is checked on this side, so an engine cannot use
/// the host as a way into its network. `open` goes out once connected; then the upstream's bytes as `data` chunks, and
/// `stdin` frames to it (their end shuts its write side). Answered when the upstream ends
pub(super) async fn tcp(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let host = a.get("host").and_then(Value::as_str).unwrap_or("");
  let port = a.get("port").and_then(Value::as_u64).filter(|p| (1..=65535).contains(p)).ok_or_else(|| WireError::new("invalid", "a port is needed"))? as u16;
  if !matches!(host, "127.0.0.1" | "::1" | "localhost") {
    return Err(WireError::new("denied", format!("only loopback addresses can be reached on this host, not {host}")));
  }
  let stream = tokio::net::TcpStream::connect((host, port)).await.map_err(|e| WireError::io(&e, &format!("{host}:{port}")))?;
  pump_stream(stream, id, out, stdin_rx, false).await
}

/// One connection carried by a call: `open` once it is up, its bytes as `data` chunks, `stdin` frames to it (their end
/// shuts its write side); answered when it ends
async fn pump_stream<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>(
  stream: S,
  id: u64,
  out: &Out,
  stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>,
  deflate: bool,
) -> Result<Value, WireError> {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  // The `open` chunk says whether the bytes that follow are compressed, so a client that asked meets a helper that
  // could not (an older release a local build deployed) and reads plain bytes
  let mut z = deflate.then(crate::remote::deflate::Deflater::default);
  out.chunks(id, "open", if z.is_some() { crate::remote::deflate::OPEN_DEFLATE } else { b"1" }).await;
  let (mut r, mut w) = tokio::io::split(stream);
  let writer = tokio::spawn(async move {
    let Some(mut rx) = stdin_rx else { return };
    while let Some(m) = rx.recv().await {
      match m {
        Some(bytes) => {
          if w.write_all(&bytes).await.is_err() {
            return;
          }
        }
        None => {
          let _ = w.shutdown().await;
          return;
        }
      }
    }
  });
  // A whole wire chunk per read when compressing: fewer sync flushes, each of which costs ratio
  let mut buf = vec![0u8; if z.is_some() { crate::remote::wire::CHUNK } else { 16 * 1024 }];
  let mut packed = vec![];
  loop {
    match r.read(&mut buf).await {
      Ok(0) | Err(_) => break,
      Ok(n) => match &mut z {
        None => out.chunks(id, "data", &buf[..n]).await,
        Some(z) => {
          packed.clear();
          if z.pack(&buf[..n], &mut packed).is_err() {
            break;
          }
          out.chunks(id, "data", &packed).await;
        }
      },
    }
  }
  writer.abort();
  Ok(json!({}))
}

/// The machine's engine on this host for a peer (remote run, docs/dev/remote.md), the same one the host's own windows
/// use: started when none runs, never replaced by this build when it is a local one (`machine_engine::policy`), and an
/// older release replaced by a newer one like any shell would (`machine_engine::ensure`). The call then carries the
/// connection to its socket, deflated when the call asks (`deflate`, `remote/deflate.rs`). `home`: another data root
/// (tests)
#[cfg(unix)]
pub(super) async fn engine(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>, _version: &str) -> Result<Value, WireError> {
  let home = a.get("home").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let log_path = home.join("run").join("engine.log");
  let log = move |line: &str| {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
      let _ = writeln!(f, "[acpira remote] {line}");
    }
  };
  // A few rounds: the engine can go between its answer and the connect (a newer build took over, or it idled out)
  let mut attempt = 0;
  let stream = loop {
    let reached = crate::machine_engine::ensure::ensure(&home, false, &log)
      .await
      .map_err(|e| WireError::new("unavailable", format!("the engine on this host: {e:#}")))?;
    match tokio::net::UnixStream::connect(&reached.socket).await {
      Ok(s) => break s,
      Err(e) if attempt < 4 && matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
        attempt += 1;
        tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
      }
      Err(e) => return Err(WireError::io(&e, "the engine on this host")),
    }
  };
  let deflate = a.get("deflate").and_then(Value::as_bool).unwrap_or(false);
  pump_stream(stream, id, out, stdin_rx, deflate).await
}
