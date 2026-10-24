//! This engine as a peer of an SSH host's engine (remote run, docs/dev/remote.md): the helper's `engine` op reaches the
//! host's `acpira serve` of the same version (starting it when none runs), and the call carries the sidecar wire. The
//! hello says `role: peer`, so the host's windows keep their place (`sidecar/server.rs`)

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION;

use crate::remote::client::{Event, RemoteClient};

/// How long the host's engine gets to come up and answer hello
const HELLO_WAIT: Duration = Duration::from_secs(30);

/// What the peer says about itself in hello
pub struct PeerHello<'a> {
  pub version: &'a str,
  /// The platform methods this engine serves for the host (the requests its views cause come back here)
  pub capabilities: &'a [&'a str],
  pub host_language: &'a str,
}

/// One connection to a host's engine: envelopes out with `send`, envelopes in from `recv`. Dropping it closes the
/// connection there, which detaches its views like a window closing
pub struct PeerLink {
  out: mpsc::UnboundedSender<String>,
  inbox: mpsc::UnboundedReceiver<Value>,
  /// The host engine's `helloOk`
  pub hello_ok: Value,
  task: tokio::task::JoinHandle<()>,
}

impl PeerLink {
  /// Reach the host's engine and say hello. `home`: another data root on the host (tests); the host's own otherwise
  pub async fn connect(client: &RemoteClient, home: Option<&str>, me: PeerHello<'_>) -> Result<PeerLink> {
    let mut args = json!({});
    if let Some(h) = home {
      args["home"] = json!(h);
    }
    let mut call = client.request("engine", args);
    match tokio::time::timeout(HELLO_WAIT, call.next()).await {
      Ok(Some(Event::Chunk { stream, .. })) if stream == "open" => {}
      Ok(Some(Event::Done(Err(e)))) => bail!("the host's engine cannot be reached: {e}"),
      Ok(_) => bail!("the host closed the connection to its engine"),
      Err(_) => bail!("the host's engine did not come up within {}s", HELLO_WAIT.as_secs()),
    }
    let (out, mut lines) = mpsc::unbounded_channel::<String>();
    let (to_inbox, mut inbox) = mpsc::unbounded_channel::<Value>();
    let task = tokio::spawn(async move {
      let mut pending: Vec<u8> = vec![];
      loop {
        tokio::select! {
          line = lines.recv() => match line {
            Some(mut l) => {
              l.push('\n');
              call.stdin(l.as_bytes());
            }
            None => break,
          },
          ev = call.next() => match ev {
            Some(Event::Chunk { bytes, .. }) => {
              pending.extend_from_slice(&bytes);
              while let Some(at) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=at).collect();
                if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                  let _ = to_inbox.send(v);
                }
              }
            }
            _ => break,
          },
        }
      }
    });
    let hello = json!({
      "type": "hello",
      "protocolVersion": SIDECAR_PROTOCOL_VERSION,
      "requestId": "peer-hello",
      "client": { "name": "acpira-peer", "version": me.version, "capabilities": me.capabilities, "role": "peer" },
      // The host's own home: a peer has no workspace there, and its sessions follow their folders
      "env": { "cwd": client.hello().home, "hostLanguage": me.host_language, "projects": true },
      "settings": {},
    });
    let _ = out.send(hello.to_string());
    let answer = tokio::time::timeout(HELLO_WAIT, async {
      while let Some(v) = inbox.recv().await {
        match v.get("type").and_then(Value::as_str) {
          Some("helloOk") => return Ok(v),
          Some("helloReject") => return Err(anyhow!("the host's engine refused: {}", v.get("reason").and_then(Value::as_str).unwrap_or(""))),
          _ => {}
        }
      }
      Err(anyhow!("the host's engine closed the connection before it answered"))
    })
    .await
    .map_err(|_| anyhow!("the host's engine did not answer hello within {}s", HELLO_WAIT.as_secs()))??;
    Ok(PeerLink { out, inbox, hello_ok: answer, task })
  }

  /// One envelope to the host's engine (`ShellMsg`)
  pub fn send(&self, msg: &Value) {
    let _ = self.out.send(msg.to_string());
  }

  /// The next envelope from the host's engine; None once the connection is gone
  pub async fn recv(&mut self) -> Option<Value> {
    self.inbox.recv().await
  }
}

impl Drop for PeerLink {
  fn drop(&mut self) {
    self.task.abort();
  }
}
