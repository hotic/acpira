//! Acpira's built-in agent, served over ACP by `acpira agent --home <root>`. The engine launches it like any other agent
//! (same spawn, pool, process group and stderr log); it reads its model sources from `<root>/providers.json` and runs
//! the turn loop itself. Layering follows Pi (a small loop with tools and a model client underneath), the feature set
//! OpenCode (modes, permission rules, tool fault tolerance)

pub mod acp;
pub mod budget;
pub mod config;
pub mod llm;
#[cfg(feature = "mock")]
pub mod mock;
pub mod modes;
pub mod permission;
pub mod prompt;
pub mod tools;
pub mod turn;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use acpira_rpc::rpc::{Connection, Inbound};

/// Serve ACP over a reader / writer pair until the client closes its end; `home` is the data root (ACPIRA_HOME).
/// Not bound to stdio: the subcommand passes stdin / stdout, tests a `tokio::io::duplex` pair
pub async fn run<R, W>(reader: R, writer: W, home: PathBuf, version: &str) -> i32
where
  R: AsyncRead + Unpin + Send + 'static,
  W: AsyncWrite + Unpin + Send + 'static,
{
  let server = acp::Server::new(home, version);
  let handler: Arc<dyn Inbound> = Arc::new(acp::Handler(server.clone()));
  let conn = Connection::start(reader, writer, Arc::new(move || handler.clone()), Arc::new(|line: &str| eprintln!("[acpira agent] {line}")));
  server.attach(conn.clone());
  conn.closed().await;
  0
}
