//! The engine's plugins hooked into this agent's loop (docs/dev/plugins.md, Built-in agent hooks). The engine answers
//! two requests: `_acpira/pluginHooks` (which of `NAMES` some plugin's process declared, asked once per turn) and
//! `_acpira/pluginHook` `{ sessionId, hook, payload }` (the payload as the plugins left it: each answer's fields replace
//! the payload's, `block` stops the chain). A client that is not the engine answers neither, and nothing changes
//!
//! Only declared hooks are asked, so a session no plugin hooks into sends no extra message but the one per turn

use serde_json::{Value, json};

use crate::acp::{Server, Session};
use crate::llm::{Item, Part, Request, ToolCall, ToolSpec};

pub const HOOKS: &str = "_acpira/pluginHooks";
pub const HOOK: &str = "_acpira/pluginHook";
/// The hooks of this loop a plugin may declare
pub const NAMES: &[&str] = &["context", "model.request", "compaction", "shell", "tools", "tool.before", "tool.after", "websearch"];

/// Which hooks the engine's plugins declare now; asked at the start of every turn
pub async fn refresh(server: &Server, session: &Session) {
  let declared = match server.conn().request(HOOKS, json!({ "sessionId": session.id })).await {
    Ok(v) => v.get("hooks").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).filter(|h| NAMES.contains(h)).map(str::to_owned).collect()).unwrap_or_default(),
    Err(_) => vec![],
  };
  session.state.lock().plugin_hooks = declared;
}

pub fn has(session: &Session, hook: &str) -> bool {
  session.state.lock().plugin_hooks.iter().any(|h| h == hook)
}

/// The payload as the plugins left it, or None when `hook` is not declared or the engine could not be asked
pub async fn ask(server: &Server, session: &Session, hook: &str, payload: Value) -> Option<Value> {
  if !has(session, hook) {
    return None;
  }
  let out = server.conn().request(HOOK, json!({ "sessionId": session.id, "hook": hook, "payload": payload })).await.ok()?;
  out.is_object().then_some(out)
}

/// Why a plugin stopped the chain, when one did
pub fn blocked(out: &Value) -> Option<String> {
  out.get("block").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned)
}

/// One model call as a `context` hook sees it. Images go without their data, and each item carries its index so an
/// answer can keep it as it was (`{ "keep": 3 }`), which also keeps what a provider must get back verbatim
pub fn request_json(r: &Request) -> Value {
  let items: Vec<Value> = r.items.iter().enumerate().map(|(i, item)| item_json(i, item)).collect();
  json!({
    "system": r.system,
    "items": items,
    "tools": tools_json(&r.tools),
    "options": {
      "model": r.model,
      "maxTokens": r.max_tokens,
      "temperature": r.sampling.temperature,
      "topP": r.sampling.top_p,
      "topK": r.sampling.top_k,
      "effort": r.effort,
    },
  })
}

pub fn tools_json(tools: &[ToolSpec]) -> Value {
  Value::Array(tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "parameters": t.parameters })).collect())
}

/// An answer's `tools` applied: the listed tools in its order, each with its `description` / `parameters` when given;
/// a tool not in the list is dropped, and a name the loop has no tool for is ignored (it could not run)
pub fn apply_tools(tools: &mut Vec<ToolSpec>, out: &Value) {
  let Some(list) = out.get("tools").and_then(Value::as_array) else { return };
  let old = std::mem::take(tools);
  *tools = list
    .iter()
    .filter_map(|t| {
      let base = old.iter().find(|o| Some(o.name.as_str()) == t.get("name").and_then(Value::as_str))?;
      Some(ToolSpec {
        name: base.name.clone(),
        description: t.get("description").and_then(Value::as_str).map_or_else(|| base.description.clone(), str::to_owned),
        parameters: t.get("parameters").filter(|p| p.is_object()).cloned().unwrap_or_else(|| base.parameters.clone()),
      })
    })
    .collect();
}

/// A `model.request` answer's `headers` (string values only)
pub fn headers_of(out: &Value) -> Vec<(String, String)> {
  out.get("headers").and_then(Value::as_object).into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()
}

fn item_json(index: usize, item: &Item) -> Value {
  match item {
    Item::User(parts) => {
      let parts: Vec<Value> = parts
        .iter()
        .map(|p| match p {
          Part::Text(t) => json!({ "type": "text", "text": t }),
          Part::Image { mime, .. } => json!({ "type": "image", "mime": mime }),
        })
        .collect();
      json!({ "index": index, "role": "user", "parts": parts })
    }
    Item::Assistant { text, reasoning, tool_calls, .. } => {
      let calls: Vec<Value> = tool_calls.iter().map(|c| json!({ "id": c.id, "name": c.name, "arguments": c.arguments })).collect();
      json!({ "index": index, "role": "assistant", "text": text, "reasoning": reasoning, "toolCalls": calls })
    }
    Item::ToolResult { call_id, name, content, is_error, images } => {
      json!({ "index": index, "role": "tool", "callId": call_id, "name": name, "content": content, "isError": is_error, "images": images.len() })
    }
  }
}

