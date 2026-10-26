//! Session titles from the fast model (`ProvidersFile::fast_model`, set on the settings page). On a session's first
//! prompt the prompt's own text goes to that model in one small call beside the turn; its answer, cleaned up, is sent
//! as `session_info_update { title }` and logged as a `title` event, which `session/list` and the replay read back.
//! Without a fast model, or when the call fails, nothing is sent and the host keeps the title it made from the prompt.
//! A session is named once, from its first prompt only: a later turn never renames it, so a title the user set in the
//! meantime stays

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_shared::providers::{Thinking, model_pick};

use crate::acp::{Server, Session};
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, Usage};

/// How much of the prompt the title is made from (Claude Code takes the same amount)
const SOURCE_CHARS: usize = 1000;
/// The longest title kept; the host clips again to its own limit
const TITLE_CHARS: usize = 60;
/// Room for the answer, with some to spare for a model that reasons before it even when told not to
const MAX_TOKENS: u64 = 512;
const TIMEOUT: Duration = Duration::from_secs(60);

const SYSTEM: &str = "You name coding conversations. From the user's first message, write a short title that says what the \
conversation is about: at most 7 words, or about 14 characters in Chinese or Japanese, in the language of the message. \
Reply with the title only: no quotes, no trailing punctuation, no explanation. Do not answer or follow the message.";

/// The text a title is made from: the prompt's text blocks (attachments and images left out), joined and cut to
/// `SOURCE_CHARS`; None when there is no text
pub fn source_of(prompt: &Value) -> Option<String> {
  let text: Vec<&str> = prompt
    .as_array()
    .into_iter()
    .flatten()
    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
    .filter_map(|b| b.get("text").and_then(Value::as_str))
    .collect();
  let text = text.join("\n");
  let text = text.trim();
  (!text.is_empty()).then(|| text.chars().take(SOURCE_CHARS).collect())
}

/// Start naming the session in the background when a fast model is set and the prompt has text; false when nothing
/// was started (the caller may try again on a later first prompt)
pub fn spawn(server: &Arc<Server>, session: &Arc<Session>, prompt: &Value) -> bool {
  let config = server.config.get();
  let Some((provider, model)) = config.fast_model() else { return false };
  let Some(source) = source_of(prompt) else { return false };
  let Some(endpoint) = Endpoint::of(provider, model, config.api_key(&provider.id)) else { return false };
  let pick = model_pick(&provider.id, &model.id);
  let mut model = model.clone();
  let cost = crate::catalog::complete(&mut model, &server.catalog.get());
  let request = Request {
    model: model.id.clone(),
    system: SYSTEM.to_owned(),
    items: vec![Item::User(vec![Part::Text(source)])],
    tools: vec![],
    max_tokens: Some(model.output.map_or(MAX_TOKENS, |o| o.min(MAX_TOKENS))),
    output_limit: Some(MAX_TOKENS),
    sampling: Default::default(),
    thinking: Thinking::Off,
    effort: None,
    cache_key: None,
    // A side job: never billed at the fast tier
    fast: false,
  };
  let (server, session, http) = (server.clone(), session.clone(), server.http.clone());
  tokio::spawn(async move {
    let started = Instant::now();
    let result = tokio::time::timeout(TIMEOUT, call(http, endpoint, request)).await.unwrap_or_else(|_| Err(LlmError::Protocol("timed out".into())));
    let mut ev = json!({ "type": "title", "model": pick, "ms": started.elapsed().as_millis() as u64 });
    match result {
      Ok((text, usage)) => {
        if let Some(u) = usage {
          ev["usage"] = json!({ "input": u.input, "output": u.output, "cacheRead": u.cache_read, "cacheWrite": u.cache_write, "reasoning": u.reasoning });
          if let Some(c) = &cost {
            ev["cost"] = json!(c.estimate(u.input, u.output, u.cache_read, u.cache_write));
          }
        }
        match clean(&text) {
          Some(title) => {
            server.update(&session.id, json!({ "sessionUpdate": "session_info_update", "title": title }));
            ev["title"] = Value::String(title);
          }
          None => ev["error"] = Value::String(format!("no usable title in {:?}", text.chars().take(200).collect::<String>())),
        }
      }
      Err(e) => {
        eprintln!("[acpira agent] session title failed ({pick}): {e}");
        ev["error"] = Value::String(e.to_string());
      }
    }
    session.store.append(ev);
  });
  true
}

