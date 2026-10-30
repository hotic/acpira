//! The gateway's control channel (`acpira_shared::local_ipc`: `<ACPIRA_HOME>/run/gateway.sock` in the engines' private
//! directory, a pipe of this user's on Windows): one JSON request per line, one JSON answer per line. The settings page
//! (through the engine) and the `acpira-gateway` commands use it; `watch` keeps answering with the status on every
//! change until the client hangs up.

use std::path::Path;
use std::sync::Arc;

use acpira_shared::local_ipc::{self, Listener, ServerStream};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};

use crate::gateway::Gateway;

pub use acpira_shared::devices::GatewayRequest as Request;

/// Another gateway answers on the control channel
#[derive(Debug)]
pub struct AlreadyRunning(pub std::path::PathBuf);

impl std::fmt::Display for AlreadyRunning {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "a gateway is already running ({})", self.0.display())
  }
}

impl std::error::Error for AlreadyRunning {}

/// Bind the control channel. One that answers belongs to a running gateway: this one must not start
pub async fn bind(endpoint: &Path) -> Result<Listener> {
  if let Some(dir) = endpoint.parent().filter(|_| cfg!(unix)) {
    acpira_desktop::engine::private_dir(dir)?;
  }
  match Listener::bind(endpoint).await {
    Ok(l) => Ok(l),
    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Err(AlreadyRunning(endpoint.to_path_buf()).into()),
    Err(e) => Err(e).with_context(|| format!("bind {}", endpoint.display())),
  }
}

pub async fn serve(g: Arc<Gateway>, mut l: Listener, stop: Arc<tokio::sync::Notify>) {
  loop {
    let Ok(s) = l.accept().await else { return };
    let (g, stop) = (g.clone(), stop.clone());
    tokio::spawn(async move {
      let _ = client(g, s, stop).await;
    });
  }
}

async fn client(g: Arc<Gateway>, s: ServerStream, stop: Arc<tokio::sync::Notify>) -> Result<()> {
  let (r, mut w) = tokio::io::split(s);
  let mut lines = BufReader::new(r).lines();
  while let Some(line) = lines.next_line().await? {
    let req: Request = match serde_json::from_str(&line) {
      Ok(r) => r,
      Err(e) => {
        write(&mut w, &json!({ "error": e.to_string() })).await?;
        continue;
      }
    };
    let answer = match req {
      Request::Status => serde_json::to_value(g.status())?,
      Request::Watch => {
        let mut rx = g.watch();
        loop {
          write(&mut w, &serde_json::to_value(g.status())?).await?;
          if rx.changed().await.is_err() {
            return Ok(());
          }
        }
      }
      Request::Pair => serde_json::to_value(g.open_pairing())?,
      Request::CancelPair => {
        g.cancel_pairing();
        json!({ "ok": true })
      }
      Request::Revoke { id } => json!({ "ok": g.revoke(&id)? }),
      Request::Rename { id, name } => json!({ "ok": g.rename(&id, &name)? }),
      Request::Reload => {
        g.reload();
        json!({ "ok": true })
      }
      Request::Notify(note) => {
        g.notify(note);
        json!({ "ok": true })
      }
      Request::Stop => {
        write(&mut w, &json!({ "ok": true })).await?;
        stop.notify_one();
        return Ok(());
      }
    };
    write(&mut w, &answer).await?;
  }
  Ok(())
}

async fn write(w: &mut WriteHalf<ServerStream>, v: &Value) -> Result<()> {
  let mut line = v.to_string();
  line.push('\n');
  w.write_all(line.as_bytes()).await?;
  Ok(())
}

/// One request to the running gateway and its answer
pub async fn request(endpoint: &Path, req: &Request) -> Result<Value> {
  let answer = local_ipc::exchange(endpoint, &serde_json::to_string(req)?)
    .await
    .map_err(|_| anyhow!("the gateway is not running (nothing serves {})", endpoint.display()))?
    .ok_or_else(|| anyhow!("the gateway hung up"))?;
  let v: Value = serde_json::from_str(&answer)?;
  if let Some(e) = v.get("error").and_then(Value::as_str) {
    bail!("{e}");
  }
  Ok(v)
}