/// A `context` hook's answer applied to the call: `system`, `items`, `tools` and `options` replace theirs. An item
/// `{ "keep": i }`, or one still exactly as it was sent (the payload's own items come back with every answer), is the
/// call's item `i` unchanged; any other is built from its fields (an assistant message rebuilt so loses what its
/// provider must get back verbatim, and a user message its images). A tool not in the call is dropped: the loop can
/// only run its own tools. `options.model` is shown, not applied: the endpoint is picked before the hook
pub fn apply_request(r: &mut Request, out: &Value) {
  if let Some(system) = out.get("system").and_then(Value::as_str) {
    r.system = system.to_owned();
  }
  if let Some(items) = out.get("items").and_then(Value::as_array) {
    let old = std::mem::take(&mut r.items);
    r.items = items.iter().filter_map(|v| item_of(v, &old)).collect();
  }
  apply_tools(&mut r.tools, out);
  if let Some(o) = out.get("options") {
    if let Some(v) = o.get("maxTokens") {
      r.max_tokens = v.as_u64();
    }
    if let Some(v) = o.get("temperature") {
      r.sampling.temperature = v.as_f64();
    }
    if let Some(v) = o.get("topP") {
      r.sampling.top_p = v.as_f64();
    }
    if let Some(v) = o.get("topK") {
      r.sampling.top_k = v.as_u64();
    }
    if let Some(v) = o.get("effort") {
      r.effort = v.as_str().map(str::to_owned);
    }
  }
}

fn item_of(v: &Value, old: &[Item]) -> Option<Item> {
  if let Some(i) = v.get("keep").and_then(Value::as_u64) {
    return old.get(i as usize).cloned();
  }
  if let Some(i) = v.get("index").and_then(Value::as_u64).map(|i| i as usize)
    && let Some(item) = old.get(i)
    && *v == item_json(i, item)
  {
    return Some(item.clone());
  }
  let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
  match v.get("role").and_then(Value::as_str)? {
    "user" => {
      let parts: Vec<Part> = match v.get("parts").and_then(Value::as_array) {
        Some(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).map(|t| Part::Text(t.to_owned())).collect(),
        None => vec![Part::Text(text("text"))],
      };
      Some(Item::User(parts))
    }
    "assistant" => {
      let tool_calls = v
        .get("toolCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
          Some(ToolCall { id: c.get("id")?.as_str()?.to_owned(), name: c.get("name")?.as_str()?.to_owned(), arguments: c.get("arguments").and_then(Value::as_str).unwrap_or("{}").to_owned() })
        })
        .collect();
      Some(Item::Assistant { text: text("text"), reasoning: text("reasoning"), tool_calls, native: None })
    }
    "tool" => Some(Item::ToolResult {
      call_id: text("callId"),
      name: text("name"),
      content: text("content"),
      is_error: v.get("isError").and_then(Value::as_bool).unwrap_or(false),
      images: vec![],
    }),
    _ => None,
  }
}

/// What the plugins made of one tool call before it runs
#[derive(Debug)]
pub enum Before {
  /// Run it with these arguments
  Run(Value),
  /// Refused, with the reason the model reads
  Block(String),
  /// Not run: the plugin's answer is the result (`tool.before` with `result`, a `websearch` backend)
  Answer(Value, crate::tools::Output),
}

/// A `result` answer as a tool's output; `isError` left out keeps `was_error`
fn answered(out: &Value, was_error: bool) -> Option<crate::tools::Output> {
  let r = out.get("result")?;
  let text = r.get("text").and_then(Value::as_str)?.to_owned();
  let is_error = r.get("isError").and_then(Value::as_bool).unwrap_or(was_error);
  Some(crate::tools::Output { content: vec![crate::tools::text_content(&text)], model: text, is_error, raw_output: None })
}

/// `tool.before` for every call (`{ arguments }`, `{ block }` or `{ result: { text, isError } }`), then `shell` for a
/// command (`{ command, env }` or `{ block }`) and `websearch` for a search (`{ text }`, the results in the model's words)
pub async fn before_call(server: &Server, session: &Session, tool: &str, mut arguments: Value) -> Before {
  if let Some(out) = ask(server, session, "tool.before", json!({ "tool": tool, "arguments": arguments })).await {
    if let Some(reason) = blocked(&out) {
      return Before::Block(format!("Refused by a plugin: {reason}"));
    }
    if let Some(a) = out.get("arguments").filter(|a| a.is_object()) {
      arguments = a.clone();
    }
    if let Some(output) = answered(&out, false) {
      return Before::Answer(arguments, output);
    }
  }
  if tool == crate::tools::BASH
    && let Some(command) = arguments.get("command").and_then(Value::as_str)
  {
    match shell(server, session, command, arguments.get("workdir").cloned(), arguments.get("background").cloned()).await {
      Err(reason) => return Before::Block(reason),
      Ok(Some(line)) => arguments["command"] = Value::String(line),
      Ok(None) => {}
    }
  }
  if tool == crate::tools::WEBSEARCH
    && let Some(out) = ask(server, session, "websearch", json!({ "query": arguments.get("query"), "count": arguments.get("count") })).await
    && let Some(text) = out.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty())
  {
    let output = crate::tools::Output { content: vec![crate::tools::text_content(text)], model: text.to_owned(), is_error: false, raw_output: None };
    return Before::Answer(arguments, output);
  }
  Before::Run(arguments)
}

