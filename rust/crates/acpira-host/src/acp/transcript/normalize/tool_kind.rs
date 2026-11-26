//! Which tool a call is: its kind and verb from the adapter's kind, title or name (shell, web search, image
//! generation, Acpira's own MCP tools, other MCP servers), and the English verb stripped from a title

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use acpira_shared::session_links::{session_link, session_ref_id};
use acpira_shared::transcript::*;

use crate::i18n::t;
use crate::json::str_of;

use super::tool_target::Target;

pub(super) fn verb_key(kind: ToolKind) -> &'static str {
  match kind {
    ToolKind::Read => "verb.read",
    ToolKind::Edit => "verb.edit",
    ToolKind::Delete => "verb.delete",
    ToolKind::Move => "verb.move",
    ToolKind::Search => "verb.search",
    ToolKind::Execute => "verb.execute",
    ToolKind::Think => "verb.think",
    ToolKind::Fetch => "verb.fetch",
    ToolKind::SwitchMode => "verb.switch_mode",
    ToolKind::Other => "verb.other",
  }
}

pub fn verb_of(kind: ToolKind) -> String {
  t(verb_key(kind))
}

pub(super) static TODO_TITLE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)^todo([_\s-]?(write|update|read|list))?$"));
pub(super) static ASK_TITLE: LazyLock<Regex> =
  LazyLock::new(|| acpira_shared::re::compile(r"(?i)^(ask_?user_?questions?|ask(ed|ing)?\s+(the\s+)?(user\s+)?(\d+\s+)?questions?\b)"));
/// Devin's `get_output` / `kill_shell`, and the built-in agent's `job` tool: the bare name its row opens with, then
/// "Check job 3 (wait 30s)" / "Stop job 3" once the arguments are known (`acpira-agent` `tools/mod.rs`)
static SHELL_WAIT: LazyLock<Regex> =
  LazyLock::new(|| acpira_shared::re::compile(r"(?i)^(get_output|read(ing)?\s+shell(\s+output)?|job|check\s+job\s+\S+(\s+\(wait\s+\d+s\))?)$"));
static SHELL_KILL: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)^(kill_shell|kill(ing)?\s+shell|stop\s+job\s+\S+)$"));
/// The built-in agent's `job` typing into an interactive job: the typed text is the row's point, so it keeps its title
pub(super) static JOB_INPUT: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)^type\s+into\s+job\s"));
/// Image generation tools by name: codex-acp's "Image generation" title, Grok's `image_gen` / `image_edit` (`x.ai/tool` name)
pub(super) static IMAGE_GEN: LazyLock<Regex> =
  LazyLock::new(|| acpira_shared::re::compile(r"(?i)^(image[_\s-]?gen(eration)?|generate[_\s-]?image|image[_\s-]?edit)$"));
/// Grok's display titles for the same tools (`imagine: <prompt>` / `imagine-edit: <prompt>`; source strings, not yet seen on the wire)
pub(super) static IMAGE_GEN_TITLE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)^imagine(-edit)?:\s"));
/// An absolute image path (POSIX, `C:\`, or file://) inside a tool's text result; group 1 is the path. The leading boundary keeps
/// a relative `images/1.jpg` from matching as `/1.jpg`
pub(super) static SAVED_IMAGE_PATH: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r#"(?i)(?:^|[\s"'`(<\[=:])((?:file://|[A-Za-z]:[\\/]|/)[^\s"'`<>()\[\]]*?\.(?:png|jpe?g|gif|webp))\b"#)
});
pub const IMAGE_GEN_VERB: &str = "verb.imagegen";
/// A web search keeps the plain search verb as its key: the `fetch` kind (globe icon) tells it from a local search,
/// and the key marks the row so later packets keep it a web search
pub const WEB_SEARCH_VERB: &str = "verb.search";
/// A web search by title: the placeholder "Web search" (claude-agent-acp 0.83, codex-acp 1.13) and codex-acp's
/// "Web search: <query>"; group 1 is the query
static WEB_SEARCH_TITLE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)^web[ _]?search(?::\s*(.*))?$"));
/// claude-agent-acp 0.83's WebSearch once its input is complete: `Search "<query>"` (counted only on a `fetch` row,
/// since a grep could be titled the same); group 1 is the query
static CLAUDE_SEARCH_TITLE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r#"^Search "(.+)"$"#));

