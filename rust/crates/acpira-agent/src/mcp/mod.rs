//! The MCP client: the servers a session is handed in `mcpServers` (`session/new`, `load`, `resume`). The engine puts
//! there its own `acpira mcp` (show_image, workspace_hooks, ask_agent) and the user's shared servers
//! (`~/.agents/mcp.json`, the project's `.mcp.json`); stdio and streamable HTTP are spoken, the legacy SSE transport is
//! not (`initialize` advertises `mcpCapabilities.http` only, so the engine never sends one).
//!
//! Every server starts connecting when the session opens; a turn waits for the ones still starting (each bounded by
//! `START_TIMEOUT`) before it freezes its tool set, so the set is the same from the first request on and a slow
//! `npx` server does not hold up `session/new`. A server's tools reach the model as `<server>_<tool>` (OpenCode's
//! naming, which the engine already recognizes for `acpira_show_image` and `acpira_ask_agent`), behind the built-in
//! tools, in the order the servers were given. A `tools/list_changed` notification makes the next turn list again

mod http;
mod stdio;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::llm::ToolSpec;

/// The protocol revision offered in `initialize`; a server may answer with an older one, which is then sent back in
/// the HTTP transport's `MCP-Protocol-Version` header
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long a server may take to start and list its tools (Claude Code's `MCP_TIMEOUT` default)
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one tool call may run. Cancelling the turn ends the wait sooner; nothing is sent to the server then
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The longest tool name providers accept (OpenAI and Anthropic: `^[a-zA-Z0-9_-]{1,64}$`)
const MAX_NAME: usize = 64;

/// Progress a server reports during a call (`notifications/progress`), as one line of text
pub type Progress = Arc<dyn Fn(String) + Send + Sync>;

/// One entry of the ACP `mcpServers` list
#[derive(Debug, Clone, PartialEq)]
pub enum Spec {
  Stdio { name: String, command: String, args: Vec<String>, env: Vec<(String, String)> },
  Http { name: String, url: String, headers: Vec<(String, String)> },
}

impl Spec {
  pub fn name(&self) -> &str {
    match self {
      Spec::Stdio { name, .. } | Spec::Http { name, .. } => name,
    }
  }
}

/// The ACP `mcpServers` list as specs, plus a note per entry that cannot be used. A stdio entry has no `type` (or
/// `stdio`); `env` and `headers` are `[{ name, value }]` lists
pub fn parse_specs(list: &Value) -> (Vec<Spec>, Vec<String>) {
  let (mut specs, mut skipped) = (vec![], vec![]);
  for entry in list.as_array().into_iter().flatten() {
    let s = |k: &str| entry.get(k).and_then(Value::as_str).map(str::to_owned);
    let pairs = |k: &str| -> Vec<(String, String)> {
      entry.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(|p| Some((p.get("name")?.as_str()?.to_owned(), p.get("value")?.as_str()?.to_owned()))).collect()
    };
    let Some(name) = s("name").filter(|n| !n.trim().is_empty()) else {
      skipped.push("an entry without a name".to_owned());
      continue;
    };
    match entry.get("type").and_then(Value::as_str).unwrap_or("stdio") {
      "stdio" => match s("command").filter(|c| !c.is_empty()) {
        Some(command) => {
          let args = entry.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
          specs.push(Spec::Stdio { name, command, args, env: pairs("env") });
        }
        None => skipped.push(format!("{name} (no command)")),
      },
      "http" => match s("url").filter(|u| !u.is_empty()) {
        Some(url) => specs.push(Spec::Http { name, url, headers: pairs("headers") }),
        None => skipped.push(format!("{name} (no url)")),
      },
      other => skipped.push(format!("{name} (the {other} transport is not supported)")),
    }
  }
  (specs, skipped)
}

/// A tool as the model is offered it
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
  /// `<server>_<tool>`, what the model calls
  pub name: String,
  /// The server's key in the hub (its name made safe for tool names)
  pub server: String,
  /// The server's name as configured, for the row's label
  pub server_name: String,
  /// The tool's own name, what `tools/call` takes
  pub tool: String,
  /// The tool's display title (`title`, else `annotations.title`), for the row's target
  pub title: Option<String>,
  pub description: String,
  pub schema: Value,
  /// `annotations.readOnlyHint`: such calls run without a card and side by side with other reads
  pub read_only: bool,
}

