//! `acpira mcp`: the MCP server Acpira hands every agent in `session/new` / `load` / `resume` (`mcpServers`).
//! It offers `show_image`, that lets the model put a local image in front of the user: the call's arguments
//! name the files, and the host reads them into the session's blob store when the tool call completes
//! (`normalize/content.rs` `attach_shown_images`). The server itself only validates the paths and answers a receipt, so no
//! pixels travel back through the model's context. `show_widget` does the same for one self-contained HTML file, which
//! the webview runs as an interactive widget in a sandboxed frame (`attach_shown_widget`, `webview/chat/AgentWidget.tsx`).
//!
//! `workspace_hooks` explains `.agents/hooks.json` (`hooks/`, the gates Acpira runs for every agent) and checks a
//! project's copy, so whichever model the user asks to add a gate writes one Acpira can read.
//!
//! With the relay env on its entry (`relay/`), it also offers `ask_agent`: summon one of the user's cross-harness
//! personas. That call is forwarded to the sidecar over its loopback hub and answers with the child's reply. Over the
//! same hub, `open_browser` shows an http(s) page in the user's editor, offered only while the serving shell can, and
//! `search_sessions` / `read_session` read other Acpira sessions in bounded pieces (`session_library.rs`).
//!
//! Wire: newline-delimited JSON-RPC 2.0 over stdio (MCP stdio transport). stdout carries protocol messages only.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::sync::Arc;

use serde_json::{Map, Value, json};

// The names live in acpira-shared, where the built-in agent's permission rules list them one by one
pub use acpira_shared::host_tools::{
  ASK_AGENT as ASK_TOOL, ASK_CALLER as CALLER_TOOL, OPEN_BROWSER as BROWSER_TOOL, READ_SESSION as READ_SESSION_TOOL, SEARCH_SESSIONS as SEARCH_SESSIONS_TOOL,
  SERVER as SERVER_NAME, SHOW_IMAGE as TOOL_NAME, SHOW_WIDGET as WIDGET_TOOL, WORKSPACE_HOOKS as HOOKS_TOOL,
};

mod relay_client;
mod tools;

use self::relay_client::{Relay, listing_print};
use self::tools::{
  ask_tool_def, browser_tool_def, call_show_image, call_show_widget, call_workspace_hooks, caller_tool_def, hooks_tool_def, plugin_activate_def,
  plugin_guide_def, session_tool_defs, tool_def, tool_error, widget_tool_def,
};
pub use self::tools::widget_mime_of;

/// Answered when the client does not name a protocol version
const PROTOCOL_VERSION: &str = "2025-06-18";

const INSTRUCTIONS: &str = "The user reads this conversation in Acpira, which can display images and interactive HTML widgets inline. \
When the user should see an image (a screenshot you took, a chart or diagram you rendered, a generated or downloaded picture), \
call show_image with its absolute path instead of only printing the path. When a small interactive view would serve them better \
than more text (tuning parameters with sliders, comparing options in tabs, exploring numbers), write it as one HTML file and \
call show_widget. Project gates and reviews live in `.agents/hooks.json`, which Acpira runs for every agent: before writing or changing that file (or a hook script it names), call workspace_hooks for the format, and again afterwards to check it.";

/// Appended to the instructions when the relay env is on the entry. Server instructions reach the model even where
/// the client defers tools behind a search, so `@name` is tied to `ask_agent` before the model goes looking for it
const RELAY_INSTRUCTIONS: &str = "When the user writes @name for an agent listed by the ask_agent tool, that is an \
Acpira agent, not one of your own subagents: call ask_agent for it right away, without listing agents first. \
Several agents named = one ask_agent call each, all in the same message; they run in parallel. A group named = one call \
with the group as `agent`, which summons every member.";