pub(super) enum WebTool {
  Search,
  OpenPage,
}

/// Which web tool a call is. claude-agent-acp 0.83 files WebSearch as `fetch` titled "Web search", then `Search "<query>"`
/// with `rawInput.query` (WebFetch keeps `fetch` with `rawInput.url`); codex-acp 1.13 files its web_search item as `search`
/// titled "Web search[: <query>]" / "Open page: <url>" with `rawInput { type: "webSearch", query, action }`
pub(super) fn web_tool(b: &ToolCallBlock, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<WebTool> {
  if b.mcp_server.is_some() || b.verb_key.as_deref().is_some_and(|k| k != WEB_SEARCH_VERB) {
    return None;
  }
  // codex-acp's action decides, and may only arrive with the completion
  if let Some(raw) = raw.filter(|r| r.get("type").and_then(Value::as_str) == Some("webSearch")) {
    return match raw.get("action").and_then(|a| a.get("type")).and_then(Value::as_str) {
      Some("openPage") => Some(WebTool::OpenPage),
      Some("findInPage") => None,
      _ => Some(WebTool::Search),
    };
  }
  let title = title.map(str::trim).unwrap_or("");
  let fetch_query = b.kind == ToolKind::Fetch
    && (CLAUDE_SEARCH_TITLE.is_match(title) || raw.is_some_and(|r| r.get("query").is_some_and(Value::is_string) && r.get("url").is_none()));
  (b.verb_key.is_some() || WEB_SEARCH_TITLE.is_match(title) || fetch_query).then_some(WebTool::Search)
}

/// The query of a web search: codex-acp's `action.query` / `action.queries`, then `rawInput.query`, then the title
pub(super) fn web_search_query(title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<String> {
  let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
  if let Some(raw) = raw {
    let action = raw.get("action");
    if let Some(q) = text(action.and_then(|a| a.get("query"))) {
      return Some(q);
    }
    let queries: Vec<&str> = action
      .and_then(|a| a.get("queries"))
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(Value::as_str)
      .map(str::trim)
      .filter(|q| !q.is_empty())
      .collect();
    if !queries.is_empty() {
      return Some(queries.join(", "));
    }
    if let Some(q) = text(raw.get("query")) {
      return Some(q);
    }
  }
  let title = title?.trim();
  let found = WEB_SEARCH_TITLE.captures(title).and_then(|c| c.get(1)).or_else(|| CLAUDE_SEARCH_TITLE.captures(title).and_then(|c| c.get(1)));
  found.map(|m| m.as_str().trim().to_owned()).filter(|q| !q.is_empty())
}
static KIND_BY_TITLE: LazyLock<Vec<(Regex, ToolKind)>> = LazyLock::new(|| {
  [
    (r"(?i)^(read|open|view|cat)(_[a-z]+)*$", ToolKind::Read),
    (r"(?i)^(write|edit|create|patch|apply_?patch|str_?replace|insert)(_[a-z]+)*$", ToolKind::Edit),
    (r"(?i)^(list|ls|dir|glob|grep|find|search)(_[a-z]+)*$", ToolKind::Search),
    (r"(?i)^(web_?search|google|bing)(_[a-z]+)*$", ToolKind::Search),
    (r"(?i)^(bash|shell|terminal|exec|execute|run|command)(_[a-z]+)*$", ToolKind::Execute),
    (r"(?i)^(web_?fetch|fetch|browse|curl)(_[a-z]+)*$", ToolKind::Fetch),
  ]
  .into_iter()
  .map(|(r, k)| (acpira_shared::re::compile(r), k))
  .collect()
});
static STRIP_VERB: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r"(?i)^(read(ing)?|edit(ing)?|write|writing|search(ing)?|run(ning)?|execute|executing|fetch(ing)?|delete|deleting|move|moving|list(ing)?)\s+(?:(?:files?|directory|command)(?:\s+|$))?")
});
pub(super) static DATA_URL: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?s)^data:([\w.+-]+/[\w.+-]+)?;base64,(.+)$"));