impl Tool {
  pub fn spec(&self) -> ToolSpec {
    ToolSpec { name: self.name.clone(), description: self.description.clone(), parameters: self.schema.clone() }
  }

  /// What the tool card carries so the engine can show the row as "Call <Server>" with the tool's title
  /// (`acpira/mcp`, read by `normalize.rs` `mcp_identity`)
  pub fn meta(&self) -> Value {
    json!({ "acpira/mcp": { "server": self.server_name, "tool": self.tool, "title": self.title } })
  }
}

/// A server name as it can stand in a tool name
fn safe(name: &str) -> String {
  name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// A tool's input schema as providers take it: always an object schema with `properties`, without `$schema` (some
/// OpenAI-compatible servers refuse the keyword)
fn schema_of(raw: Option<&Value>) -> Value {
  let mut schema = match raw {
    Some(Value::Object(m)) => m.clone(),
    _ => Map::new(),
  };
  schema.remove("$schema");
  if schema.get("type").and_then(Value::as_str) != Some("object") {
    schema.insert("type".into(), "object".into());
  }
  if !schema.get("properties").is_some_and(Value::is_object) {
    schema.insert("properties".into(), json!({}));
  }
  Value::Object(schema)
}

/// A server's `tools/list` entries as tools, named `<server>_<tool>`; names over the providers' limit or taken already
/// (`taken`: the built-in tools and the servers before this one) are left out with a note
fn tools_of(server: &str, server_name: &str, listed: &[Value], taken: &mut Vec<String>, notes: &mut Vec<String>) -> Vec<Tool> {
  let mut out = vec![];
  for t in listed {
    let Some(tool) = t.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) else { continue };
    let name = format!("{server}_{}", safe(tool));
    if name.len() > MAX_NAME {
      notes.push(format!("{server}: the tool {tool} was left out (its name is over {MAX_NAME} characters)"));
      continue;
    }
    if taken.contains(&name) {
      notes.push(format!("{server}: the tool {tool} was left out ({name} is taken)"));
      continue;
    }
    taken.push(name.clone());
    let s = |k: &str| t.get(k).and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty());
    let description = s("description").or_else(|| s("title")).map(str::to_owned).unwrap_or_else(|| format!("The {tool} tool of the {server} MCP server."));
    let title = s("title").or_else(|| t.pointer("/annotations/title").and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty())).map(str::to_owned);
    out.push(Tool {
      name,
      server: server.to_owned(),
      server_name: server_name.to_owned(),
      tool: tool.to_owned(),
      title,
      description,
      schema: schema_of(t.get("inputSchema")),
      read_only: t.pointer("/annotations/readOnlyHint").and_then(Value::as_bool) == Some(true),
    });
  }
  out
}

enum Transport {
  // Boxed: the stdio side (process, pipes, pending map) is far larger than an `Arc`, which clippy flags on Windows
  Stdio(Box<stdio::Stdio>),
  Http(Arc<http::Http>),
}

impl Transport {
  async fn request(&self, method: &str, params: Value, progress: Option<Progress>, timeout: Duration) -> Result<Value, String> {
    let work = async {
      match self {
        Transport::Stdio(s) => s.request(method, params, progress).await,
        Transport::Http(h) => {
          let (h, method) = (h.clone(), method.to_owned());
          tokio::task::spawn_blocking(move || h.request(&method, params, progress)).await.map_err(|e| format!("the request crashed: {e}"))?
        }
      }
    };
    tokio::time::timeout(timeout, work).await.unwrap_or_else(|_| Err(format!("no answer within {} s", timeout.as_secs())))
  }

