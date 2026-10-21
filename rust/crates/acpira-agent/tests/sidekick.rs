//! The sidekick select: offered to a client that runs a sidekick (`_meta["acpira/sidekick"]`, the engine), kept with
//! the session, and told to the model without touching its tool table once a message went out. Before the first
//! message a pick lists acpira's tools again (the engine's ask_agent names the sidekick first); later the next
//! message carries a `<sidekick>` note, unless acpira offered no ask_agent at all. The `acpira` server here is a
//! streamable HTTP stand-in whose ask_agent description the test changes, as the engine's hub would

mod support;

use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::Harness;

/// An `acpira` MCP server: `show_image` always, `ask_agent` while `ask` holds a description
struct FakeAcpira {
  url: String,
  ask: Arc<parking_lot::Mutex<Option<String>>>,
  lists: Arc<AtomicUsize>,
}

impl FakeAcpira {
  fn start(ask: Option<&str>) -> FakeAcpira {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let ask = Arc::new(parking_lot::Mutex::new(ask.map(str::to_owned)));
    let lists: Arc<AtomicUsize> = Default::default();
    let (description, listed) = (ask.clone(), lists.clone());
    std::thread::spawn(move || {
      for stream in listener.incoming() {
        let Ok(mut stream) = stream else { return };
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut len = 0usize;
        loop {
          let mut line = String::new();
          if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
          }
          if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
          }
        }
        let mut body = vec![0; len];
        if reader.read_exact(&mut body).is_err() {
          continue;
        }
        let Ok(msg) = serde_json::from_slice::<Value>(&body) else { continue };
        let id = msg["id"].clone();
        let result = match msg["method"].as_str().unwrap_or("") {
          "initialize" => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": { "listChanged": false } }, "serverInfo": { "name": "acpira", "version": "1" } }),
          "tools/list" => {
            listed.fetch_add(1, Ordering::SeqCst);
            let mut tools = vec![json!({ "name": "show_image", "description": "Show an image", "inputSchema": { "type": "object", "properties": {} } })];
            if let Some(d) = description.lock().clone() {
              tools.push(json!({ "name": "ask_agent", "description": d, "inputSchema": { "type": "object", "properties": { "agent": { "type": "string" } } }, "annotations": { "readOnlyHint": true } }));
            }
            json!({ "tools": tools })
          }
          _ => Value::Null,
        };
        let text = if id.is_null() { String::new() } else { json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string() };
        let status = if id.is_null() { "202 Accepted" } else { "200 OK" };
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nMcp-Session-Id: s1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
      }
    });
    FakeAcpira { url, ask, lists }
  }

  fn servers(&self) -> Value {
    json!([{ "type": "http", "name": "acpira", "url": self.url, "headers": [] }])
  }

  fn lists(&self) -> usize {
    self.lists.load(Ordering::SeqCst)
  }

  /// The listing the session started with has been answered
  async fn listed_once(&self) {
    for _ in 0..500 {
      if self.lists() >= 1 {
        return;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("acpira was never listed");
  }
}

/// The engine's client capabilities, as far as this agent reads them
fn engine_caps() -> Value {
  json!({ "elicitation": { "form": {} }, "_meta": { "acpira/sidekick": true } })
}

async fn engine() -> Harness {
  Harness::start_with(tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), engine_caps()).await
}

fn models(h: &Harness, model: &MockModel) {
  h.providers(&model.base_url(), json!([{ "id": "m1", "name": "Model One" }, { "id": "m2", "name": "Model Two" }]));
}

async fn session(h: &Harness, servers: Value) -> (String, Value) {
  let r = h.conn.request("session/new", json!({ "cwd": h.cwd(), "mcpServers": servers })).await.unwrap();
  (r["sessionId"].as_str().unwrap().to_owned(), r["configOptions"].clone())
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

async fn pick(h: &Harness, sid: &str, value: &str) -> Result<Value, String> {
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "sidekick", "value": value })).await.map_err(|e| e.message)
}

fn sidekick(options: &Value) -> Option<Value> {
  options.as_array().unwrap().iter().find(|o| o["id"] == "sidekick").cloned()
}

/// The tools of a request, with their descriptions
fn tools(body: &Value) -> Vec<(String, String)> {
  body["tools"].as_array().unwrap().iter().map(|t| (t["function"]["name"].as_str().unwrap().to_owned(), t["function"]["description"].as_str().unwrap_or("").to_owned())).collect()
}

/// The text of a request's last user message
fn last_user(body: &Value) -> String {
  let msgs = body["messages"].as_array().unwrap();
  let m = msgs.iter().rev().find(|m| m["role"] == "user").unwrap();
  m["content"].as_str().map(str::to_owned).unwrap_or_else(|| m["content"][0]["text"].as_str().unwrap_or("").to_owned())
}

