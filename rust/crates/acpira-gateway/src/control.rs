//! The gateway's control socket (`<ACPIRA_HOME>/run/gateway.sock`, in the engines' private directory): one JSON request
//! per line, one JSON answer per line. The settings page (through the engine) and the `acpira-gateway` commands use it;
//! `watch` keeps answering with the status on every change until the client hangs up.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::gateway::Gateway;

pub use acpira_shared::devices::GatewayRequest as Request;

/// Bind the control socket. One that answers belongs to a running gateway: this one must not start
pub async fn bind(path: &Path) -> Result<UnixListener> {
  if let Some(dir) = path.parent() {
    acpira_desktop::engine::private_dir(dir)?;
  }
  if UnixStream::connect(path).await.is_ok() {
    bail!("a gateway is already running ({})", path.display());
  }
  let _ = std::fs::remove_file(path);
  let l = UnixListener::bind(path).with_context(|| format!("bind {}", path.display()))?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
  }
  Ok(l)
}

pub async fn serve(g: Arc<Gateway>, l: UnixListener, stop: Arc<tokio::sync::Notify>) {
  loop {
    let Ok((s, _)) = l.accept().await else { return };
    let (g, stop) = (g.clone(), stop.clone());
    tokio::spawn(async move {
      let _ = client(g, s, stop).await;
    });
  }
}

async fn client(g: Arc<Gateway>, s: UnixStream, stop: Arc<tokio::sync::Notify>) -> Result<()> {
  let (r, mut w) = s.into_split();
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

async fn write(w: &mut tokio::net::unix::OwnedWriteHalf, v: &Value) -> Result<()> {
  let mut line = v.to_string();
  line.push('\n');
  w.write_all(line.as_bytes()).await?;
  Ok(())
}

/// One request to the running gateway and its answer
pub async fn request(path: &Path, req: &Request) -> Result<Value> {
  let s = UnixStream::connect(path).await.map_err(|_| anyhow!("the gateway is not running (no {})", path.display()))?;
  let (r, mut w) = s.into_split();
  let mut line = serde_json::to_string(req)?;
  line.push('\n');
  w.write_all(line.as_bytes()).await?;
  let answer = BufReader::new(r).lines().next_line().await?.ok_or_else(|| anyhow!("the gateway hung up"))?;
  let v: Value = serde_json::from_str(&answer)?;
  if let Some(e) = v.get("error").and_then(Value::as_str) {
    bail!("{e}");
  }
  Ok(v)
}
