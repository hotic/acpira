//! An in-process ACP client for the agent: the agent runs on a `tokio::io::duplex` pair, the client records every
//! `session/update` and answers permission requests with a scripted choice
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
}

pub struct Client {
  pub updates: parking_lot::Mutex<Vec<Value>>,
  pub permissions: parking_lot::Mutex<Vec<Value>>,
  pub answer: parking_lot::Mutex<Answer>,
}

impl Inbound for Client {
  fn notification(&self, method: &str, params: Value) {
    if method == "session/update" {
      self.updates.lock().push(params["update"].clone());
    }
  }
  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let answer = *self.answer.lock();
    if method == "session/request_permission" {
      self.permissions.lock().push(params.clone());
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
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let (agent_r, client_w) = tokio::io::duplex(1 << 20);
    let (client_r, agent_w) = tokio::io::duplex(1 << 20);
    let agent_home = home.path().to_owned();
    tokio::spawn(async move { acpira_agent::run(agent_r, agent_w, agent_home, "0.0.0-test").await });
    let client = Arc::new(Client { updates: Default::default(), permissions: Default::default(), answer: parking_lot::Mutex::new(Answer::Allow) });
    let inbound: Arc<dyn Inbound> = client.clone();
    let conn = Connection::start(client_r, client_w, Arc::new(move || inbound.clone()), Arc::new(|l: &str| eprintln!("client: {l}")));
    conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": {} })).await.unwrap();
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
    std::fs::write(
      self.home().join("providers.json"),
      json!({ "version": 1, "providers": [{ "id": "mock", "name": "Mock", "baseUrl": base_url, "models": models }] }).to_string(),
    )
    .unwrap();
    std::fs::write(self.home().join("secrets.json"), json!({ "acpira.provider.mock": "sk-test" }).to_string()).unwrap();
  }

  pub async fn new_session(&self) -> Value {
    self.conn.request("session/new", json!({ "cwd": self.cwd.path(), "mcpServers": [] })).await.unwrap()
  }

  pub fn updates(&self) -> Vec<Value> {
    self.client.updates.lock().clone()
  }
}