/// `shell`: the command line to run instead (its variables in front), None when no plugin changed it, or why it is
/// refused
async fn shell(server: &Server, session: &Session, command: &str, workdir: Option<Value>, background: Option<Value>) -> Result<Option<String>, String> {
  let Some(out) = ask(server, session, "shell", json!({ "command": command, "workdir": workdir, "background": background })).await else { return Ok(None) };
  if let Some(reason) = blocked(&out) {
    return Err(format!("Refused by a plugin: {reason}"));
  }
  let line = out.get("command").and_then(Value::as_str).unwrap_or(command);
  let env: Vec<(String, String)> = out.get("env").and_then(Value::as_object).into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect();
  Ok(Some(format!("{}{line}", crate::tools::bash::env_prefix(&env))).filter(|l| l != command))
}

/// The `shell` hook on an edit's `then_run` command, as on a `bash` call: the action to run, or why it is refused
pub async fn shell_rider(server: &Server, session: &Session, action: &crate::tools::Action) -> Result<crate::tools::Action, String> {
  let mut action = action.clone();
  if let crate::tools::Action::Bash { command, workdir, background, .. } = &mut action
    && let Some(line) = shell(server, session, command, Some(json!(workdir)), Some(json!(background))).await?
  {
    *command = line;
  }
  Ok(action)
}

/// `tool.after`: `{ result: { text, isError } }` replaces what the model reads of a finished call. `arguments` are the
/// ones the call ran with, after the `tool.before` and `shell` hooks
pub async fn after_call(server: &Server, session: &Session, tool: &str, arguments: &Value, text: &str, is_error: bool) -> Option<(String, bool)> {
  let out = ask(server, session, "tool.after", json!({ "tool": tool, "arguments": arguments, "result": { "text": text, "isError": is_error } })).await?;
  answered(&out, is_error).map(|o| (o.model, o.is_error))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn request() -> Request {
    Request {
      model: "m".into(),
      system: "sys".into(),
      items: vec![
        Item::User(vec![Part::Text("hi".into()), Part::Image { mime: "image/png".into(), data: "AAAA".into() }]),
        Item::Assistant { text: "ok".into(), reasoning: String::new(), tool_calls: vec![], native: None },
      ],
      tools: vec![
        ToolSpec { name: "read".into(), description: "Read".into(), parameters: json!({ "type": "object" }) },
        ToolSpec { name: "bash".into(), description: "Run".into(), parameters: json!({ "type": "object" }) },
      ],
      max_tokens: None,
      output_limit: None,
      sampling: Default::default(),
      thinking: Default::default(),
      effort: None,
      cache_key: None,
      fast: false,
    }
  }

  #[test]
  fn a_context_answer_keeps_rewrites_drops_and_adds() {
    let mut r = request();
    let seen = request_json(&r);
    assert_eq!(seen["items"][0]["parts"][1], json!({ "type": "image", "mime": "image/png" }));
    let answer = json!({
      "system": "sys + rules",
      "items": [{ "keep": 0 }, { "role": "user", "text": "remember: tabs" }, { "keep": 1 }],
      "tools": [{ "name": "bash", "description": "Run a shell command, never rm -rf" }, { "name": "invented" }],
      "options": { "maxTokens": 512, "temperature": 0.2 },
    });
    apply_request(&mut r, &answer);
    assert_eq!(r.system, "sys + rules");
    assert_eq!(r.items.len(), 3);
    assert_eq!(r.items[0], request().items[0]);
    assert_eq!(r.items[1], Item::User(vec![Part::Text("remember: tabs".into())]));
    assert_eq!(r.tools.iter().map(|t| (t.name.as_str(), t.description.as_str())).collect::<Vec<_>>(), [("bash", "Run a shell command, never rm -rf")]);
    assert_eq!((r.max_tokens, r.sampling.temperature), (Some(512), Some(0.2)));
  }

  #[test]
  fn items_that_come_back_as_they_were_sent_keep_what_the_hook_cannot_see() {
    let mut original = request();
    original.items[1] = Item::Assistant { text: "ok".into(), reasoning: "r".into(), tool_calls: vec![], native: Some(crate::llm::Native { origin: "x".into(), blocks: vec![json!({ "signature": "sig" })] }) };
    let mut r = original.clone();
    // The engine's merge hands the payload's own items back with an answer that changed only the system prompt
    let mut answer = request_json(&r);
    answer["system"] = json!("sys + rules");
    apply_request(&mut r, &answer);
    assert_eq!(r.items, original.items, "the image data and the signed reasoning survive");
    assert_eq!(r.system, "sys + rules");
  }
}
