//! test/questions.test.ts: the three question dialects, from the form schema and through the session

use serde_json::{Value, json};

use acpira_host::acp::questions::{RawOption, RawQuestion, form_content, form_question_count, form_questions, spare_message};

use crate::acp_session::{agent_blocks, last_turn, prompt, spawn_prompt, view};
use crate::fake_or_skip;
use crate::support::{Disposing, Harness, expect_absent, expect_eq, expect_match, until, v};

#[test]
fn devins_form_reads_labels_descriptions_question_and_allow_other() {
  let raw = vec![RawQuestion {
    header: Some("文件名".into()),
    question: Some("这个新文件应该叫什么名字？".into()),
    options: Some(vec![RawOption { label: "report".into(), description: Some("通用报告文件名".into()) }, RawOption { label: "notes".into(), description: Some("笔记".into()) }]),
  }];
  let qs = form_questions(&json!({
    "type": "object", "required": ["q0"],
    "properties": { "q0": { "type": "string", "title": "文件名", "description": "这个新文件应该叫什么名字？", "oneOf": [{ "const": "report", "title": "通用报告文件名" }, { "const": "notes", "title": "笔记" }] } },
  }), "这个新文件应该叫什么名字？", Some(&json!({ "cognition.ai/allowOther": true })), Some(&raw));
  expect_eq(qs, json!([{
    "id": "q0", "title": "文件名", "text": "这个新文件应该叫什么名字？", "kind": "single", "required": true, "other": true,
    "options": [{ "id": "report", "label": "report", "description": "通用报告文件名" }, { "id": "notes", "label": "notes", "description": "笔记" }],
  }]));
  // Without the tool input the schema alone still names the choices by their titles
  let bare = v(form_questions(&json!({ "properties": { "q0": { "type": "string", "oneOf": [{ "const": "report", "title": "通用报告文件名" }] } } }), "Pick one", None, None));
  expect_match(&bare[0], json!({ "text": "Pick one", "options": [{ "id": "report", "label": "通用报告文件名" }] }));
  expect_absent(&bare[0], "other");
}

#[test]
fn kimis_form_takes_one_question_per_message_line_and_no_free_text() {
  let qs = form_questions(&json!({ "type": "object", "properties": {
    "q0": { "type": "string", "title": "文件名", "oneOf": [{ "const": "index.ts", "title": "index.ts" }, { "const": "main.ts", "title": "main.ts" }] },
    "q1": { "type": "string", "title": "风格", "oneOf": [{ "const": "kebab-case", "title": "kebab-case" }] },
  } }), "新文件用什么文件名？\n文件名采用哪种命名风格？", None, None);
  let j = v(&qs);
  assert_eq!(j.as_array().unwrap().iter().map(|q| (q["title"].clone(), q["text"].clone())).collect::<Vec<_>>(),
    [(json!("文件名"), json!("新文件用什么文件名？")), (json!("风格"), json!("文件名采用哪种命名风格？"))]);
  expect_eq(&j[0]["options"], json!([{ "id": "index.ts", "label": "index.ts" }, { "id": "main.ts", "label": "main.ts" }]));
  assert!(j.as_array().unwrap().iter().all(|q| q["other"] != true));
  assert!(spare_message("新文件用什么文件名？\n文件名采用哪种命名风格？", &qs).is_none());
  assert_eq!(spare_message("Help me decide", &qs).as_deref(), Some("Help me decide"));
}