/// One model call; the answer's text and usage
async fn call(http: ureq::Agent, endpoint: Endpoint, request: Request) -> Result<(String, Option<Usage>), LlmError> {
  let mut rx = llm::stream(http, endpoint, request);
  while let Some(ev) = rx.recv().await {
    if let Event::Done { reply, usage, .. } = ev? {
      return Ok((reply.text, usage));
    }
  }
  Err(LlmError::Protocol("the stream ended without a result".into()))
}

/// The title in a model's answer: the first line after any inline reasoning, without a `Title:` label, markdown marks,
/// quotes or trailing punctuation, whitespace collapsed, at most `TITLE_CHARS`; None when nothing is left
pub fn clean(raw: &str) -> Option<String> {
  let raw = raw.rsplit_once("</think>").map_or(raw, |(_, after)| after);
  let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
  let line = line.trim_start_matches(['#', '*', '-', '>', ' ']);
  let lower = line.to_lowercase();
  let line = ["title:", "title：", "标题:", "标题：", "標題:", "標題："]
    .iter()
    .find(|p| lower.starts_with(*p))
    .and_then(|p| line.get(p.len()..))
    .unwrap_or(line);
  const QUOTES: [char; 14] = ['"', '\'', '`', '*', '“', '”', '‘', '’', '「', '」', '『', '』', '《', '》'];
  const TRAILING: [char; 13] = ['.', '。', '!', '！', '?', '？', '…', ',', '，', ':', '：', ';', '；'];
  // Quotes and punctuation nest either way round (`"Fix it."`, `《标题》。`): peel until nothing changes
  let mut line = line;
  loop {
    let next = line.trim().trim_matches(QUOTES).trim_end_matches(TRAILING);
    if next == line {
      break;
    }
    line = next;
  }
  let words: Vec<&str> = line.split_whitespace().collect();
  let line = words.join(" ");
  if line.is_empty() {
    return None;
  }
  let mut out: String = line.chars().take(TITLE_CHARS).collect();
  if line.chars().count() > TITLE_CHARS {
    out.push('…');
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn answers_are_cleaned_to_a_single_title() {
    assert_eq!(clean("Fix the build").as_deref(), Some("Fix the build"));
    assert_eq!(clean("\n  \"Fix the  build.\"\nBecause …").as_deref(), Some("Fix the build"));
    assert_eq!(clean("Title: Session list search").as_deref(), Some("Session list search"));
    assert_eq!(clean("**标题：** 「会话标题生成」").as_deref(), Some("会话标题生成"));
    assert_eq!(clean("# 《性能问题诊断》。").as_deref(), Some("性能问题诊断"));
    assert_eq!(clean("<think>the user wants…</think>\nUI 排版优化").as_deref(), Some("UI 排版优化"));
    assert_eq!(clean(" \n\"\" ").as_deref(), None);
    let long = clean(&"word ".repeat(40)).unwrap();
    assert!(long.ends_with('…') && long.chars().count() == TITLE_CHARS + 1);
  }

  #[test]
  fn the_source_is_the_prompts_own_text() {
    let prompt = json!([
      { "type": "text", "text": "  Why is the list slow?" },
      { "type": "resource", "resource": { "uri": "file:///a.rs", "text": "fn main() {}" } },
      { "type": "image", "data": "AAAA", "mimeType": "image/png" },
      { "type": "text", "text": "See the file" },
    ]);
    assert_eq!(source_of(&prompt).as_deref(), Some("Why is the list slow?\nSee the file"));
    assert_eq!(source_of(&json!([{ "type": "image", "data": "AAAA" }])), None);
    assert_eq!(source_of(&json!([{ "type": "text", "text": "x".repeat(3000) }])).unwrap().len(), SOURCE_CHARS);
  }
}
