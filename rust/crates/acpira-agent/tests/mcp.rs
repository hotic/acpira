//! MCP servers handed over in `session/new`: their tools reach the model behind the built-in ones as `<server>_<tool>`,
//! a read-only tool runs without a card and one that changes things asks, a result's images go back to a model that
//! takes images, the server's instructions and an unreachable server are told in the first user message. The server
//! here is a streamable HTTP one on a local port
#![allow(clippy::unwrap_used, reason = "test code")]

mod support;

use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::{Answer, Harness};

/// A streamable HTTP MCP server with two tools: `shot` (read-only, returns text and an image) and `remove`
struct FakeMcp {
  url: String,
  /// Every JSON-RPC message received
  seen: Arc<parking_lot::Mutex<Vec<Value>>>,
}

impl FakeMcp {
  fn start() -> FakeMcp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let seen: Arc<parking_lot::Mutex<Vec<Value>>> = Default::default();
    let log = seen.clone();
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
        log.lock().push(msg.clone());
        let id = msg["id"].clone();
        let result = match msg["method"].as_str().unwrap_or("") {
          "initialize" => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "bench", "version": "1" },
            "instructions": "Take a shot after every change." }),
          "tools/list" => json!({ "tools": [
            { "name": "shot", "title": "Take Screenshot", "description": "Take a screenshot of the model", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } },
            { "name": "remove", "description": "Remove a cube", "inputSchema": { "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] } },
          ] }),
          "tools/call" if msg["params"]["name"] == "shot" => json!({ "content": [
            { "type": "text", "text": "shot taken" },
            { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
          ] }),
          "tools/call" => json!({ "content": [{ "type": "text", "text": format!("removed {}", msg["params"]["arguments"]["id"].as_str().unwrap_or("?")) }] }),
          _ => Value::Null,
        };
        let text = if id.is_null() { String::new() } else { json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string() };
        let status = if id.is_null() { "202 Accepted" } else { "200 OK" };
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nMcp-Session-Id: s1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
      }
    });
    FakeMcp { url, seen }
  }

  fn calls(&self) -> Vec<Value> {
    self.seen.lock().iter().filter(|m| m["method"] == "tools/call").map(|m| m["params"].clone()).collect()
  }
}