#[test]
fn the_other_property_types_map_to_yes_no_multiple_plain_and_numeric() {
  let qs = v(form_questions(&json!({ "properties": {
    "yes": { "type": "boolean", "title": "Proceed?" },
    "many": { "type": "array", "title": "Folders", "items": { "enum": ["src", "docs"] } },
    "one": { "type": "string", "enum": ["a", "b"] },
    "n": { "type": "integer", "title": "How many" },
    "free": { "type": "string", "title": "Anything else" },
  } }), "", Some(&json!({ "cognition.ai/allowOther": true })), None));
  let shape: Vec<Value> = qs.as_array().unwrap().iter().map(|q| json!([q["id"], q["kind"], q["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), q["other"].as_bool().unwrap_or(false), q["numeric"].as_bool().unwrap_or(false)])).collect();
  assert_eq!(shape, [
    json!(["yes", "single", ["true", "false"], false, false]),
    json!(["many", "multiple", ["src", "docs"], true, false]),
    json!(["one", "single", ["a", "b"], true, false]),
    json!(["n", "text", [], false, true]),
    json!(["free", "text", [], false, false]),
  ]);
}

#[test]
fn claudes_custom_answer_boxes_fold_into_their_questions_and_answers_split_back() {
  let custom = |q: &str| json!({ "type": "string", "title": "Other",
    "description": "Type your own answer, or add a note to the option you chose above (optional).",
    "_meta": { "_askUserQuestionCustomAnswer": { "questionId": q, "isCustomAnswer": true } } });
  let schema = json!({ "type": "object", "properties": {
    "question_0": { "type": "string", "title": "范围", "description": "往回看几个月？", "oneOf": [{ "const": "1 个月", "title": "1 个月" }, { "const": "3 个月", "title": "3 个月" }] },
    "question_0_custom": custom("question_0"),
    "question_1": { "type": "array", "title": "目录", "description": "改哪些目录？", "items": { "anyOf": [{ "const": "src", "title": "src" }, { "const": "docs", "title": "docs" }] } },
    "question_1_custom": custom("question_1"),
  } });
  let message = "Please answer the following questions.";
  let qs = form_questions(&schema, message, None, None);
  assert_eq!(form_question_count(&schema), 2);
  let j = v(&qs);
  assert_eq!(j.as_array().unwrap().iter().map(|q| (q["id"].clone(), q["text"].clone(), q["other"].clone())).collect::<Vec<_>>(),
    [(json!("question_0"), json!("往回看几个月？"), json!(true)), (json!("question_1"), json!("改哪些目录？"), json!(true))]);
  assert!(spare_message(message, &qs).is_none());
  // A pick stays on the question, typed text goes to the companion
  expect_eq(form_content(&schema, &qs, &answers(json!({ "question_0": "3 个月", "question_1": ["docs", "只改 README"] }))),
    json!({ "question_0": "3 个月", "question_1": ["docs"], "question_1_custom": "只改 README" }));
  expect_eq(form_content(&schema, &qs, &answers(json!({ "question_0": "半年" }))), json!({ "question_0_custom": "半年" }));
}

#[test]
fn codexs_note_field_folds_in_and_typed_text_picks_its_other_choice() {
  let schema = json!({ "type": "object", "required": ["q"], "properties": {
    "q": { "type": "string", "title": "Which one?", "oneOf": [{ "const": "A", "title": "A" }, { "const": "None of the above", "title": "None of the above" }], "_meta": { "codex": { "isOther": true } } },
    "q_note": { "type": "string", "title": "Additional answer or note", "_meta": { "codex": { "questionId": "q", "role": "user_note" } } },
  } });
  let qs = form_questions(&schema, "Codex needs your input to continue.", None, None);
  expect_match(v(&qs), json!([{ "id": "q", "other": true, "options": [{ "id": "A" }] }]));
  assert_eq!(v(&qs)[0]["options"].as_array().unwrap().len(), 1);
  assert!(spare_message("Codex needs your input to continue.", &qs).is_none());
  expect_eq(form_content(&schema, &qs, &answers(json!({ "q": "B" }))), json!({ "q": "None of the above", "q_note": "B" }));
  expect_eq(form_content(&schema, &qs, &answers(json!({ "q": "A" }))), json!({ "q": "A" }));
}

fn questions(s: &acpira_host::acp::session::AcpSession) -> Vec<Value> {
  agent_blocks(&view(s)).into_iter().filter(|b| b["type"] == "question").collect()
}

fn pending(s: &acpira_host::acp::session::AcpSession) -> Option<Value> {
  questions(s).into_iter().find(|b| b["outcome"].is_null())
}

fn reply(s: &acpira_host::acp::session::AcpSession) -> Value {
  let text: String = agent_blocks(&view(s)).iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap().to_owned()).collect();
  serde_json::from_str(&text).expect("the fake echoes the answer as JSON")
}

async fn asked(h: &Harness, script: &str) -> (Disposing, tokio::task::JoinHandle<()>, Value) {
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let p = spawn_prompt(&s, script);
  until(|| pending(&s).is_some(), 5000).await;
  let card = pending(&s).unwrap();
  (s, p, card)
}

fn answers(j: Value) -> acpira_shared::transcript::QuestionAnswers {
  serde_json::from_value(j).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn grok_turns_the_request_into_a_card_and_answers_go_back_keyed_by_question_text() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let (s, p, card) = asked(&h, "ask-grok").await;
  assert_eq!(card["toolCallId"], "ask1");
  let shape: Vec<Value> = card["questions"].as_array().unwrap().iter().map(|q| json!([q["id"], q["kind"], q["other"]])).collect();
  assert_eq!(shape, [json!(["What should the file be called?", "single", true]), json!(["Where should it go?", "multiple", true])]);
  expect_eq(&card["questions"][0]["options"][0], json!({ "id": "report", "label": "report", "description": "A generic report" }));
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "activity": { "label": "Waiting for your answers" } }));
  s.answer_questions(card["id"].as_str().unwrap(), &answers(json!({ "What should the file be called?": "my-own-name", "Where should it go?": ["src", "docs"] })), false);
  p.await.unwrap();
  expect_eq(reply(&s), json!({ "outcome": "accepted", "answers": { "What should the file be called?": "my-own-name", "Where should it go?": ["src", "docs"] } }));
  // The card stays in the transcript as the record, and is persisted
  expect_match(&questions(&s)[0], json!({ "outcome": "answered", "answers": { "What should the file be called?": "my-own-name" } }));
  assert!(s.to_record().turns.iter().any(|t| v(t)["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["type"] == "question" && b["outcome"] == "answered"))));
  let tool = agent_blocks(&view(&s)).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&tool, json!({ "verbKey": "verb.ask", "status": "completed" }));
  expect_absent(&tool, "target");
}

#[tokio::test(flavor = "multi_thread")]
async fn grok_skip_sends_skip_interview_with_what_was_answered() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let (s, p, card) = asked(&h, "ask-grok").await;
  s.answer_questions(card["id"].as_str().unwrap(), &answers(json!({ "What should the file be called?": "report", "Where should it go?": [] })), true);
  p.await.unwrap();
  expect_eq(reply(&s), json!({ "outcome": "skip_interview", "partial_answers": { "What should the file be called?": "report" } }));
  expect_match(&questions(&s)[0], json!({ "outcome": "skipped", "answers": { "What should the file be called?": "report" } }));
}

#[tokio::test(flavor = "multi_thread")]
async fn devins_form_is_enriched_from_the_tool_input_and_answered_typed() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let (s, p, card) = asked(&h, "ask-devin").await;
  expect_absent(&card, "toolCallId");
  expect_absent(&card, "message");
  expect_match(&card["questions"][0], json!({ "id": "q0", "title": "Name", "text": "What should the file be called?", "kind": "single", "other": true, "required": true,
    "options": [{ "id": "report", "label": "report", "description": "A generic report" }, { "id": "notes", "label": "notes", "description": "Loose notes" }] }));
  expect_match(&card["questions"][1], json!({ "id": "q1", "kind": "multiple", "options": [{ "id": "src", "label": "src" }, { "id": "docs", "label": "docs" }] }));
  s.answer_questions(card["id"].as_str().unwrap(), &answers(json!({ "q0": "report", "q1": ["docs"] })), false);
  p.await.unwrap();
  expect_eq(reply(&s), json!({ "action": "accept", "content": { "q0": "report", "q1": ["docs"] } }));
}

