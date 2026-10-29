//! The tools' definitions as `tools/list` hands them out, and the calls this server answers by itself: `show_image`,
//! `show_widget` and `workspace_hooks` check their arguments here and answer a receipt

use std::path::Path;

use serde_json::{Map, Value, json};

use acpira_shared::attachments::image_mime_of;
use acpira_shared::host_tools::{
  ASK_AGENT as ASK_TOOL, ASK_CALLER as CALLER_TOOL, OPEN_BROWSER as BROWSER_TOOL, PLUGIN_ACTIVATE, PLUGIN_GUIDE, READ_SESSION as READ_SESSION_TOOL,
  SEARCH_SESSIONS as SEARCH_SESSIONS_TOOL, SHOW_IMAGE as TOOL_NAME, SHOW_WIDGET as WIDGET_TOOL, WORKSPACE_HOOKS as HOOKS_TOOL,
};

use crate::limits::{MAX_OUT_IMAGE_BYTES, MAX_WIDGET_BYTES};

const SEARCH_SESSIONS_DESCRIPTION: &str = "Find earlier Acpira sessions, the user's conversations with any agent (Claude, \
Codex, Kimi, Devin, the built-in agent …), by words in their title or conversation; without a query, list the most recent ones. \
Searches this project unless scope is \"all\". With `session` (a link or id), it searches inside that one session instead, \
tool output included, and answers turn numbers. Sessions come back as `[title](acpira://session/<id>)` links: pass one to \
read_session, and quote it as it is when you mention the session to the user.";

const READ_SESSION_DESCRIPTION: &str = "Read another Acpira session, whichever agent ran it. Without from / to it answers \
the outline (about 12 KB): what was asked, edited files, plan state, one line per turn and the last reply. Then read only the \
turns you need with from / to: one page of about 16 KB, and a page that stops early names where to go on (a turn longer than \
a page goes on with offset). detail \"full\" adds \
tool output and diffs. `session` takes a link (acpira://session/<id>, vscode://hotic.acpira/session/<id>), an id, or the \
agent's own session id.";

const PLUGIN_GUIDE_DESCRIPTION: &str = "How to write an Acpira plugin (the plugin.json format, script tools, themes, hooks, \
with complete examples) and the state of every plugin here, with what its last check found. Read it before writing or \
fixing a plugin.";

const PLUGIN_ACTIVATE_DESCRIPTION: &str = "Offer the plugin in ~/.acpira/plugins/<id>/ that you wrote or changed. It is \
frozen and checked first; a problem comes back for you to fix, nothing is offered. Otherwise a card for it joins your \
reply and the user applies it with one click, now or later; the answer says when they applied it and which tools it \
added. Call it again after every change: an edit alone takes no effect.";

const CALLER_DESCRIPTION: &str = "Ask the agent that summoned you a question and wait for its answer. Use it only for a \
decision that blocks you and that you cannot reasonably make yourself; one question at a time, short and self-contained, with \
`options` when there are clear choices. The user sees the question too and may answer first. When no answer comes in time the \
result says so: go on with the most reasonable assumption and state it in your final reply.";

const BROWSER_DESCRIPTION: &str = "Open an http(s) page in a browser tab inside the user's editor, next to the chat. \
Use it when the user asks to see a page or would clearly want to: a local dev server, a tool's web UI, a preview. \
A URL on this machine's loopback (localhost, 127.0.0.1, 0.0.0.0) must be served here first; when the user's editor runs \
on another machine (SSH, containers), Acpira forwards the port to it and checks from there that the page answers. \
The result says whether the page opened and at which address. Loading a page can change the state of the app behind it \
(one that hands its session to the open page, for example), so open it when it is wanted, not as a check.";

const DESCRIPTION: &str = "Display local image files (PNG, JPEG, GIF, WebP) to the user, inline in the conversation. \
Use it whenever the user should look at an image you produced or found: a screenshot, a rendered UI, a chart, a generated picture. \
Paths must be absolute. The image is shown to the user only; it is not returned to you. \
Prefer this tool. To place an image at a specific point of a reply instead, embed it there with Markdown image syntax and an absolute path \
(`![caption](/abs/path.png)`), which renders the same way; use one or the other for a given image, never both.";

