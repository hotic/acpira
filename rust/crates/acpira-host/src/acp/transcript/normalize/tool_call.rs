//! Tool rows from tool_call / tool_call_update: a new block, the merge of an update's optional fields into it, and
//! the fields a permission request's copy of the call may improve

use serde_json::{Map, Value};

use acpira_shared::todo_tools::{is_todo_tool, todo_entries};
use acpira_shared::transcript::*;

use crate::acp::transcript::diff::diff_lines;
use crate::i18n::{t, tp};
use crate::json::{pretty, slice16, str_of, text_of};
use crate::limits::TOOL_OUTPUT_MAX;
use crate::util::{js_num, now_ms};

use super::ToolCtx;
use super::content::{
  append_exit_code, attach_saved_images, attach_shown_images, attach_shown_widget, formatted_output, show_image_paths, tool_contents,
};
use super::tool_kind::{
  ASK_TITLE, IMAGE_GEN, IMAGE_GEN_TITLE, IMAGE_GEN_VERB, READ_SESSION_VERB, SEARCH_SESSIONS_VERB, SHOW_IMAGE_VERB, SHOW_WIDGET_VERB,
  TODO_TITLE, WEB_SEARCH_VERB, WebTool, infer_kind, is_show_image_call, is_show_widget_call, mcp_identity, mcp_label, session_tool_target,
  session_tool_verb, shell_verb, show_image_args, verb_of, web_search_query, web_tool,
};
use super::tool_target::{
  Target, path_from_raw, pick_target, prompt_target, read_range_from_raw, shell_target, show_image_target, show_widget_target,
};

/// Sparse updates retain the first observed start and the first terminal timestamp
pub fn time_tool(b: &mut ToolCallBlock, live: bool) {
  if live && b.status == ToolStatus::InProgress && b.ended_at.is_none() && b.started_at.is_none() {
    b.started_at = Some(now_ms());
  }
  if b.started_at.is_some() && !b.status.is_open() && b.ended_at.is_none() {
    b.ended_at = Some(now_ms());
  }
}

pub fn empty_tool(id: String) -> ToolCallBlock {
  ToolCallBlock {
    observation: None,
    id,
    kind: ToolKind::Other,
    verb: String::new(),
    verb_key: None,
    mcp_server: None,
    target: None,
    target_mono: None,
    locations: None,
    read_range: None,
    status: ToolStatus::Pending,
    background: None,
    async_task: None,
    started_at: None,
    ended_at: None,
    meta: None,
    diff_stat: None,
    todo_entries: None,
    content: None,
    contents: None,
    subagent_id: None,
  }
}

fn kind_field(u: &Value) -> Option<ToolKind> {
  str_of(u, "kind").filter(|k| !k.is_empty()).and_then(ToolKind::parse)
}

pub fn tool_block(tc: &Value, ctx: Option<&mut ToolCtx>) -> ToolCallBlock {
  let kind = kind_field(tc).unwrap_or(ToolKind::Other);
  let mut b = ToolCallBlock {
    kind,
    verb: verb_of(kind),
    status: str_of(tc, "status").and_then(ToolStatus::parse).unwrap_or(ToolStatus::Pending),
    ..empty_tool(str_of(tc, "toolCallId").unwrap_or("").to_owned())
  };
  merge_tool(&mut b, tc, ctx);
  b
}

