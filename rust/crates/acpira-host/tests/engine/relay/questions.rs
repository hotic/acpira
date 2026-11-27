//! A child's questions: answered by the waiting caller, on the user's card, or through ask_caller

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_call_answers_with_the_childs_question_and_the_caller_answers_it_by_label() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme which")).await);
  assert!(ok && text.contains("is waiting for an answer") && text.contains("1. Colour: Which colour?") && text.contains("- Blue: calm"), "{text}");
  let thread = thread_of(&text);
  // Only the caller is asked: while it can answer the user is offered no card, the child's transcript holds it
  assert!(nodes(&s)[0]["question"].is_null(), "{}", nodes(&s)[0]);
  let id = nodes(&s)[0]["id"].as_str().unwrap().to_owned();
  let turns = serde_json::to_string(&s.subagent_transcript(&id).unwrap().0).unwrap();
  assert!(turns.contains("summoned it is asked too"), "{turns}");
  // Waiting again answers with the same question at once
  let (_, again) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "") }).await);
  assert!(again.contains("1. Colour: Which colour?"), "{again}");
  // A label in any case picks the option
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), answers: Some(vec!["green".into()]), ..ask("", "") }).await);
  assert!(ok && text.contains(r#""content":{"q0":"Green"}"#), "{text}");
  // Nothing is open any more: another answer is refused
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec!["blue".into()]), ..ask("", "") }).await);
  assert!(!ok, "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_question_takes_the_prompt_as_its_answer_and_the_users_card_counts_first() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme one")).await);
  let thread = thread_of(&text);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "Blue") }).await);
  assert!(ok && text.contains(r#""q0":"Blue""#), "{text}");
  // The user answers first (on a card that reached them): the caller is told the question is gone
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme two")).await);
  let thread = thread_of(&text);
  let card = open_card(&s, 1);
  let qid = card["questions"][0]["id"].as_str().unwrap().to_owned();
  let mut answers = serde_json::Map::new();
  answers.insert(qid, json!("Green"));
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec!["Blue".into()]), ..ask("", "") }).await);
  assert!(!ok && text.contains("no open question"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_answers_on_a_finished_thread_count_as_left_out_and_real_ones_are_still_refused() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: first look")).await);
  let thread = thread_of(&text);
  // A client that fills in every parameter sends `answers: []` with a follow-up: the thread takes it as its next round
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), answers: Some(vec![]), ..ask("", "relay: and now?") }).await);
  assert!(ok && text.starts_with("reply to relay: and now? (round 2"), "{text}");
  // Blank entries and no prompt read the last reply, like a plain wait
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), answers: Some(vec![" ".into()]), ..ask("", "") }).await);
  assert!(ok && text.starts_with("reply to relay: and now? (round 2"), "{text}");
  // A real answer still has no question to go to, prompt or not
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec!["Blue".into()]), ..ask("", "relay: again") }).await);
  assert!(!ok && text.contains("no open question: its round is over"), "{text}");
  assert_eq!(nodes(&s).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_answers_on_a_running_round_answer_nothing_and_a_prompt_still_answers_a_single_question() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme which")).await);
  let thread = thread_of(&text);
  // Empty answers and no prompt answer nothing: the call waits, which repeats the open question
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), answers: Some(vec![]), ..ask("", "") }).await);
  assert!(ok && text.contains("1. Colour: Which colour?"), "{text}");
  assert!(open_card(&s, 0).is_object());
  // With a prompt, the single question still takes it as its answer
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec![]), ..ask("", "Blue") }).await);
  assert!(ok && text.contains(r#""q0":"Blue""#), "{text}");

  // A round held on a permission card asks nothing: a real answer is refused, empty answers wait like none at all
  last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: perm hold")).await);
  until(|| nodes(&s).get(1).is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  let thread = nodes(&s)[1]["harness"]["thread"].as_str().unwrap().to_owned();
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), answers: Some(vec!["Blue".into()]), ..ask("", "") }).await);
  assert!(!ok && text.contains("no open question now"), "{text}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec![]), wait_secs: Some(0), ..ask("", "") }).await);
  assert!(ok && text.contains("is still working"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_root_hears_a_childs_question_once_and_then_its_reply() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: askme later")).await);
  until(|| deliveries(&s).len() == 1 && !s.is_running(), 8000).await;
  let text = deliveries(&s)[0]["text"].as_str().unwrap().to_owned();
  assert!(text.contains("## Fake Worker (fake) asks") && text.contains("Which colour?"), "{text}");
  // The root ended the turn that told it without answering: the card goes to the user. Answered there, the child
  // finishes, and its reply is the next delivery (the question is not repeated)
  let card = question_card(&s, 0).await;
  let mut answers = serde_json::Map::new();
  answers.insert(card["questions"][0]["id"].as_str().unwrap().to_owned(), json!("Blue"));
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  until(|| deliveries(&s).len() == 2 && !s.is_running(), 8000).await;
  let text = deliveries(&s)[1]["text"].as_str().unwrap().to_owned();
  assert!(text.contains("reply to relay: askme later") && !text.contains("asks"), "{text}");
}

/// The node's open question card as its transcript holds it, whether the user is offered it yet or not
fn open_card(s: &Disposing, i: usize) -> Value {
  let id = nodes(s)[i]["id"].as_str().unwrap().to_owned();
  let (turns, ..) = s.subagent_transcript(&id).unwrap();
  let turns: Value = serde_json::from_str(turns.get()).unwrap();
  let mut blocks = turns.as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default());
  blocks.rfind(|b| b["type"] == "question" && b["outcome"].is_null()).unwrap_or_else(|| panic!("no open card: {turns}"))
}

/// The node's open question card once the user is offered it
async fn question_card(s: &Disposing, i: usize) -> Value {
  until(|| nodes(s).get(i).is_some_and(|n| n["question"].is_object()), 8000).await;
  nodes(s)[i]["question"].clone()
}

/// A summoned child held on a permission card (its round stays running), and the reply lines of one `ask_caller` from it
async fn ask_caller_from_held_child(s: &Disposing, roster: &Arc<Roster>, wait_secs: u64) -> tokio::sync::mpsc::UnboundedReceiver<HubReply> {
  let (tx, _) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, 0, send_off("fake-worker", "relay: perm hold"), tx));
  until(|| nodes(s).first().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  let thread = nodes(s)[0]["harness"]["thread"].as_str().unwrap().to_owned();
  let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
  let args = AskCallerArgs { question: "Tabs or spaces?".into(), options: vec!["Tabs".into(), "Spaces".into(), "Tabs".into()], wait_secs: Some(wait_secs) };
  tokio::spawn(s.0.clone().relay_ask_caller(thread, args, tx));
  rx
}

