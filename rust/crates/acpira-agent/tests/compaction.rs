//! Summary compaction against the scripted model: at the host's threshold inside a turn, on /compact, and after a
//! provider refused a request as too long; the session log and a reopen keep the compacted view

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel, Reply};
use support::Harness;

async fn setup(models: Value) -> (Harness, MockModel, String) {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), models);
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

/// A prompt as the host sends it, with its auto-compaction setting in `_meta`
async fn prompt(h: &Harness, sid: &str, text: &str, auto: bool, at: u64) -> Value {
  let params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }], "_meta": { "acpira/compaction": { "auto": auto, "atTokens": at } } });
  h.conn.request("session/prompt", params).await.unwrap()
}

/// A plain answer whose usage reports `input` prompt tokens
fn answer(text: &str, input: u64) -> Reply {
  Reply::Sse(vec![mock::delta(json!({ "role": "assistant", "content": text })), mock::finish("stop"), mock::usage(input, 10, 0)])
}

fn log(h: &Harness, sid: &str) -> Vec<Value> {
  let text = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn compaction_updates(h: &Harness) -> Vec<Value> {
  h.updates().into_iter().filter(|u| u["sessionUpdate"] == "compaction_update").collect()
}

fn user_texts(req: &mock::Recorded) -> Vec<String> {
  req.body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "user").map(|m| m["content"].as_str().unwrap_or("").to_owned()).collect()
}

#[tokio::test]
async fn a_turn_compacts_at_the_hosts_threshold_and_keeps_the_last_two_turns() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000, "output": 1000 }])).await;
  server.push(answer("A done", 100)).push(answer("B done", 100));
  prompt(&h, &sid, "first task", true, 150).await;
  prompt(&h, &sid, "second task", true, 150).await;
  assert!(compaction_updates(&h).is_empty(), "110 tokens are under the threshold");
  // The third turn's call reports 210: over 150, so the turn compacts once the call is done, with the summary
  // written by the same model, and ends with the summary in place
  server.push(answer("C done", 200)).push(mock::text("## Objective\n- the summary"));
  let r = prompt(&h, &sid, "third task", true, 150).await;
  assert_eq!(r["stopReason"], "end_turn");
  let ups = compaction_updates(&h);
  assert_eq!(ups.len(), 2, "{ups:?}");
  assert_eq!((ups[0]["status"].as_str(), ups[1]["status"].as_str()), (Some("in_progress"), Some("completed")));
  assert_eq!(ups[0]["compactionId"], ups[1]["compactionId"]);
  let all = h.updates();
  let done = all.iter().position(|u| u["sessionUpdate"] == "compaction_update" && u["status"] == "completed").unwrap();
  assert!(all[done..].iter().any(|u| u["sessionUpdate"] == "usage_update"), "the context after the compaction is reported");

  // The summary call: no tools, the first turn as a transcript, the last two turns kept out of it
  let reqs = server.requests();
  assert_eq!(reqs.len(), 4);
  let summary = &reqs[3];
  assert!(summary.body.get("tools").is_none_or(|t| t.as_array().is_some_and(|a| a.is_empty())), "{}", summary.body);
  let text = user_texts(summary).join("\n");
  assert!(text.contains("<conversation>\n[User]: first task\n\n[Assistant]: A done\n</conversation>"), "{text}");
  assert!(!text.contains("second task") && text.contains("## Next Move"));
  let ev = log(&h, &sid).into_iter().find(|e| e["type"] == "compaction").unwrap();
  assert_eq!((ev["reason"].as_str(), ev["tail"].as_u64(), ev["summary"].as_str()), (Some("threshold"), Some(4), Some("## Objective\n- the summary")));
  assert_eq!(ev["usage"]["output"], 10);
  let req_ev = log(&h, &sid).into_iter().find(|e| e["type"] == "request").unwrap();
  assert_eq!(req_ev["policies"]["compaction"], json!({ "auto": true, "threshold": 150 }));

  // From then on the model sees the summary, then the two kept turns, then the new prompt
  server.push(answer("D done", 100));
  prompt(&h, &sid, "fourth task", true, 150).await;
  let next = &server.requests()[4];
  let users = user_texts(next);
  assert!(users[0].starts_with("<conversation-summary>\n## Objective\n- the summary\n</conversation-summary>"), "{users:?}");
  assert_eq!((users[1].as_str(), users[2].as_str()), ("second task", "third task"));
  // The mode note was in what the summary replaced, so the next prompt carries it again
  assert!(users[3].starts_with("<mode>") && users[3].ends_with("fourth task") && !next.body.to_string().contains("first task"), "{users:?}");
  assert_eq!(next.body["messages"].as_array().unwrap().len(), 7, "system, summary, two kept turns, the prompt");
}