/// Fields of tool_call and tool_call_update are all optional; overwrite only the ones provided
pub fn merge_tool(b: &mut ToolCallBlock, u: &Value, mut ctx: Option<&mut ToolCtx>) {
  let meta = u.get("_meta").and_then(Value::as_object);
  let raw = u.get("rawInput").and_then(Value::as_object);
  let raw_present = crate::json::truthy(u.get("rawInput"));
  let title = str_of(u, "title");
  // An MCP row stays `other`: codex-acp files every MCP call as `execute`
  if let Some(k) = kind_field(u).filter(|_| b.mcp_server.is_none()) {
    b.kind = k;
    b.verb = verb_of(k);
  }
  let set_verb = |b: &mut ToolCallBlock, key: &str| {
    b.verb_key = Some(key.to_owned());
    b.verb = t(key);
  };
  if title.is_some_and(|x| !x.is_empty() && TODO_TITLE.is_match(x.trim())) {
    set_verb(b, "verb.todo");
  }
  let tool_name = meta
    .and_then(|m| m.get("x.ai/tool"))
    .and_then(|x| x.get("name"))
    .filter(|v| !v.is_null())
    .or_else(|| meta.and_then(|m| m.get("cognition.ai/inferenceToolName")));
  if let Some(name) = tool_name.and_then(Value::as_str)
    && TODO_TITLE.is_match(name)
  {
    set_verb(b, "verb.todo");
  }
  if title.is_some_and(|x| !x.is_empty() && ASK_TITLE.is_match(x.trim())) {
    set_verb(b, "verb.ask");
  }
  if let Some(shell) = shell_verb(meta, title) {
    set_verb(b, shell);
  }
  let image_gen_title = title.map(str::trim).is_some_and(|x| IMAGE_GEN.is_match(x) || IMAGE_GEN_TITLE.is_match(x));
  if image_gen_title || tool_name.and_then(Value::as_str).is_some_and(|n| IMAGE_GEN.is_match(n)) {
    set_verb(b, IMAGE_GEN_VERB);
  }
  // Web tools read the same whichever CLI ran them: a search is "Search" with its query, an opened page a fetch of its URL.
  // Re-applied on every packet, since a later `kind` field would otherwise reset the row to the agent's own kind
  match web_tool(b, title, raw) {
    Some(WebTool::Search) => {
      set_verb(b, WEB_SEARCH_VERB);
      b.kind = ToolKind::Fetch;
    }
    Some(WebTool::OpenPage) => {
      // codex-acp may name the action only on completion, after the start packet looked like a search
      if b.verb_key.as_deref() == Some(WEB_SEARCH_VERB) {
        b.verb_key = None;
      }
      b.kind = ToolKind::Fetch;
      b.verb = verb_of(ToolKind::Fetch);
    }
    None => {}
  }
  if b.verb_key.as_deref() == Some(SHOW_IMAGE_VERB) || is_show_image_call(u, title, raw) {
    set_verb(b, SHOW_IMAGE_VERB);
    // codex-acp reports every MCP call as `execute`; this one is no command
    b.kind = ToolKind::Other;
    if let Some(paths) = show_image_args(raw).map(show_image_paths).filter(|p| !p.is_empty()) {
      b.locations = Some(paths.into_iter().map(|path| Location { path, line: None }).collect());
    }
  }
  // Acpira's show_widget: one HTML file, kept as the row's location until the call completes and the file is read
  if b.verb_key.as_deref() == Some(SHOW_WIDGET_VERB) || is_show_widget_call(u, title, raw) {
    set_verb(b, SHOW_WIDGET_VERB);
    b.kind = ToolKind::Other;
    if let Some(path) = show_image_args(raw).and_then(|a| a.get("path")).and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) {
      b.locations = Some(vec![Location { path: path.to_owned(), line: None }]);
    }
  }
  // Acpira's session tools read "Search sessions" / "Read session" with what they looked for, not "Call Acpira"
  if let Some(key) = b.verb_key.as_deref().filter(|k| matches!(*k, SEARCH_SESSIONS_VERB | READ_SESSION_VERB)).map(str::to_owned).or_else(|| session_tool_verb(u, title, raw).map(str::to_owned)) {
    set_verb(b, &key);
    b.kind = ToolKind::Other;
  }
  // Any other MCP call reads "Call <Server>" with the tool's title as its target, whichever naming it came in
  let mut mcp_target = None;
  if b.verb_key.is_none()
    && let Some(call) = mcp_identity(u, meta, title, raw)
  {
    let label = mcp_label(&call.server);
    b.verb = tp("verb.mcp", &[("server", &label)]);
    b.mcp_server = Some(label);
    b.kind = ToolKind::Other;
    mcp_target = Some(call.title.unwrap_or_else(|| call.tool.replace(['_', '-'], " ")));
  }
  if b.verb_key.is_none()
    && b.mcp_server.is_none()
    && matches!(b.kind, ToolKind::Other | ToolKind::Think)
    && let Some(inferred) = infer_kind(title)
  {
    b.kind = inferred;
    b.verb = verb_of(inferred);
  }
  // A row an async task owns takes its status from the task's state updates
  if let Some(st) = str_of(u, "status").filter(|x| !x.is_empty()).and_then(ToolStatus::parse)
    && b.async_task.is_none()
  {
    b.status = st;
  }
  if let Some(m) = meta
    && m.get("cognition.ai/background") == Some(&Value::Bool(true))
  {
    b.background = Some(true);
    if let (Some(ctx), Some(shell_id)) = (ctx.as_deref_mut(), m.get("cognition.ai/backgroundShellId").and_then(Value::as_str)) {
      let command = m.get("cognition.ai/backgroundCommand").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
      ctx.shells.insert(shell_id.to_owned(), command.or_else(|| b.target.clone()).unwrap_or_else(|| shell_id.to_owned()));
    }
  }
  if meta.and_then(|m| m.get("jetbrains")).and_then(|j| j.get("air")).and_then(|a| a.get("asyncTasks")).and_then(|t| t.get("backgrounded"))
    == Some(&Value::Bool(true))
  {
    b.background = Some(true);
  }
  if let Some(locs) = u.get("locations").and_then(Value::as_array).filter(|_| !matches!(b.verb_key.as_deref(), Some(SHOW_IMAGE_VERB | SHOW_WIDGET_VERB))) {
    b.locations = Some(
      locs
        .iter()
        .map(|l| Location {
          path: str_of(l, "path").unwrap_or("").to_owned(),
          line: l.get("line").and_then(Value::as_f64).map(|n| n.max(0.0) as u64),
        })
        .collect(),
    );
  }
  if b.kind == ToolKind::Read
    && raw_present
    && let Some(range) = raw.and_then(|r| read_range_from_raw(r, b.locations.as_deref()))
  {
    b.read_range = Some(range);
  }
  let file_kind = matches!(b.kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move);
  if b.locations.as_ref().is_none_or(Vec::is_empty)
    && file_kind
    && let Some(path) = path_from_raw(raw)
  {
    b.locations = Some(vec![Location { path, line: None }]);
  }
  // OpenCode's write sends the contents only in rawInput.content: park them for a new-file completion
  if let Some(ctx) = ctx.as_deref_mut()
    && b.kind == ToolKind::Edit
    && let Some(content) = raw.and_then(|r| r.get("content")).and_then(Value::as_str)
    && let Some(path) = path_from_raw(raw).or_else(|| b.locations.as_ref().and_then(|l| l.first()).map(|l| l.path.clone()))
  {
    ctx.pending_writes.insert(b.id.clone(), (path, content.to_owned()));
  }
  let named = matches!(
    b.verb_key.as_deref(),
    Some("verb.todo" | "verb.ask" | "verb.wait" | "verb.kill" | IMAGE_GEN_VERB | SHOW_IMAGE_VERB | SHOW_WIDGET_VERB | SEARCH_SESSIONS_VERB | READ_SESSION_VERB)
  );
  let target = if matches!(b.verb_key.as_deref(), Some("verb.wait" | "verb.kill")) {
    shell_target(raw, ctx.as_deref())
  } else if b.verb_key.as_deref() == Some(WEB_SEARCH_VERB) {
    // No target until the query is known: the placeholder title ("Web search") only repeats the verb
    web_search_query(title, raw).map(|text| Target { text, mono: false, from_title: false })
  } else if b.verb_key.as_deref() == Some(IMAGE_GEN_VERB) {
    prompt_target(raw)
  } else if b.verb_key.as_deref() == Some(SHOW_IMAGE_VERB) {
    show_image_target(show_image_args(raw), b.locations.as_deref())
  } else if b.verb_key.as_deref() == Some(SHOW_WIDGET_VERB) {
    show_widget_target(show_image_args(raw), b.locations.as_deref())
  } else if let Some(key @ (SEARCH_SESSIONS_VERB | READ_SESSION_VERB)) = b.verb_key.as_deref() {
    session_tool_target(key, show_image_args(raw))
  } else if b.mcp_server.is_some() {
    // Set once from the call's identity; a later update's arguments do not replace it
    mcp_target.map(|text| Target { text, mono: false, from_title: false })
  } else {
    pick_target(u, b.kind)
  };
  if let Some(tg) = target
    && !(named && tg.from_title)
    && (!tg.from_title || b.target.is_none())
  {
    b.target = Some(tg.text);
    b.target_mono = Some(tg.mono);
  }
  let status_completed = str_of(u, "status") == Some("completed");
  let pending_write = match ctx.as_deref_mut() {
    Some(ctx) if status_completed && !matches!(b.content, Some(ToolContent::Diff { .. })) => ctx.pending_writes.remove(&b.id),
    _ => None,
  };
  if let Some((path, content)) = pending_write
    && u.get("rawOutput").and_then(|o| o.get("metadata")).and_then(|m| m.get("exists")) == Some(&Value::Bool(false))
  {
    let lines = diff_lines("", &content);
    let add = lines.iter().filter(|l| l.kind == DiffKind::Add).count() as u64;
    let diff = ToolContent::Diff { lines, source: Some(DiffSource { path, old_text: String::new(), new_text: content, omitted: false }) };
    let items =
      u.get("content").and_then(Value::as_array).filter(|a| !a.is_empty()).map(|a| tool_contents(a, ctx.as_deref())).unwrap_or_default();
    let receipts: Vec<ToolContent> = items.into_iter().filter(|x| matches!(x, ToolContent::Text { text } if !text.is_empty())).collect();
    b.contents = if receipts.is_empty() {
      None
    } else {
      let mut all = vec![diff.clone()];
      all.extend(receipts);
      Some(all)
    };
    b.content = Some(diff);
    b.diff_stat = Some(DiffStat { add, del: 0 });
  }
  if let Some(items) = u.get("content").and_then(Value::as_array).filter(|a| !a.is_empty()) {
    let list = tool_contents(items, ctx.as_deref());
    let primary = list.iter().find(|x| matches!(x, ToolContent::Diff { .. })).or(list.first()).cloned();
    // Kimi sends the edit diff before execution, then a plain success receipt: keep the diff
    let keep_diff = b.kind == ToolKind::Edit
      && b.status == ToolStatus::Completed
      && matches!(b.content, Some(ToolContent::Diff { .. }))
      && list.len() == 1
      && matches!(primary, Some(ToolContent::Text { .. }));
    if let Some(c) = primary
      && !keep_diff
    {
      b.content = Some(c);
      let diffs: Vec<&Vec<DiffLine>> = list
        .iter()
        .filter_map(|x| match x {
          ToolContent::Diff { lines, .. } => Some(lines),
          _ => None,
        })
        .collect();
      b.diff_stat = if diffs.is_empty() {
        None
      } else {
        Some(DiffStat {
          add: diffs.iter().map(|d| d.iter().filter(|l| l.kind == DiffKind::Add).count() as u64).sum(),
          del: diffs.iter().map(|d| d.iter().filter(|l| l.kind == DiffKind::Del).count() as u64).sum(),
        })
      };
      b.contents = if list.len() > 1 { Some(list) } else { None };
    }
  }
  if b.verb_key.as_deref() == Some(IMAGE_GEN_VERB) && !b.status.is_open() {
    attach_saved_images(b, u, ctx.as_deref());
  }
  if b.verb_key.as_deref() == Some(SHOW_IMAGE_VERB) {
    attach_shown_images(b, ctx.as_deref());
  }
  if b.verb_key.as_deref() == Some(SHOW_WIDGET_VERB) {
    attach_shown_widget(b, ctx.as_deref());
  }
  // pi-acp and the claude / codex adapters stream terminal output through _meta
  let term_out = meta.and_then(|m| m.get("terminal_output").filter(|v| !v.is_null()).or_else(|| m.get("terminal_output_delta")));
  if let Some(data) = term_out.and_then(|o| o.get("data")).and_then(Value::as_str).filter(|d| !d.is_empty()) {
    let prev = match &b.content {
      Some(ToolContent::Text { text }) => text.clone(),
      _ => String::new(),
    };
    let note = term_out.and_then(|o| o.get("terminal_id")).and_then(Value::as_str).map(|id| tp("host.terminalNotWired", &[("id", id)]));
    let base = if note.as_deref() == Some(prev.as_str()) { String::new() } else { prev };
    b.content = Some(ToolContent::Text { text: slice16(&(base + data), TOOL_OUTPUT_MAX) });
  }
  if let Some(code) =
    meta.and_then(|m| m.get("terminal_exit")).and_then(|e| e.get("exit_code")).and_then(Value::as_f64).filter(|c| *c != 0.0)
  {
    let prev = match &b.content {
      Some(ToolContent::Text { text }) => text.clone(),
      _ => String::new(),
    };
    let code_text = js_num(code);
    if !prev.ends_with(&format!("exit code {code_text}")) {
      b.content = Some(ToolContent::Text { text: slice16(&append_exit_code(&prev, &code_text), TOOL_OUTPUT_MAX) });
    }
  }
  // A terminal item the agent cannot stream through _meta expects a client-side terminal, which is not provided
  let agent_wired =
    meta.is_some_and(|m| ["terminal_info", "terminal_output", "terminal_output_delta", "terminal_exit"].iter().any(|k| m.contains_key(*k)));
  if b.content.is_none()
    && !agent_wired
    && let Some(term) = u.get("content").and_then(Value::as_array).and_then(|a| a.iter().find(|c| str_of(c, "type") == Some("terminal")))
  {
    b.content = Some(ToolContent::Text { text: tp("host.terminalNotWired", &[("id", str_of(term, "terminalId").unwrap_or("undefined"))]) });
  }
  if b.content.is_none()
    && let Some(out) = u.get("rawOutput").filter(|v| !v.is_null())
  {
    let text = match formatted_output(out) {
      Some((text, Some(code))) => append_exit_code(&text, &code),
      Some((text, None)) => text,
      None => match out {
        Value::String(s) => s.clone(),
        other => pretty(other),
      },
    };
    if !text.trim().is_empty() {
      b.content = Some(ToolContent::Text { text: slice16(&text, TOOL_OUTPUT_MAX) });
    }
  }
  if is_todo_tool(b) && b.status == ToolStatus::Completed {
    // Parse the full wire result before the preview's size limit
    let entries = todo_entries(u.get("rawOutput").unwrap_or(&Value::Null)).or_else(|| match &b.content {
      Some(ToolContent::Text { text }) => todo_entries(&Value::String(text.clone())),
      _ => None,
    });
    if entries.is_some() {
      b.todo_entries = entries;
    }
  }
}

