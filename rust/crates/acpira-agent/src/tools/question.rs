//! `question`: ask the user one to four questions on the host's question card. The turn runs it itself
//! (`turn::question`): the questions go out as an `elicitation/create` form tied to the tool call (Kimi's dialect,
//! `docs/dev/protocol-gotchas.md`), one property per question, and the answers come back to the model as plain lines.
//! The call's rawInput keeps the model's own `header` / `question` / `options[{ label, description }]`, which the host
//! remembers per tool call and reads the card's labels from

use serde_json::{Map, Value, json};

use super::Action;
use crate::llm::ToolSpec;

pub const MAX_QUESTIONS: usize = 4;
pub const MAX_OPTIONS: usize = 8;

/// The `_meta` key that lets the user type an answer of their own beside the choices
pub const ALLOW_OTHER: &str = "acpira/allowOther";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
  pub label: String,
  pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
  pub question: String,
  /// A short label above the question
  pub header: Option<String>,
  /// Empty: a free-text answer
  pub options: Vec<Choice>,
  /// Several options may be picked
  pub multiple: bool,
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::QUESTION.into(),
    description: "Ask the user one to four questions and wait for the answers. Use it when a decision is the user's to make \
                  and changes what you do next; do not ask what you can find out with the other tools. Give each question two \
                  to four options with the one you recommend first; the user can always type an answer of their own. Leave \
                  options out for a question that needs a free-text answer."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "questions": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "question": { "type": "string", "description": "The full question" },
              "header": { "type": "string", "description": "A label of one to three words shown above it" },
              "options": {
                "type": "array",
                "items": {
                  "type": "object",
                  "properties": {
                    "label": { "type": "string", "description": "The answer as the user picks it, a few words" },
                    "description": { "type": "string", "description": "What choosing it means" },
                  },
                  "required": ["label"],
                },
              },
              "multiple": { "type": "boolean", "description": "Several options may be picked" },
            },
            "required": ["question"],
          },
        },
      },
      "required": ["questions"],
    }),
  }
}

fn text(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

pub fn prepare(args: &Value) -> Result<Action, String> {
  let list = args.get("questions").and_then(Value::as_array).ok_or("Missing required array argument \"questions\"")?;
  if list.is_empty() || list.len() > MAX_QUESTIONS {
    return Err(format!("Ask between 1 and {MAX_QUESTIONS} questions at a time, not {}", list.len()));
  }
  let mut questions = vec![];
  for (i, item) in list.iter().enumerate() {
    let question = text(item.get("question")).ok_or_else(|| format!("questions[{i}] needs a non-empty \"question\""))?;
    let mut options: Vec<Choice> = vec![];
    for (j, o) in item.get("options").and_then(Value::as_array).into_iter().flatten().enumerate() {
      // Models send plain strings for options too
      let (label, description) = match o {
        Value::String(_) => (text(Some(o)), None),
        _ => (text(o.get("label")), text(o.get("description"))),
      };
      let label = label.ok_or_else(|| format!("questions[{i}].options[{j}] needs a non-empty \"label\""))?;
      if options.iter().any(|c| c.label == label) {
        return Err(format!("questions[{i}] lists the option \"{label}\" twice"));
      }
      options.push(Choice { description: description.filter(|d| *d != label), label });
    }
    if options.len() > MAX_OPTIONS {
      return Err(format!("questions[{i}] has {} options; give at most {MAX_OPTIONS}", options.len()));
    }
    let multiple = matches!(item.get("multiple").or_else(|| item.get("multiSelect")), Some(Value::Bool(true))) && !options.is_empty();
    questions.push(Question { question, header: text(item.get("header")), options, multiple });
  }
  Ok(Action::Question { questions })
}

/// The tool card's title, which the host reads as an ask row (`normalize/tool_kind.rs` `ASK_TITLE`)
pub fn title(n: usize) -> String {
  if n == 1 { "Ask 1 question".to_owned() } else { format!("Ask {n} questions") }
}

/// The form property of the n-th question
fn key(i: usize) -> String {
  format!("q{i}")
}

/// The `elicitation/create` params for these questions, tied to the tool call
pub fn form(session_id: &str, tool_call_id: &str, questions: &[Question]) -> Value {
  let mut props = Map::new();
  for (i, q) in questions.iter().enumerate() {
    let choices: Vec<Value> = q
      .options
      .iter()
      .map(|c| {
        let mut e = json!({ "const": c.label, "title": c.label });
        if let Some(d) = &c.description {
          e["description"] = json!(d);
        }
        e
      })
      .collect();
    let mut prop = if q.options.is_empty() {
      json!({ "type": "string" })
    } else if q.multiple {
      json!({ "type": "array", "items": { "anyOf": choices }, "_meta": { ALLOW_OTHER: true } })
    } else {
      json!({ "type": "string", "oneOf": choices, "_meta": { ALLOW_OTHER: true } })
    };
    prop["description"] = json!(q.question);
    if let Some(h) = &q.header {
      prop["title"] = json!(h);
    }
    props.insert(key(i), prop);
  }
  let message = questions.iter().map(|q| q.question.as_str()).collect::<Vec<_>>().join("\n");
  json!({
    "sessionId": session_id,
    "toolCallId": tool_call_id,
    "mode": "form",
    "message": message,
    "requestedSchema": { "type": "object", "properties": props },
  })
}

/// How the user left the card
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
  /// The model's copy of the answers (some may be missing)
  Answered(String),
  /// Skipped without a single answer
  Skipped,
  /// Closed or stopped: the turn ends
  Dismissed,
}