pub(super) fn shell_verb(meta: Option<&Map<String, Value>>, title: Option<&str>) -> Option<&'static str> {
  match meta.and_then(|m| m.get("cognition.ai/inferenceToolName")).and_then(Value::as_str) {
    Some("get_output") => return Some("verb.wait"),
    Some("kill_shell") => return Some("verb.kill"),
    _ => {}
  }
  let title = title.filter(|t| !t.is_empty())?.trim();
  if SHELL_WAIT.is_match(title) {
    Some("verb.wait")
  } else if SHELL_KILL.is_match(title) {
    Some("verb.kill")
  } else {
    None
  }
}

pub(super) fn infer_kind(title: Option<&str>) -> Option<ToolKind> {
  let name = title.filter(|t| !t.is_empty())?.trim();
  KIND_BY_TITLE.iter().find(|(re, _)| re.is_match(name)).map(|(_, k)| *k)
}

/// Acpira's own MCP tool (`acpira mcp`, see `host_mcp.rs`) as each adapter names it: claude-agent-acp
/// `mcp__acpira__show_image`, codex-acp `mcp.acpira.show_image` (+ rawInput `{ server, tool, arguments }`), OpenCode
/// `acpira_show_image`, `agy acp` `mcp_acpira_show_image` or, under its alias for the server (`AGY_MCP`),
/// `mcp_acpira_0db272fb_show_image`, and a bare `show_image` from adapters that drop the server prefix
static SHOW_IMAGE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2}(?:[0-9a-f]{8}_)?)show_image$"));
pub const SHOW_IMAGE_VERB: &str = "verb.showImage";

pub(super) fn is_show_image_call(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> bool {
  let named = |x: Option<&str>| x.map(str::trim).is_some_and(|x| SHOW_IMAGE.is_match(x));
  named(title) || named(str_of(u, "name")) || named(raw.and_then(|r| r.get("tool")).and_then(Value::as_str))
}

/// Acpira's `show_video`, in the same adapter namings as show_image.
static SHOW_VIDEO: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2}(?:[0-9a-f]{8}_)?)show_video$"));
pub const SHOW_VIDEO_VERB: &str = "verb.showVideo";

pub(super) fn is_show_video_call(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> bool {
  let named = |x: Option<&str>| x.map(str::trim).is_some_and(|x| SHOW_VIDEO.is_match(x));
  named(title) || named(str_of(u, "name")) || named(raw.and_then(|r| r.get("tool")).and_then(Value::as_str))
}

/// Acpira's `show_widget`, in the same namings as show_image
static SHOW_WIDGET: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2}(?:[0-9a-f]{8}_)?)show_widget$"));
pub const SHOW_WIDGET_VERB: &str = "verb.showWidget";

pub(super) fn is_show_widget_call(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> bool {
  let named = |x: Option<&str>| x.map(str::trim).is_some_and(|x| SHOW_WIDGET.is_match(x));
  named(title) || named(str_of(u, "name")) || named(raw.and_then(|r| r.get("tool")).and_then(Value::as_str))
}

/// Acpira's session tools (`host_mcp.rs` search_sessions / read_session), in the same namings as show_image
static SESSION_TOOL: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2}(?:[0-9a-f]{8}_)?)(search_sessions|read_session)$"));
pub const SEARCH_SESSIONS_VERB: &str = "verb.searchSessions";
pub const READ_SESSION_VERB: &str = "verb.readSession";

