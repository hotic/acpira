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
  acpira_shared::re::compile(r"(?i)^(read(ing)?|edit(ing)?|write|writing|search(ing)?|run(ning)?|execute|executing|fetch(ing)?|delete|deleting|move|moving|list(ing)?)\s+(file|files|directory|command)?\s*")
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
/// `acpira_show_image`, and a bare `show_image` from adapters that drop the server prefix
static SHOW_IMAGE: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2})show_image$"));
pub const SHOW_IMAGE_VERB: &str = "verb.showImage";

pub(super) fn is_show_image_call(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> bool {
  let named = |x: Option<&str>| x.map(str::trim).is_some_and(|x| SHOW_IMAGE.is_match(x));
  named(title) || named(str_of(u, "name")) || named(raw.and_then(|r| r.get("tool")).and_then(Value::as_str))
}

/// Acpira's `show_widget`, in the same namings as show_image
static SHOW_WIDGET: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2})show_widget$"));
pub const SHOW_WIDGET_VERB: &str = "verb.showWidget";

pub(super) fn is_show_widget_call(u: &Value, title: Option<&str>, raw: Option<&Map<String, Value>>) -> bool {
  let named = |x: Option<&str>| x.map(str::trim).is_some_and(|x| SHOW_WIDGET.is_match(x));
  named(title) || named(str_of(u, "name")) || named(raw.and_then(|r| r.get("tool")).and_then(Value::as_str))
}

/// Acpira's session tools (`host_mcp.rs` search_sessions / read_session), in the same namings as show_image
static SESSION_TOOL: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2})(search_sessions|read_session)$"));
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

/// An MCP tool call: the server's name as configured, the tool's own name, and its title when the agent knows it
pub(super) struct McpCall {
  pub(super) server: String,
  pub(super) tool: String,
  pub(super) title: Option<String>,
}

/// Who an MCP call went to, from whichever naming the adapter used: the built-in agent says so in
/// `_meta["acpira/mcp"] { server, tool, title }`; claude-agent-acp names the call `mcp__<server>__<tool>` (title or
/// `name`); codex-acp titles it `mcp.<server>.<tool>` with rawInput `{ server, tool, arguments }`. OpenCode's
/// `<server>_<tool>` cannot be told from an ordinary tool name and is left alone. The engine's own `ask_agent` keeps its
/// row as it is: the summoned agent's node stands for it
pub(super) fn mcp_identity(u: &Value, meta: Option<&Map<String, Value>>, title: Option<&str>, raw: Option<&Map<String, Value>>) -> Option<McpCall> {
  let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
  let call = if let Some(m) = meta.and_then(|m| m.get("acpira/mcp")) {
    McpCall { server: s(m.get("server"))?, tool: s(m.get("tool"))?, title: s(m.get("title")) }
  } else if let Some(c) = [title, str_of(u, "name")].into_iter().flatten().find_map(|x| CLAUDE_MCP.captures(x.trim())) {
    McpCall { server: c[1].to_owned(), tool: c[2].to_owned(), title: None }
  } else {
    let c = title.and_then(|x| CODEX_MCP.captures(x.trim()))?;
    let server = s(raw.and_then(|r| r.get("server"))).unwrap_or_else(|| c[1].to_owned());
    let tool = s(raw.and_then(|r| r.get("tool"))).unwrap_or_else(|| c[2].to_owned());
    McpCall { server, tool, title: None }
  };
  (!(call.server == crate::host_mcp::SERVER_NAME && call.tool == crate::host_mcp::ASK_TOOL)).then_some(call)
}

/// A server name as the row shows it: words split at `-`, `_`, `.` and spaces, each capitalized (`chrome-devtools` →
/// `Chrome Devtools`, `blockbench` → `Blockbench`)
pub(super) fn mcp_label(server: &str) -> String {
  server
    .split(['-', '_', '.', ' '])
    .filter(|w| !w.is_empty())
    .map(|w| {
      let mut c = w.chars();
      c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    })
    .collect::<Vec<_>>()
    .join(" ")
}

/// An agent's title is often like "Read file foo.ts"; the verb is ours, so strip the English one
pub(super) fn strip_verb(title: &str) -> String {
  let s = STRIP_VERB.replace(title, "");
  let s = s.strip_prefix('`').unwrap_or(&s);
  let s = s.strip_suffix('`').unwrap_or(s);
  let s = s.trim();
  if s.is_empty() { title.to_owned() } else { s.to_owned() }
}

/// A skill load reads "Load skill <name>" whichever way the agent ran it
pub const SKILL_VERB: &str = "verb.skill";
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
