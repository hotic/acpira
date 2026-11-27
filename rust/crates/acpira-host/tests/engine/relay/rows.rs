//! The root's ask_agent row tied to the node of its round, however its arguments stream in

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn the_root_ask_agent_row_is_tied_to_its_node_and_hides_behind_it() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The row arrives before the hub hears of the call (the agent announces it, then runs it)
  prompt(&s, "relay-call:relay: tied").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: tied")).await);
  assert!(ok);
  let node = nodes(&s)[0].clone();
  assert_eq!(node["peer"]["toolCallId"], "ak1");
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  assert_eq!(row["subagentId"], node["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_row_whose_arguments_stream_in_without_its_name_still_ties_to_its_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // claude-agent-acp 0.83.0: the name only on the first, empty-input tool_call; the prompt arrives in title-less refinements
  prompt(&s, "relay-call-stream:relay: streamed").await;
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  // An empty input is not a wait call
  assert!(row["verbKey"].is_null());
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: streamed")).await);
  assert!(ok);
  let node = nodes(&s)[0].clone();
  assert_eq!(node["peer"]["toolCallId"], "ak1");
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  assert_eq!(row["subagentId"], node["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_row_named_before_any_input_still_ties_to_its_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // claude-agent-acp 0.83.0 to an AIR client: the name without rawInput, then the whole input without the name
  prompt(&s, "relay-call-air:relay: air").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: air")).await);
  assert!(ok);
  let node = nodes(&s)[0].clone();
  assert_eq!(node["peer"]["toolCallId"], "ak1");
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  assert_eq!(row["subagentId"], node["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_row_waits_for_the_round_of_its_own_persona() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The row names fake-review; a fake-worker round with the very same task must not take it
  prompt(&s, "relay-call:relay: same task").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: same task")).await);
  assert!(ok);
  assert!(nodes(&s)[0]["peer"]["toolCallId"].is_null());
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: same task")).await);
  assert!(ok);
  assert_eq!(nodes(&s)[1]["peer"]["toolCallId"], "ak1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_that_ended_without_a_round_never_takes_its_retrys_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // ak1 failed before any round existed; ak2 is the retry with the same arguments
  prompt(&s, "relay-call-failed:relay: retried").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: retried")).await);
  assert!(ok);
  assert_eq!(nodes(&s)[0]["peer"]["toolCallId"], "ak2");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_row_whose_arguments_come_with_its_end_ties_even_while_another_call_waits() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  prompt(&s, "relay-call-late:relay: late").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: late")).await);
  assert!(ok);
  assert_eq!(nodes(&s)[0]["peer"]["toolCallId"], "ak1");
}