  async fn notify(&self, method: &str, params: Value) {
    match self {
      Transport::Stdio(s) => s.notify(method, params),
      Transport::Http(h) => {
        let (h, method) = (h.clone(), method.to_owned());
        let _ = tokio::task::spawn_blocking(move || h.notify(&method, params)).await;
      }
    }
  }

  fn close(&self) {
    match self {
      Transport::Stdio(s) => s.close(),
      Transport::Http(h) => h.close(),
    }
  }
}

/// A started server
struct Live {
  transport: Transport,
  /// The latest `tools/list`
  listed: parking_lot::Mutex<Vec<Value>>,
  instructions: Option<String>,
  /// Set by `notifications/tools/list_changed`: the next turn lists again
  stale: Arc<AtomicBool>,
}

struct Server {
  spec: Spec,
  /// `safe(name)`, the prefix of its tools
  key: String,
  cwd: PathBuf,
  live: tokio::sync::OnceCell<Result<Arc<Live>, String>>,
}

impl Server {
  async fn live(&self) -> Result<Arc<Live>, String> {
    self
      .live
      .get_or_init(|| async {
        let started = tokio::time::timeout(START_TIMEOUT, connect(&self.spec, &self.cwd)).await;
        let result = started.unwrap_or_else(|_| Err(format!("it did not start within {} s", START_TIMEOUT.as_secs())));
        match &result {
          Ok(live) => eprintln!("[acpira agent] MCP {}: {} tools", self.spec.name(), live.listed.lock().len()),
          Err(e) => eprintln!("[acpira agent] MCP {}: {e}", self.spec.name()),
        }
        result
      })
      .await
      .clone()
  }

  /// The started server, if it started (without waiting)
  fn started(&self) -> Option<Arc<Live>> {
    self.live.get().and_then(|r| r.as_ref().ok()).cloned()
  }
}

async fn connect(spec: &Spec, cwd: &Path) -> Result<Arc<Live>, String> {
  let stale = Arc::new(AtomicBool::new(false));
  let transport = match spec {
    Spec::Stdio { name, command, args, env } => Transport::Stdio(Box::new(stdio::Stdio::spawn(name, command, args, env, cwd, stale.clone())?)),
    Spec::Http { url, headers, .. } => Transport::Http(Arc::new(http::Http::new(url, headers, cwd, stale.clone()))),
  };
  let init = json!({
    "protocolVersion": PROTOCOL_VERSION,
    "capabilities": { "roots": { "listChanged": false } },
    // The engine's `acpira mcp` lets an `ask_agent` call of this client wait as long as a child round takes
    "clientInfo": { "name": "acpira", "title": "Acpira", "version": env!("CARGO_PKG_VERSION") },
  });
  let answer = match transport.request("initialize", init, None, START_TIMEOUT).await {
    Ok(a) => a,
    Err(e) => {
      transport.close();
      return Err(format!("initialize failed: {e}"));
    }
  };
  if let (Transport::Http(h), Some(v)) = (&transport, answer.get("protocolVersion").and_then(Value::as_str)) {
    h.set_protocol(v);
  }
  transport.notify("notifications/initialized", json!({})).await;
  let instructions = answer.get("instructions").and_then(Value::as_str).map(str::trim).filter(|i| !i.is_empty()).map(str::to_owned);
  let live = Live { transport, listed: Default::default(), instructions, stale };
  // A server without tools (prompts or resources only) is started all the same: it just offers nothing
  if answer.pointer("/capabilities/tools").is_some() {
    match list(&live.transport).await {
      Ok(tools) => *live.listed.lock() = tools,
      Err(e) => {
        live.transport.close();
        return Err(format!("tools/list failed: {e}"));
      }
    }
  }
  Ok(Arc::new(live))
}

