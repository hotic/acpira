//! A device's tunnel streams (acpira-net `tunnel`): a dev server on the machine of a project folder, shown on the phone.
//! The grant is decided here: a paired device already drives sessions and shells under the computer's account, so the
//! only rule is the target's, loopback on the folder's machine (`tunnel::loopback`), never this computer's network. A
//! local folder means this computer; an `ssh://` one, a host whose connection is up (through the engine's `SshDial`,
//! never starting a connection for a device). Every stream lives in the device's `JoinSet`, so a link that goes ends
//! them all, sockets and helper calls with them.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use acpira_net::iroh::endpoint::{RecvStream, SendStream};
use acpira_net::tunnel::{self, Loopback, TcpRequest};
pub use acpira_net::tunnel::{TcpCode, TcpFailure};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::engine::Log;

/// A byte stream to a port on the folder's machine
pub trait Pipe: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Pipe for T {}

pub type Dialing = Pin<Box<dyn Future<Output = Result<Box<dyn Pipe>, TcpFailure>> + Send>>;

/// The SSH hosts of the engine the gateway runs in (acpira-host `devices::tunnel`)
pub trait SshDial: Send + Sync {
  /// Whether the host's connection is up now
  fn up(&self, alias: &str) -> bool;
  /// A connection to `host:port` on the host's loopback, over its connection when it is up (an error otherwise).
  /// `host` is `localhost` or an address `tunnel::loopback` took
  fn dial(&self, alias: &str, host: String, port: u16) -> Dialing;
}

/// How long a device has to send its request once the stream is open
const REQUEST_WAIT: Duration = Duration::from_secs(10);
/// How long a connection to the target may take: a local one answers at once, a host's goes over its SSH channel
const DIAL_WAIT: Duration = Duration::from_secs(15);

/// What one device's tunnel streams moved, logged when it disconnects
#[derive(Default)]
pub struct Tally {
  streams: AtomicU64,
  to_device: AtomicU64,
  from_device: AtomicU64,
}

impl Tally {
  /// `tunnels: 12 streams, 1.2 MB to the device, 30 KB from it`, or none when no stream opened
  pub fn line(&self) -> Option<String> {
    let n = self.streams.load(Ordering::Relaxed);
    (n > 0).then(|| {
      let size = crate::wire_stats::size;
      format!("tunnels: {n} streams, {} to the device, {} from it", size(self.to_device.load(Ordering::Relaxed)), size(self.from_device.load(Ordering::Relaxed)))
    })
  }
}

/// Whose machine a request's folder names
#[derive(Debug, PartialEq, Eq)]
enum Machine {
  Here,
  Ssh(String),
}

fn machine(folder: &str) -> Result<Machine, TcpFailure> {
  if acpira_shared::remote::is_remote_folder(folder) {
    let f = acpira_shared::remote::parse_remote_folder(folder).ok_or_else(|| TcpFailure::new(TcpCode::Denied, format!("not a folder on an SSH host: {folder}")))?;
    return Ok(Machine::Ssh(f.alias));
  }
  if std::path::Path::new(folder).is_absolute() {
    Ok(Machine::Here)
  } else {
    Err(TcpFailure::new(TcpCode::Denied, format!("not a project folder: {folder}")))
  }
}

/// Serves one tunnel stream: the request, the target's rules, the header, then the pipe
pub async fn serve(send: SendStream, recv: RecvStream, ssh: Option<Arc<dyn SshDial>>, tally: Arc<Tally>, log: Log, device: String) {
  let Ok(Ok((req, recv, first))) = tokio::time::timeout(REQUEST_WAIT, tunnel::read_request(recv)).await else { return };
  match open(&req, ssh.as_deref()).await {
    Err(f) => {
      if f.code == TcpCode::Denied {
        log(&format!("{device}: tunnel to {}:{} in {} refused: {f}", req.host, req.port, req.folder));
      }
      if let Ok(mut send) = tunnel::answer(send, Err(&f)).await {
        let _ = send.finish();
      }
    }
    // A check: the folder's machine is there
    Ok(None) => {
      if let Ok(mut send) = tunnel::answer(send, Ok(())).await {
        let _ = send.finish();
      }
    }
    Ok(Some(pipe)) => {
      let Ok(send) = tunnel::answer(send, Ok(())).await else { return };
      tally.streams.fetch_add(1, Ordering::Relaxed);
      let moved = tunnel::splice(pipe, send, recv, first).await;
      tally.to_device.fetch_add(moved.from_socket, Ordering::Relaxed);
      tally.from_device.fetch_add(moved.to_socket, Ordering::Relaxed);
    }
  }
}