#[tokio::test]
async fn the_select_is_offered_only_to_a_client_that_runs_a_sidekick() {
  let model = MockModel::start();
  let plain = Harness::start().await;
  models(&plain, &model);
  let (_, options) = session(&plain, json!([])).await;
  assert!(sidekick(&options).is_none(), "{options}");

  let h = engine().await;
  models(&h, &model);
  let (sid, options) = session(&h, json!([])).await;
  let select = sidekick(&options).unwrap();
  // A custom category: no client takes it for a model or a model parameter
  assert_eq!((&select["category"], &select["type"], &select["currentValue"]), (&json!("_sidekick"), &json!("select"), &json!("none")));
  let values: Vec<&str> = select["options"].as_array().unwrap().iter().map(|o| o["value"].as_str().unwrap()).collect();
  assert_eq!(values, ["none", "mock/m1", "mock/m2"]);
  assert_eq!(select["options"][2]["name"], "Model Two");

  let picked = pick(&h, &sid, "mock/m2").await.unwrap();
  assert_eq!(sidekick(&picked["configOptions"]).unwrap()["currentValue"], "mock/m2");
  assert!(pick(&h, &sid, "mock/nope").await.unwrap_err().contains("Unknown model"));
  let cleared = pick(&h, &sid, "none").await.unwrap();
  assert_eq!(sidekick(&cleared["configOptions"]).unwrap()["currentValue"], "none");
}

#[tokio::test]
async fn a_pick_before_the_first_message_lists_acpira_again_and_needs_no_note() {
  let h = engine().await;
  let model = MockModel::start();
  models(&h, &model);
  let acpira = FakeAcpira::start(Some("Agents:\n- reviewer"));
  let (sid, _) = session(&h, acpira.servers()).await;
  acpira.listed_once().await;
  // What the engine's hub answers once the session has a sidekick
  *acpira.ask.lock() = Some("Agents:\n- sidekick (@Sidekick)\n- reviewer".into());
  pick(&h, &sid, "mock/m2").await.unwrap();
  model.push(mock::text("ok"));
  prompt(&h, &sid, "start").await;

  assert_eq!(acpira.lists(), 2, "listed again for the first message");
  let body = &model.requests()[0].body;
  let ask = tools(body).into_iter().find(|(n, _)| n == "acpira_ask_agent").unwrap();
  assert!(ask.1.contains("- sidekick (@Sidekick)"), "{}", ask.1);
  assert!(!last_user(body).contains("<sidekick>"), "the tool list says it already");
}

#[tokio::test]
async fn a_later_pick_keeps_the_tool_table_and_is_told_with_the_next_message() {
  let h = engine().await;
  let model = MockModel::start();
  models(&h, &model);
  let acpira = FakeAcpira::start(Some("Agents:\n- reviewer"));
  let (sid, _) = session(&h, acpira.servers()).await;
  model.push(mock::text("one"));
  prompt(&h, &sid, "first").await;
  let table = tools(&model.requests()[0].body);

  pick(&h, &sid, "mock/m2").await.unwrap();
  model.push(mock::text("two"));
  prompt(&h, &sid, "second").await;
  let body = model.requests()[1].body.clone();
  assert_eq!(tools(&body), table, "the table stays as listed, so the prompt cache holds");
  assert_eq!(acpira.lists(), 1);
  let text = last_user(&body);
  assert!(text.starts_with("<sidekick>\nYou now have a sidekick, Model Two: summon it with acpira_ask_agent, agent \"sidekick\"."), "{text}");
  assert!(text.ends_with("</sidekick>\n\nsecond"), "{text}");

  // Told once
  model.push(mock::text("three"));
  prompt(&h, &sid, "third").await;
  assert_eq!(last_user(&model.requests()[2].body), "third");

  // Another model, then none
  pick(&h, &sid, "mock/m1").await.unwrap();
  model.push(mock::text("four"));
  prompt(&h, &sid, "fourth").await;
  assert!(last_user(&model.requests()[3].body).contains("Your sidekick now runs Model One"));
  pick(&h, &sid, "none").await.unwrap();
  model.push(mock::text("five"));
  prompt(&h, &sid, "fifth").await;
  let body = model.requests()[4].body.clone();
  assert!(last_user(&body).contains("You no longer have a sidekick"));
  assert_eq!(tools(&body), table);

  // Picked and dropped again between two messages: nothing to tell
  pick(&h, &sid, "mock/m2").await.unwrap();
  pick(&h, &sid, "none").await.unwrap();
  model.push(mock::text("six"));
  prompt(&h, &sid, "sixth").await;
  assert_eq!(last_user(&model.requests()[5].body), "sixth");
}

