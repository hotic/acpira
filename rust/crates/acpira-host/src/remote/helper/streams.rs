//! Connections a call carries: a loopback port here, or this host's engine for a peer

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::Out;
use crate::remote::wire::WireError;

// Only the engine connection (a Unix socket) uses these
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use std::time::Duration;

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
  pump_stream(stream, id, out, stdin_rx).await
}

/// One connection carried by a call: `open` once it is up, its bytes as `data` chunks, `stdin` frames to it (their end
/// shuts its write side); answered when it ends
async fn pump_stream<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>(
  stream: S,
  id: u64,
  out: &Out,
  stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>,
) -> Result<Value, WireError> {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  out.chunks(id, "open", b"1").await;
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
  let mut buf = vec![0u8; 16 * 1024];
  loop {
    match r.read(&mut buf).await {
      Ok(0) | Err(_) => break,
      Ok(n) => out.chunks(id, "data", &buf[..n]).await,
    }
  }
  writer.abort();
  Ok(json!({}))
}

/// How long a just started engine gets to listen
#[cfg(unix)]
const ENGINE_START: Duration = Duration::from_secs(15);

/// The socket of the engine peers reach on this host: one per data root and version (only the same version ever
/// connects, so it always knows the peer role). A path too long for a Unix socket moves under the temp dir
#[cfg(unix)]
fn peer_socket(home: &Path, version: &str) -> PathBuf {
  let own = home.join("run").join(format!("peer-{version}.sock"));
  if own.as_os_str().len() < 100 {
    return own;
  }
  // FNV-1a: stable across runs, which is all a socket name needs
  let mut h: u64 = 0xcbf29ce484222325;
  for b in home.as_os_str().as_encoded_bytes().iter().chain(version.as_bytes()) {
    h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
  }
  // SAFETY: geteuid has no preconditions and cannot fail
  let uid = unsafe { libc::geteuid() };
  std::env::temp_dir().join(format!("acpira-{uid}")).join(format!("peer-{h:016x}.sock"))
}

/// The engine of this host for a peer (remote run, docs/dev/remote.md): the running `acpira serve` of this version, or
/// one started here and left running past this helper (its own session, output to `peer-<version>.log`), then the call
/// carries the connection to its socket. `home`: another data root (tests)
#[cfg(unix)]
pub(super) async fn engine(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>, version: &str) -> Result<Value, WireError> {
  let home = a.get("home").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let socket = peer_socket(&home, version);
  let stream = match tokio::net::UnixStream::connect(&socket).await {
    Ok(s) => s,
    Err(_) => {
      let exe = std::env::current_exe().map_err(|e| WireError::io(&e, "this helper's binary"))?;
      let log_dir = home.join("run");
      std::fs::create_dir_all(&log_dir).map_err(|e| WireError::io(&e, &log_dir.to_string_lossy()))?;
      let log = std::fs::OpenOptions::new().create(true).append(true).open(log_dir.join(format!("peer-{version}.log"))).map_err(|e| WireError::io(&e, "the engine log"))?;
      let err = log.try_clone().map_err(|e| WireError::io(&e, "the engine log"))?;
      let mut cmd = std::process::Command::new(exe);
      cmd.arg("serve").arg("--socket").arg(&socket).arg("--home").arg(&home).stdin(Stdio::null()).stdout(log).stderr(err);
      // SAFETY: setsid is async-signal-safe and touches no memory of this process
      unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
          libc::setsid();
          Ok(())
        });
      }
      let mut child = cmd.spawn().map_err(|e| WireError::io(&e, "acpira serve"))?;
      // Reaped by a thread of its own: it outlives this helper, which only must not leave a zombie meanwhile
      std::thread::spawn(move || {
        let _ = child.wait();
      });
      let deadline = tokio::time::Instant::now() + ENGINE_START;
      let mut wait = Duration::from_millis(5);
      loop {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
          break s;
        }
        if tokio::time::Instant::now() >= deadline {
          return Err(WireError::new("unavailable", format!("the engine on this host did not listen within {}s", ENGINE_START.as_secs())));
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_millis(100));
      }
    }
  };
  pump_stream(stream, id, out, stdin_rx).await
}