async fn session(h: &Harness, servers: Value) -> String {
  let r = h.conn.request("session/new", json!({ "cwd": h.cwd(), "mcpServers": servers })).await.unwrap();
  r["sessionId"].as_str().unwrap().to_owned()
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

fn tool_names(body: &Value) -> Vec<String> {
  body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn a_read_only_tool_runs_without_a_card_and_its_image_reaches_the_model() {
  let h = Harness::start().await;
  let model = MockModel::start();
  h.providers(&model.base_url(), json!([{ "id": "m1", "input": ["text", "image"] }]));
  let mcp = FakeMcp::start();
  let sid = session(&h, json!([{ "type": "http", "name": "bench", "url": mcp.url, "headers": [] }])).await;
  model.push(mock::tools(&[("c1", "bench_shot", json!({}))]));
  model.push(mock::text("Looks right."));
  assert_eq!(prompt(&h, &sid, "check the model").await["stopReason"], "end_turn");

  let requests = model.requests();
  let names = tool_names(&requests[0].body);
  // Behind the built-in tools, in the server's order
  assert_eq!(names[names.len() - 2..], ["bench_shot", "bench_remove"]);
  assert_eq!(names[0], "read");
  // The server's instructions come with the first message
  let first = requests[0].body["messages"][1]["content"].as_str().unwrap().to_owned();
  assert!(first.starts_with("<mcp>") && first.contains("## bench\nTake a shot after every change.") && first.ends_with("check the model"), "{first}");

  assert!(h.client.permissions.lock().is_empty(), "a read-only tool asks nothing");
  let calls = mcp.calls();
  assert_eq!((calls.len(), &calls[0]["name"], &calls[0]["arguments"]), (1, &json!("shot"), &json!({})));
  // A progress token rides along, so a long call's steps can show on the card
  assert!(calls[0]["_meta"]["progressToken"].is_string());
  // The text is the tool result; the image follows it in a user message (Chat takes text only in a tool message)
  let msgs = requests[1].body["messages"].as_array().unwrap().clone();
  let tool = msgs.iter().position(|m| m["role"] == "tool").unwrap();
  assert_eq!(msgs[tool]["content"], "shot taken");
  let images = &msgs[tool + 1];
  assert_eq!(images["role"], "user");
  assert_eq!(images["content"][0]["text"], "[An image returned by the tool call c1]");
  assert_eq!(images["content"][1]["image_url"]["url"], "data:image/png;base64,iVBORw0KGgo=");

  // The card is titled by the tool's name and shows the image
  let ups = h.updates();
  let call = ups.iter().find(|u| u["sessionUpdate"] == "tool_call").unwrap();
  assert_eq!(call["title"], "bench_shot");
  // The engine reads the server and the tool's own title off the card, to show the row as "Call Bench" with "Take Screenshot"
  assert_eq!(call["_meta"]["acpira/mcp"], json!({ "server": "bench", "tool": "shot", "title": "Take Screenshot" }));
  let done = ups.iter().rev().find(|u| u["toolCallId"] == call["toolCallId"] && u["status"] == "completed").unwrap();
  assert_eq!(done["content"][1], json!({ "type": "content", "content": { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" } }));

  // Told once: the next prompt has no note, and the request prefix is unchanged
  model.push(mock::text("ok"));
  prompt(&h, &sid, "thanks").await;
  let third = &model.requests()[2].body;
  assert_eq!(tool_names(third), names);
  let last = third["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
  assert_eq!(last, "thanks");
}

#[tokio::test]
async fn a_tool_that_changes_things_asks_and_a_text_only_model_is_told_about_images() {
  let h = Harness::start().await;
  let model = MockModel::start();
  h.providers(&model.base_url(), json!([{ "id": "m1" }]));
  let mcp = FakeMcp::start();
  let sid = session(&h, json!([{ "type": "http", "name": "bench", "url": mcp.url, "headers": [] }])).await;
  *h.client.answer.lock() = Answer::Always;
  model.push(mock::tools(&[("c1", "bench_remove", json!({ "id": "cube1" })), ("c2", "mcp__bench__shot", json!({}))]));
  model.push(mock::tools(&[("c3", "bench_remove", json!({ "id": "cube2" }))]));
  model.push(mock::text("Removed."));
  prompt(&h, &sid, "remove two cubes").await;

  // One card: "always allow" covered the second removal
  let cards = h.client.permissions.lock().clone();
  assert_eq!(cards.len(), 1);
  assert_eq!(cards[0]["toolCall"]["title"], "bench_remove");
  assert_eq!(cards[0]["toolCall"]["_meta"]["acpira/mcp"], json!({ "server": "bench", "tool": "remove", "title": null }));
  assert_eq!(cards[0]["toolCall"]["rawInput"], json!({ "id": "cube1" }));
  assert!(cards[0]["options"].as_array().unwrap().iter().any(|o| o["name"] == "Always allow bench_remove in this session"));
  assert_eq!(mcp.calls().iter().filter(|c| c["name"] == "remove").map(|c| c["arguments"]["id"].clone()).collect::<Vec<_>>(), [json!("cube1"), json!("cube2")]);

  // Another harness's spelling is taken as the tool, and a model without image input hears that it missed one
  let msgs = model.requests()[1].body["messages"].as_array().unwrap().clone();
  let shot = msgs.iter().find(|m| m["role"] == "tool" && m["tool_call_id"] == "c2").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(shot.contains("taken as the bench_shot tool") && shot.contains("shot taken") && shot.contains("does not accept images"), "{shot}");
  assert!(!msgs.iter().any(|m| m["role"] == "user" && m["content"].is_array()), "no image message for a text-only model");
}

#[tokio::test]
async fn an_unreachable_server_is_named_to_the_model() {
  let h = Harness::start().await;
  let model = MockModel::start();
  h.providers(&model.base_url(), json!([{ "id": "m1" }]));
  // A port nothing listens on
  let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
  let sid = session(&h, json!([
    { "type": "http", "name": "gone", "url": format!("http://{closed}/mcp"), "headers": [] },
    { "type": "sse", "name": "old", "url": "http://127.0.0.1:1/sse", "headers": [] },
  ]))
  .await;
  model.push(mock::text("Sorry."));
  prompt(&h, &sid, "use gone").await;
  let body = &model.requests()[0].body;
  assert!(!tool_names(body).iter().any(|n| n.starts_with("gone_")));
  let first = body["messages"][1]["content"].as_str().unwrap().to_owned();
  assert!(first.contains("configured but unavailable") && first.contains("- gone: initialize failed") && first.contains("- old (the sse transport is not supported)"), "{first}");
}

#[tokio::test]
async fn initialize_advertises_http_mcp() {
  let h = Harness::start().await;
  let r = h.conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": {} })).await.unwrap();
  assert_eq!(r["agentCapabilities"]["mcpCapabilities"], json!({ "http": true, "sse": false }));
}
