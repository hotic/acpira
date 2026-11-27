//! Turn shapes: question cards, images with answers, cancels, steering, the context split, reopening, edits and goals

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn the_question_tool_opens_the_question_card_and_the_answers_reach_the_model() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "question", json!({ "questions": [
    { "question": "Which file name?", "header": "Name", "options": [{ "label": "report", "description": "A generic report" }, { "label": "notes" }] },
    { "question": "Where should it go?", "options": ["src", "docs"], "multiple": true },
  ] }))]));
  env.server.push(mock::text("Going with my-own-name in docs."));
  let task = spawn_prompt(&s, "make a file");
  let open = || agent_blocks(&view(&s)).into_iter().find(|b| b["type"] == "question" && b["outcome"].is_null());
  until(|| open().is_some(), 10_000).await;
  let card = open().unwrap();
  // The labels and descriptions are the model's own; both questions take a typed answer beside the choices
  expect_match(&card["questions"][0], json!({ "id": "q0", "title": "Name", "text": "Which file name?", "kind": "single", "other": true,
    "options": [{ "id": "report", "label": "report", "description": "A generic report" }, { "id": "notes", "label": "notes" }] }));
  expect_match(&card["questions"][1], json!({ "id": "q1", "text": "Where should it go?", "kind": "multiple", "other": true }));
  let answers = serde_json::from_value(json!({ "q0": "my-own-name", "q1": ["docs"] })).unwrap();
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  task.await.unwrap();
  let reqs = env.server.requests();
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  assert_eq!(msgs.last().unwrap()["content"], "The user answered:\n\nQ: Which file name?\nA: my-own-name\n\nQ: Where should it go?\nA: docs");
  let vw = view(&s);
  let blocks = agent_blocks(&vw);
  expect_match(blocks.iter().find(|b| b["type"] == "question").unwrap(), json!({ "outcome": "answered", "answers": { "q0": "my-own-name", "q1": ["docs"] } }));
  expect_match(blocks.iter().find(|b| b["type"] == "tool_call").unwrap(), json!({ "verbKey": "verb.ask", "status": "completed" }));
  expect_match(last_turn(&vw), json!({ "stop": "end_turn" }));
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-question", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn images_sent_with_an_answer_reach_the_model_and_stay_on_the_card() {
  let env = Env::new();
  // A model that reads images
  let path = env.root.path().join("providers.json");
  let mut providers: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  providers["providers"][0]["models"][0]["input"] = json!(["text", "image"]);
  std::fs::write(&path, providers.to_string()).unwrap();
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "question", json!({ "questions": [{ "question": "Which screen?", "options": ["list", "chat"] }] }))]));
  env.server.push(mock::text("I see it."));
  let task = spawn_prompt(&s, "fix the layout");
  let open = || agent_blocks(&view(&s)).into_iter().find(|b| b["type"] == "question" && b["outcome"].is_null());
  until(|| open().is_some(), 10_000).await;
  let card = open().unwrap();
  // The model takes images, so the card offers to take some
  assert_eq!(card["images"], true, "{card}");
  // 1x1 PNG
  let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
  let images: std::collections::BTreeMap<String, Vec<acpira_shared::transcript::Draft>> =
    serde_json::from_value(json!({ "q0": [{ "kind": "image", "mimeType": "image/png", "data": png }] })).unwrap();
  // An image alone answers the question
  s.answer_questions_with(card["id"].as_str().unwrap(), &serde_json::Map::new(), Some(&images), false);
  task.await.unwrap();
  let body = env.server.requests()[1].body.to_string();
  assert!(body.contains("A: [image 1 attached]"), "{body}");
  assert!(body.contains(png), "the picture went to the model: {body}");
  let vw = view(&s);
  let q = agent_blocks(&vw).into_iter().find(|b| b["type"] == "question").unwrap();
  expect_match(&q, json!({ "outcome": "answered", "answerImages": { "q0": [{ "mimeType": "image/png" }] } }));
  assert!(q["answerImages"]["q0"][0]["blob"].as_str().is_some_and(|b| b.ends_with(".png")), "{q}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_the_first_chunk_ends_the_turn_at_once_and_nothing_follows() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(Reply::Hang);
  let task = spawn_prompt(&s, "wait for it");
  until(|| env.server.hanging() == 1, 10_000).await;
  let t0 = Instant::now();
  s.cancel().await;
  task.await.unwrap();
  assert!(t0.elapsed() < Duration::from_secs(2), "the cancel took {:?}", t0.elapsed());
  let vw = view(&s);
  expect_match(last_turn(&vw), json!({ "role": "agent", "stop": "cancelled" }));
  // Nothing arrives for the cancelled turn afterwards
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert_eq!(view(&s)["turns"], vw["turns"]);
  // The session takes the next prompt as usual
  env.server.push(mock::text("Ready."));
  run(&s, "go on").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn", "blocks": [{ "type": "text", "markdown": "Ready." }] }));
  let log = std::fs::read_to_string(env.log_of(&s)).unwrap();
  assert!(log.lines().any(|l| l.contains(r#""type":"request""#) && l.contains(r#""stop":"Cancelled""#)), "the cancelled call has its record\n{log}");
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-cancel", &view(&s)["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_steered_into_a_command_round_reaches_the_next_call_in_the_same_turn() {
  let env = Env::new();
  let s = env.session().await;
  assert_eq!(view(&s)["canSteer"], true);
  env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "echo hi" }))]));
  env.server.push(mock::text("Switched to printf."));
  let task = spawn_prompt(&s, "print hi");
  // The command waits on its card while the user sends a correction with a file attached (the composer's steer send)
  let card = wait_block(&s, "permission").await;
  s.steer_prompt("use printf instead".into(), drafts(json!([{ "kind": "text", "name": "note.txt", "text": "printf is portable" }]))).await.unwrap();
  let allow = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").unwrap()["id"].clone();
  s.resolve_permission(card["id"].as_str().unwrap(), allow.as_str().unwrap());
  task.await.unwrap();
  // The model got it behind the command's result, attachment included, and answered in the same turn
  let reqs = env.server.requests();
  assert_eq!(reqs.len(), 2);
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  let steered = msgs.last().unwrap()["content"].to_string();
  assert!(steered.contains("use printf instead") && steered.contains("printf is portable"), "{steered}");
  assert_eq!(msgs[msgs.len() - 2]["role"], "tool");
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("print hi")], "the message joined the turn instead of opening one");
  assert!(vw["queued"].is_null());
  let blocks = agent_blocks(&vw);
  let at = blocks.iter().position(|b| b["type"] == "steer").expect("a steer block");
  expect_match(&blocks[at], json!({ "text": "use printf instead" }));
  assert!(blocks[..at].iter().any(|b| b["type"] == "tool_call"));
  assert!(blocks[at + 1..].iter().any(|b| b["type"] == "text" && b["markdown"] == "Switched to printf."));
  expect_match(last_turn(&vw), json!({ "role": "agent", "stop": "end_turn" }));
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-steer", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_context_split_adds_up_to_the_last_call_and_opens_what_it_sent() {
  use acpira_shared::context::{ContextCategory, ContextContent, ContextNode};
  let env = Env::new();
  std::fs::write(env.cwd().join("notes.txt"), "alpha\n").unwrap();
  let s = env.session().await;
  // Nothing went out yet: no split, the card keeps the bare figure
  assert!(s.context_report(true).await.is_none());
  env.server.push(mock::tools(&[("c1", "read", json!({ "path": "notes.txt" }))]));
  env.server.push(mock::text("It says alpha."));
  run(&s, "what is in notes.txt?").await;
  let r = s.context_report(true).await.expect("the built-in agent reports its split");
  // The mock counts 100 in and 10 out per call: the parts add up to the last one
  assert_eq!(r.used, 110);
  assert_eq!(r.categories.iter().map(|c| c.tokens).sum::<u64>(), 110);
  let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).expect("a conversation part");
  let turn: &ContextNode = &chat.nodes.as_ref().unwrap()[0];
  assert_eq!((turn.index, turn.title.as_deref()), (Some(1), Some("what is in notes.txt?")));
  let call = turn.children.as_ref().unwrap().iter().find(|n| n.tool_kind.as_deref() == Some("read")).expect("the read call");
  assert_eq!(call.title.as_deref(), Some("notes.txt"));
  let Some(ContextContent::Call { input, output, .. }) = s.context_item(&r.rev, &call.id).await else { panic!("the call's content") };
  assert!(input.contains("notes.txt") && output.is_some_and(|o| o.contains("alpha")));
  assert!(s.context_report(false).await.unwrap().categories.iter().all(|c| c.nodes.is_none()));
  let raw = s.context_raw().await.unwrap().expect("the last request");
  assert_eq!(raw.format, "openai-chat");
  // The last request carried the read's result; the reply it got back is not part of it
  let messages = raw.body["messages"].to_string();
  assert!(messages.contains("alpha") && !messages.contains("It says alpha"), "{messages}");
  assert!(!raw.body.to_string().contains("sk-test"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reopened_session_resumes_with_the_same_prefix() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::text("Noted: 42."));
  run(&s, "remember 42").await;
  let log = env.log_of(&s);
  let record = s.to_record();
  s.dispose();
  // A new window: the record reopens the agent's session in a fresh process
  let again = Disposing(AcpSession::new(record.clone(), env.deps.clone()));
  again.start().await;
  assert_eq!(again.to_record().acp_session_id, record.acp_session_id);
  env.server.push(mock::text("42."));
  run(&again, "which number?").await;
  let reqs = env.server.requests();
  let last = reqs.last().unwrap().body["messages"].to_string();
  assert!(last.contains("remember 42") && last.contains("Noted: 42."), "{last}");
  // The reopened process rebuilt the history the first one sent, byte for byte
  env.assert_prefix_stable(&log);
  let turns = view(&again)["turns"].clone();
  assert_eq!(turns.as_array().unwrap().len(), 4, "{turns}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_message_rewinds_the_agents_own_history_instead_of_superseding_it() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::text("one"));
  run(&s, "first message").await;
  env.server.push(mock::text("bad answer"));
  run(&s, "the wrong message").await;
  let peer = s.to_record().acp_session_id;
  env.server.push(mock::text("good answer"));
  s.edit_turn(crate::acp_session::history_edit(&s, 2, "the right message")).await.unwrap();
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 4 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  // The same native session, which no longer holds the original: no rebuilt history, no superseded note
  assert_eq!(s.to_record().acp_session_id, peer);
  let sent = env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("first message") && sent.contains("one") && sent.contains("the right message"), "{sent}");
  assert!(!sent.contains("the wrong message") && !sent.contains("bad answer") && !sent.contains("superseded"), "{sent}");
  let turns = view(&s)["turns"].clone();
  expect_match(&turns[2], json!({ "text": "the right message", "edited": true }));
  // The rewound history is what the model saw before the original: the prefix up to it stays byte for byte
  let reqs = env.server.requests();
  let (before, after) = (reqs[0].body["messages"].as_array().unwrap(), reqs[2].body["messages"].as_array().unwrap());
  assert!(after.len() > before.len() && before.iter().zip(after).all(|(x, y)| x == y), "{before:?}\n{after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_goal_set_from_the_dock_is_continued_until_the_model_marks_it_complete() {
  let env = Env::new();
  std::fs::write(env.cwd().join("a.txt"), "alpha\n").unwrap();
  let s = env.session().await;
  assert_eq!(view(&s)["goalActions"], json!(["set", "pause", "resume", "clear"]));
  // The set turn reads and stops short; the engine continues the goal, where the edit lands and the model marks it met
  env.server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))]));
  env.server.push(mock::text("It says alpha; the edit comes next."));
  env.server.push(mock::tools(&[("c2", "edit", json!({ "path": "a.txt", "old_string": "alpha", "new_string": "beta" }))]));
  env.server.push(mock::tools(&[("c3", "update_goal", json!({ "status": "complete" }))]));
  env.server.push(mock::text("Done: a.txt says beta."));
  s.control_goal(GoalAction::Set, Some("make a.txt say beta".into())).await.unwrap();
  // The continuation's edit asks like any other: answered here, since no prompt of the test's own runs it
  let mut answered = HashSet::new();
  let t0 = Instant::now();
  while s.is_running() || env.server.requests().len() < 5 || !view(&s)["goal"].is_null() {
    assert!(t0.elapsed() < Duration::from_secs(20), "the goal did not settle: {}", view(&s));
    if let Some(card) = find_block(&view(&s), "permission").filter(|c| !answered.contains(&c["id"].to_string())) {
      let allow = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").expect("an allow-once option")["id"].clone();
      answered.insert(card["id"].to_string());
      s.resolve_permission(card["id"].as_str().unwrap(), allow.as_str().unwrap());
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  assert_eq!(std::fs::read_to_string(env.cwd().join("a.txt")).unwrap(), "beta\n");
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(turns.len(), 4, "{}", vw["turns"]);
  expect_match(&turns[0], json!({ "role": "user", "text": "/goal make a.txt say beta", "auto": true, "autoReason": "goal" }));
  expect_match(&turns[2], json!({ "role": "user", "text": "/goal continue", "auto": true, "autoReason": "goal", "goalContinue": true }));
  let blocks = agent_blocks(&vw);
  let goals: Vec<Value> = blocks.iter().filter(|b| b["type"] == "goal").map(|b| b["event"].clone()).collect();
  assert_eq!(goals, [json!("set"), json!("complete")]);
  // The goal's tool shows no row; the goal rows stand for it
  let tools: Vec<Value> = blocks.iter().filter(|b| b["type"] == "tool_call").map(|b| b["kind"].clone()).collect();
  assert_eq!(tools, [json!("read"), json!("edit")]);
  // The continuation opened with the goal's note, and a met goal is not continued
  let reqs = env.server.requests();
  let note = reqs[2].body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
  assert!(note.starts_with("<goal>\nContinue working toward the active goal.") && note.contains("<objective>\nmake a.txt say beta\n</objective>"), "{note}");
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(env.server.requests().len(), 5);
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-goal", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_first_message_carries_the_workspace_context_again_after_the_rewind() {
  let env = Env::new();
  let git = std::process::Command::new("git").arg("init").arg("-q").current_dir(env.cwd()).status();
  if !git.is_ok_and(|s| s.success()) {
    return;
  }
  std::fs::create_dir_all(env.cwd().join(".agents")).unwrap();
  std::fs::write(env.cwd().join(".agents/hooks.json"), json!({ "context": ["RULES.md"] }).to_string()).unwrap();
  std::fs::write(env.cwd().join("RULES.md"), "house rule: answer briefly\n").unwrap();
  let s = env.session().await;
  env.server.push(mock::text("one"));
  run(&s, "first message").await;
  assert!(env.server.requests()[0].body["messages"].to_string().contains("house rule"));
  env.server.push(mock::text("two"));
  s.edit_turn(crate::acp_session::history_edit(&s, 0, "first message, edited")).await.unwrap();
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 2 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  // The rewound session holds nothing, so the edited message is its first and brings the context files along
  let sent = env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("house rule") && sent.contains("first message, edited") && !sent.contains("\"one\""), "{sent}");
}
