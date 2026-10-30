//! A local control channel only this user can serve or reach: a Unix socket (mode 600, in a private directory the caller
//! prepares), or on Windows a named pipe whose DACL admits this user alone, refuses remote clients and is checked from the
//! client side to be served by this user (`win_pipe`). The gateway's control channel speaks it; the engines' devices
//! module is its client.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The gateway's control endpoint for a data root: `<root>/run/gateway.sock`, or on Windows a pipe named after the root
/// (paths there are case-insensitive, so the name is too)
pub fn gateway_endpoint(root: &Path) -> PathBuf {
  if cfg!(windows) {
    use sha2::{Digest, Sha256};
    let key = root.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    let d = Sha256::digest(key.as_bytes());
    let hex: String = d.iter().take(8).map(|b| format!("{b:02x}")).collect();
    return PathBuf::from(format!(r"\\.\pipe\acpira-gateway-{hex}"));
  }
  root.join("run").join("gateway.sock")
}

/// Whether something may be serving `endpoint`, without connecting: a socket file exists. Always true for a pipe, where
/// opening it is the only test (and fails at once when nobody serves it)
pub fn may_serve(endpoint: &Path) -> bool {
  cfg!(windows) || endpoint.exists()
}

#[cfg(unix)]
pub type ServerStream = tokio::net::UnixStream;
#[cfg(unix)]
pub type ClientStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type ServerStream = tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(windows)]
pub type ClientStream = tokio::net::windows::named_pipe::NamedPipeClient;

/// The serving side. Binding fails with `AddrInUse` while another server answers there
pub struct Listener {
  #[cfg(unix)]
  inner: tokio::net::UnixListener,
  #[cfg(windows)]
  next: tokio::net::windows::named_pipe::NamedPipeServer,
  #[cfg(windows)]
  name: PathBuf,
}

impl Listener {
  #[cfg(unix)]
  pub async fn bind(endpoint: &Path) -> io::Result<Listener> {
    use std::os::unix::fs::PermissionsExt;
    if tokio::net::UnixStream::connect(endpoint).await.is_ok() {
      return Err(io::Error::new(io::ErrorKind::AddrInUse, format!("{} is served already", endpoint.display())));
    }
    // Nobody answers: a socket file left here belongs to a server that died
    let _ = std::fs::remove_file(endpoint);
    let inner = tokio::net::UnixListener::bind(endpoint)?;
    std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600))?;
    Ok(Listener { inner })
  }

  /// The first instance claims the name: a pipe of that name someone else created first fails the bind
  #[cfg(windows)]
  pub async fn bind(endpoint: &Path) -> io::Result<Listener> {
    match pipe_instance(endpoint, true) {
      Ok(next) => Ok(Listener { next, name: endpoint.to_path_buf() }),
      // ERROR_ACCESS_DENIED: the name exists (a running server, or another user's pipe)
      Err(e) if e.raw_os_error() == Some(5) => Err(io::Error::new(io::ErrorKind::AddrInUse, format!("{} is served already", endpoint.display()))),
      Err(e) => Err(e),
    }
  }

  #[cfg(unix)]
  pub async fn accept(&mut self) -> io::Result<ServerStream> {
    self.inner.accept().await.map(|(s, _)| s)
  }

  /// A pipe instance serves one connection: the next one is created before the connected one is handed over, so a client
  /// arriving meanwhile finds it
  #[cfg(windows)]
  pub async fn accept(&mut self) -> io::Result<ServerStream> {
    loop {
      if self.next.connect().await.is_err() {
        // This instance broke before a client got it: replace it
        self.next = pipe_instance(&self.name, false)?;
        continue;
      }
      let fresh = pipe_instance(&self.name, false)?;
      return Ok(std::mem::replace(&mut self.next, fresh));
    }
  }
}

#[cfg(windows)]
fn pipe_instance(name: &Path, first: bool) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
  let sa = crate::win_pipe::OwnerOnly::new()?;
  let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
  options.first_pipe_instance(first).reject_remote_clients(true);
  // SAFETY: the security attributes outlive the call that copies them into the pipe
  unsafe { options.create_with_security_attributes_raw(name, sa.as_ptr()) }
}

#[cfg(unix)]
pub async fn connect(endpoint: &Path) -> io::Result<ClientStream> {
  tokio::net::UnixStream::connect(endpoint).await
}

/// The pipe, once its server is known to run as this user (anyone may create a pipe of a given name first). Every
/// instance taken answers busy for the moment the server needs to offer the next one
#[cfg(windows)]
pub async fn connect(endpoint: &Path) -> io::Result<ClientStream> {
  const ERROR_PIPE_BUSY: i32 = 231;
  let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
  loop {
    match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
      Ok(c) if crate::win_pipe::served_by_this_user(&c) => return Ok(c),
      Ok(_) => {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is served by a process of another user", endpoint.display())));
      }
      Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && std::time::Instant::now() < deadline => {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
      }
      Err(e) => return Err(e),
    }
  }
}

/// One line out and the first line back
pub async fn exchange(endpoint: &Path, line: &str) -> io::Result<Option<String>> {
  let s = connect(endpoint).await?;
  let (r, mut w) = tokio::io::split(s);
  w.write_all(line.as_bytes()).await?;
  w.write_all(b"\n").await?;
  BufReader::new(r).lines().next_line().await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
mod tests {
  use super::*;

  #[test]
  fn endpoints_follow_the_root() {
    let a = gateway_endpoint(Path::new("/home/u/.acpira"));
    let b = gateway_endpoint(Path::new("/home/u/.acpira/harness"));
    assert_ne!(a, b);
    if cfg!(windows) {
      assert_eq!(gateway_endpoint(Path::new(r"C:\Users\U\.acpira\")), gateway_endpoint(Path::new(r"c:\users\u\.acpira")));
      assert!(a.to_string_lossy().starts_with(r"\\.\pipe\acpira-gateway-"));
    } else {
      assert_eq!(a, Path::new("/home/u/.acpira/run/gateway.sock"));
    }
  }

  #[cfg(any(unix, windows))]
  #[tokio::test]
  async fn one_server_per_endpoint_and_a_line_each_way() {
    let dir = tempfile::tempdir().unwrap();
    let ep = gateway_endpoint(dir.path());
    if let Some(run) = ep.parent().filter(|_| cfg!(unix)) {
      std::fs::create_dir_all(run).unwrap();
    }
    let mut l = Listener::bind(&ep).await.unwrap();
    let server = tokio::spawn(async move {
      let s = l.accept().await.unwrap();
      let (r, mut w) = tokio::io::split(s);
      let line = BufReader::new(r).lines().next_line().await.unwrap().unwrap();
      w.write_all(format!("{line}!\n").as_bytes()).await.unwrap();
      l
    });
    assert_eq!(exchange(&ep, "hi").await.unwrap().as_deref(), Some("hi!"));
    let l = server.await.unwrap();
    assert_eq!(Listener::bind(&ep).await.err().map(|e| e.kind()), Some(io::ErrorKind::AddrInUse));
    drop(l);
  }
}