/// Appended with the relay env: the session tools are found by their links even where the client defers them
const SESSIONS_INSTRUCTIONS: &str = "Links like `acpira://session/<id>` or `vscode://hotic.acpira/session/<id>` point at \
another Acpira session, whichever agent ran it. Read one with read_session (its outline first, then only the turns you need) \
instead of opening files under ~/.acpira; search_sessions finds earlier sessions when the user refers to one without a link. \
When a long session would flood your context, ask_agent (if offered) can hand the reading to a read-only agent that answers \
with the conclusion only. When you mention a session, write the Markdown link the tools gave for it.";

/// Appended with the relay env: the model learns that it may extend Acpira itself (`docs/dev/plugins.md`)
const PLUGINS_INSTRUCTIONS: &str = "You can extend Acpira for every agent without a build or a restart: a plugin is a folder \
under ~/.acpira/plugins/<id>/ with a plugin.json (script tools, MCP servers, hooks, a theme, prompt sections or an agent). \
Call plugin_guide for the format and examples, write the folder, then call plugin_activate: the user confirms on a card.";

pub use acpira_shared::host_tools::{PLUGIN_ACTIVATE, PLUGIN_GUIDE};

/// Appended to the instructions of a summoned child's server, so a deferred `ask_caller` is still found
const CALLER_INSTRUCTIONS: &str = "Another agent summoned you through Acpira and reads only your final reply. When a decision \
only it can make blocks you (a choice between approaches it would care about, a missing requirement), ask it with ask_caller \
and wait for the answer; for anything you can reasonably decide yourself, decide and say what you assumed.";

/// Appended to the instructions while the serving shell can show pages, so a deferred `open_browser` is still found
const BROWSER_INSTRUCTIONS: &str = "When the user wants to see a web page (a local dev server, a tool's web UI), call open_browser \
with its URL: Acpira opens it in a browser tab inside their editor, forwarding the port when the workspace is remote. \
Do not tell them to open it by hand unless the call fails.";

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

  /// The entry for a project stdio MCP server on the SSH host of the session's folder: a local `acpira mcp-host` shim
  /// whose grant names the session and whose bytes go to the server there (`run_host_stdio`); None without a hub
  pub fn host_stdio_entry(&self, session: &std::sync::Arc<crate::acp::session::AcpSession>, name: &str) -> Option<Value> {
    let hub = self.hub.as_ref()?;
    let mut env = hub.env(session, None, 0);
    env.as_array_mut()?.push(json!({ "name": crate::relay::ENV_HOST_SERVER, "value": name }));
    Some(json!({ "name": name, "command": self.entry["command"], "args": ["mcp-host"], "env": env }))
  }

  pub fn refuse(&self, agent: &str) {
    self.refused.lock().insert(agent.to_owned());
  }

  /// The agent failed to start the server once and gets no entry any more
  pub fn refused(&self, agent: &str) -> bool {
    self.refused.lock().contains(agent)
  }
}

/// The `mcpServers` entry for a session request (ACP stdio server: no `type` field, env as name/value pairs)
pub fn server_entry(exe: &str) -> Value {
  json!({ "name": SERVER_NAME, "command": exe, "args": ["mcp"], "env": [] })
}

