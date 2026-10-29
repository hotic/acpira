//! One device's envelope stream, bridged to the engine as a `peer` wire. The device speaks the sidecar protocol like any
//! shell; the gateway writes its hello (role peer, `toast` as its only capability, no settings), passes on only the
//! webview messages a device may send (`devices::device_allowed`), answers every platform request but a toast itself,
//! and never lets a slow reader grow its queue without bound.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use acpira_net::frame::FrameKind;
use acpira_net::link::{self, Control, EnvelopeRx, EnvelopeTx};
use acpira_net::pair::DeviceKind;
use acpira_shared::devices::{PushTarget, device_allowed};
use acpira_shared::protocol::WebviewMsg;
use acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::engine::{EngineSpec, Log, connect};

/// What a device may leave unread before the gateway drops it: it reconnects and gets whole views again, where a queue
/// with no bound would grow with every patch of every running session
const MAX_BEHIND: usize = 32 << 20;

#[derive(Clone)]
pub struct Who {
  /// The device, as the engine's log names the wire
  pub device: String,
  pub kind: DeviceKind,
  /// The computer, as the device shows it
  pub computer: String,
  pub version: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
  /// The device closed its stream or went away
  Device,
  Engine(String),
  Behind,
  /// The device sent something else before its hello
  NoHello,
}

/// Where a device's notifications go, or none to stop them (`Control::Push`, read before and after its hello)
pub type OnPush = Arc<dyn Fn(Option<PushTarget>) + Send + Sync>;

/// Runs until either side ends. `sessions_dir` receives the engine's (from `helloOk`) for the blob streams
pub async fn run(
  spec: &EngineSpec,
  log: &Log,
  who: &Who,
  mut tx: EnvelopeTx,
  mut rx: EnvelopeRx,
  sessions_dir: Arc<parking_lot::Mutex<Option<PathBuf>>>,
  on_push: OnPush,
) -> Ended {
  let welcome = Control::Welcome { name: who.computer.clone(), version: who.version.clone(), protocol: SIDECAR_PROTOCOL_VERSION };
  if tx.send(FrameKind::Control, &link::control(&welcome)).await.is_err() {
    return Ended::Device;
  }
  // The device's hello, answering its pings meanwhile
  let hello = loop {
    match rx.next().await {
      Ok(Some((FrameKind::Control, body))) => match serde_json::from_slice(&body) {
        Ok(Control::Ping { n }) => {
          if tx.send(FrameKind::Control, &link::control(&Control::Pong { n })).await.is_err() {
            return Ended::Device;
          }
        }
        Ok(Control::Push { target }) => on_push(target),
        _ => {}
      },
      Ok(Some((FrameKind::Envelope, body))) => match serde_json::from_slice::<Value>(&body) {
        Ok(v) if v["type"] == "hello" => break v,
        _ => return Ended::NoHello,
      },
      _ => return Ended::Device,
    }
  };
  let hello = peer_hello(&hello, who);

  let stream = match connect(spec, log).await {
    Ok(s) => s,
    Err(e) => return Ended::Engine(e.to_string()),
  };
  let (engine_read, mut engine_write) = tokio::io::split(stream);

  // To the engine: one writer, fed by the device loop and by the refusals of platform requests
  let (to_engine, mut engine_lines) = mpsc::unbounded_channel::<String>();
  let _ = to_engine.send(hello.to_string());
  let engine_writer = tokio::spawn(async move {
    while let Some(line) = engine_lines.recv().await {
      if engine_write.write_all(line.as_bytes()).await.is_err() || engine_write.write_all(b"\n").await.is_err() {
        break;
      }
    }
  });

  // To the device: one writer owning the stream's send half; `queued` counts what it has not written yet
  let queued = Arc::new(AtomicUsize::new(0));
  let (to_device, mut device_frames) = mpsc::unbounded_channel::<(FrameKind, Vec<u8>)>();
  let (behind_tx, behind) = oneshot::channel::<()>();
  let device_writer = {
    let queued = queued.clone();
    tokio::spawn(async move {
      while let Some((kind, body)) = device_frames.recv().await {
        let n = body.len();
        let ok = tx.send(kind, &body).await.is_ok();
        queued.fetch_sub(n, Ordering::Relaxed);
        if !ok {
          break;
        }
      }
    })
  };
  let push = {
    let queued = queued.clone();
    let to_device = to_device.clone();
    let behind_tx = parking_lot::Mutex::new(Some(behind_tx));
    move |kind: FrameKind, body: Vec<u8>| {
      if queued.fetch_add(body.len(), Ordering::Relaxed) + body.len() > MAX_BEHIND {
        if let Some(t) = behind_tx.lock().take() {
          let _ = t.send(());
        }
        return;
      }
      let _ = to_device.send((kind, body));
    }
  };
  let push = Arc::new(push);

  // From the engine
  let (engine_end_tx, engine_end) = oneshot::channel::<String>();
  let engine_reader = {
    let push = push.clone();
    let to_engine = to_engine.clone();
    tokio::spawn(async move {
      let mut lines = BufReader::new(engine_read).lines();
      let why = loop {
        match lines.next_line().await {
          Ok(Some(line)) => from_engine(line, &*push, &to_engine, &sessions_dir),
          Ok(None) => break "the engine closed the connection".to_owned(),
          Err(e) => break e.to_string(),
        }
      };
      let _ = engine_end_tx.send(why);
    })
  };

  // From the device
  let device_loop = async {
    loop {
      match rx.next().await {
        Ok(Some((FrameKind::Control, body))) => match serde_json::from_slice(&body) {
          Ok(Control::Ping { n }) => push(FrameKind::Control, link::control(&Control::Pong { n })),
          Ok(Control::Push { target }) => on_push(target),
          _ => {}
        },
        Ok(Some((FrameKind::Envelope, body))) => from_device(&body, &*push, &to_engine, log),
        _ => return,
      }
    }
  };

  let ended = tokio::select! {
    () = device_loop => Ended::Device,
    why = engine_end => Ended::Engine(why.unwrap_or_default()),
    _ = behind => Ended::Behind,
  };
  engine_reader.abort();
  engine_writer.abort();
  drop(push);
  drop(to_device);
  // Let what is queued reach a device that is still there (an engine error it should read), within reason
  let _ = tokio::time::timeout(std::time::Duration::from_secs(2), device_writer).await;
  ended
}

