//! A stream that goes silent is given up after the idle limit and retried. Its own test binary, since the limit is
//! read once per process from `ACPIRA_AGENT_STREAM_IDLE_SECS`

mod support;

use serde_json::json;

use acpira_agent::mock::{self, MockModel, Reply};
use support::Harness;

#[tokio::test]
async fn a_silent_stream_is_dropped_after_the_idle_limit_and_retried() {
  // SAFETY: the only test in this binary, and no other thread reads the environment yet
  unsafe { std::env::set_var("ACPIRA_AGENT_STREAM_IDLE_SECS", "1") };
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  server.push(Reply::Hang);
  server.push(mock::text("after the stall"));
  let r = h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hi" }] })).await.unwrap();
  assert_eq!(r["stopReason"], "end_turn");
  assert_eq!(server.requests().len(), 2);
  let text: String = h.updates().iter().filter(|u| u["sessionUpdate"] == "agent_message_chunk").map(|u| u["content"]["text"].as_str().unwrap().to_owned()).collect();
  assert_eq!(text, "after the stall");
  let notice = h.updates().into_iter().find(|u| u["sessionUpdate"] == "session_info_update").expect("a retry notice");
  let failure = &notice["_meta"]["jetbrains"]["air"]["sessionFailure"];
  assert_eq!(failure["category"], "connection");
  assert!(failure["details"].as_str().unwrap().contains("no data from the provider for 1 s"), "{failure}");
}