#[tokio::test]
async fn a_request_the_summary_took_in_comes_back_verbatim() {
  // A 20k window leaves a tail budget of 4750 tokens; a prompt of about 5000 never fits it, so a compaction inside its
  // turn keeps only the reply
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 20000, "output": 1000 }])).await;
  let long = format!("Please do the following task. {}", "detail ".repeat(3000));
  server.push(answer("working on it", 5000)).push(mock::text("## Objective\n- the long task"));
  prompt(&h, &sid, &long, true, 1000).await;
  let ev = log(&h, &sid).into_iter().find(|e| e["type"] == "compaction").unwrap();
  assert_eq!(ev["tail"], 1, "{ev}");
  assert!(ev["request"].as_str().unwrap().starts_with("Please do the following task.") && ev["request"].as_str().unwrap().ends_with("detail"), "{ev}");
  server.push(answer("done", 100));
  prompt(&h, &sid, "next", true, 1_000_000).await;
  let users = user_texts(&server.requests()[2]);
  assert!(users[0].contains("</conversation-summary>") && users[0].contains("<request>\nPlease do the following task."), "{}", &users[0][..300]);
  // The kept reply follows the summary, then the new prompt
  let msgs = server.requests()[2].body["messages"].as_array().unwrap().clone();
  assert_eq!(msgs[2]["role"], "assistant");
  // A reopen rebuilds the same item from the log
  let h = Harness::start_in(h.home, h.cwd).await;
  h.conn.request("session/resume", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  server.push(answer("again", 100));
  prompt(&h, &sid, "more", true, 1_000_000).await;
  assert_eq!(user_texts(&server.requests()[3])[0], users[0]);
}

#[tokio::test]
async fn a_vague_refusal_near_the_window_counts_as_an_overflow() {
  // What a gateway's Claude route sends for an oversized request, inside a 200 stream (2026-10-13)
  let vague = || Reply::Sse(vec![json!({ "error": { "type": "invalid_request_error", "message": "There is an issue with this request, please try a different model." } })]);
  // The system prompt and the tool schemas alone are over 80% of a 1000-token window. The prompt also carries the
  // real home's AGENTS.md and skills, so the window sits below the bare prompt (about 2000 tokens with an empty HOME,
  // as on CI) instead of relying on a developer machine's larger one
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 1000 }])).await;
  server.push(mock::text("A done")).push(mock::text("B done"));
  prompt(&h, &sid, "first task", false, 0).await;
  prompt(&h, &sid, "second task", false, 0).await;
  server.push(vague()).push(mock::text("## Objective\n- recovered")).push(mock::text("C done"));
  let r = prompt(&h, &sid, "third task", false, 0).await;
  assert_eq!(r["stopReason"], "end_turn", "{r}");
  assert_eq!(log(&h, &sid).into_iter().filter(|e| e["type"] == "compaction").map(|e| e["reason"].clone()).collect::<Vec<_>>(), [json!("overflow")]);
  assert_eq!(server.requests().len(), 5);

  // Far from the window the same refusal is the turn's error: neither compacted nor retried
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 1000000 }])).await;
  server.push(mock::text("A done"));
  prompt(&h, &sid, "first task", false, 0).await;
  server.push(vague());
  let r = h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "again" }] })).await;
  assert!(r.unwrap_err().message.contains("There is an issue with this request"));
  assert_eq!(server.requests().len(), 2, "no retry of a refused request");
  assert!(compaction_updates(&h).is_empty());
}

