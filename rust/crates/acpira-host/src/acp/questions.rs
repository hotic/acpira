//! Structured questions: elicitation forms and Grok's question
//! request become one QuestionBlock; answers go back typed the way the schema declared them. The pending-card
//! bookkeeping lives in the session, under its lock

use serde_json::{Map, Value, json};

use acpira_shared::transcript::{Question, QuestionAnswers, QuestionKind, QuestionOption};

use crate::i18n::t;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawOption {
  pub label: String,
  pub description: Option<String>,
}

/// The agent's own tool input, the richer source for labels than the flattened form schema
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawQuestion {
  pub header: Option<String>,
  pub question: Option<String>,
  pub options: Option<Vec<RawOption>>,
}

const RAW_KEEP: usize = 8;

/// Remembered ask-user-question tool inputs, a handful is plenty
#[derive(Default)]
pub struct RawMemory {
  items: Vec<(String, Vec<RawQuestion>)>,
}

impl RawMemory {
  pub fn remember(&mut self, u: &Value) {
    let Some(questions) = u.get("rawInput").and_then(|r| r.get("questions")).and_then(Value::as_array) else { return };
    let id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    let parsed: Vec<RawQuestion> = questions
      .iter()
      .filter_map(Value::as_object)
      .map(|item| RawQuestion {
        header: item.get("header").and_then(Value::as_str).map(str::to_owned),
        question: item.get("question").and_then(Value::as_str).map(str::to_owned),
        options: item.get("options").and_then(Value::as_array).map(|opts| {
          opts
            .iter()
            .filter_map(|o| {
              let label = o.get("label")?.as_str()?.to_owned();
              Some(RawOption { label, description: o.get("description").and_then(Value::as_str).map(str::to_owned) })
            })
            .collect()
        }),
      })
      .collect();
    self.items.retain(|(i, _)| *i != id);
    self.items.push((id, parsed));
    if self.items.len() > RAW_KEEP {
      let drop = self.items.len() - RAW_KEEP;
      self.items.drain(..drop);
    }
  }

  /// Devin sends no toolCallId with the form: fall back to the latest input with the same number of questions
  pub fn for_call(&self, tool_call_id: Option<&str>, count: usize) -> Option<Vec<RawQuestion>> {
    if let Some(id) = tool_call_id.filter(|x| !x.is_empty()) {
      return self.items.iter().find(|(i, _)| i == id).map(|(_, q)| q.clone());
    }
    self.items.iter().rev().find(|(_, q)| q.len() == count).map(|(_, q)| q.clone())
  }
}

fn nonblank(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned)
}

fn plain(values: &[Value]) -> Vec<QuestionOption> {
  values
    .iter()
    .filter_map(|v| match v {
      Value::String(s) => Some(s.clone()),
      Value::Number(n) => Some(crate::json::pretty(&Value::Number(n.clone()))),
      _ => None,
    })
    .map(|s| QuestionOption { id: s.clone(), label: s, description: None })
    .collect()
}

fn titled(entries: &[Value], rq: Option<&RawQuestion>) -> Vec<QuestionOption> {
  entries
    .iter()
    .filter_map(|e| {
      let konst = e.get("const")?.as_str()?.to_owned();
      let ro = rq.and_then(|r| r.options.as_ref()).and_then(|opts| opts.iter().find(|o| o.label == konst));
      let label = ro.map(|o| o.label.clone()).or_else(|| nonblank(e.get("title"))).unwrap_or_else(|| konst.clone());
      let description = match ro {
        Some(o) => o.description.clone(),
        None => nonblank(e.get("description")),
      };
      Some(QuestionOption { id: konst, description: description.filter(|d| !d.is_empty() && *d != label), label })
    })
    .collect()
}

/// One question per form property, in schema order
pub fn form_questions(schema: &Value, message: &str, meta: Option<&Value>, raw: Option<&[RawQuestion]>) -> Vec<Question> {
  let empty = Map::new();
  let props = schema.get("properties").and_then(Value::as_object).unwrap_or(&empty);
  let keys: Vec<&String> = props.keys().collect();
  let required: Vec<&str> =
    schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
  let allow_other = meta.and_then(|m| m.get("cognition.ai/allowOther")) == Some(&Value::Bool(true));
  let lines: Vec<&str> = message.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
  let aligned = raw.filter(|r| r.len() == keys.len());
  let mut out = vec![];
  for (i, key) in keys.iter().enumerate() {
    let Some(prop) = props.get(*key).filter(|p| p.is_object()) else { continue };
    let rq = aligned.and_then(|a| a.get(i));
    let ptitle = nonblank(prop.get("title"));
    let pdesc = nonblank(prop.get("description"));
    let usable = rq.filter(|rq| {
      (ptitle.is_none() || rq.header.is_none() || ptitle == rq.header) && (pdesc.is_none() || rq.question.is_none() || pdesc == rq.question)
    });
    let title = ptitle.clone().or_else(|| usable.and_then(|u| u.header.clone()));
    let text = pdesc
      .clone()
      .or_else(|| usable.and_then(|u| u.question.clone()))
      .or_else(|| (lines.len() == keys.len()).then(|| lines[i].to_owned()))
      .or_else(|| if keys.len() == 1 { nonblank(Some(&Value::from(message))) } else { None })
      .or_else(|| title.clone())
      .unwrap_or_else(|| (*key).clone());
    let ptype = prop.get("type").and_then(Value::as_str);
    let mut q = Question {
      id: (*key).clone(),
      title: title.filter(|t| *t != text),
      text,
      kind: QuestionKind::Single,
      options: vec![],
      other: None,
      numeric: None,
      required: required.contains(&key.as_str()).then_some(true),
    };
    let items = if ptype == Some("array") { prop.get("items").filter(|i| i.is_object()) } else { None };
    if ptype == Some("boolean") {
      q.options = vec![
        QuestionOption { id: "true".into(), label: t("question.yes"), description: None },
        QuestionOption { id: "false".into(), label: t("question.no"), description: None },
      ];
    } else if let Some(items) = items {
      q.kind = QuestionKind::Multiple;
      q.options = if let Some(any) = items.get("anyOf").and_then(Value::as_array) {
        titled(any, usable)
      } else if let Some(e) = items.get("enum").and_then(Value::as_array) {
        plain(e)
      } else {
        vec![]
      };
    } else if let Some(one) = prop.get("oneOf").and_then(Value::as_array) {
      q.options = titled(one, usable);
    } else if let Some(e) = prop.get("enum").and_then(Value::as_array) {
      q.options = plain(e);
    } else {
      q.kind = QuestionKind::Text;
      if matches!(ptype, Some("number" | "integer")) {
        q.numeric = Some(true);
      }
    }
    let prop_other = prop.get("_meta").and_then(|m| m.get("cognition.ai/allowOther")) == Some(&Value::Bool(true));
    if q.kind != QuestionKind::Text && ptype != Some("boolean") && (allow_other || prop_other) {
      q.other = Some(true);
    }
    if q.kind != QuestionKind::Text && q.options.is_empty() {
      q.kind = QuestionKind::Text;
    }
    out.push(q);
  }
  out
}