pub(super) fn session_tool_verb(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<&'static str> {
  let tool = [title, str_of(u, "name"), raw.and_then(|r| r.get("tool")).and_then(Value::as_str)]
    .into_iter()
    .flatten()
    .find_map(|x| SESSION_TOOL.captures(x.trim()).map(|c| c[1].to_ascii_lowercase()))?;
  Some(if tool == "search_sessions" { SEARCH_SESSIONS_VERB } else { READ_SESSION_VERB })
}

/// A session tool's target, which the webview reads back (`splitSessionTarget`): the session as its canonical link first
/// when the call names one, then the turn range (`3-5`, `3-`, `-5`) of a read or the query of a search
pub(super) fn session_tool_target(key: &str, args: Option<&Map<String, Value>>) -> Option<Target> {
  let args = args?;
  let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
  let num = |k: &str| args.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
  let session = text("session").map(|r| session_ref_id(r).map(|id| session_link(&id)).unwrap_or_else(|| r.to_owned()));
  let rest = if key == READ_SESSION_VERB {
    match (num("from"), num("to")) {
      (None, None) => None,
      (from, to) => Some(format!("{}-{}", from.map(|n| n.to_string()).unwrap_or_default(), to.map(|n| n.to_string()).unwrap_or_default())),
    }
  } else {
    text("query").map(str::to_owned)
  };
  let joined: Vec<String> = [session, rest].into_iter().flatten().collect();
  (!joined.is_empty()).then(|| Target { text: joined.join(" "), mono: false, from_title: false })
}

/// The tool's own arguments: codex-acp nests them under `arguments`, the others send them as rawInput itself
pub(super) fn show_image_args(raw: Option<&Map<String, Value>>) -> Option<&Map<String, Value>> {
  let raw = raw?;
  raw.get("arguments").and_then(Value::as_object).or(Some(raw))
}

/// claude-agent-acp's `mcp__<server>__<tool>` and codex-acp's `mcp.<server>.<tool>`
static CLAUDE_MCP: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^mcp__([A-Za-z0-9-][A-Za-z0-9_-]*?)__(.+)$"));
static CODEX_MCP: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^mcp\.([^.\s]+)\.(\S+)$"));
/// `agy acp`'s `mcp_<server>_<tool>` (agy 1.5.9, in `name`, with the title `Run <name>`). The server ends at the first
/// underscore, so a server whose own name holds one is cut short. A server whose name the process already holds under
/// another configuration is registered as `<server>_<8 hex>` (agy 1.5.13: a second session request of one process, each
/// carrying a fresh hub grant), which is no part of the tool's name
static AGY_MCP: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^mcp_([^_\s]+)_(?:[0-9a-f]{8}_)?(\S+)$"));

/// An MCP tool call: the server's name as configured, the tool's own name, and its title when the agent knows it
pub(super) struct McpCall {
  pub(super) server: String,
  pub(super) tool: String,
  pub(super) title: Option<String>,
}

