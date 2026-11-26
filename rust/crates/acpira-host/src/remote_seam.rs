//! Lower-layer traits for SSH remote operations so `hooks`, `workspace`, and `acp::session`
//! do not depend upward on `remote`. Concrete implementations live in `remote`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::acp::agents::registry::AgentDef;
use crate::acp::transport::rpc::BoxFuture;

/// One finished helper RPC reply (JSON result plus gathered `data` stream bytes)
#[derive(Debug, Default)]
pub struct RemoteReply {
  pub value: Value,
  pub data: Vec<u8>,
}

/// One event from a streaming helper request
#[derive(Debug)]
pub enum RemoteEvent {
  Chunk { stream: String, bytes: Vec<u8> },
  Done(Result<Value, String>),
}

/// Streaming handle to an in-flight helper request (`tcp`, `tool`, `exec`)
pub trait RemoteStream: Send {
  fn next_event<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Option<RemoteEvent>> + Send + 'a>>;
  fn send_stdin(&self, bytes: &[u8]);
  fn close_stdin(&self);
}

/// Narrow RPC surface of a connected remote host helper
pub trait RemoteClientPort: Send + Sync {
  fn call(&self, op: String, args: Value) -> BoxFuture<Result<RemoteReply, String>>;
  fn request(&self, op: String, args: Value) -> Box<dyn RemoteStream>;
}

/// Live Codex exec-server tunnel on a remote host; dropping it closes the tunnel
pub trait ExecTunnelPort: Send + Sync {
  fn url(&self) -> String;
  fn stop_commands(&self);
}

/// Opened Codex exec-server tunnel metadata and handle
pub struct CodexExecOpened {
  pub version: String,
  pub env_key: &'static str,
  pub url: String,
  pub redacted_url: String,
  pub tunnel: Box<dyn ExecTunnelPort>,
}

/// Narrow SSH connection manager surface consumed by `workspace` and `acp::session`
pub trait RemoteConnector: Send + Sync {
  fn ensure(&self, alias: String, wait: Duration) -> BoxFuture<Result<Arc<dyn RemoteClientPort>>>;
  fn client(&self, alias: &str) -> Option<Arc<dyn RemoteClientPort>>;
  fn open_codex_exec(&self, alias: String, binary: String, def: AgentDef) -> BoxFuture<Result<CodexExecOpened>>;
}

impl<T: RemoteConnector + ?Sized> RemoteConnector for Arc<T> {
  fn ensure(&self, alias: String, wait: Duration) -> BoxFuture<Result<Arc<dyn RemoteClientPort>>> {
    (**self).ensure(alias, wait)
  }

  fn client(&self, alias: &str) -> Option<Arc<dyn RemoteClientPort>> {
    (**self).client(alias)
  }

  fn open_codex_exec(&self, alias: String, binary: String, def: AgentDef) -> BoxFuture<Result<CodexExecOpened>> {
    (**self).open_codex_exec(alias, binary, def)
  }
}