async fn final_of(rx: &mut tokio::sync::mpsc::UnboundedReceiver<HubReply>) -> HubReply {
  loop {
    match tokio::time::timeout(Duration::from_secs(20), rx.recv()).await.expect("reply in time") {
      Some(HubReply::Progress(_)) => continue,
      Some(r) => return r,
      None => panic!("closed without a reply"),
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_childs_ask_caller_goes_through_acpira_mcp_and_the_hub_to_the_waiting_caller_and_back() {
  let fake = fake_or_skip!();
  let (_h, s, roster) = hub_session(&fake).await;
  let call = r#"mcp-call:ask_caller {"question":"Tabs or spaces?","options":["Tabs","Spaces"]}"#;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", call)).await);
  assert!(ok && text.contains("is waiting for an answer") && text.contains("1. Tabs or spaces?") && text.contains("   - Spaces"), "{text}");
  assert!(nodes(&s)[0]["question"].is_null(), "{}", nodes(&s)[0]);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread_of(&text)), answers: Some(vec!["spaces".into()]), ..ask("", "") }).await);
  assert!(ok && text.contains("Answer: Spaces") && text.contains(r#""ask_caller""#), "{text}");
  expect_match(&nodes(&s)[0], json!({ "state": "completed" }));
  // The root's own server has no caller: the tool is not listed, and a call anyway is refused
  // The question went to the waiting call only: no automatic message told it again
  assert!(deliveries(&s).is_empty(), "{:?}", deliveries(&s));
  prompt(&s, r#"mcp-call:ask_caller {"question":"x"}"#).await;
  until(|| !s.is_running(), 8000).await;
  let text = agent_blocks(&view(&s)).into_iter().filter(|b| b["type"] == "text").filter_map(|b| b["markdown"].as_str().map(str::to_owned)).next_back().unwrap_or_else(|| panic!("{}", view(&s)["turns"]));
  let reply: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
  assert!(!reply["tools"].as_array().unwrap().iter().any(|t| t == "ask_caller"), "{reply}");
  assert!(reply["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("Only a summoned agent")), "{reply}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_caller_the_user_answers_on_the_card_reaches_the_child_and_one_nobody_answers_is_skipped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let mut rx = ask_caller_from_held_child(&s, &roster, 20).await;
  let card = question_card(&s, 0).await;
  // Duplicate options are offered once
  let labels: Vec<&str> = card["questions"][0]["options"].as_array().unwrap().iter().map(|o| o["label"].as_str().unwrap()).collect();
  assert_eq!(labels, ["Tabs", "Spaces"]);
  let mut answers = serde_json::Map::new();
  answers.insert(card["questions"][0]["id"].as_str().unwrap().to_owned(), json!("Tabs"));
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  let HubReply::Done(text) = final_of(&mut rx).await else { panic!() };
  assert_eq!(text, "Answer: Tabs");
  // Asked again with a short bound and left alone: the call answers that no answer came, and the card is settled skipped
  let thread = nodes(&s)[0]["harness"]["thread"].as_str().unwrap().to_owned();
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask_caller(thread, AskCallerArgs { question: "Which file?".into(), wait_secs: Some(1), ..Default::default() }, tx));
  let HubReply::Done(text) = final_of(&mut rx).await else { panic!() };
  assert!(text.starts_with("No answer came in time"), "{text}");
  until(|| nodes(&s)[0]["question"].is_null(), 4000).await;
  let id = nodes(&s)[0]["id"].as_str().unwrap().to_owned();
  let turns = serde_json::to_string(&s.subagent_transcript(&id).unwrap().0).unwrap();
  assert!(turns.contains(r#""outcome":"skipped""#), "{turns}");
  let block = nodes(&s)[0]["permissions"][0]["id"].as_str().unwrap().to_owned();
  s.resolve_permission(&block, "no");
  until(|| !s.background_running(), 8000).await;
}
