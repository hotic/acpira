//! `acpira mcp`: the MCP server Acpira hands every agent in `session/new` / `load` / `resume` (`mcpServers`).
//! It offers one tool, `show_image`, that lets the model put a local image in front of the user: the call's arguments
//! name the files, and the host reads them into the session's blob store when the tool call completes
//! (`normalize.rs` `attach_shown_images`). The server itself only validates the paths and answers a receipt, so no
//! pixels travel back through the model's context.
//!
//! `workspace_hooks` explains `.agents/hooks.json` (`hooks/`, the gates Acpira runs for every agent) and checks a
//! project's copy, so whichever model the user asks to add a gate writes one Acpira can read.
//!
//! With the relay env on its entry (`relay/`), it also offers `ask_agent`: summon one of the user's cross-harness
//! personas. That call is forwarded to the sidecar over its loopback hub and answers with the child's reply.
//!
//! Wire: newline-delimited JSON-RPC 2.0 over stdio (MCP stdio transport). stdout carries protocol messages only.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use acpira_shared::attachments::image_mime_of;

use crate::limits::MAX_OUT_IMAGE_BYTES;

/// Server name in `mcpServers`; adapters prefix the tool with it (`mcp__acpira__show_image`, `mcp.acpira.show_image`)
pub const SERVER_NAME: &str = "acpira";
pub const TOOL_NAME: &str = "show_image";
pub const HOOKS_TOOL: &str = "workspace_hooks";
/// Answered when the client does not name a protocol version
const PROTOCOL_VERSION: &str = "2025-06-18";

const INSTRUCTIONS: &str = "The user reads this conversation in Acpira, which can display images inline. \
When the user should see an image (a screenshot you took, a chart or diagram you rendered, a generated or downloaded picture), \
call show_image with its absolute path instead of only printing the path. Project gates and reviews live in `.agents/hooks.json`, which Acpira runs for every agent: before writing or changing that file (or a hook script it names), call workspace_hooks for the format, and again afterwards to check it.";

/// Appended to the instructions when the relay env is on the entry. Server instructions reach the model even where
/// the client defers tools behind a search, so `@name` is tied to `ask_agent` before the model goes looking for it
const RELAY_INSTRUCTIONS: &str = "When the user writes @name for an agent listed by the ask_agent tool, that is an \
Acpira agent, not one of your own subagents: call ask_agent for it right away, without listing agents first. \
Several agents named = one ask_agent call each, all in the same message; they run in parallel.";

const DESCRIPTION: &str = "Display local image files (PNG, JPEG, GIF, WebP) to the user, inline in the conversation. \
Use it whenever the user should look at an image you produced or found: a screenshot, a rendered UI, a chart, a generated picture. \
Paths must be absolute. The image is shown to the user only; it is not returned to you. \
Prefer this tool. To place an image at a specific point of a reply instead, embed it there with Markdown image syntax and an absolute path \
(`![caption](/abs/path.png)`), which renders the same way; use one or the other for a given image, never both.";

/// The host's server for session requests, shared by every session of one manager. An agent whose session/new failed
/// with it (and worked without) is remembered, and its later requests leave the server out
#[derive(Clone)]
pub struct HostMcp {
  entry: Value,
  refused: Arc<parking_lot::Mutex<HashSet<String>>>,
  /// The relay listener (`relay/hub.rs`): with it, entries carry the env that leads `ask_agent` back to the session
  pub hub: Option<Arc<crate::relay::hub::RelayHub>>,
}

impl HostMcp {
  pub fn new(exe: &str) -> Self {
    HostMcp { entry: server_entry(exe), refused: Default::default(), hub: None }
  }

  pub fn with_hub(mut self, hub: Arc<crate::relay::hub::RelayHub>) -> Self {
    self.hub = Some(hub);
    self
  }

  /// The `mcpServers` entry for this agent's requests, unless it refused the server before
  pub fn entry_for(&self, agent: &str) -> Option<Value> {
    (!self.refused.lock().contains(agent)).then(|| self.entry.clone())
  }