/// OpenCode's permission request embeds a low-fidelity copy of the call: with a block already there, take only what
/// the request can improve. Without a block the request's toolCall applies whole
pub fn permission_tool_update(existing: Option<&ToolCallBlock>, tc: &Value) -> Value {
  let mut u = Map::new();
  u.insert("sessionUpdate".into(), Value::from("tool_call_update"));
  let Some(existing) = existing else {
    if let Some(o) = tc.as_object() {
      for (k, v) in o {
        u.insert(k.clone(), v.clone());
      }
    }
    return Value::Object(u);
  };
  u.insert("toolCallId".into(), tc.get("toolCallId").cloned().unwrap_or(Value::Null));
  if let Some(st) = text_of(tc, "status") {
    u.insert("status".into(), Value::from(st));
  }
  if let Some(k) = text_of(tc, "kind")
    && k != "other"
    && matches!(existing.kind, ToolKind::Other | ToolKind::Think)
  {
    u.insert("kind".into(), Value::from(k));
  }
  let file_kind = matches!(existing.kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move);
  if existing.target.is_none() || (file_kind && existing.locations.as_ref().is_none_or(Vec::is_empty)) {
    if let Some(title) = tc.get("title") {
      u.insert("title".into(), title.clone());
    }
    if let Some(raw) = tc.get("rawInput") {
      u.insert("rawInput".into(), raw.clone());
    }
  }
  let raw_for_path = u.get("rawInput").or(tc.get("rawInput")).and_then(Value::as_object);
  if let Some(locs) = tc.get("locations").and_then(Value::as_array)
    && !locs.is_empty()
    && existing.locations.as_ref().is_none_or(Vec::is_empty)
    && path_from_raw(raw_for_path).is_none()
  {
    u.insert("locations".into(), Value::Array(locs.clone()));
  }
  Value::Object(u)
}