/// Add the host's entry to a session's `mcpServers`. Its name is the engine's own: a shared server called the same (a
/// project's .mcp.json) is left out first, or an agent that keeps the first of two names (the built-in agent) would drop
/// Acpira's server and trust that one's tools as Acpira's. True when one was left out
pub fn push_entry(servers: &mut Vec<Value>, entry: Option<Value>) -> bool {
  let Some(entry) = entry else { return false };
  let before = servers.len();
  servers.retain(|s| s.get("name").and_then(Value::as_str) != Some(SERVER_NAME));
  let shadowed = servers.len() < before;
  servers.push(entry);
  shadowed
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

pub fn is_host_entry(s: &Value) -> bool {
  s.get("name").and_then(Value::as_str) == Some(SERVER_NAME) && s.get("args").and_then(Value::as_array).is_some_and(|a| a.first().and_then(Value::as_str) == Some("mcp"))
}

/// `acpira mcp-host`: stand in for a project MCP server on the SSH host of the session's folder. The agent starts this
/// like any stdio server; the hub starts the real one there (`HubOp::HostStdio`), and this process copies stdin to it
/// and its output back. Its exit code is the server's
pub fn run_host_stdio() -> i32 {
  use crate::relay::hub::Frame;
  use crate::relay::wire::{HubOp, HubReply, HubRequest};
  use std::io::{BufRead, Read, Write};
  let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
  let (Some(addr), Some(token), Some(name)) = (var(crate::relay::ENV_ADDR), var(crate::relay::ENV_TOKEN), var(crate::relay::ENV_HOST_SERVER)) else {
    eprintln!("acpira mcp-host: started without the relay's environment");
    return 2;
  };
  let sock = match std::net::TcpStream::connect(&addr) {
    Ok(s) => s,
    Err(e) => {
      eprintln!("acpira mcp-host: the relay cannot be reached: {e}");
      return 1;
    }
  };
  let mut line = serde_json::to_string(&HubRequest { token, op: HubOp::HostStdio { name: name.clone() } }).unwrap_or_default();
  line.push('\n');
  let Ok(mut up) = sock.try_clone() else { return 1 };
  if up.write_all(line.as_bytes()).is_err() {
    return 1;
  }
  let mut down = std::io::BufReader::new(sock);
  let mut first = String::new();
  if down.read_line(&mut first).is_err() {
    return 1;
  }
  match serde_json::from_str::<HubReply>(first.trim()) {
    Ok(HubReply::Done(_)) => {}
    Ok(HubReply::Error(e)) => {
      eprintln!("acpira mcp-host: {name} did not start on the host: {e}");
      return 1;
    }
    _ => {
      eprintln!("acpira mcp-host: the relay answered something else");
      return 1;
    }
  }
  std::thread::spawn(move || {
    let mut stdin = std::io::stdin().lock();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
      match stdin.read(&mut buf) {
        Ok(0) | Err(_) => break,
        Ok(n) => {
          if up.write_all(&buf[..n]).is_err() {
            return;
          }
        }
      }
    }
    let _ = up.shutdown(std::net::Shutdown::Write);
  });
  let (mut stdout, mut stderr) = (std::io::stdout(), std::io::stderr());
  let mut head = [0u8; 5];
  loop {
    if down.read_exact(&mut head).is_err() {
      return 1;
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let mut body = vec![0u8; len];
    if down.read_exact(&mut body).is_err() {
      return 1;
    }
    match head[0] {
      t if t == Frame::Out as u8 => {
        if stdout.write_all(&body).and_then(|_| stdout.flush()).is_err() {
          return 1;
        }
      }
      t if t == Frame::Err as u8 => {
        let _ = stderr.write_all(&body);
      }
      _ => {
        let end = String::from_utf8_lossy(&body);
        return match end.parse::<i32>() {
          Ok(code) => code,
          Err(_) => {
            eprintln!("acpira mcp-host: {name} on the host: {end}");
            1
          }
        };
      }
    }
  }
}

/// Serve MCP on stdin / stdout until stdin closes. `ask_agent` calls run on threads of their own (an agent may summon
/// several children at once) and share stdout through a lock; everything else answers inline
/// The server's stdout, and the calls whose results the plugins' `tool.after` hooks see before the client does
struct Out {
  stdout: parking_lot::Mutex<std::io::Stdout>,
  /// By the call's JSON-RPC id: the tool and its arguments
  after: parking_lot::Mutex<std::collections::HashMap<String, (String, Value)>>,
  relay: Option<Relay>,
  /// The hub sockets of the project tool calls running on an SSH host, by the call's JSON-RPC id: the client's cancel
  /// shuts one down, which cancels the run there
  running: parking_lot::Mutex<std::collections::HashMap<String, std::net::TcpStream>>,
}

impl Out {
  /// A call's response, as the `tool.after` hooks left it when one asked to see it
  fn reviewed(&self, v: &Value) -> Option<Value> {
    let id = v.get("id").filter(|_| v.get("result").is_some())?.to_string();
    let (name, arguments) = self.after.lock().remove(&id)?;
    let relay = self.relay.as_ref()?;
    let answer = relay.tool_hook("after", &name, &arguments, v.get("result"));
    let result = answer.get("result").filter(|r| r.is_object())?;
    let mut next = v.clone();
    next["result"] = result.clone();
    Some(next)
  }
}

pub fn run(version: &str) -> i32 {
  let relay = Relay::from_env();
  let out = Arc::new(Out { stdout: parking_lot::Mutex::new(std::io::stdout()), after: Default::default(), relay: relay.clone(), running: Default::default() });
  let write = |out: &Out, v: &Value| {
    let reviewed = out.reviewed(v);
    let v = reviewed.as_ref().unwrap_or(v);
    let mut o = out.stdout.lock();
    writeln!(o, "{v}").and_then(|_| o.flush()).is_ok()
  };
  for line in std::io::stdin().lock().lines() {
    let Ok(mut line) = line else { break };
    if line.trim().is_empty() {
      continue;
    }
    // The plugins' `tool.before` hooks may refuse a call or change its arguments; a `tool.after` hook marks the call
    // for its result
    if let Some(relay) = &relay
      && let Ok(mut msg) = serde_json::from_str::<Value>(&line)
      && msg.get("method").and_then(Value::as_str) == Some("tools/call")
      && let Some(id) = msg.get("id").cloned()
      && let Some(name) = msg.pointer("/params/name").and_then(Value::as_str).map(str::to_owned)
    {
      let arguments = msg.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
      let hook = relay.tool_hook("before", &name, &arguments, None);
      if let Some(reason) = hook.get("block").and_then(Value::as_str) {
        if !write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": tool_error(reason.to_owned()) })) {
          break;
        }
        continue;
      }
      let arguments = match hook.get("arguments").filter(|a| a.is_object() && **a != arguments) {
        Some(next) => {
          msg["params"]["arguments"] = next.clone();
          line = msg.to_string();
          next.clone()
        }
        None => arguments,
      };
      if hook.get("after") == Some(&Value::Bool(true)) {
        out.after.lock().insert(id.to_string(), (name, arguments));
      }
    }
    let mut initialize = false;
    let mut features = crate::relay::wire::HostFeatures::default();
    if let Some(relay) = &relay
      && let Ok(msg) = serde_json::from_str::<Value>(&line)
    {
      let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
      if method == "initialize" {
        relay.note_client(msg.get("params"));
        initialize = true;
        features = relay.features();
      }
      if method == "tools/list"
        && let Some(id) = msg.get("id").cloned()
      {
        let mut tools = vec![tool_def(), widget_tool_def(), hooks_tool_def()];
        let roster = relay.roster();
        let plugins = relay.plugins();
        relay.served(listing_print(roster.as_ref(), plugins.as_ref()));
        tools.extend(roster.and_then(|(personas, groups)| (!personas.is_empty()).then(|| ask_tool_def(&personas, &groups))));
        let features = relay.features();
        if features.summoned {
          tools.push(caller_tool_def());
        }
        if features.browser {
          tools.push(browser_tool_def());
        }
        tools.extend(session_tool_defs());
        // A Claude Code session in a folder on an SSH host: the project tools that run there
        if let Some(alias) = &features.remote_tools {
          tools.extend(crate::acp::vendors::claude_remote::tool_defs(alias));
        }
        // The tool groups the plugin page switched off leave every agent's list; the plugins' own tools join it
        let (plugin_tools, off) = plugins.unwrap_or_default();
        tools.retain(|t| t["name"].as_str().and_then(acpira_shared::host_tools::group_of).is_none_or(|g| !off.iter().any(|o| o == g)));
        tools.extend([plugin_guide_def(), plugin_activate_def()]);
        tools.extend(plugin_tools.into_iter().filter(|t| t["name"].as_str().is_some_and(|n| !acpira_shared::host_tools::is_engine_tool(n))));
        if !write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } })) {
          break;
        }
        continue;
      }
      // A tool of a group switched off since the client listed: refused, as the list no longer has it
      if method == "tools/call"
        && let Some(name) = msg.get("params").and_then(|p| p.get("name")).and_then(Value::as_str)
        && let Some(group) = acpira_shared::host_tools::group_of(name)
        && let Some(id) = msg.get("id").cloned()
        && relay.plugins().is_some_and(|(_, off)| off.iter().any(|o| o == group))
      {
        let result = tool_error(format!("`{name}` was switched off on Acpira's plugin page."));
        if !write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result })) {
          break;
        }
        continue;
      }
      // The plugin tools of the server itself, and every name that is not the engine's: a plugin's script tool
      if method == "tools/call"
        && let Some(name) = msg.get("params").and_then(|p| p.get("name")).and_then(Value::as_str)
        && (name == PLUGIN_GUIDE || name == PLUGIN_ACTIVATE || !acpira_shared::host_tools::is_engine_tool(name))
        && let Some(id) = msg.get("id").cloned()
      {
        let (relay, out, params, name) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null), name.to_owned());
        std::thread::spawn(move || {
          let mut progress = |progress: Value| {
            write(&out, &progress);
          };
          let result = match name.as_str() {
            PLUGIN_GUIDE => relay.answer(crate::relay::wire::HubOp::PluginGuide),
            PLUGIN_ACTIVATE => relay.plugin_activate(&params, &mut progress),
            _ => relay.answer(crate::relay::wire::HubOp::PluginCall { name, arguments: params.get("arguments").cloned().unwrap_or(json!({})) }),
          };
          write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        });
        continue;
      }
      if method == "tools/call"
        && let Some(name @ (ASK_TOOL | CALLER_TOOL)) = msg.get("params").and_then(|p| p.get("name")).and_then(Value::as_str)
        && let Some(id) = msg.get("id").cloned()
      {
        let (relay, out, params) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null));
        let caller = name == CALLER_TOOL;
        std::thread::spawn(move || {
          let mut progress = |progress: Value| {
            write(&out, &progress);
          };
          let result = if caller { relay.ask_caller(&params, &mut progress) } else { relay.ask(&params, &mut progress) };
          write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        });
        continue;
      }
      // Session reads answer from the sidecar's warm caches, but each still gets its own thread (a model reads several at once)
      let name = msg.get("params").and_then(|p| p.get("name")).and_then(Value::as_str);
      // A project tool of a Claude Code session on an SSH host: run there through the hub, each on a thread of its own (the
      // model reads several files at once). A cancelled call gets no answer, as MCP asks
      if method == "tools/call"
        && name.is_some_and(|n| acpira_shared::host_tools::REMOTE.contains(&n))
        && let Some(id) = msg.get("id").cloned()
      {
        let (relay, out, params, key) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null), id.to_string());
        std::thread::spawn(move || {
          let result = relay.remote_tool(&params, |sock| {
            if let Ok(s) = sock.try_clone() {
              out.running.lock().insert(key.clone(), s);
            }
          });
          if out.running.lock().remove(&key).is_some() {
            write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
          }
        });
        continue;
      }
      if method == "notifications/cancelled" {
        if let Some(sock) = msg.pointer("/params/requestId").and_then(|id| out.running.lock().remove(&id.to_string())) {
          let _ = sock.shutdown(std::net::Shutdown::Both);
        }
        continue;
      }
      if method == "tools/call"
        && matches!(name, Some(SEARCH_SESSIONS_TOOL | READ_SESSION_TOOL))
        && let Some(id) = msg.get("id").cloned()
      {
        let (relay, out, params) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null));
        std::thread::spawn(move || {
          let result = relay.session_tool(&params);
          write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        });
        continue;
      }
      // A session folder on an SSH host: the files are checked and read there, through the hub
      if method == "tools/call"
        && matches!(name, Some(TOOL_NAME | WIDGET_TOOL | HOOKS_TOOL))
        && let Some(id) = msg.get("id").cloned()
        && relay.features().remote
      {
        let (relay, out, params) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null));
        std::thread::spawn(move || {
          let result = relay.show_on_host(&params);
          write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        });
        continue;
      }
      if method == "tools/call"
        && msg.get("params").and_then(|p| p.get("name")).and_then(Value::as_str) == Some(BROWSER_TOOL)
        && let Some(id) = msg.get("id").cloned()
      {
        let (relay, out, params) = (relay.clone(), out.clone(), msg.get("params").cloned().unwrap_or(Value::Null));
        std::thread::spawn(move || {
          let result = relay.open_browser(&params);
          write(&out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        });
        continue;
      }
    }
    let Some(mut reply) = handle_line(&line, version) else { continue };
    if initialize && let Some(result) = reply.get_mut("result") {
      let mut extra = String::new();
      if features.summoned {
        extra.push_str(&format!("\n\n{CALLER_INSTRUCTIONS}"));
      }
      if features.browser {
        extra.push_str(&format!("\n\n{BROWSER_INSTRUCTIONS}"));
      }
      result["instructions"] = json!(format!("{INSTRUCTIONS}\n\n{RELAY_INSTRUCTIONS}{extra}\n\n{SESSIONS_INSTRUCTIONS}\n\n{PLUGINS_INSTRUCTIONS}"));
      // ask_agent's agents follow the roster while the session runs (see `Relay::watch`)
      result["capabilities"]["tools"]["listChanged"] = json!(true);
    }
    if !write(&out, &reply) {
      break;
    }
    if initialize && let Some(relay) = &relay {
      let out = out.clone();
      relay.watch(move |v| write(&out, v));
    }
  }
  0
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
    "tools/list" => json!({ "tools": [tool_def(), widget_tool_def(), hooks_tool_def()] }),
    "tools/call" => {
      let name = params.get("name").and_then(Value::as_str).unwrap_or("");
      let empty = Map::new();
      let args = params.get("arguments").and_then(Value::as_object).unwrap_or(&empty);
      match name {
        TOOL_NAME => call_show_image(args),
        WIDGET_TOOL => call_show_widget(args),
        HOOKS_TOOL => call_workspace_hooks(args),
        _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": format!("unknown tool: {name}") } })),
      }
    }
    _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {method}") } })),
  };
  Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[cfg(test)]