/// Answers a device that holds `tunnel::MAX_STREAMS` already, without reading its request
pub async fn busy(send: SendStream) {
  let f = TcpFailure::new(TcpCode::Busy, format!("too many connections at once (at most {})", tunnel::MAX_STREAMS));
  if let Ok(Ok(mut send)) = tokio::time::timeout(Duration::from_secs(2), tunnel::answer(send, Err(&f))).await {
    let _ = send.finish();
  }
}

/// The connection a request asks for; none for a check (port 0) that passed
async fn open(req: &TcpRequest, ssh: Option<&dyn SshDial>) -> Result<Option<Box<dyn Pipe>>, TcpFailure> {
  let at = machine(&req.folder)?;
  let target = tunnel::loopback(&req.host)
    .ok_or_else(|| TcpFailure::new(TcpCode::Denied, format!("only localhost and loopback addresses can be reached, not {}", req.host)))?;
  if let Machine::Ssh(alias) = &at {
    let Some(ssh) = ssh else { return Err(TcpFailure::new(TcpCode::Offline, format!("{alias}: SSH hosts are not reachable from here"))) };
    if !ssh.up(alias) {
      return Err(TcpFailure::new(TcpCode::Offline, format!("{alias} is not connected on the computer")));
    }
  }
  if req.port == 0 {
    return Ok(None);
  }
  let dialing = async {
    match at {
      Machine::Here => dial_here(target, req.port).await,
      Machine::Ssh(alias) => {
        // The helper takes these three names only (remote/helper/streams.rs); any name under localhost means its own
        let host = match target {
          Loopback::Name => "localhost".to_owned(),
          Loopback::Ip(ip) => ip.to_string(),
        };
        ssh.ok_or_else(|| TcpFailure::new(TcpCode::Failed, "no SSH hosts here"))?.dial(&alias, host, req.port).await
      }
    }
  };
  match tokio::time::timeout(DIAL_WAIT, dialing).await {
    Ok(r) => r.map(Some),
    Err(_) => Err(TcpFailure::new(TcpCode::Failed, format!("{}:{} did not answer in time", req.host, req.port))),
  }
}

/// A port on this computer's loopback; a name tries IPv4, then IPv6, since a dev server may listen on either alone
async fn dial_here(target: Loopback, port: u16) -> Result<Box<dyn Pipe>, TcpFailure> {
  let addrs: Vec<IpAddr> = match target {
    Loopback::Name => vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()],
    Loopback::Ip(ip) => vec![ip],
  };
  let mut last = None;
  for ip in addrs {
    match tokio::net::TcpStream::connect((ip, port)).await {
      Ok(s) => {
        let _ = s.set_nodelay(true);
        return Ok(Box::new(s));
      }
      Err(e) => last = Some(e),
    }
  }
  let e = last.map(|e| e.to_string()).unwrap_or_default();
  Err(TcpFailure::new(TcpCode::Refused, format!("nothing answers on port {port} of the computer ({e})")))
}

#[cfg(test)]
mod tests {
  #![allow(clippy::unwrap_used, reason = "test code")]

  use super::*;

  #[test]
  fn a_folder_names_its_machine() {
    assert_eq!(machine(&std::env::temp_dir().join("web").to_string_lossy()).unwrap(), Machine::Here);
    assert_eq!(machine("ssh://gen1/home/a/web").unwrap(), Machine::Ssh("gen1".into()));
    for bad in ["web", "", "ssh://gen1", "ssh://-oProxyCommand=x/a", "ssh://gen1/a/../b"] {
      assert_eq!(machine(bad).unwrap_err().code, TcpCode::Denied, "{bad}");
    }
  }
}
