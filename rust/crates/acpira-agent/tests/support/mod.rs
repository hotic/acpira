//! An in-process ACP client for the agent: the agent runs on a `tokio::io::duplex` pair, the client records every
//! `session/update` and answers permission requests with a scripted choice, and `elicitation/create` forms with a
//! scripted response
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Connection, Inbound, RpcError};

/// How the client answers `session/request_permission`
#[derive(Clone, Copy, PartialEq)]
pub enum Answer {
  Allow,
  /// The allow_always option
  Always,
  Reject,
  /// Never answer (the turn stays parked on the card)
  Hold,
  /// Answer once the test calls `Harness::release`: allow once, or reject
  Wait { allow: bool },
}

pub struct Client {
  pub updates: parking_lot::Mutex<Vec<Value>>,
  pub permissions: parking_lot::Mutex<Vec<Value>>,
  pub answer: parking_lot::Mutex<Answer>,
  pub forms: parking_lot::Mutex<Vec<Value>>,
  /// The response to every form
  pub form_reply: parking_lot::Mutex<Value>,
  /// Lets an `Answer::Wait` card be answered
  pub gate: Arc<tokio::sync::Notify>,
}

impl Inbound for Client {
  fn notification(&self, method: &str, params: Value) {
    if method == "session/update" {
      self.updates.lock().push(params["update"].clone());
    }
  }
  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let answer = *self.answer.lock();
    let gate = self.gate.clone();
    if method == "session/request_permission" {
      self.permissions.lock().push(params.clone());
    }
    if method == "elicitation/create" {
      self.forms.lock().push(params);
      let reply = self.form_reply.lock().clone();
      return Box::pin(async move { Ok(reply) });
    }
    Box::pin(async move {
      if method != "session/request_permission" {
        return Err(RpcError::method_not_found(&method));
      }
      let kind = match answer {
        Answer::Allow => "allow_once",
        Answer::Always => "allow_always",
        Answer::Reject => "reject_once",
        Answer::Hold => {
          cancel.cancelled().await;
          return Ok(json!({ "outcome": { "outcome": "cancelled" } }));
        }
        Answer::Wait { allow } => {
          tokio::select! {
            _ = gate.notified() => {}
            _ = cancel.cancelled() => return Ok(json!({ "outcome": { "outcome": "cancelled" } })),
          }
          if allow { "allow_once" } else { "reject_once" }
        }
      };
      let option = params["options"].as_array().into_iter().flatten().find(|o| o["kind"] == kind).cloned().unwrap_or(Value::Null);
      Ok(json!({ "outcome": { "outcome": "selected", "optionId": option["optionId"] } }))
    })
  }
}

pub struct Harness {
  pub conn: Connection,
  pub client: Arc<Client>,
  pub home: tempfile::TempDir,
  pub cwd: tempfile::TempDir,
}

impl Harness {
  pub async fn start() -> Harness {
    Harness::start_in(tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()).await
  }

  /// A fresh agent process on an existing data root and project (a reopened window)
  pub async fn start_in(home: tempfile::TempDir, cwd: tempfile::TempDir) -> Harness {
    Harness::start_with(home, cwd, json!({ "elicitation": { "form": {} } })).await
  }

  /// A client declaring these capabilities
  pub async fn start_with(home: tempfile::TempDir, cwd: tempfile::TempDir, caps: Value) -> Harness {
    let (agent_r, client_w) = tokio::io::duplex(1 << 20);
    let (client_r, agent_w) = tokio::io::duplex(1 << 20);
    let agent_home = home.path().to_owned();
    tokio::spawn(async move { acpira_agent::run(agent_r, agent_w, agent_home, "0.0.0-test").await });
    let client = Arc::new(Client {
      updates: Default::default(),
      permissions: Default::default(),
      answer: parking_lot::Mutex::new(Answer::Allow),
      forms: Default::default(),
      form_reply: parking_lot::Mutex::new(json!({ "action": "cancel" })),
      gate: Default::default(),
    });
    let inbound: Arc<dyn Inbound> = client.clone();
    let conn = Connection::start(client_r, client_w, Arc::new(move || inbound.clone()), Arc::new(|l: &str| eprintln!("client: {l}")));
    conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": caps })).await.unwrap();
    Harness { conn, client, home, cwd }
  }

  pub fn home(&self) -> &Path {
    self.home.path()
  }

  pub fn cwd(&self) -> PathBuf {
    self.cwd.path().to_owned()
  }

  /// One OpenAI-compatible source with the given models at `base_url`, and its key
  pub fn providers(&self, base_url: &str, models: Value) {
    self.providers_in("openai-chat", base_url, models);
  }

  /// One source of this API format
  pub fn providers_in(&self, format: &str, base_url: &str, models: Value) {
    std::fs::write(
      self.home().join("providers.json"),
      json!({ "version": 1, "providers": [{ "id": "mock", "name": "Mock", "format": format, "baseUrl": base_url, "models": models }] }).to_string(),
    )
    .unwrap();
    std::fs::write(self.home().join("secrets.json"), json!({ "acpira.provider.mock": "sk-test" }).to_string()).unwrap();
  }

  /// The approval level as the settings page sets it: a field of providers.json, read at the start of every turn
  pub fn approval(&self, level: &str) {
    let path = self.home().join("providers.json");
    let mut file: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({ "version": 1 }));
    file["approval"] = json!(level);
    std::fs::write(&path, file.to_string()).unwrap();
  }

  pub async fn new_session(&self) -> Value {
    self.conn.request("session/new", json!({ "cwd": self.cwd.path(), "mcpServers": [] })).await.unwrap()
  }

  /// Answer the permission card an `Answer::Wait` holds
  pub fn release(&self) {
    self.client.gate.notify_one();
  }

  pub fn updates(&self) -> Vec<Value> {
    self.client.updates.lock().clone()
  }
}