  /// The entry for one session's requests: with a hub, its env carries a grant for that session (and the summoned
  /// thread whose CLI this is, one level deeper), so `ask_agent` calls reach that session and no other
  pub fn entry_for_session(&self, agent: &str, session: &std::sync::Arc<crate::acp::session::AcpSession>, thread: Option<&str>, depth: u32) -> Option<Value> {
    let mut entry = self.entry_for(agent)?;
    if let Some(hub) = &self.hub {
      entry["env"] = hub.env(session, thread, depth);
    }
    Some(entry)
  }

  pub fn refuse(&self, agent: &str) {
    self.refused.lock().insert(agent.to_owned());
  }
}

/// The `mcpServers` entry for a session request (ACP stdio server: no `type` field, env as name/value pairs)
pub fn server_entry(exe: &str) -> Value {
  json!({ "name": SERVER_NAME, "command": exe, "args": ["mcp"], "env": [] })
}

/// True when a session request's `mcpServers` carries the host's own entry
pub fn has_server(req: &Value) -> bool {
  req.get("mcpServers").and_then(Value::as_array).is_some_and(|l| l.iter().any(is_host_entry))
}

/// The same request without the host's entry (for an agent that refuses it)
pub fn without_server(mut req: Value) -> Value {
  if let Some(list) = req.get_mut("mcpServers").and_then(Value::as_array_mut) {
    list.retain(|s| !is_host_entry(s));
  }
  req
}

fn is_host_entry(s: &Value) -> bool {
  s.get("name").and_then(Value::as_str) == Some(SERVER_NAME) && s.get("args").and_then(Value::as_array).is_some_and(|a| a.first().and_then(Value::as_str) == Some("mcp"))
}

/// Serve MCP on stdin / stdout until stdin closes. `ask_agent` calls run on threads of their own (an agent may summon
/// several children at once) and share stdout through a lock; everything else answers inline
pub fn run(version: &str) -> i32 {
  let relay = Relay::from_env();
  let out = Arc::new(parking_lot::Mutex::new(std::io::stdout()));
  let write = |out: &parking_lot::Mutex<std::io::Stdout>, v: &Value| {
    let mut o = out.lock();
    writeln!(o, "{v}").and_then(|_| o.flush()).is_ok()
  };
  for line in std::io::stdin().lock().lines() {
    let Ok(line) = line else { break };
    if line.trim().is_empty() {
      continue;
    }
    let mut initialize = false;
    if let Some(relay) = &relay
      && let Ok(msg) = serde_json::from_str::<Value>(&line)
    {
      let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
      if method == "initialize" {
        relay.note_client(msg.get("params"));
        initialize = true;
      }
      if method == "tools/list"
        && let Some(id) = msg.get("id").cloned()
      {
        let mut tools = vec![tool_def(), hooks_tool_def()];
        tools.extend(relay.tool_def());
        if !write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } })) {
          break;
        }
        continue;
      }
      if method == "tools/call"
        && msg.get("params").and_then(|p| p.get("name")).and_then(Value::as_str) == Some(ASK_TOOL)
        && let Some(id) = msg.get("id").cloned()
      {
        let (relay, out, params) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null));
        std::thread::spawn(move || {
          let result = relay.ask(&params, &mut |progress: Value| {
            write(&out, &progress);
          });
          write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        });
        continue;
      }
    }
    let Some(mut reply) = handle_line(&line, version) else { continue };
    if initialize && let Some(result) = reply.get_mut("result") {
      result["instructions"] = json!(format!("{INSTRUCTIONS}\n\n{RELAY_INSTRUCTIONS}"));
    }
    if !write(&out, &reply) {
      break;
    }
  }
  0
}

pub const ASK_TOOL: &str = "ask_agent";

/// The way back to the sidecar (`relay/hub.rs`), from the env the session put on this server's entry
#[derive(Clone)]
struct Relay {
  addr: String,
  token: String,
  /// How long one call may wait, picked from the MCP client at initialize
  wait_secs: Arc<std::sync::atomic::AtomicU64>,
}

