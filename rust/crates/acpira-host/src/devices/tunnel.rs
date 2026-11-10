//! The engine's SSH hosts as a paired device's tunnel streams reach them (acpira-gateway `tunnel.rs`): a dev server on
//! a host whose connection is up, through its helper's `tcp` op. A device never starts a connection: a host that is off
//! answers so, and the phone says to connect it on the computer.

use std::sync::Arc;

use acpira_gateway::tunnel::{Dialing, Pipe, SshDial, TcpCode, TcpFailure};

use crate::remote::client::RemoteError;
use crate::remote::connections::Connections;

/// Finds the engine's connections when a stream asks (none once the engine is gone)
pub struct SshHosts(Box<dyn Fn() -> Option<Arc<Connections>> + Send + Sync>);

impl SshHosts {
  pub fn new(connections: impl Fn() -> Option<Arc<Connections>> + Send + Sync + 'static) -> Arc<SshHosts> {
    Arc::new(SshHosts(Box::new(connections)))
  }
}

fn offline(alias: &str) -> TcpFailure {
  TcpFailure::new(TcpCode::Offline, format!("{alias} is not connected on the computer"))
}

impl SshDial for SshHosts {
  fn up(&self, alias: &str) -> bool {
    (self.0)().is_some_and(|c| c.is_up(alias))
  }

  fn dial(&self, alias: &str, host: String, port: u16) -> Dialing {
    let client = (self.0)().and_then(|c| c.client(alias)).filter(|c| c.closed_reason().is_none());
    let alias = alias.to_owned();
    Box::pin(async move {
      let client = client.ok_or_else(|| offline(&alias))?;
      match crate::remote::tcp::connect(&client, &host, port).await {
        Ok(s) => Ok(Box::new(s) as Box<dyn Pipe>),
        Err(RemoteError::Disconnected(_)) => Err(offline(&alias)),
        Err(RemoteError::Wire(e)) if e.code == "denied" => Err(TcpFailure::new(TcpCode::Denied, format!("on {alias}: {}", e.message))),
        // Mostly nothing listening there; the helper's words say which
        Err(RemoteError::Wire(e)) => Err(TcpFailure::new(TcpCode::Refused, format!("on {alias}: {}", e.message))),
      }
    })
  }
}
