//! The `question` tool against the scripted model: the questions go out as an `elicitation/create` form tied to the
//! tool call, the answers come back to the model as lines, closing the card ends the turn, and a client without form
//! support gets the model to ask in plain text

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::Harness;

async fn setup(h: Harness) -> (Harness, MockModel, String) {
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

/// The last message of the request: the tool result the call before it produced
fn last_message(server: &MockModel, request: usize) -> String {
  let msgs = server.requests()[request].body["messages"].as_array().unwrap().clone();
  msgs.last().unwrap()["content"].as_str().unwrap().to_owned()
}

fn ask() -> mock::Reply {
  mock::tools(&[("c1", "question", json!({ "questions": [
    { "question": "Which file name?", "header": "Name", "options": [{ "label": "report", "description": "A generic report" }, { "label": "notes" }] },
    { "question": "Where should it go?", "options": ["src", "docs"], "multiple": true },
  ] }))])
}

#[tokio::test]
async fn answers_reach_the_model_and_the_turn_goes_on() {
  let (h, server, sid) = setup(Harness::start().await).await;
  *h.client.form_reply.lock() = json!({ "action": "accept", "content": { "q0": "my-own-name", "q1": ["src", "docs"] } });
  server.push(ask());
  server.push(mock::text("Done."));
  let r = prompt(&h, &sid, "make a file").await;
  assert_eq!(r["stopReason"], "end_turn");

  let forms = h.client.forms.lock().clone();
  assert_eq!(forms.len(), 1);
  // Tied to the tool call the host already shows, so the card reads the model's own labels from its rawInput
  let call = h.updates().into_iter().find(|u| u["sessionUpdate"] == "tool_call").unwrap();
  assert_eq!(forms[0]["toolCallId"], call["toolCallId"]);
  assert_eq!(call["title"], "Ask questions");
  assert_eq!(forms[0]["mode"], "form");
  let props = &forms[0]["requestedSchema"]["properties"];
  assert_eq!(props["q0"]["oneOf"][0], json!({ "const": "report", "title": "report", "description": "A generic report" }));
  assert_eq!(props["q1"]["type"], "array");
  let titled = h.updates().into_iter().any(|u| u["toolCallId"] == call["toolCallId"] && u["title"] == "Ask 2 questions");
  assert!(titled, "the call is titled by its question count");

  assert_eq!(server.requests().len(), 2);
  assert_eq!(last_message(&server, 1), "The user answered:\n\nQ: Which file name?\nA: my-own-name\n\nQ: Where should it go?\nA: src, docs");
  let done = h.updates().into_iter().rev().find(|u| u["toolCallId"] == call["toolCallId"] && u["status"].is_string()).unwrap();
  assert_eq!(done["status"], "completed");
}

#[tokio::test]
async fn a_skip_lets_the_model_go_on_and_closing_the_card_ends_the_turn() {
  let (h, server, sid) = setup(Harness::start().await).await;
  *h.client.form_reply.lock() = json!({ "action": "decline" });
  server.push(ask());
  server.push(mock::text("Going with report."));
  prompt(&h, &sid, "make a file").await;
  assert!(last_message(&server, 1).starts_with("The user skipped the questions"), "{}", last_message(&server, 1));

  *h.client.form_reply.lock() = json!({ "action": "cancel" });
  server.push(ask());
  let r = prompt(&h, &sid, "again").await;
  assert_eq!(r["stopReason"], "end_turn");
  // No model call after the closed card
  assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn without_form_support_the_model_is_told_to_ask_in_plain_text() {
  let h = Harness::start_with(tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), json!({})).await;
  let (h, server, sid) = setup(h).await;
  server.push(ask());
  server.push(mock::text("Which file name?"));
  prompt(&h, &sid, "make a file").await;
  assert!(h.client.forms.lock().is_empty());
  assert!(last_message(&server, 1).contains("Ask in plain text"), "{}", last_message(&server, 1));
}