const WIDGET_DESCRIPTION: &str = "Show an interactive HTML widget to the user, inline in the conversation: a small \
self-contained view (sliders, toggles, tabs, a live calculator, a mini chart) that lets them explore numbers or options without \
more back-and-forth. Write it to an .html file first (absolute path, under 512 KB), then call this tool; the widget is shown \
to the user only, not returned to you. Call it again after editing the file to show the new version.\n\n\
It runs in a sandboxed frame with no network: no CDN, fetch, external images or fonts, so put all CSS and JavaScript inline \
(classic scripts; inline event handlers such as onclick work). Tailwind CSS v4 is built in: use utility classes directly, no \
build step (`<style type=\"text/tailwindcss\">` works too). Follow the conversation's light or dark theme with these colours \
instead of fixed ones: bg-background, bg-card, bg-muted, text-foreground, text-muted-foreground, border-border, bg-primary with \
text-primary-foreground, text-primary, ring-ring, and text-success / text-warning / text-danger; the same values are CSS \
variables (--background, --card, --primary …) for canvas or SVG. The page background is transparent and the font is the \
editor's. The frame is as wide as the conversation (from about 300 px) and as tall as the content: keep it compact, ideally \
under 600 px. Forms do not submit and alerts, dialogs and new windows are blocked.";

/// The groups' part of `ask_agent`'s description: each with its members, and what one call naming it does
fn group_lines(groups: &[acpira_shared::subagents::SubagentGroup]) -> String {
  if groups.is_empty() {
    return String::new();
  }
  let lines: Vec<String> = groups
    .iter()
    .map(|g| {
      let when = if g.when.trim().is_empty() { String::new() } else { format!(": {}", g.when.trim()) };
      format!("- {} (@{}){when} [group: {}]", g.id, g.name, g.members.join(", "))
    })
    .collect();
  format!(
    "\n\nGroups: one call naming a group gives every member the same task (and the same handoff pack), each working on \
its own without seeing the others; their replies come back together, in one message or, with `wait`, in this call's \
answer. Follow up with one member through the `thread` its reply named.\n{}",
    lines.join("\n")
  )
}