/// Who an MCP call went to, from whichever naming the adapter used: the built-in agent says so in
/// `_meta["acpira/mcp"] { server, tool, title }`; claude-agent-acp names the call `mcp__<server>__<tool>` (title or
/// `name`); codex-acp titles it `mcp.<server>.<tool>` with rawInput `{ server, tool, arguments }`. OpenCode's
/// `<server>_<tool>` cannot be told from an ordinary tool name and is left alone; `agy acp` names the call
/// `mcp_<server>_<tool>` and titles it `Run <name>`, and only that pair counts. The engine's own `ask_agent` keeps its
/// row as it is: the summoned agent's node stands for it
pub(super) fn mcp_identity(u: &Value, meta: Option<&Map<String, Value>>, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<McpCall> {
  let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
  let call = if let Some(m) = meta.and_then(|m| m.get("acpira/mcp")) {
    McpCall { server: s(m.get("server"))?, tool: s(m.get("tool"))?, title: s(m.get("title")) }
  } else if let Some(c) = [title, str_of(u, "name")].into_iter().flatten().find_map(|x| CLAUDE_MCP.captures(x.trim())) {
    McpCall { server: c[1].to_owned(), tool: c[2].to_owned(), title: None }
  } else if let Some(c) = str_of(u, "name").filter(|n| title.is_some_and(|t| t.trim().strip_prefix("Run ") == Some(*n))).and_then(|n| AGY_MCP.captures(n)) {
    McpCall { server: c[1].to_owned(), tool: c[2].to_owned(), title: None }
  } else {
    let c = title.and_then(|x| CODEX_MCP.captures(x.trim()))?;
    let server = s(raw.and_then(|r| r.get("server"))).unwrap_or_else(|| c[1].to_owned());
    let tool = s(raw.and_then(|r| r.get("tool"))).unwrap_or_else(|| c[2].to_owned());
    McpCall { server, tool, title: None }
  };
  (!(call.server == crate::host_mcp::SERVER_NAME && call.tool == crate::host_mcp::ASK_TOOL)).then_some(call)
}

/// A server name as the row shows it: the configured name unchanged (`chrome-devtools`, `acpira`), as Claude Code and
/// Codex print it, so it reads the same as skill names and the MCP settings list
pub(super) fn mcp_label(server: &str) -> String {
  server.trim().to_owned()
}

/// A tool title that opens with its server's name ("Acpira plugin guide" on server acpira) drops it, since the verb
/// already names the server. Separators in the name match any separator or space (`chrome-devtools` ~ "Chrome DevTools")
pub(super) fn drop_server_prefix(target: String, label: &str) -> String {
  let sep = |c: char| matches!(c, ' ' | ':' | '-' | '_' | '.');
  let mut rest = target.chars();
  for want in label.chars() {
    match rest.next() {
      Some(got) if got.eq_ignore_ascii_case(&want) || (sep(got) && sep(want)) => {}
      _ => return target,
    }
  }
  let rest = rest.as_str();
  if !rest.starts_with(sep) {
    return target;
  }
  let rest = rest.trim_start_matches(sep);
  if rest.is_empty() { target } else { rest.to_owned() }
}

/// An agent's title is often like "Read file foo.ts"; the verb is ours, so strip the English one
pub(super) fn strip_verb(title: &str) -> String {
  let s = STRIP_VERB.replace(title, "");
  let s = s.strip_prefix('`').unwrap_or(&s);
  let s = s.strip_suffix('`').unwrap_or(s);
  let s = s.trim();
  if s.is_empty() { title.to_owned() } else { s.to_owned() }
}

/// A row named by the tool behind it, whose title is only that tool's bare name: Claude's agent-control tools and
/// Antigravity's own ones. Its target, when any, is the one the row shows; the title never is
pub(super) struct NamedTool {
  pub(super) verb: &'static str,
  pub(super) target: Option<Target>,
  /// `update_steps` always sends the whole list and answers a bare `OK`, so its input is the resulting list
  pub(super) steps: Option<Vec<PlanEntry>>,
}

/// The verb keys `named_tool` gives besides the shared ones (`verb.todo` / `ask` / `wait` / `kill`)
pub(super) const NAMED_TOOL_VERBS: [&str; 11] = [
  "verb.awaitSubagent",
  "verb.stopSubagent",
  "verb.readBoard",
  "verb.sendMessage",
  "verb.subagentStatus",
  "verb.swarmTitle",
  "verb.listTasks",
  "verb.loadTools",
  "verb.monitor",
  "verb.listSessions",
  "verb.notify",
];

pub(super) fn named_tool(u: &Value, meta: Option<&Map<String, Value>>, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<NamedTool> {
  claude_tool(u, meta, raw).or_else(|| agy_tool(u, title, raw))
}

fn str_arg<'a>(raw: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a str> {
  raw.and_then(|r| r.get(key)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn plain(text: &str) -> Target {
  Target { text: text.to_owned(), mono: false, from_title: false }
}

/// The first of a question tool's `questions: [{ question, … }]`, both CLIs' shape
fn first_question(raw: Option<&Map<String, Value>>) -> Option<Target> {
  let first = raw?.get("questions")?.as_array()?.first()?;
  first.get("question").and_then(Value::as_str).map(str::trim).filter(|q| !q.is_empty()).map(plain)
}

/// A message's first line of prose
fn prose(text: Option<&str>) -> Option<Target> {
  text.and_then(crate::acp::vendors::antigravity_subagents::first_plain_line).map(|text| Target { text, mono: false, from_title: false })
}

/// claude-agent-acp (0.111.0) titles these with the bare tool name and names the tool in `_meta.claudeCode.toolName`
/// on the call and on its result. `AskUserQuestion` is titled with its question or "Asking for your input"
fn claude_tool(u: &Value, meta: Option<&Map<String, Value>>, raw: Option<&Map<String, Value>>) -> Option<NamedTool> {
  let name = meta?.get("claudeCode")?.get("toolName")?.as_str()?;
  let arg = |k: &str| str_arg(raw, k);
  let (verb, target) = match name {
    "AskUserQuestion" => ("verb.ask", first_question(raw)),
    // `select:A,B` loads those tools by name; anything else is a keyword search
    "ToolSearch" => (
      "verb.loadTools",
      arg("query").map(|q| match q.strip_prefix("select:") {
        Some(names) => plain(&names.split(',').map(str::trim).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ")),
        None => plain(q),
      }),
    ),
    "Monitor" => ("verb.monitor", arg("description").map(plain).or_else(|| arg("command").map(|c| Target { mono: true, ..plain(c) }))),
    "TaskStop" => ("verb.kill", stopped_command(u).or_else(|| arg("task_id").map(|id| Target { mono: true, ..plain(id) }))),
    "ListAgents" => ("verb.listSessions", None),
    // `to` is often an agent id, `summary` the few words the model gave the message
    "SendMessage" => ("verb.sendMessage", arg("summary").map(plain).or_else(|| prose(arg("message")))),
    "PushNotification" => ("verb.notify", prose(arg("message"))),
    _ => return None,
  };
  Some(NamedTool { verb, target, steps: None })
}

/// The command a `TaskStop` result names (`{ message, task_id, task_type, command }`, as JSON text or blocks of it)
fn stopped_command(u: &Value) -> Option<Target> {
  let parsed;
  let result = match u.get("rawOutput")? {
    Value::Object(_) => u.get("rawOutput")?,
    Value::String(s) => {
      parsed = serde_json::from_str::<Value>(s).ok()?;
      &parsed
    }
    Value::Array(items) => {
      parsed = serde_json::from_str::<Value>(items.iter().find_map(|i| i.get("text").and_then(Value::as_str))?).ok()?;
      &parsed
    }
    _ => return None,
  };
  let command = result.get("command")?.as_str()?.trim();
  (!command.is_empty()).then(|| Target { mono: true, ..plain(command) })
}

/// Antigravity's own tools (`agy` 1.5.16), all `Run <name>` rows of kind `other`: the step list, the swarm board,
/// swarm / subagent bookkeeping and background tasks. `swarm add` and `invoke_subagent` stay with the subagent tree
fn agy_tool(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<NamedTool> {
  let name = str_of(u, "name")
    .map(str::trim)
    .filter(|n| !n.is_empty())
    .or_else(|| title.map(str::trim).map(|t| t.strip_prefix("Run ").unwrap_or(t)))?;
  let arg = |k: &str| str_arg(raw, k);
  // `<conversation>/task-<n>`: the conversation half repeats the session id
  let task = || arg("TaskId").map(|id| Target { mono: true, ..plain(id.rsplit('/').next().unwrap_or(id)) });
  let (verb, target) = match (name, arg("action").or_else(|| arg("Action"))) {
    ("update_steps", _) => ("verb.todo", None),
    // Its questions go over `elicitation/create`; the row is only the call
    ("ask_question", _) => ("verb.ask", first_question(raw)),
    ("swarm_wait", _) => ("verb.awaitSubagent", None),
    ("read_board", _) => ("verb.readBoard", None),
    ("post_message", _) => ("verb.sendMessage", prose(arg("text"))),
    ("send_message", _) => ("verb.sendMessage", prose(arg("Message"))),
    ("swarm", Some("status")) | ("manage_subagents", Some("list")) => ("verb.subagentStatus", None),
    ("swarm", Some("title")) => ("verb.swarmTitle", arg("title").map(plain)),
    ("manage_subagents", Some("kill")) => ("verb.stopSubagent", None),
    // `status` answers with the task's latest output, like Devin's `get_output`
    ("manage_task", Some("status")) => ("verb.wait", task()),
    ("manage_task", Some("kill")) => ("verb.kill", task()),
    ("manage_task", Some("list")) => ("verb.listTasks", None),
    _ => return None,
  };
  let steps = (name == "update_steps").then(|| raw.and_then(|r| r.get("steps")).and_then(agy_steps)).flatten();
  Some(NamedTool { verb, target, steps })
}

/// `[{ title, status: pending | in_progress | done }]`, as an array or its JSON text
fn agy_steps(value: &Value) -> Option<Vec<PlanEntry>> {
  let parsed;
  let list = match value {
    Value::String(s) => {
      parsed = serde_json::from_str::<Value>(s).ok()?;
      parsed.as_array()?
    }
    other => other.as_array()?,
  };
  list
    .iter()
    .map(|item| {
      let title = item.get("title")?.as_str()?.trim();
      let status = match item.get("status")?.as_str()? {
        "pending" => PlanStatus::Pending,
        "in_progress" => PlanStatus::InProgress,
        "done" | "completed" => PlanStatus::Completed,
        _ => return None,
      };
      (!title.is_empty()).then(|| PlanEntry { title: title.to_owned(), status, priority: None })
    })
    .collect()
}

/// A skill load reads "Load skill <name>" whichever way the agent ran it
pub const SKILL_VERB: &str = "verb.skill";
/// Claude's `ScheduleWakeup` (`vendors::claude_wakeup`): setting a wakeup, or `stop: true` ending them. The keys also mark
/// the row for the session's pending-wakeup tracking
pub const WAKEUP_VERB: &str = "verb.wakeup";
pub const WAKEUP_STOP_VERB: &str = "verb.wakeupStop";
pub const WAKEUP_VERBS: [&str; 2] = [WAKEUP_VERB, WAKEUP_STOP_VERB];

/// A `ScheduleWakeup` row's verb key, by the tool name claude-agent-acp puts in `_meta.claudeCode.toolName`; a row
/// already named one keeps it, since later packets may carry only a status
pub(super) fn wakeup_verb(b: &ToolCallBlock, meta: Option<&Map<String, Value>>, raw: Option<&Map<String, Value>>) -> Option<&'static str> {
  let named = meta.and_then(|m| m.get("claudeCode")).and_then(|c| c.get("toolName")).and_then(Value::as_str)
    == Some(crate::acp::vendors::claude_wakeup::TOOL);
  let before = b.verb_key.as_deref().and_then(|k| WAKEUP_VERBS.iter().copied().find(|v| *v == k));
  if !named && before.is_none() {
    return None;
  }
  match raw.and_then(|r| r.get("stop")).and_then(Value::as_bool) {
    Some(true) => Some(WAKEUP_STOP_VERB),
    Some(false) => Some(WAKEUP_VERB),
    None => before.or(Some(WAKEUP_VERB)),
  }
}

/// "Next wakeup scheduled for 02:22:00 (in 1860s)": the clock time, to the minute, as the CLI printed it
pub(super) fn wakeup_time(text: &str) -> Option<String> {
  let (_, rest) = text.split_once("scheduled for ")?;
  let time: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == ':').collect();
  let mut parts = time.split(':');
  let (h, m) = (parts.next()?, parts.next()?);
  (!h.is_empty() && m.len() == 2).then(|| format!("{h}:{m}"))
}
/// claude-agent-acp's `Skill` ("Load skill" until the input is known, then "Load skill: <name>"), the built-in agent's
/// `skill` ("Skill <name>") and OpenCode's bare `skill`; group 1 or 2 is the name when the title carries it
static SKILL_TITLE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)^(?:load\s+skill(?::\s*(\S.*))?|skill(?:\s+(\S+))?)$"));
/// A skill's instructions file: `.../skills/<name>/SKILL.md`; group 1 is the name
static SKILL_FILE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?:^|[\\/])skills[\\/]([^\\/]+)[\\/]SKILL\.md$"));
/// A command that only prints one SKILL.md (Codex, Kimi and Grok load a skill this way); group 1 is the file
static SKILL_PRINT: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(
    r#"^(?:cat|head(?:\s+-n\s*\d+|\s+-\d+)?|sed\s+-n\s+['"]?\d+(?:,\d+)?p['"]?|nl\s+-ba)\s+['"]?([^\s'"]*[\\/]skills[\\/][^\\/\s'"]+[\\/]SKILL\.md)['"]?$"#,
  )
});