/// Claude Code waits for an MCP tool call far longer than a child round takes; Codex gives up after 60 s by default
/// (`tool_timeout_sec`), and the others are not known, so they get an answer within the shorter bound
const SHORT_WAIT: u64 = 50;
const LONG_WAIT: u64 = 25 * 60;

impl Relay {
  fn from_env() -> Option<Relay> {
    use crate::relay::{ENV_ADDR, ENV_TOKEN};
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    Some(Relay {
      addr: var(ENV_ADDR)?,
      token: var(ENV_TOKEN)?,
      wait_secs: Arc::new(SHORT_WAIT.into()),
    })
  }

  fn note_client(&self, params: Option<&Value>) {
    let name = params.and_then(|p| p.get("clientInfo")).and_then(|c| c.get("name")).and_then(Value::as_str).unwrap_or("");
    let secs = if name.to_lowercase().contains("claude") { LONG_WAIT } else { SHORT_WAIT };
    self.wait_secs.store(secs, std::sync::atomic::Ordering::Relaxed);
  }

  fn request(&self, op: crate::relay::wire::HubOp) -> crate::relay::wire::HubRequest {
    crate::relay::wire::HubRequest { token: self.token.clone(), op }
  }

  /// The tool, listing the personas the hub offers; none (or no hub) = no tool
  fn tool_def(&self) -> Option<Value> {
    use crate::relay::wire::{HubOp, HubReply};
    let mut personas = vec![];
    let _ = crate::relay::hub::call(&self.addr, &self.request(HubOp::List), |r| {
      if let HubReply::Personas(p) = r {
        personas = p;
      }
      false
    });
    (!personas.is_empty()).then(|| ask_tool_def(&personas))
  }

  fn ask(&self, params: &Value, progress: &mut dyn FnMut(Value)) -> Value {
    use crate::relay::wire::{AskArgs, HubOp, HubReply};
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
    let ask = AskArgs {
      agent: text("agent").unwrap_or_default(),
      prompt: text("prompt").unwrap_or_default(),
      title: text("title"),
      mode: text("mode"),
      thread: text("thread"),
      wait_secs: Some(self.wait_secs.load(std::sync::atomic::Ordering::Relaxed)),
    };
    let token = params.get("_meta").and_then(|m| m.get("progressToken")).cloned();
    let mut n = 0u64;
    let mut end: Option<Value> = None;
    let r = crate::relay::hub::call(&self.addr, &self.request(HubOp::Ask(ask)), |reply| match reply {
      HubReply::Progress(message) => {
        if let Some(t) = &token {
          n += 1;
          progress(json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": t, "progress": n, "message": message } }));
        }
        true
      }
      HubReply::Done(text) => {
        end = Some(json!({ "content": [{ "type": "text", "text": text }], "isError": false }));
        false
      }
      HubReply::Error(text) => {
        end = Some(tool_error(text));
        false
      }
      HubReply::Personas(_) => true,
    });
    match (end, r) {
      (Some(v), _) => v,
      (None, Err(e)) => tool_error(format!("Acpira could not be reached: {e}")),
      (None, Ok(())) => tool_error("Acpira closed the call without an answer.".into()),
    }
  }
}

/// One incoming message → the reply to write, if any (notifications and responses get none)
pub fn handle_line(line: &str, version: &str) -> Option<Value> {
  let msg: Value = match serde_json::from_str(line) {
    Ok(v) => v,
    Err(e) => return Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } })),
  };
  let id = msg.get("id").cloned().filter(|i| !i.is_null())?;
  let method = msg.get("method").and_then(Value::as_str)?;
  let params = msg.get("params").cloned().unwrap_or(Value::Null);
  let result = match method {
    "initialize" => json!({
      "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION),
      "capabilities": { "tools": { "listChanged": false } },
      "serverInfo": { "name": SERVER_NAME, "title": "Acpira", "version": version },
      "instructions": INSTRUCTIONS,
    }),
    "ping" => json!({}),
    "tools/list" => json!({ "tools": [tool_def(), hooks_tool_def()] }),
    "tools/call" => {
      let name = params.get("name").and_then(Value::as_str).unwrap_or("");
      let empty = Map::new();
      let args = params.get("arguments").and_then(Value::as_object).unwrap_or(&empty);
      match name {
        TOOL_NAME => call_show_image(args),
        HOOKS_TOOL => call_workspace_hooks(args),
        _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": format!("unknown tool: {name}") } })),
      }
    }
    _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {method}") } })),
  };
  Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// `ask_agent`: the personas are the enum of `agent`, and their "when" lines are the menu the model picks from
