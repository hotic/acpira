//! Chunk compaction (`compaction: "chunk"` in providers.json) against the scripted model: `/compact` folds finished
//! turns without a model call, the history tools read the folded output back, a reopen renders the same view, a long
//! turn folds its earlier steps into a work-state index while it runs, and an overflow falls back to the summary

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel, Reply};
use support::Harness;

/// A session on a scripted model with chunk compaction picked on the settings page
async fn setup(chunked: bool) -> (Harness, MockModel, String) {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1", "context": 64000 }]));
  if chunked {
    let path = h.home().join("providers.json");
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["compaction"] = json!("chunk");
    std::fs::write(&path, file.to_string()).unwrap();
  }
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

async fn prompt(h: &Harness, sid: &str, text: &str, at: u64) -> Value {
  let params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }], "_meta": { "acpira/compaction": { "auto": true, "atTokens": at } } });
  h.conn.request("session/prompt", params).await.unwrap()
}

/// One tool call whose usage reports `input` prompt tokens
fn call(id: &str, name: &str, args: Value, input: u64) -> Reply {
  Reply::Sse(vec![
    mock::delta(json!({ "tool_calls": [{ "index": 0, "id": id, "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })),
    mock::finish("tool_calls"),
    mock::usage(input, 10, 0),
  ])
}

fn log(h: &Harness, sid: &str) -> Vec<Value> {
  let text = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn compactions(h: &Harness, sid: &str) -> Vec<Value> {
  log(h, sid).into_iter().filter(|e| e["type"] == "compaction").collect()
}

fn body(req: &mock::Recorded) -> String {
  req.body["messages"].to_string()
}

fn tool_names(req: &mock::Recorded) -> Vec<String> {
  req.body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect()
}

fn first_user(req: &mock::Recorded) -> String {
  req.body["messages"].as_array().unwrap().iter().find(|m| m["role"] == "user").and_then(|m| m["content"].as_str()).unwrap_or("").to_owned()
}

#[tokio::test]
async fn only_chunk_compaction_offers_the_history_tools() {
  let (h, server, sid) = setup(false).await;
  server.push(mock::text("ok"));
  prompt(&h, &sid, "hi", 0).await;
  assert!(!tool_names(&server.requests()[0]).iter().any(|t| t.starts_with("history_")));
  let (h2, server2, sid2) = setup(true).await;
  server2.push(mock::text("ok"));
  prompt(&h2, &sid2, "hi", 0).await;
  let names = tool_names(&server2.requests()[0]);
  // Right behind the goal tools, so the earlier specs keep their place (a session with skills has `skill` after them)
  let at = names.iter().position(|n| n == "update_goal").unwrap();
  assert_eq!(names[at + 1..at + 3], ["history_grep".to_owned(), "history_list".to_owned()], "{names:?}");
  drop((h, h2));
}

#[tokio::test]
async fn slash_compact_folds_finished_turns_and_the_history_tools_read_them_back() {
  let (h, server, sid) = setup(true).await;
  std::fs::write(h.cwd().join("notes.txt"), "SECRET-ALPHA is the code word\n").unwrap();
  server.push(call("c1", "read", json!({ "path": "notes.txt" }), 100)).push(mock::text("The note holds a code word."));
  prompt(&h, &sid, "read notes.txt", 0).await;
  server.push(mock::text("B done"));
  prompt(&h, &sid, "second task", 0).await;
  assert_eq!(server.requests().len(), 3);

  // /compact: no model call, the two turns become chunks
  let r = prompt(&h, &sid, "/compact", 0).await;
  assert_eq!(r["stopReason"], "end_turn");
  assert_eq!(server.requests().len(), 3, "chunking calls no model");
  let ups: Vec<Value> = h.updates().into_iter().filter(|u| u["sessionUpdate"] == "compaction_update").collect();
  assert_eq!(ups.iter().map(|u| u["status"].as_str().unwrap()).collect::<Vec<_>>(), ["in_progress", "completed"]);
  let ev = &compactions(&h, &sid)[0];
  assert_eq!((ev["strategy"].as_str(), ev["reason"].as_str(), ev["tail"].as_u64()), (Some("chunk"), Some("manual"), Some(0)));
  let chunks = ev["chunks"].as_array().unwrap();
  assert_eq!(chunks.iter().map(|c| (c["status"].as_str().unwrap(), c["from"].as_u64().unwrap(), c["to"].as_u64().unwrap())).collect::<Vec<_>>(), [("completed", 0, 4), ("completed", 4, 6)]);
  let first_id = chunks[0]["id"].as_str().unwrap().to_owned();

  // The next request: the checkpoint and the chunks, the users' words and the final replies, the read output folded
  server.push(call("c2", "history_grep", json!({ "pattern": "secret" }), 100)).push(mock::text("It was SECRET-ALPHA."));
  prompt(&h, &sid, "what was the code word?", 0).await;
  let reqs = server.requests();
  let next = &reqs[3];
  let head = first_user(next);
  assert!(head.starts_with("<conversation-checkpoint strategy=\"chunk\">") && head.contains("read notes.txt"), "{head}");
  let text = body(next);
  assert!(text.contains("The note holds a code word.") && text.contains("B done") && !text.contains("SECRET-ALPHA"), "{text}");
  assert_eq!(next.body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").count(), 0);
  // history_grep finds the folded output in the first chunk
  let result = reqs[4].body["messages"].as_array().unwrap().iter().rfind(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(result.contains(&format!("chunk {first_id} (sequence 1) TOOL_OUTPUT line")) && result.contains("SECRET-ALPHA"), "{result}");

  // A new process renders the same view, and history_list still reads the chunk
  let h = Harness::start_in(h.home, h.cwd).await;
  h.conn.request("session/resume", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  server.push(call("c3", "history_list", json!({ "chunk_id": first_id }), 100)).push(mock::text("Listed."));
  prompt(&h, &sid, "list it", 0).await;
  let reqs = server.requests();
  assert_eq!(first_user(&reqs[5]), head, "the reopened view starts the same");
  let listed = reqs[6].body["messages"].as_array().unwrap().iter().rfind(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(listed.starts_with(&format!("Chunk {first_id} (sequence 1, completed), lines 0..")) && listed.contains("0 USER: read notes.txt") && listed.contains("SECRET-ALPHA"), "{listed}");
}

#[tokio::test]
async fn a_long_turn_folds_its_earlier_steps_and_becomes_one_chunk_when_it_ends() {
  let (h, server, sid) = setup(true).await;
  std::fs::write(h.cwd().join("a.txt"), "A-CONTENT\n").unwrap();
  std::fs::write(h.cwd().join("b.txt"), "B-CONTENT\n").unwrap();
  server.push(mock::text("A done"));
  prompt(&h, &sid, "first task", 1000).await;
  // The second turn's second call reports 2000 tokens, over the 1000 threshold: once its result is in, the first turn
  // is sealed and this turn's first step folds into a work-state index, the latest step stays verbatim
  server.push(call("c1", "read", json!({ "path": "a.txt" }), 100)).push(call("c2", "read", json!({ "path": "b.txt" }), 2000)).push(mock::text("All read."));
  let r = prompt(&h, &sid, "read both files", 1000).await;
  assert_eq!(r["stopReason"], "end_turn");
  let reqs = server.requests();
  assert_eq!(reqs.len(), 4);
  let after = body(&reqs[3]);
  assert!(after.contains("<work-state chunk=") && after.contains("- a.txt @ 1") && after.contains("read both files"), "{after}");
  assert!(!after.contains("A-CONTENT") && after.contains("B-CONTENT"), "the folded read is gone, the latest stays: {after}");
  let ev = &compactions(&h, &sid)[0];
  let statuses: Vec<&str> = ev["chunks"].as_array().unwrap().iter().map(|c| c["status"].as_str().unwrap()).collect();
  assert_eq!((ev["reason"].as_str(), statuses), (Some("threshold"), vec!["completed", "partial"]));
  let partial_id = ev["chunks"][1]["id"].as_str().unwrap().to_owned();

  // /compact after the turn ended: the partial chunk becomes the whole turn, same id
  prompt(&h, &sid, "/compact", 0).await;
  let ev = &compactions(&h, &sid)[1];
  let c = &ev["chunks"][0];
  assert_eq!((c["id"].as_str(), c["status"].as_str(), c["from"].as_u64(), c["to"].as_u64()), (Some(partial_id.as_str()), Some("completed"), Some(2), Some(8)), "{ev}");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "next", 1000).await;
  let text = body(&server.requests()[4]);
  assert!(!text.contains("<work-state") && text.contains("All read.") && !text.contains("B-CONTENT"), "{text}");
}

#[tokio::test]
async fn an_overflow_chunks_first_and_summarizes_when_it_overflows_again() {
  let (h, server, sid) = setup(true).await;
  server.push(mock::text("A done")).push(mock::text("B done"));
  prompt(&h, &sid, "first task", 0).await;
  prompt(&h, &sid, "second task", 0).await;
  let too_long = || Reply::Status(400, r#"{"error":{"message":"This model's maximum context length is 64000 tokens","code":"context_length_exceeded"}}"#.into());
  server.push(too_long()).push(too_long()).push(mock::text("## Objective\n- recovered")).push(mock::text("C done"));
  let r = prompt(&h, &sid, "third task", 0).await;
  assert_eq!(r["stopReason"], "end_turn", "{r}");
  let reqs = server.requests();
  assert_eq!(reqs.len(), 6);
  // The first retry went out on the chunks, the second refusal was summarized, chunks included
  assert!(first_user(&reqs[3]).starts_with("<conversation-checkpoint"));
  assert!(body(&reqs[4]).contains("conversation-checkpoint"), "the summary took the chunks in");
  assert!(first_user(&reqs[5]).starts_with("<conversation-summary>\n## Objective\n- recovered"));
  let kinds: Vec<String> = compactions(&h, &sid).iter().map(|e| format!("{}:{}", e["strategy"].as_str().unwrap_or("summary"), e["reason"].as_str().unwrap())).collect();
  assert_eq!(kinds, ["chunk:overflow", "summary:overflow"]);
}