pub(super) struct SkillLoad {
  pub(super) name: Option<String>,
  pub(super) path: Option<String>,
}

/// Whether a call loads a skill, and which: claude-agent-acp names its tool in `_meta.claudeCode.toolName` and, for an
/// AIR client, the skill and its file in `_meta.jetbrains.air.skill { name, path }` (0.84 source); the built-in agent and
/// OpenCode by title; any other agent by reading one `skills/<name>/SKILL.md`, as a read or a one-file print command.
/// A row that already has another identity (to-do, MCP, …) is left alone
pub(super) fn skill_load(b: &ToolCallBlock, meta: Option<&Map<String, Value>>, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<SkillLoad> {
  if b.mcp_server.is_some() || b.verb_key.as_deref().is_some_and(|k| k != SKILL_VERB) {
    return None;
  }
  let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
  let air = meta.and_then(|m| m.get("jetbrains")).and_then(|j| j.get("air")).and_then(|a| a.get("skill"));
  let claude = meta.and_then(|m| m.get("claudeCode")).and_then(|c| c.get("toolName")).and_then(Value::as_str) == Some("Skill");
  let titled = title.map(str::trim).and_then(|x| SKILL_TITLE.captures(x));
  let file = |path: &str| SKILL_FILE.captures(path).map(|c| (c[1].to_owned(), path.to_owned()));
  // The file the call read: its one location, or the one file a print command names
  let read = match b.kind {
    ToolKind::Read => b.locations.as_deref().filter(|l| l.len() == 1).and_then(|l| file(&l[0].path)),
    ToolKind::Execute if b.target_mono == Some(true) => {
      b.target.as_deref().and_then(|c| SKILL_PRINT.captures(c.trim())).and_then(|c| file(&c[1]))
    }
    _ => None,
  };
  if b.verb_key.is_none() && air.is_none() && !claude && titled.is_none() && read.is_none() {
    return None;
  }
  let name = text(air.and_then(|a| a.get("name")))
    .or_else(|| text(raw.and_then(|r| r.get("skill"))))
    .or_else(|| text(raw.and_then(|r| r.get("name"))))
    .or_else(|| titled.as_ref().and_then(|c| c.get(1).or_else(|| c.get(2))).map(|m| m.as_str().trim().to_owned()))
    .or_else(|| read.as_ref().map(|(n, _)| n.clone()));
  let path = text(air.and_then(|a| a.get("path"))).or_else(|| read.map(|(_, p)| p));
  Some(SkillLoad { name, path })
}