/// Every page of `tools/list`
async fn list(transport: &Transport) -> Result<Vec<Value>, String> {
  let mut out = vec![];
  let mut cursor: Option<String> = None;
  // A server that keeps handing out cursors is cut off rather than followed forever
  for _ in 0..50 {
    let params = match &cursor {
      Some(c) => json!({ "cursor": c }),
      None => json!({}),
    };
    let page = transport.request("tools/list", params, None, START_TIMEOUT).await?;
    out.extend(page.get("tools").and_then(Value::as_array).into_iter().flatten().cloned());
    cursor = page.get("nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
    if cursor.is_none() {
      break;
    }
  }
  Ok(out)
}

/// A session's MCP servers
#[derive(Default)]
pub struct Hub {
  servers: Vec<Arc<Server>>,
  /// Entries that could not be used at all (`parse_specs`)
  skipped: Vec<String>,
}

/// What a turn gets from the hub: the tools in request order, and the note for the model (instructions and failures)
pub struct Offer {
  pub tools: Vec<Tool>,
  pub note: Option<String>,
}

impl Hub {
  /// Start connecting to every server of the list (in the background: `offer` waits for them)
  pub fn start(list: &Value, cwd: &Path) -> Arc<Hub> {
    let (specs, skipped) = parse_specs(list);
    for s in &skipped {
      eprintln!("[acpira agent] MCP skipped: {s}");
    }
    let mut keys: Vec<String> = vec![];
    let mut servers = vec![];
    for spec in specs {
      let key = safe(spec.name());
      // Two entries of one name: the first wins, as the engine's own merge does
      if keys.contains(&key) {
        eprintln!("[acpira agent] MCP skipped: {} (a second server of that name)", spec.name());
        continue;
      }
      keys.push(key.clone());
      servers.push(Arc::new(Server { spec, key, cwd: cwd.to_owned(), live: Default::default() }));
    }
    for s in &servers {
      let s = s.clone();
      tokio::spawn(async move {
        let _ = s.live().await;
      });
    }
    Arc::new(Hub { servers, skipped })
  }

  /// Wait for every server still starting, list again where the list changed, and return the tools after the
  /// built-in ones (`builtin`: their names, which a server's tool cannot take)
  pub async fn offer(&self, builtin: &[&str]) -> Offer {
    let mut taken: Vec<String> = builtin.iter().map(|n| (*n).to_owned()).collect();
    let (mut tools, mut notes, mut failed, mut told) = (vec![], vec![], vec![], vec![]);
    // Every start was spawned when the hub was made, so waiting in order waits only for the slowest
    for server in &self.servers {
      let live = match server.live().await {
        Ok(l) => l,
        Err(e) => {
          failed.push(format!("- {}: {e}", server.spec.name()));
          continue;
        }
      };
      if live.stale.swap(false, Ordering::AcqRel) {
        match list(&live.transport).await {
          Ok(listed) => *live.listed.lock() = listed,
          Err(e) => eprintln!("[acpira agent] MCP {}: listing again failed ({e}); keeping the earlier list", server.spec.name()),
        }
      }
      let listed = live.listed.lock().clone();
      tools.extend(tools_of(&server.key, server.spec.name(), &listed, &mut taken, &mut notes));
      if let Some(i) = &live.instructions {
        told.push(format!("## {}\n{i}", server.key));
      }
    }
    for n in &notes {
      eprintln!("[acpira agent] MCP {n}");
    }
    let mut note = String::new();
    if !told.is_empty() {
      note.push_str(&format!("Instructions from the MCP servers whose tools you have (named <server>_<tool>):\n\n{}", told.join("\n\n")));
    }
    if !failed.is_empty() || !self.skipped.is_empty() {
      if !note.is_empty() {
        note.push_str("\n\n");
      }
      let skipped = self.skipped.iter().map(|s| format!("- {s}"));
      let lines: Vec<String> = failed.into_iter().chain(skipped).collect();
      note.push_str(&format!("These MCP servers are configured but unavailable, so their tools are missing; tell the user if the task needs them:\n{}", lines.join("\n")));
    }
    Offer { tools, note: (!note.is_empty()).then_some(note) }
  }

  /// Call a tool of a started server
  pub async fn call(&self, server: &str, tool: &str, args: Value, progress: Option<Progress>) -> Result<Value, String> {
    let s = self.servers.iter().find(|s| s.key == server).ok_or_else(|| format!("the MCP server {server} is not configured in this session"))?;
    let live = s.started().ok_or_else(|| format!("the MCP server {server} is not running"))?;
    live.transport.request("tools/call", json!({ "name": tool, "arguments": args }), progress, CALL_TIMEOUT).await
  }

  /// End every server process (and HTTP session); idempotent
  pub fn close(&self) {
    for s in &self.servers {
      if let Some(live) = s.started() {
        live.transport.close();
      }
    }
  }
}

impl Drop for Hub {
  fn drop(&mut self) {
    self.close();
  }
}

/// A `tools/call` result as the model's text, the tool card's ACP content and the images in it. Text blocks are kept
/// as they are; an embedded text resource is its text; a link, audio or a binary resource is named; `structuredContent`
/// stands in when there is no text at all
pub struct CallResult {
  pub text: String,
  pub images: Vec<(String, String)>,
  pub is_error: bool,
}

pub fn read_result(result: &Value) -> CallResult {
  let mut parts: Vec<String> = vec![];
  let mut images = vec![];
  for c in result.get("content").and_then(Value::as_array).into_iter().flatten() {
    let s = |p: &str| c.pointer(p).and_then(Value::as_str);
    match s("/type") {
      Some("text") => parts.push(s("/text").unwrap_or("").to_owned()),
      Some("image") => match (s("/data"), s("/mimeType")) {
        (Some(data), Some(mime)) if !data.is_empty() => images.push((mime.to_owned(), data.to_owned())),
        _ => parts.push("[An image without data]".to_owned()),
      },
      Some("audio") => parts.push(format!("[Audio ({}) omitted]", s("/mimeType").unwrap_or("unknown type"))),
      Some("resource_link") => {
        let uri = s("/uri").unwrap_or("");
        parts.push(format!("[{}]({uri})", s("/name").or(s("/title")).unwrap_or(uri)));
      }
      Some("resource") => match (s("/resource/text"), s("/resource/blob"), s("/resource/mimeType")) {
        (Some(text), _, _) => parts.push(format!("<resource uri=\"{}\">\n{text}\n</resource>", s("/resource/uri").unwrap_or(""))),
        (None, Some(blob), Some(mime)) if mime.starts_with("image/") => images.push((mime.to_owned(), blob.to_owned())),
        _ => parts.push(format!("[Resource {} ({}) omitted]", s("/resource/uri").unwrap_or(""), s("/resource/mimeType").unwrap_or("binary"))),
      },
      _ => {}
    }
  }
  if parts.iter().all(|p| p.trim().is_empty())
    && let Some(structured) = result.get("structuredContent").filter(|v| !v.is_null())
  {
    parts = vec![serde_json::to_string_pretty(structured).unwrap_or_default()];
  }
  CallResult { text: parts.join("\n"), images, is_error: result.get("isError").and_then(Value::as_bool) == Some(true) }
}

/// A folder as a `file://` URI (`roots/list`)
fn file_uri(path: &Path) -> String {
  let s = path.to_string_lossy().replace('\\', "/");
  let s = if s.starts_with('/') { s } else { format!("/{s}") };
  let encoded: String = s
    .bytes()
    .map(|b| if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
    .collect();
  format!("file://{encoded}")
}

/// What a server may ask the client: `ping` and `roots/list` are answered, sampling and elicitation are not offered
fn answer(method: &str, cwd: &Path) -> Result<Value, acpira_rpc::rpc::RpcError> {
  match method {
    "ping" => Ok(json!({})),
    "roots/list" => {
      let name = cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
      Ok(json!({ "roots": [{ "uri": file_uri(cwd), "name": name }] }))
    }
    other => Err(acpira_rpc::rpc::RpcError::method_not_found(other)),
  }
}

/// A progress notification as one line: the server's message, else `progress / total`
fn progress_text(params: &Value) -> String {
  if let Some(m) = params.get("message").and_then(Value::as_str).filter(|m| !m.trim().is_empty()) {
    return m.trim().to_owned();
  }
  let n = |k: &str| params.get(k).and_then(Value::as_f64);
  match (n("progress"), n("total")) {
    (Some(p), Some(t)) => format!("{p} / {t}"),
    (Some(p), None) => format!("{p}"),
    _ => String::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn acp_entries_become_specs() {
    let (specs, skipped) = parse_specs(&json!([
      { "name": "acpira", "command": "/bin/acpira", "args": ["mcp"], "env": [{ "name": "K", "value": "v" }] },
      { "type": "http", "name": "hilfa", "url": "http://localhost:4262/mcp", "headers": [{ "name": "Authorization", "value": "Bearer t" }] },
      { "type": "sse", "name": "old", "url": "https://x/sse", "headers": [] },
      { "name": "broken" },
    ]));
    assert_eq!(specs, vec![
      Spec::Stdio { name: "acpira".into(), command: "/bin/acpira".into(), args: vec!["mcp".into()], env: vec![("K".into(), "v".into())] },
      Spec::Http { name: "hilfa".into(), url: "http://localhost:4262/mcp".into(), headers: vec![("Authorization".into(), "Bearer t".into())] },
    ]);
    assert_eq!(skipped, vec!["old (the sse transport is not supported)", "broken (no command)"]);
  }

  #[test]
  fn tools_are_named_after_their_server_and_never_shadow_another() {
    let listed = vec![
      json!({ "name": "show_image", "description": "Show", "inputSchema": { "$schema": "x", "type": "object", "properties": { "path": { "type": "string" } } }, "annotations": { "readOnlyHint": true } }),
      json!({ "name": "screen.shot", "inputSchema": {} }),
      json!({ "name": "show_image" }),
      json!({ "name": "x".repeat(70) }),
    ];
    let (mut taken, mut notes) = (vec!["read".to_owned()], vec![]);
    let tools = tools_of("acpira", "acpira", &listed, &mut taken, &mut notes);
    assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["acpira_show_image", "acpira_screen_shot"]);
    assert!(tools[0].read_only && !tools[1].read_only);
    assert_eq!(tools[0].schema, json!({ "type": "object", "properties": { "path": { "type": "string" } } }));
    assert_eq!(tools[1].schema, json!({ "type": "object", "properties": {} }));
    assert_eq!(tools[1].tool, "screen.shot");
    assert!(tools[1].description.contains("screen.shot"));
    assert_eq!(notes.len(), 2, "{notes:?}");
  }

  #[test]
  fn results_keep_text_and_images_apart() {
    let r = read_result(&json!({ "content": [
      { "type": "text", "text": "done" },
      { "type": "image", "data": "AAA", "mimeType": "image/png" },
      { "type": "resource", "resource": { "uri": "file:///a.txt", "text": "body" } },
      { "type": "resource_link", "uri": "file:///b.rs", "name": "b.rs" },
    ], "isError": true }));
    assert_eq!(r.images, vec![("image/png".to_owned(), "AAA".to_owned())]);
    assert!(r.is_error && r.text.contains("done") && r.text.contains("<resource uri=\"file:///a.txt\">\nbody") && r.text.contains("[b.rs](file:///b.rs)"));
    let structured = read_result(&json!({ "content": [], "structuredContent": { "ok": true } }));
    assert!(structured.text.contains("\"ok\": true") && !structured.is_error);
  }

  #[test]
  fn roots_are_file_uris() {
    assert_eq!(file_uri(Path::new("/w/my repo")), "file:///w/my%20repo");
    assert_eq!(file_uri(Path::new("C:\\w\\x")), "file:///C:/w/x");
  }
}