#[tokio::test(flavor = "multi_thread")]
async fn kimi_reads_question_texts_from_the_message_and_skipping_an_untouched_form_declines() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let (s, p, card) = asked(&h, "ask-kimi").await;
  assert_eq!(card["toolCallId"], "ask1");
  assert_eq!(card["questions"].as_array().unwrap().iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("What should the file be called?"), json!("Where should it go?")]);
  assert!(card["questions"].as_array().unwrap().iter().all(|q| q["other"] != true));
  s.answer_questions(card["id"].as_str().unwrap(), &answers(json!({})), true);
  p.await.unwrap();
  expect_eq(reply(&s), json!({ "action": "decline" }));
  expect_match(&questions(&s)[0], json!({ "outcome": "skipped" }));
  expect_absent(&questions(&s)[0], "answers");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partially_answered_form_is_accepted_as_far_as_it_goes_even_when_skipped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let (s, p, card) = asked(&h, "ask-devin").await;
  s.answer_questions(card["id"].as_str().unwrap(), &answers(json!({ "q0": "something else" })), true);
  p.await.unwrap();
  expect_eq(reply(&s), json!({ "action": "accept", "content": { "q0": "something else" } }));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_turn_closes_the_card_as_cancelled_and_a_late_answer_is_ignored() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let (s, p, _) = asked(&h, "ask-kimi").await;
  s.cancel().await;
  p.await.unwrap();
  expect_match(&questions(&s)[0], json!({ "outcome": "cancelled" }));
  assert!(pending(&s).is_none());
  assert_eq!(view(&s)["running"], false);
  let id = questions(&s)[0]["id"].as_str().unwrap().to_owned();
  s.answer_questions(&id, &answers(json!({ "q0": "report" })), false);
  expect_absent(&questions(&s)[0], "answers");
  let _ = prompt;
}