/// `ask_agent`: the personas (and groups) are the enum of `agent`, and their "when" lines are the menu the model picks from. The
/// description is also where the caller learns to brief (goal, constraints, done, leads, what to return), to keep a
/// review independent of its own account, and to weigh a reply against the code before acting on it
pub(super) fn ask_tool_def(personas: &[acpira_shared::subagents::SubagentPersona], groups: &[acpira_shared::subagents::SubagentGroup]) -> Value {
  use acpira_shared::subagents::{RelayMode, SubagentDuty};
  let lines: Vec<String> = personas
    .iter()
    .map(|p| {
      let duty = match p.duty {
        SubagentDuty::General => "",
        SubagentDuty::Scout => "scout · ",
        SubagentDuty::Executor => "executor · ",
        SubagentDuty::Reviewer => "reviewer · ",
      };
      let model = p.model.as_deref().map(|m| format!(" · {m}")).unwrap_or_default();
      // The effort is its own field: a model id may carry an unrelated level (Devin's `gemini-4-argon-medium` runs at `high`)
      let effort = p.effort.as_deref().map(|e| format!(" · {e} effort")).unwrap_or_default();
      let mode = match p.mode {
        RelayMode::Consult => " · read-only",
        RelayMode::Work => "",
        RelayMode::Bypass => " · edits without approval",
        RelayMode::Inherit => " · follows your mode",
      };
      let when = if p.when.trim().is_empty() { String::new() } else { format!(": {}", p.when.trim()) };
      format!("- {} (@{}){when} [{duty}{}{model}{effort}{mode}]", p.id, p.name, p.agent)
    })
    .collect();
  let description = format!(
    "Summon another coding agent that runs in its own CLI on this same repository, and answer with its reply. \
Use it when one of the agents below fits the task better (a second opinion, a review, fast mechanical edits), and always when \
the user names one with @name: those names are these agents, not your own subagents, so call this tool directly without \
listing or searching for agents. To ask several agents, make one call per agent in the same message; they run in parallel.\n\n\
It cannot see this conversation but reads files and runs commands itself, so the prompt is its whole brief: state the goal, \
the constraints, what counts as done, what you already know (point at paths instead of pasting code) and what its reply \
should contain. To have work reviewed, give the original requirement and where the change is, not your own account of it, so \
the reviewer judges the change itself. A reply is advice: check each point against the code before acting on it, and tell \
the user which points you rejected and why.\n\n\
A new task answers at once with its thread while the agent works: carry on with other work or end your turn, and Acpira \
sends you its reply in a message when it is done (several sent together come back together), so do not poll and do not do \
the task yourself meanwhile. Set `wait` when you need the reply before going on; a call with a thread and an empty prompt \
waits for that thread. To follow up with the same agent, pass the `thread` its answer named. A scout or an executor \
continues its idle conversation in this session even without `thread` (set `new_thread` to start over); a reviewer starts \
fresh on every call without `thread`, which keeps it independent.\n\n\
Set `changes` to hand the agent your uncommitted changes as a pack (their diff and the small changed files in full), so it \
starts from them instead of reading its way there; a later round of the same thread is told only what moved since. Agents \
whose user switched this on get it without asking.\n\nAgents:\n{}{}",
    lines.join("\n"),
    group_lines(groups)
  );
  let ids: Vec<&str> = personas.iter().map(|p| p.id.as_str()).chain(groups.iter().map(|g| g.id.as_str())).collect();
  json!({
    "name": ASK_TOOL,
    "title": "Ask another agent",
    "description": description,
    "inputSchema": {
      "type": "object",
      "properties": {
        "agent": { "type": "string", "enum": ids, "description": "Which agent (or group) to summon. Not needed with `thread`." },
        "prompt": { "type": "string", "description": "The task or question. Empty only when waiting on a thread." },
        "title": { "type": "string", "description": "A few words naming the task, shown to the user." },
        "mode": { "type": "string", "enum": ["consult", "work", "bypass"], "description": "Only for an agent marked \"follows your mode\": consult = read-only opinion; work = may edit files; bypass = edits without approval. Without it that agent takes your own current access. Every other agent always runs in the access its user set, whatever you pass." },
        "thread": { "type": "string", "description": "Continue the conversation an earlier answer named." },
        "new_thread": { "type": "boolean", "description": "Start a new conversation with a scout or an executor instead of continuing its idle one." },
        "wait": { "type": "boolean", "description": "Wait in this call for the reply (up to your tool timeout) instead of getting it in a message later." },
        "answers": { "type": "array", "items": { "type": "string" }, "description": "With `thread`, when that agent asks you something: one answer per question, in order (an option's label or your own words)." },
        "changes": { "type": "boolean", "description": "Hand over your uncommitted changes (diff plus small changed files in full) with a new task." },
      },
      "required": [],
    },
    // readOnlyHint doubles as the concurrency flag in Claude Code (2.1.224:
    // `isConcurrencySafe() { return annotations?.readOnlyHint ?? false }`), so
    // `false` made it run several ask_agent calls of one message one after
    // another. The call itself only relays a prompt: a work-mode child's own
    // edits still go through its CLI's permission cards, routed to Acpira.
    "annotations": { "readOnlyHint": true, "openWorldHint": false },
    // Claude Code defers MCP tools behind ToolSearch, which cost a summon a search step (and the model, seeing only
    // the name, took `@name` for one of its own agents and listed those first). `anthropic/alwaysLoad` keeps this
    // tool and its persona list in the prompt (read per tool since 2.1.224 / agent SDK 0.3.284); `searchHint` is
    // what a client that still defers it matches
    "_meta": {
      "anthropic/alwaysLoad": true,
      "anthropic/searchHint": "summon ask subagent persona @name second opinion review delegate another agent CLI",
    },
  })
}

/// `ask_caller`, offered only to a summoned child's server
pub(super) fn plugin_guide_def() -> Value {
  json!({
    "name": PLUGIN_GUIDE,
    "title": "Acpira plugin guide",
    "description": PLUGIN_GUIDE_DESCRIPTION,
    "inputSchema": { "type": "object", "properties": {} },
    "annotations": { "readOnlyHint": true },
  })
}

pub(super) fn plugin_activate_def() -> Value {
  json!({
    "name": PLUGIN_ACTIVATE,
    "title": "Enable an Acpira plugin",
    "description": PLUGIN_ACTIVATE_DESCRIPTION,
    "inputSchema": {
      "type": "object",
      "properties": { "id": { "type": "string", "description": "The plugin's folder name under ~/.acpira/plugins" } },
      "required": ["id"],
    },
  })
}

pub(super) fn caller_tool_def() -> Value {
  json!({
    "name": CALLER_TOOL,
    "title": "Ask the caller",
    "description": CALLER_DESCRIPTION,
    "inputSchema": {
      "type": "object",
      "properties": {
        "question": { "type": "string", "description": "The question, self-contained: the caller has not seen your work." },
        "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers, when there are clear choices; the caller may still answer in its own words." },
      },
      "required": ["question"],
    },
    "annotations": { "readOnlyHint": true, "openWorldHint": false },
    "_meta": { "anthropic/searchHint": "ask caller parent question decision clarify summoned" },
  })
}