fn ask_tool_def(personas: &[acpira_shared::subagents::SubagentPersona]) -> Value {
  use acpira_shared::subagents::RelayMode;
  let lines: Vec<String> = personas
    .iter()
    .map(|p| {
      let model = p.model.as_deref().map(|m| format!(" · {m}")).unwrap_or_default();
      let mode = if p.mode == RelayMode::Consult { " · read-only" } else { "" };
      let when = if p.when.trim().is_empty() { String::new() } else { format!(": {}", p.when.trim()) };
      format!("- {} (@{}){when} [{}{model}{mode}]", p.id, p.name, p.agent)
    })
    .collect();
  let description = format!(
    "Summon another coding agent that runs in its own CLI on this same repository, and answer with its reply. \
Use it when one of the agents below fits the task better (a second opinion, a review, fast mechanical edits), and always when \
the user names one with @name: those names are these agents, not your own subagents, so call this tool directly without \
listing or searching for agents. To ask several agents, make one call per agent in the same message; they run in parallel. \
It cannot see this conversation but reads files and runs commands itself: write a short, \
self-contained prompt that points at paths instead of pasting code. To follow up with the same agent, pass the `thread` its \
previous answer named. If the answer says it is still working, call again with that thread and an empty prompt to wait.\n\nAgents:\n{}",
    lines.join("\n")
  );
  let ids: Vec<&str> = personas.iter().map(|p| p.id.as_str()).collect();
  json!({
    "name": ASK_TOOL,
    "title": "Ask another agent",
    "description": description,
    "inputSchema": {
      "type": "object",
      "properties": {
        "agent": { "type": "string", "enum": ids, "description": "Which agent to summon. Not needed with `thread`." },
        "prompt": { "type": "string", "description": "The task or question. Empty only when waiting on a thread." },
        "title": { "type": "string", "description": "A few words naming the task, shown to the user." },
        "mode": { "type": "string", "enum": ["consult", "work"], "description": "consult = read-only opinion; work = may edit files. Defaults to the agent's own setting." },
        "thread": { "type": "string", "description": "Continue the conversation an earlier answer named." },
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

fn tool_def() -> Value {
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

fn hooks_tool_def() -> Value {
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
fn call_workspace_hooks(args: &Map<String, Value>) -> Value {
  let cwd = args.get("cwd").and_then(Value::as_str).map(str::trim).unwrap_or("");
  let status = if cwd.is_empty() {
    "Pass `cwd` (the project's absolute path) to have its current file checked.".to_owned()
  } else {
    crate::hooks::describe(cwd)
  };
  json!({ "content": [{ "type": "text", "text": format!("{}

## This project

{status}", crate::hooks::GUIDE) }], "isError": false })
}

/// Validate the named files; the host shows them once the call completes. Failures go back to the model as a tool error
fn call_show_image(args: &Map<String, Value>) -> Value {
  let one = args.get("path").and_then(Value::as_str).into_iter();
  let many = args.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
  let paths: Vec<&str> = one.chain(many).map(str::trim).filter(|p| !p.is_empty()).collect();
  if paths.is_empty() {
    return tool_error("Pass `path` (or `paths`) with the absolute path of an image file.".into());
  }
  let problems: Vec<String> = paths.iter().filter_map(|p| check_image(p).err()).collect();
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

fn tool_error(text: String) -> Value {
  json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn call(line: Value) -> Value {
    handle_line(&line.to_string(), "9.9.9").expect("reply")
  }

  #[test]
  fn initialize_echoes_the_protocol_version_and_lists_the_tool() {
    let init = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }));
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["version"], "9.9.9");
    assert!(handle_line(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(), "").is_none());
    let list = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    assert_eq!(list["result"]["tools"][0]["name"], "show_image");
    // The Markdown route is spelled out in the tool description, which every CLI hands the model (server instructions are optional)
    assert!(list["result"]["tools"][0]["description"].as_str().unwrap().contains("![caption](/abs/path.png)"));
    let unknown = call(json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }));
    assert_eq!(unknown["error"]["code"], -32601);
  }

  #[test]
  fn show_image_accepts_an_existing_absolute_image_and_reports_what_is_wrong_otherwise() {
    let dir = std::env::temp_dir().join(format!("acpira-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("shot.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
    let ok = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "show_image", "arguments": { "path": png } } }));
    assert_eq!(ok["result"]["isError"], false);
    assert!(ok["result"]["content"][0]["text"].as_str().unwrap().contains("shot.png"));
    let bad = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "show_image",
      "arguments": { "paths": ["shot.png", dir.join("missing.png"), dir.join("notes.txt")] } } }));
    assert_eq!(bad["result"]["isError"], true);
    let text = bad["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not an absolute path") && text.contains("missing.png") && text.contains("not a PNG"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn workspace_hooks_explains_the_format_and_checks_the_project_file() {
    let list = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    assert_eq!(list["result"]["tools"][1]["name"], HOOKS_TOOL);
    let init = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {} }));
    assert!(init["result"]["instructions"].as_str().unwrap().contains("workspace_hooks"));

    let dir = std::env::temp_dir().join(format!("acpira-mcp-hooks-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join(".agents")).unwrap();
    let ask = |id: u32| {
      let r = call(json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": HOOKS_TOOL, "arguments": { "cwd": dir } } }));
      assert_eq!(r["result"]["isError"], false);
      r["result"]["content"][0]["text"].as_str().unwrap().to_owned()
    };
    let none = ask(3);
    assert!(none.contains("\"beforeEdit\"") && none.contains("permissionDecision") && none.contains("has no `.agents/hooks.json` yet"), "{none}");
    std::fs::write(dir.join(".agents/hooks.json"), r#"{ "afterTurn": "x", "context": ["NOPE.md"] }"#).unwrap();
    let valid = ask(4);
    assert!(valid.contains("is valid") && valid.contains("NOPE.md"), "{valid}");
    std::fs::write(dir.join(".agents/hooks.json"), r#"{ "afterturn": "x" }"#).unwrap();
    assert!(ask(5).contains("is invalid"));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn ask_agent_runs_concurrently_and_stays_loaded_in_claude_code() {
    let persona: acpira_shared::subagents::SubagentPersona =
      serde_json::from_value(json!({ "id": "son-1", "name": "Son", "agent": "codex", "mode": "work" })).unwrap();
    let def = ask_tool_def(&[persona]);
    assert_eq!(def["name"], ASK_TOOL);
    assert_eq!(def["annotations"]["readOnlyHint"], true);
    // Kept out of ToolSearch, so a summon is one step
    assert_eq!(def["_meta"]["anthropic/alwaysLoad"], true);
    assert!(def["description"].as_str().unwrap().contains("- son-1 (@Son)"));
  }

  #[test]
  fn the_host_entry_is_found_and_removed_by_name_and_subcommand() {
    let req = json!({ "cwd": "/w", "mcpServers": [{ "name": "user", "command": "x", "args": [], "env": [] }, server_entry("/bin/acpira")] });
    assert!(has_server(&req));
    let stripped = without_server(req);
    assert!(!has_server(&stripped));
    assert_eq!(stripped["mcpServers"].as_array().unwrap().len(), 1);
  }
}