#[tokio::test]
async fn auto_compaction_off_leaves_the_history_alone() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000 }])).await;
  server.push(answer("done", 500));
  prompt(&h, &sid, "task", false, 50).await;
  assert!(compaction_updates(&h).is_empty());
  assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn slash_compact_summarizes_everything_and_the_session_reopens_compacted() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000 }])).await;
  // Nothing yet
  let r = prompt(&h, &sid, "/compact", true, 0).await;
  assert_eq!(r["stopReason"], "end_turn");
  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "agent_message_chunk" && u["content"]["text"] == "Nothing to compact yet."));
  assert!(server.requests().is_empty());

  server.push(mock::text("A done")).push(mock::text("B done"));
  prompt(&h, &sid, "first task", true, 0).await;
  prompt(&h, &sid, "second task", true, 0).await;
  // Two turns that fit the tail budget whole would be the whole conversation: all of it is summarized
  server.push(mock::text("## Objective\n- both"));
  let r = prompt(&h, &sid, "/compact", true, 0).await;
  assert_eq!(r["stopReason"], "end_turn");
  assert!(r.get("usage").is_none(), "the summary call is not the turn's usage");
  let ups = compaction_updates(&h);
  assert_eq!(ups.iter().map(|u| u["status"].as_str().unwrap()).collect::<Vec<_>>(), ["in_progress", "completed"]);
  assert!(!h.updates().iter().any(|u| u["sessionUpdate"] == "user_message_chunk" && u["content"]["text"] == "/compact"), "the command is not a message");
  let text = user_texts(&server.requests()[2]).join("\n");
  assert!(text.contains("[User]: first task") && text.contains("[User]: second task") && text.contains("[Assistant]: B done"));
  let ev = log(&h, &sid).into_iter().find(|e| e["type"] == "compaction").unwrap();
  assert_eq!((ev["reason"].as_str(), ev["tail"].as_u64()), (Some("manual"), Some(0)));
  // The session file still holds every item; a title listing reads the first prompt
  assert_eq!(log(&h, &sid).iter().filter(|e| e["type"] == "item").count(), 4);

  // A new process reads the compacted view back and the next prompt goes out on it
  let h = Harness::start_in(h.home, h.cwd).await;
  h.conn.request("session/resume", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  server.push(mock::text("C done"));
  prompt(&h, &sid, "third task", true, 0).await;
  let next = &server.requests()[3];
  let users = user_texts(next);
  assert_eq!(users.len(), 2, "{users:?}");
  assert!(users[0].starts_with("<conversation-summary>\n## Objective\n- both\n</conversation-summary>"));
  assert!(users[1].ends_with("third task"));
  assert_eq!(next.body["messages"].as_array().unwrap().len(), 3);
  // A second compaction updates the previous summary instead of starting over
  server.push(mock::text("## Objective\n- all three"));
  prompt(&h, &sid, "/compact", true, 0).await;
  let text = user_texts(&server.requests()[4]).join("\n");
  assert!(text.contains("<previous-summary>\n## Objective\n- both\n</previous-summary>") && text.contains("[User]: third task") && !text.contains("[User]: first task"), "{text}");
}