pub(super) fn tool_def() -> Value {
  json!({
    "name": TOOL_NAME,
    "title": "Show image",
    "description": DESCRIPTION,
    "inputSchema": {
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute path of the image file to show." },
        "paths": { "type": "array", "items": { "type": "string" }, "description": "Several absolute image paths, shown in order." },
        "caption": { "type": "string", "description": "Optional short caption naming what the image shows." },
      },
    },
    "annotations": { "readOnlyHint": true, "openWorldHint": false },
  })
}

pub(super) fn widget_tool_def() -> Value {
  json!({
    "name": WIDGET_TOOL,
    "title": "Show widget",
    "description": WIDGET_DESCRIPTION,
    "inputSchema": {
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute path of the .html file to show." },
        "title": { "type": "string", "description": "Optional short title naming what the widget shows." },
      },
      "required": ["path"],
    },
    "annotations": { "readOnlyHint": true, "openWorldHint": false },
    "_meta": { "anthropic/searchHint": "show interactive html widget ui component slider chart calculator inline generative ui" },
  })
}

/// The session tools, offered with the relay env. Read-only (Claude Code runs them concurrently); not `alwaysLoad`, so a
/// Claude session that never needs them does not carry their definitions: the server instructions name them instead
pub(super) fn session_tool_defs() -> Vec<Value> {
  let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
  vec![
    json!({
      "name": SEARCH_SESSIONS_TOOL,
      "title": "Search sessions",
      "description": SEARCH_SESSIONS_DESCRIPTION,
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "Words that must all occur in the title or the conversation. Empty lists the most recent sessions." },
          "scope": { "type": "string", "enum": ["project", "all"], "description": "project (default): this folder's sessions; all: every project." },
          "agent": { "type": "string", "description": "Only sessions of this agent (id or name, e.g. codex, Claude)." },
          "session": { "type": "string", "description": "Search inside this one session (link or id) instead; needs a query." },
          "limit": { "type": "integer", "minimum": 1, "maximum": 20, "description": "At most this many results (default 8)." },
        },
      },
      "annotations": read_only,
      "_meta": { "anthropic/searchHint": "search find earlier previous past acpira session conversation chat history link" },
    }),
    json!({
      "name": READ_SESSION_TOOL,
      "title": "Read session",
      "description": READ_SESSION_DESCRIPTION,
      "inputSchema": {
        "type": "object",
        "properties": {
          "session": { "type": "string", "description": "The session's link (acpira://session/<id>), id, or the agent's own session id." },
          "from": { "type": "integer", "minimum": 1, "description": "First turn to read (1-based). Omit from and to for the outline." },
          "to": { "type": "integer", "minimum": 1, "description": "Last turn to read (inclusive); defaults to the last turn." },
          "offset": { "type": "integer", "minimum": 0, "description": "Where to go on inside the from turn, as named by a page that cut it." },
          "detail": { "type": "string", "enum": ["brief", "full"], "description": "full adds tool output and diffs to the turns." },
        },
        "required": ["session"],
      },
      "annotations": read_only,
      "_meta": { "anthropic/searchHint": "read open acpira session link transcript outline turns another conversation" },
    }),
  ]
}

pub(super) fn browser_tool_def() -> Value {
  json!({
    "name": BROWSER_TOOL,
    "title": "Open in browser",
    "description": BROWSER_DESCRIPTION,
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string", "description": "The page's absolute http:// or https:// URL, as it is reached from this machine (e.g. http://localhost:3000/)." },
      },
      "required": ["url"],
    },
    "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": true },
    "_meta": { "anthropic/searchHint": "open web page browser url localhost preview dev server port forward show site" },
  })
}

pub(super) fn hooks_tool_def() -> Value {
  json!({
    "name": HOOKS_TOOL,
    "title": "Workspace hooks",
    "description": "Explain the format of `.agents/hooks.json`, the gates and reviews Acpira runs for every agent in a project (context files sent with the first prompt, a check before each edit, a check when a turn ends that can send findings back), and report whether the project's current file is valid. Call it before writing or changing that file or the scripts it runs, and again afterwards to check the result.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "cwd": { "type": "string", "description": "Absolute path of the project (or any directory inside it)." },
      },
      "required": ["cwd"],
    },
    "annotations": { "readOnlyHint": true, "openWorldHint": false },
  })
}

/// The format guide plus the state of the project's own file
pub(super) fn call_workspace_hooks(args: &Map<String, Value>) -> Value {
  workspace_hooks_with(args, &|cwd| crate::hooks::describe(cwd))
}