/// The hello the engine gets: the device's protocol version, request id, language and version; everything that grants
/// something is the gateway's
fn peer_hello(device: &Value, who: &Who) -> Value {
  let kind = match who.kind {
    DeviceKind::Phone => "phone",
    DeviceKind::Tablet => "tablet",
    DeviceKind::Computer => "computer",
  };
  json!({
    "type": "hello",
    "protocolVersion": device["protocolVersion"],
    "requestId": device["requestId"],
    "client": {
      "name": format!("{} ({kind}, paired)", who.device),
      "version": device["client"]["version"].as_str().unwrap_or(""),
      "capabilities": ["toast"],
      "role": "peer",
    },
    "env": {
      "hostLanguage": device["env"]["hostLanguage"].as_str().unwrap_or("en"),
      "projects": true,
    },
    "settings": {},
  })
}

fn from_engine(line: String, push: &dyn Fn(FrameKind, Vec<u8>), to_engine: &mpsc::UnboundedSender<String>, sessions_dir: &parking_lot::Mutex<Option<PathBuf>>) {
  // Views and patches are nearly all of it, and can be megabytes: passed on without parsing
  if line.starts_with(r#"{"type":"hostMessage""#) {
    push(FrameKind::Envelope, line.into_bytes());
    return;
  }
  let Ok(v) = serde_json::from_str::<Value>(&line) else { return };
  match v["type"].as_str().unwrap_or("") {
    "helloOk" => {
      if let Some(dir) = v["sessionsDir"].as_str() {
        *sessions_dir.lock() = Some(PathBuf::from(dir));
      }
      push(FrameKind::Envelope, line.into_bytes());
    }
    "platformRequest" => {
      if v["request"]["method"] == "toast" {
        push(FrameKind::Envelope, line.into_bytes());
      } else if let Some(id) = v["requestId"].as_str() {
        let _ = to_engine.send(json!({ "type": "platformResponse", "requestId": id, "error": "not available on a paired device" }).to_string());
      }
    }
    // Answers to what a device never sends
    "previewOpened" | "shutdownOk" => {}
    _ => push(FrameKind::Envelope, line.into_bytes()),
  }
}

fn from_device(body: &[u8], push: &dyn Fn(FrameKind, Vec<u8>), to_engine: &mpsc::UnboundedSender<String>, log: &Log) {
  let Ok(v) = serde_json::from_slice::<Value>(body) else { return };
  let ty = v["type"].as_str().unwrap_or("");
  match ty {
    "attachView" | "detachView" | "platformResponse" => {
      // Re-serialized: one line per envelope, whatever whitespace the device sent
      let _ = to_engine.send(v.to_string());
    }
    "webviewMessage" => {
      let what = v["message"]["type"].as_str().unwrap_or("?").to_owned();
      match serde_json::from_value::<WebviewMsg>(v["message"].clone()) {
        Ok(m) if device_allowed(&m) => {
          let _ = to_engine.send(v.to_string());
        }
        _ => push(FrameKind::Control, link::control(&Control::Denied { what })),
      }
    }
    // A second hello, settings and focus events, preview panels, shutting the engine down: not a device's to send
    _ => log(&format!("dropped a device's {ty}")),
  }
}