#[tokio::test]
async fn the_fast_model_writes_the_summary_and_the_sessions_model_takes_over_when_it_fails() {
  let h = Harness::start().await;
  let (main, fast) = (MockModel::start(), MockModel::start());
  // The settings page's fast model: a second source with a smaller window than the session's model
  let source = |id: &str, url: String, model: Value| json!({ "id": id, "name": id, "baseUrl": url, "models": [model] });
  std::fs::write(
    h.home().join("providers.json"),
    json!({ "version": 1, "fastModel": "fast/f1", "providers": [
      source("mock", main.base_url(), json!({ "id": "m1", "context": 64000 })),
      source("fast", fast.base_url(), json!({ "id": "f1", "context": 16000 })),
    ] })
    .to_string(),
  )
  .unwrap();
  std::fs::write(h.home().join("secrets.json"), json!({ "acpira.provider.mock": "sk-main", "acpira.provider.fast": "sk-fast" }).to_string()).unwrap();
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  // The first prompt also asks the fast model for a title, beside the turn
  fast.push(mock::text("Title"));
  main.push(mock::text("A done")).push(mock::text("B done"));
  prompt(&h, &sid, "first task", true, 0).await;
  prompt(&h, &sid, "second task", true, 0).await;
  until(|| fast.requests().len() == 1).await;

  // /compact: the summary comes from the fast model with its own key, and the session's model is not called
  fast.push(mock::text("## Objective\n- fast summary"));
  assert_eq!(prompt(&h, &sid, "/compact", true, 0).await["stopReason"], "end_turn");
  let reqs = fast.requests();
  assert_eq!((reqs.len(), main.requests().len()), (2, 2));
  assert_eq!((reqs[1].header("authorization"), reqs[1].body["model"].as_str()), (Some("Bearer sk-fast"), Some("f1")));
  assert!(user_texts(&reqs[1]).join("\n").contains("[User]: second task"));
  let ev = log(&h, &sid).into_iter().find(|e| e["type"] == "compaction").unwrap();
  assert_eq!((ev["model"].as_str(), ev["summary"].as_str()), (Some("fast/f1"), Some("## Objective\n- fast summary")));
  assert!(ev.get("fallback").is_none());

  // The fast model fails: the session's model writes the summary in the same compaction, and the log says why
  main.push(mock::text("C done"));
  prompt(&h, &sid, "third task", true, 0).await;
  fast.push(Reply::Status(500, "down".into()));
  main.push(mock::text("## Objective\n- main summary"));
  prompt(&h, &sid, "/compact", true, 0).await;
  let ups = compaction_updates(&h);
  assert_eq!(ups.iter().map(|u| u["status"].as_str().unwrap()).collect::<Vec<_>>(), ["in_progress", "completed", "in_progress", "completed"]);
  assert_eq!(main.requests()[3].body["model"], "m1");
  let ev = log(&h, &sid).into_iter().rfind(|e| e["type"] == "compaction").unwrap();
  assert_eq!((ev["model"].as_str(), ev["fallback"]["model"].as_str()), (Some("mock/m1"), Some("fast/f1")));
  assert!(ev["fallback"]["error"].as_str().is_some_and(|e| e.contains("500")), "{ev}");
}

#[tokio::test]
async fn an_overflowing_request_is_compacted_and_sent_again() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000 }])).await;
  server.push(mock::text("A done")).push(mock::text("B done"));
  prompt(&h, &sid, "first task", true, 0).await;
  prompt(&h, &sid, "second task", true, 0).await;
  // The provider refuses the third turn's request; the history is compacted and the same step goes out again
  server
    .push(Reply::Status(400, r#"{"error":{"message":"This model's maximum context length is 64000 tokens. However, you requested 70000 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#.into()))
    .push(mock::text("## Objective\n- recovered"))
    .push(mock::text("C done"));
  let r = prompt(&h, &sid, "third task", true, 0).await;
  assert_eq!(r["stopReason"], "end_turn", "{r}");
  assert_eq!(compaction_updates(&h).iter().map(|u| u["status"].as_str().unwrap()).collect::<Vec<_>>(), ["in_progress", "completed"]);
  let reqs = server.requests();
  assert_eq!(reqs.len(), 5);
  // The retry: the summary (of the first turn), the two kept turns, the prompt
  let users = user_texts(&reqs[4]);
  assert!(users[0].starts_with("<conversation-summary>\n## Objective\n- recovered"), "{users:?}");
  assert_eq!(users[1], "second task");
  assert!(users[2].ends_with("third task") && users.len() == 3, "{users:?}");
  let events = log(&h, &sid);
  let kinds: Vec<String> = events.iter().filter(|e| e["type"] == "request" || e["type"] == "compaction").map(|e| format!("{}:{}", e["type"].as_str().unwrap(), e["reason"].as_str().or(e["stop"].as_str()).or(e["error"].as_str().map(|_| "error")).unwrap_or(""))).collect();
  assert_eq!(kinds, ["request:EndTurn", "request:EndTurn", "request:error", "compaction:overflow", "request:EndTurn"], "{kinds:?}");
  // A refusal that is not about the size is not compacted
  server.push(Reply::Status(400, r#"{"error":{"message":"invalid tool schema"}}"#.into()));
  let r = h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "again" }] })).await;
  assert!(r.is_err());
  assert_eq!(compaction_updates(&h).len(), 2);
}