/// `describe`: the check of the project at a path (on the session's SSH host, through the hub, for a folder there)
pub(super) fn workspace_hooks_with(args: &Map<String, Value>, describe: &dyn Fn(&str) -> String) -> Value {
  let cwd = args.get("cwd").and_then(Value::as_str).map(str::trim).unwrap_or("");
  let status = if cwd.is_empty() {
    "Pass `cwd` (the project's absolute path) to have its current file checked.".to_owned()
  } else {
    describe(cwd)
  };
  json!({ "content": [{ "type": "text", "text": format!("{}

## This project

{status}", crate::hooks::GUIDE) }], "isError": false })
}

/// Validate the named files; the host shows them once the call completes. Failures go back to the model as a tool error
pub(super) fn call_show_image(args: &Map<String, Value>) -> Value {
  show_image_with(args, &check_image)
}

/// `check`: one file's problem, if any (a session on an SSH host had the hub check them there first)
pub(super) fn show_image_with(args: &Map<String, Value>, check: &dyn Fn(&str) -> Result<(), String>) -> Value {
  let one = args.get("path").and_then(Value::as_str).into_iter();
  let many = args.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
  let paths: Vec<&str> = one.chain(many).map(str::trim).filter(|p| !p.is_empty()).collect();
  if paths.is_empty() {
    return tool_error("Pass `path` (or `paths`) with the absolute path of an image file.".into());
  }
  let problems: Vec<String> = paths.iter().filter_map(|p| check(p).err()).collect();
  if !problems.is_empty() {
    return tool_error(problems.join("\n"));
  }
  let names: Vec<&str> = paths.iter().map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p)).collect();
  json!({
    "content": [{ "type": "text", "text": format!("Shown to the user: {}. It is displayed in the conversation; no need to link or describe it again.", names.join(", ")) }],
    "isError": false,
  })
}

fn check_image(path: &str) -> Result<(), String> {
  let p = Path::new(path);
  if !p.is_absolute() {
    return Err(format!("{path}: not an absolute path"));
  }
  if image_mime_of(path).is_none() {
    return Err(format!("{path}: not a PNG, JPEG, GIF or WebP file"));
  }
  let meta = std::fs::metadata(p).map_err(|e| format!("{path}: {e}"))?;
  if !meta.is_file() {
    return Err(format!("{path}: not a file"));
  }
  if meta.len() > MAX_OUT_IMAGE_BYTES {
    return Err(format!("{path}: larger than {} MB", MAX_OUT_IMAGE_BYTES / (1024 * 1024)));
  }
  Ok(())
}

/// `.html` / `.htm`: the files `show_widget` takes, stored as `text/html` blobs beside the session's images
pub fn widget_mime_of(path: &str) -> Option<&'static str> {
  let tail = path.rsplit(['/', '\\']).next().unwrap_or(path);
  let ext = tail.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())?;
  matches!(ext.as_str(), "html" | "htm").then_some("text/html")
}

/// Validate the widget file; the host reads it into the session once the call completes
pub(super) fn call_show_widget(args: &Map<String, Value>) -> Value {
  show_widget_with(args, &check_widget)
}

pub(super) fn show_widget_with(args: &Map<String, Value>, check: &dyn Fn(&str) -> Result<(), String>) -> Value {
  let path = args.get("path").and_then(Value::as_str).map(str::trim).unwrap_or("");
  if path.is_empty() {
    return tool_error("Pass `path` with the absolute path of an .html file.".into());
  }
  if let Err(problem) = check(path) {
    return tool_error(problem);
  }
  let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
  json!({
    "content": [{ "type": "text", "text": format!("Shown to the user: {name}, as an interactive widget in the conversation. It is not returned to you; no need to describe it again.") }],
    "isError": false,
  })
}

fn check_widget(path: &str) -> Result<(), String> {
  let p = Path::new(path);
  if !p.is_absolute() {
    return Err(format!("{path}: not an absolute path"));
  }
  if widget_mime_of(path).is_none() {
    return Err(format!("{path}: not an .html file"));
  }
  let meta = std::fs::metadata(p).map_err(|e| format!("{path}: {e}"))?;
  if !meta.is_file() {
    return Err(format!("{path}: not a file"));
  }
  if meta.len() > MAX_WIDGET_BYTES {
    return Err(format!("{path}: larger than {} KB; inline less data or split it into several widgets", MAX_WIDGET_BYTES / 1024));
  }
  Ok(())
}

pub(super) fn tool_error(text: String) -> Value {
  json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}