/// The client's answer to the form, for the model
pub fn reply(questions: &[Question], response: &Value) -> Reply {
  match response.get("action").and_then(Value::as_str) {
    Some("accept") => {}
    Some("decline") => return Reply::Skipped,
    _ => return Reply::Dismissed,
  }
  let content = response.get("content");
  let mut lines = vec![];
  let mut answered = 0;
  for (i, q) in questions.iter().enumerate() {
    let answer = match content.and_then(|c| c.get(key(i))) {
      Some(Value::String(s)) => Some(s.trim().to_owned()),
      Some(Value::Array(items)) => Some(items.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")),
      Some(Value::Bool(b)) => Some(b.to_string()),
      Some(Value::Number(n)) => Some(n.to_string()),
      _ => None,
    }
    .filter(|a| !a.is_empty());
    answered += usize::from(answer.is_some());
    lines.push(format!("Q: {}\nA: {}", q.question, answer.as_deref().unwrap_or("(no answer)")));
  }
  if answered == 0 {
    return Reply::Skipped;
  }
  Reply::Answered(format!("The user answered:\n\n{}", lines.join("\n\n")))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn qs(args: Value) -> Vec<Question> {
    match prepare(&json!({ "questions": args })).unwrap() {
      Action::Question { questions } => questions,
      other => panic!("{other:?}"),
    }
  }

  #[test]
  fn prepare_takes_string_options_and_refuses_what_the_card_cannot_show() {
    let q = qs(json!([{ "question": " Which? ", "header": "Pick", "options": ["a", { "label": "b", "description": "the second" }], "multiSelect": true }, { "question": "Name?" }]));
    assert_eq!(q[0], Question {
      question: "Which?".into(),
      header: Some("Pick".into()),
      options: vec![Choice { label: "a".into(), description: None }, Choice { label: "b".into(), description: Some("the second".into()) }],
      multiple: true,
    });
    assert!(q[1].options.is_empty() && !q[1].multiple);
    for bad in [json!([]), json!([{ "question": "" }]), json!([{ "question": "x", "options": ["a", "a"] }]), json!(vec![json!({ "question": "x" }); 5])] {
      assert!(prepare(&json!({ "questions": bad })).is_err(), "{bad}");
    }
  }

  #[test]
  fn the_form_has_one_property_per_question_and_answers_come_back_as_lines() {
    let q = qs(json!([{ "question": "Which?", "header": "Pick", "options": [{ "label": "a", "description": "first" }] }, { "question": "Where?", "options": ["x", "y"], "multiple": true }, { "question": "Name?" }]));
    let f = form("s", "call_1", &q);
    assert_eq!(f["toolCallId"], "call_1");
    assert_eq!(f["message"], "Which?\nWhere?\nName?");
    let p = &f["requestedSchema"]["properties"];
    assert_eq!(p["q0"], json!({ "type": "string", "title": "Pick", "description": "Which?", "oneOf": [{ "const": "a", "title": "a", "description": "first" }], "_meta": { ALLOW_OTHER: true } }));
    assert_eq!(p["q1"]["items"]["anyOf"][1], json!({ "const": "y", "title": "y" }));
    assert_eq!(p["q2"], json!({ "type": "string", "description": "Name?" }));
    let answered = reply(&q, &json!({ "action": "accept", "content": { "q0": "my own", "q1": ["x", "y"] } }));
    assert_eq!(answered, Reply::Answered("The user answered:\n\nQ: Which?\nA: my own\n\nQ: Where?\nA: x, y\n\nQ: Name?\nA: (no answer)".into()));
    assert_eq!(reply(&q, &json!({ "action": "accept", "content": {} })), Reply::Skipped);
    assert_eq!(reply(&q, &json!({ "action": "decline" })), Reply::Skipped);
    assert_eq!(reply(&q, &json!({ "action": "cancel" })), Reply::Dismissed);
  }
}