mod tests {
  use super::relay_client::WatchState;
  use super::*;
  use crate::limits::MAX_WIDGET_BYTES;

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
  fn every_tool_served_is_on_the_engine_s_list() {
    // The built-in agent runs exactly these without a card; a tool missing here would ask, one listed but not served
    // would leave a name a plugin could not take
    let persona: acpira_shared::subagents::SubagentPersona =
      serde_json::from_value(json!({ "id": "son-1", "name": "Son", "agent": "codex", "mode": "work" })).unwrap();
    let mut tools = vec![tool_def(), widget_tool_def(), hooks_tool_def(), ask_tool_def(std::slice::from_ref(&persona), &[]), caller_tool_def(), browser_tool_def(), plugin_guide_def(), plugin_activate_def()];
    tools.extend(session_tool_defs());
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    let mut all = acpira_shared::host_tools::ALL.to_vec();
    all.sort_unstable();
    assert_eq!(names, all);
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
  fn show_widget_takes_one_existing_html_file_and_teaches_the_sandbox() {
    let list = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    let def = &list["result"]["tools"][1];
    assert_eq!(def["name"], WIDGET_TOOL);
    assert_eq!(def["annotations"]["readOnlyHint"], true);
    // What the model must know lives in the description, which every CLI hands it: no network, Tailwind, the theme colours
    let text = def["description"].as_str().unwrap();
    assert!(text.contains("no network") && text.contains("Tailwind") && text.contains("bg-background"), "{text}");
    let init = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {} }));
    assert!(init["result"]["instructions"].as_str().unwrap().contains("show_widget"));

    let dir = std::env::temp_dir().join(format!("acpira-mcp-widget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("Tuner.HTML");
    std::fs::write(&page, "<div class=\"p-4\">hi</div>").unwrap();
    let ok = call(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": WIDGET_TOOL, "arguments": { "path": page } } }));
    assert_eq!(ok["result"]["isError"], false);
    assert!(ok["result"]["content"][0]["text"].as_str().unwrap().contains("Tuner.HTML"));
    let big = dir.join("big.html");
    std::fs::write(&big, vec![b'x'; MAX_WIDGET_BYTES as usize + 1]).unwrap();
    for (path, problem) in [
      (json!("tuner.html"), "not an absolute path"),
      (json!(dir.join("notes.txt")), "not an .html file"),
      (json!(dir.join("missing.html")), "missing.html"),
      (json!(big), "larger than 512 KB"),
      (json!(""), "Pass `path`"),
    ] {
      let bad = call(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": WIDGET_TOOL, "arguments": { "path": path } } }));
      assert_eq!(bad["result"]["isError"], true);
      let text = bad["result"]["content"][0]["text"].as_str().unwrap();
      assert!(text.contains(problem), "{text}");
    }
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn workspace_hooks_explains_the_format_and_checks_the_project_file() {
    let list = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    assert_eq!(list["result"]["tools"][2]["name"], HOOKS_TOOL);
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
    assert!(ask(5).contains("Unknown keys, skipped: afterturn"));
    std::fs::write(dir.join(".agents/hooks.json"), r#"{ "afterTurn": 3 }"#).unwrap();
    assert!(ask(6).contains("is invalid"));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn ask_agent_runs_concurrently_and_stays_loaded_in_claude_code() {
    let persona: acpira_shared::subagents::SubagentPersona =
      serde_json::from_value(json!({ "id": "son-1", "name": "Son", "agent": "codex", "mode": "work" })).unwrap();
    let def = ask_tool_def(std::slice::from_ref(&persona), &[]);
    assert_eq!(def["name"], ASK_TOOL);
    assert_eq!(def["annotations"]["readOnlyHint"], true);
    // Kept out of ToolSearch, so a summon is one step
    assert_eq!(def["_meta"]["anthropic/alwaysLoad"], true);
    assert!(def["description"].as_str().unwrap().contains("- son-1 (@Son) [codex]"));
    // A duty is named on the persona's line, so the caller knows which ones keep their conversation
    let scout: acpira_shared::subagents::SubagentPersona =
      serde_json::from_value(json!({ "id": "look", "name": "Look", "agent": "kimi", "mode": "consult", "duty": "scout" })).unwrap();
    let def = ask_tool_def(std::slice::from_ref(&scout), &[]);
    assert!(def["description"].as_str().unwrap().contains("- look (@Look) [scout · kimi · read-only]"), "{}", def["description"]);
    // The effort is named apart from the model id, which may carry a level of its own
    let devin: acpira_shared::subagents::SubagentPersona = serde_json::from_value(
      json!({ "id": "gem", "name": "Gem", "agent": "devin", "model": "gemini-4-argon-medium", "effort": "high", "mode": "work" }),
    )
    .unwrap();
    let def = ask_tool_def(std::slice::from_ref(&devin), &[]);
    assert!(def["description"].as_str().unwrap().contains("[devin · gemini-4-argon-medium · high effort]"), "{}", def["description"]);
    assert_eq!(def["inputSchema"]["properties"]["new_thread"]["type"], "boolean");
    assert!(!def["description"].as_str().unwrap().contains("Groups:"));
    // A group is one more value of `agent`, listed with its members under the agents
    let group: acpira_shared::subagents::SubagentGroup =
      serde_json::from_value(json!({ "id": "pair", "name": "Pair", "members": ["son-1", "look"], "when": "two views at once" })).unwrap();
    let def = ask_tool_def(&[persona, scout], &[group]);
    assert_eq!(def["inputSchema"]["properties"]["agent"]["enum"], json!(["son-1", "look", "pair"]));
    let text = def["description"].as_str().unwrap();
    assert!(text.contains("\n\nGroups: one call naming a group") && text.ends_with("- pair (@Pair): two views at once [group: son-1, look]"), "{text}");
  }

  // Listed with the relay env (tools/list in `run`): read-only so Claude Code runs them side by side, found through a
  // search hint and the server instructions rather than kept loaded
  #[test]
  fn session_tools_are_read_only_and_deferred() {
    let defs = session_tool_defs();
    assert_eq!(defs.iter().map(|d| d["name"].as_str().unwrap()).collect::<Vec<_>>(), vec![SEARCH_SESSIONS_TOOL, READ_SESSION_TOOL]);
    for d in &defs {
      assert_eq!(d["annotations"]["readOnlyHint"], true);
      assert!(d["_meta"]["anthropic/searchHint"].is_string());
      assert!(d["_meta"].get("anthropic/alwaysLoad").is_none());
    }
    assert_eq!(defs[1]["inputSchema"]["required"], json!(["session"]));
    assert!(SESSIONS_INSTRUCTIONS.contains("acpira://session/<id>") && SESSIONS_INSTRUCTIONS.contains("read_session"));
  }

  #[test]
  fn a_roster_change_is_told_once_until_listed_and_again_after_a_change_back_and_forth() {
    let mut st = WatchState::default();
    // Nothing listed yet: nothing to tell
    assert!(!st.poll("a".into()));
    st = WatchState { served: Some("a".into()), notified: None };
    assert!(!st.poll("a".into()));
    assert!(st.poll("b".into()));
    assert!(!st.poll("b".into()), "told twice of one change");
    // Changed back before the client listed again, then once more: the second change is news
    assert!(!st.poll("a".into()));
    assert!(st.poll("b".into()));
  }

  #[test]
  fn the_host_entry_is_found_and_removed_by_name_and_subcommand() {
    let req = json!({ "cwd": "/w", "mcpServers": [{ "name": "user", "command": "x", "args": [], "env": [] }, server_entry("/bin/acpira")] });
    assert!(has_server(&req));
    let stripped = without_server(req);
    assert!(!has_server(&stripped));
    assert_eq!(stripped["mcpServers"].as_array().unwrap().len(), 1);
  }

  #[test]
  fn a_shared_server_named_like_the_host_gives_way_to_it() {
    let user = json!({ "name": "user", "command": "x", "args": [], "env": [] });
    let mut servers = vec![user.clone(), json!({ "name": SERVER_NAME, "command": "sh", "args": ["evil.sh"], "env": [] })];
    assert!(push_entry(&mut servers, Some(server_entry("/bin/acpira"))));
    assert_eq!(servers, vec![user.clone(), server_entry("/bin/acpira")]);
    // Nothing to shadow, and no entry (the agent refused the server) leaves the list alone
    let mut plain = vec![user.clone()];
    assert!(!push_entry(&mut plain, Some(server_entry("/bin/acpira"))) && plain.len() == 2);
    let mut refused = vec![user];
    assert!(!push_entry(&mut refused, None) && refused.len() == 1);
  }
}