#[tokio::test]
async fn without_ask_agent_a_later_pick_lists_acpira_once_more() {
  let h = engine().await;
  let model = MockModel::start();
  models(&h, &model);
  // No persona when the session started: acpira had no ask_agent to offer
  let acpira = FakeAcpira::start(None);
  let (sid, _) = session(&h, acpira.servers()).await;
  model.push(mock::text("one"));
  prompt(&h, &sid, "first").await;
  assert!(!tools(&model.requests()[0].body).iter().any(|(n, _)| n == "acpira_ask_agent"));

  *acpira.ask.lock() = Some("Agents:\n- sidekick (@Sidekick)".into());
  pick(&h, &sid, "mock/m2").await.unwrap();
  model.push(mock::text("two"));
  prompt(&h, &sid, "second").await;
  let body = model.requests()[1].body.clone();
  assert_eq!(acpira.lists(), 2);
  assert!(tools(&body).iter().any(|(n, d)| n == "acpira_ask_agent" && d.contains("sidekick")));
  assert_eq!(last_user(&body), "second", "the new tool names the sidekick itself");
}

#[tokio::test]
async fn the_pick_comes_back_with_the_session() {
  let h = engine().await;
  let model = MockModel::start();
  models(&h, &model);
  let acpira = FakeAcpira::start(Some("Agents:\n- sidekick (@Sidekick)"));
  let (sid, _) = session(&h, acpira.servers()).await;
  model.push(mock::text("one"));
  prompt(&h, &sid, "first").await;
  pick(&h, &sid, "mock/m2").await.unwrap();

  let Harness { conn, home, cwd, .. } = h;
  drop(conn);
  let h = Harness::start_with(home, cwd, engine_caps()).await;
  let loaded = h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": acpira.servers() })).await.unwrap();
  assert_eq!(sidekick(&loaded["configOptions"]).unwrap()["currentValue"], "mock/m2");
  // Reopened, acpira lists its tools with the sidekick the engine holds for the session: no note
  model.push(mock::text("two"));
  prompt(&h, &sid, "second").await;
  assert_eq!(last_user(&model.requests()[1].body), "second");
}

#[tokio::test]
async fn the_sidekick_effort_follows_the_sidekicks_model_and_comes_back_with_the_session() {
  let h = engine().await;
  let model = MockModel::start();
  // m2 has levels, m3 has others, m1 none
  h.providers(&model.base_url(), json!([
    { "id": "m1", "name": "Model One" },
    { "id": "m2", "name": "Model Two", "efforts": ["low", "medium", "high"] },
    { "id": "m3", "name": "Model Three", "efforts": ["medium", "max"] },
  ]));
  let effort = |options: &Value| options.as_array().unwrap().iter().find(|o| o["id"] == "sidekick_effort").cloned();
  let set = |sid: String, value: &'static str| {
    let conn = &h.conn;
    async move { conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "sidekick_effort", "value": value })).await.map_err(|e| e.message) }
  };
  let (sid, options) = session(&h, json!([])).await;
  assert!(effort(&options).is_none(), "no sidekick, no effort: {options}");
  assert!(set(sid.clone(), "low").await.unwrap_err().contains("Unknown sidekick effort"));

  let picked = pick(&h, &sid, "mock/m2").await.unwrap();
  let select = effort(&picked["configOptions"]).unwrap();
  // Its own custom category, so the composer does not take it for the lead's effort; the model's default to begin with
  assert_eq!((&select["category"], &select["currentValue"]), (&json!("_sidekick_effort"), &json!("high")));
  let values: Vec<&str> = select["options"].as_array().unwrap().iter().map(|o| o["value"].as_str().unwrap()).collect();
  assert_eq!(values, ["low", "medium", "high"]);
  assert_eq!(effort(&set(sid.clone(), "medium").await.unwrap()["configOptions"]).unwrap()["currentValue"], "medium");
  assert!(set(sid.clone(), "max").await.unwrap_err().contains("Unknown sidekick effort"), "m2 has no max");

  // Medium carries over to a sidekick that offers it; a model without levels drops the select
  assert_eq!(effort(&pick(&h, &sid, "mock/m3").await.unwrap()["configOptions"]).unwrap()["currentValue"], "medium");
  assert!(effort(&pick(&h, &sid, "mock/m1").await.unwrap()["configOptions"]).is_none());
  pick(&h, &sid, "mock/m3").await.unwrap();
  set(sid.clone(), "max").await.unwrap();
  // A level the next sidekick lacks falls back to its default
  assert_eq!(effort(&pick(&h, &sid, "mock/m2").await.unwrap()["configOptions"]).unwrap()["currentValue"], "high");
  set(sid.clone(), "low").await.unwrap();
  model.push(mock::text("one"));
  prompt(&h, &sid, "first").await;

  let Harness { conn, home, cwd, .. } = h;
  drop(conn);
  let h = Harness::start_with(home, cwd, engine_caps()).await;
  let loaded = h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  assert_eq!(effort(&loaded["configOptions"]).unwrap()["currentValue"], "low");
}