/// Wait up to 5 s for `done`
async fn until(done: impl Fn() -> bool) {
  tokio::time::timeout(std::time::Duration::from_secs(5), async {
    while !done() {
      tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
  })
  .await
  .expect("timed out");
}

#[tokio::test]
async fn a_window_edited_mid_turn_sets_the_next_calls_threshold() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000, "output": 1000 }])).await;
  // The first call asks for a slow command; the window is corrected while it runs (2000 less 1000 for the output)
  server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 1" }))]));
  server.push(answer("done", 1500)).push(mock::text("## Objective\n- the summary"));
  let edit = async {
    until(|| h.updates().iter().any(|u| u["toolCallId"] == "call-1" && u["status"] == "in_progress")).await;
    h.providers(&server.base_url(), json!([{ "id": "m1", "context": 2000, "output": 1000 }]));
  };
  let (r, ()) = tokio::join!(prompt(&h, &sid, "task", true, 1_000_000), edit);
  assert_eq!(r["stopReason"], "end_turn");
  // Under the 64k guess the second call's 1510 tokens were far from the threshold; under the edit they are over it
  let requests: Vec<Value> = log(&h, &sid).into_iter().filter(|e| e["type"] == "request").collect();
  assert_eq!(requests[1]["policies"]["compaction"]["threshold"], 1000, "{requests:?}");
  assert_eq!(compaction_updates(&h).len(), 2, "the second call compacts");
  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "usage_update" && u["used"] == 1510 && u["size"] == 2000));
}

#[tokio::test]
async fn a_window_edited_between_turns_is_pushed_to_the_meter() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000, "output": 1000 }])).await;
  server.push(answer("done", 100));
  prompt(&h, &sid, "task", true, 1_000_000).await;
  let usage = || h.updates().into_iter().filter(|u| u["sessionUpdate"] == "usage_update").collect::<Vec<_>>();
  assert_eq!(usage().last().map(|u| (u["used"].clone(), u["size"].clone())), Some((json!(110), json!(64000))));
  // No turn runs: the next check of providers.json sends the same usage against the new window, once
  h.providers(&server.base_url(), json!([{ "id": "m1", "context": 1_000_000, "output": 1000 }]));
  until(|| usage().len() == 2).await;
  assert_eq!((usage()[1]["used"].clone(), usage()[1]["size"].clone()), (json!(110), json!(1_000_000)));
  tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
  assert_eq!(usage().len(), 2, "{sid}");
}

/// Seven turns that each read a 40 KB file, then two small ones: past the last two turns the old reads hold about 70k
/// tokens, enough for a prune by the size rules. Whether one runs is the policy's; true when the log has a prune event
async fn pruned_after(auto: bool, at: u64, chunked: bool) -> bool {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 400000 }])).await;
  if chunked {
    let path = h.home().join("providers.json");
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["compaction"] = json!("chunk");
    std::fs::write(&path, file.to_string()).unwrap();
  }
  let line = format!("{}\n", "x".repeat(49));
  std::fs::write(h.cwd().join("big.txt"), line.repeat(800)).unwrap();
  for i in 0..7 {
    server.push(mock::tools(&[(&format!("c{i}"), "read", json!({ "path": "big.txt" }))])).push(answer("read", 100));
    prompt(&h, &sid, &format!("read it {i}"), auto, at).await;
  }
  for i in 0..2 {
    server.push(answer("ok", 100));
    prompt(&h, &sid, &format!("small {i}"), auto, at).await;
  }
  log(&h, &sid).iter().any(|e| e["type"] == "prune")
}

#[tokio::test]
async fn old_tool_output_is_pruned_only_when_a_summary_compaction_is_near() {
  // The last call used 110 tokens: at a 150 threshold that is past 70%, at 100k it is far from it
  assert!(pruned_after(true, 150, false).await, "near the threshold, the summary strategy prunes");
  assert!(!pruned_after(true, 100_000, false).await, "far from the threshold the cache is kept");
  assert!(!pruned_after(false, 150, false).await, "automatic compaction off leaves the history alone");
  assert!(!pruned_after(true, 150, true).await, "chunks fold the finished turns themselves");
}