/// Grok's question request → questions keyed by question text
pub fn grok_questions(req: &Value) -> Vec<Question> {
  req
    .get("questions")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .map(|q| {
      let text = q.get("question").and_then(Value::as_str).unwrap_or("").to_owned();
      Question {
        id: text.clone(),
        title: None,
        kind: if q.get("multiSelect") == Some(&Value::Bool(true)) { QuestionKind::Multiple } else { QuestionKind::Single },
        options: q
          .get("options")
          .and_then(Value::as_array)
          .into_iter()
          .flatten()
          .map(|o| {
            let label = o.get("label").and_then(Value::as_str).unwrap_or("").to_owned();
            let description = o.get("description").and_then(Value::as_str).filter(|d| !d.is_empty() && *d != label).map(str::to_owned);
            QuestionOption { id: label.clone(), label, description }
          })
          .collect(),
        text,
        other: Some(true),
        numeric: None,
        required: None,
      }
    })
    .collect()
}

/// The form's message earns a line of its own only when it says something the questions don't
pub fn spare_message(message: &str, questions: &[Question]) -> Option<String> {
  let m = message.trim();
  if m.is_empty() {
    return None;
  }
  let texts: Vec<&str> = questions.iter().map(|q| q.text.trim()).collect();
  let joined_lines: Vec<&str> = m.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
  if texts.contains(&m) || texts.join("\n") == joined_lines.join("\n") {
    return None;
  }
  Some(m.to_owned())
}

/// Keep only answers to known questions with something in them
pub fn clean_answers(questions: &[Question], answers: &QuestionAnswers) -> QuestionAnswers {
  let mut out = QuestionAnswers::new();
  for q in questions {
    let Some(a) = answers.get(&q.id) else { continue };
    let list: Vec<String> = match a {
      Value::Array(items) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
      Value::String(s) => vec![s.clone()],
      _ => continue,
    };
    if q.kind == QuestionKind::Multiple {
      let list: Vec<Value> = list.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(Value::from).collect();
      if !list.is_empty() {
        out.insert(q.id.clone(), Value::Array(list));
      }
    } else {
      let s = list.join(", ");
      let s = s.trim();
      if !s.is_empty() {
        out.insert(q.id.clone(), Value::from(s));
      }
    }
  }
  out
}

/// Answers typed per the schema: booleans and numbers converted, lists stay lists, everything else a string
pub fn form_content(schema: &Value, questions: &[Question], answers: &QuestionAnswers) -> Value {
  let mut content = Map::new();
  for q in questions {
    let Some(a) = answers.get(&q.id) else { continue };
    if a.is_array() {
      content.insert(q.id.clone(), a.clone());
      continue;
    }
    let s = a.as_str().unwrap_or("");
    let ptype = schema.get("properties").and_then(|p| p.get(&q.id)).and_then(|p| p.get("type")).and_then(Value::as_str);
    if ptype == Some("boolean") {
      content.insert(q.id.clone(), Value::Bool(s == "true"));
    } else if q.numeric == Some(true) {
      if let Some(n) = js_number(s) {
        content.insert(q.id.clone(), acpira_shared::num::js_number(n));
      }
    } else {
      content.insert(q.id.clone(), Value::from(s));
    }
  }
  Value::Object(content)
}

/// Number(s) for the shapes a numeric answer takes; None where JS would give NaN / Infinity
fn js_number(s: &str) -> Option<f64> {
  let t = s.trim();
  if t.is_empty() {
    return Some(0.0);
  }
  t.parse::<f64>().ok().filter(|n| n.is_finite())
}

pub fn grok_response(skip: bool, answers: &QuestionAnswers) -> Value {
  let empty = answers.is_empty();
  if skip || empty {
    if empty { json!({ "outcome": "skip_interview" }) } else { json!({ "outcome": "skip_interview", "partial_answers": answers }) }
  } else {
    json!({ "outcome": "accepted", "answers": answers })
  }
}
