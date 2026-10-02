//! Shared MCP servers: `~/.agents/mcp.json` and the project's `.mcp.json` (plus a legacy `<project>/.agents/mcp.json`)
//! in the common `{ "mcpServers": { … } }` shape, sent to every session over ACP `mcpServers` (new, load, resume and an
//! edit's fresh session all build their request in `AcpSession::session_request`). Agents that load `.mcp.json`
//! themselves are not sent its servers a second time; no other CLI config is written

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value, json};

use acpira_shared::inventory::{McpCaps, McpTransport};
use acpira_shared::shared_config::SharedScope;

use super::Places;
use crate::acp::agents::launch::ProcessEnv;
use crate::platform::command::Os;
use crate::acp::agents::registry::resolve_command;
use crate::agent_ext::{McpFormat, agent_ext};
use crate::inventory::{parse_json_loose, parse_json_mcp, parse_opencode_mcp, parse_toml_mcp};

/// One server as the shared file declares it
#[derive(Debug, Clone, PartialEq)]
pub struct SharedServer {
  pub name: String,
  pub spec: Map<String, Value>,
  pub transport: McpTransport,
  pub enabled: bool,
}

impl SharedServer {
  fn from_spec(name: &str, spec: &Map<String, Value>) -> Self {
    let str_of = |k: &str| spec.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let url = str_of("url").or_else(|| str_of("serverUrl")).or_else(|| str_of("httpUrl"));
    let transport = match str_of("type").or_else(|| str_of("transport")).map(str::to_lowercase).as_deref() {
      Some("sse") => McpTransport::Sse,
      Some("http" | "streamable-http" | "streamable_http") => McpTransport::Http,
      Some("stdio") => McpTransport::Stdio,
      _ if str_of("command").is_none() && url.is_some() => McpTransport::Http,
      _ => McpTransport::Stdio,
    };
    let enabled = spec.get("enabled") != Some(&Value::Bool(false)) && spec.get("disabled") != Some(&Value::Bool(true));
    SharedServer { name: name.to_owned(), spec: spec.clone(), transport, enabled }
  }

  /// Command line or URL, for display (unexpanded, so no secret from the environment shows)
  pub fn target(&self) -> String {
    let str_of = |k: &str| self.spec.get(k).and_then(Value::as_str).unwrap_or("");
    match self.transport {
      McpTransport::Stdio => std::iter::once(str_of("command").to_owned()).chain(string_list(self.spec.get("args"))).collect::<Vec<_>>().join(" "),
      _ => ["url", "serverUrl", "httpUrl"].iter().map(|k| str_of(k)).find(|s| !s.is_empty()).unwrap_or("").to_owned(),
    }
  }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
  v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

/// The servers of one shared file, in file order; a missing or broken file has none
pub fn read_servers(path: &Path) -> Vec<SharedServer> {
  let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
  let Some(Value::Object(data)) = parse_json_loose(&text) else { return vec![] };
  let Some(servers) = data.get("mcpServers").and_then(Value::as_object) else { return vec![] };
  servers.iter().filter_map(|(name, v)| v.as_object().map(|spec| SharedServer::from_spec(name, spec))).collect()
}

/// Both scopes: (scope, server, shadowed by a project server of the same name). A disabled project entry shadows too,
/// which is how a global server is turned off for one project
pub fn all_servers(places: &Places) -> Vec<(SharedScope, SharedServer, bool)> {
  let mut project = places.mcp_file(SharedScope::Project).map(|p| read_servers(&p)).unwrap_or_default();
  let legacy = places.legacy_mcp_file().map(|p| read_servers(&p)).unwrap_or_default();
  let current: HashSet<String> = project.iter().map(|s| s.name.clone()).collect();
  project.extend(legacy.into_iter().filter(|s| !current.contains(&s.name)));
  let global = places.mcp_file(SharedScope::Global).map(|p| read_servers(&p)).unwrap_or_default();
  let local: HashSet<String> = project.iter().map(|s| s.name.clone()).collect();
  let mut out: Vec<_> = project.into_iter().map(|s| (SharedScope::Project, s, false)).collect();
  out.extend(global.into_iter().map(|s| {
    let shadowed = local.contains(&s.name);
    (SharedScope::Global, s, shadowed)
  }));
  out
}

/// Agents seen launching a project `.mcp.json` server at session/new over ACP (2026-10-01: claude-agent-acp 0.83.0,
/// Devin 3000.11.3). Grok 1.0.18 skips the file once its Claude import prompt was answered, and Kimi 0.41.0 did not
/// launch it, so both get those servers over ACP like everyone else
const LOADS_MCP_JSON: &[&str] = &["claude", "devin"];

/// Names the agent's own config files declare (those servers load anyway, so they are not sent a second time)
pub fn native_names(agent: &str, places: &Places) -> HashSet<String> {
  let Some(ext) = agent_ext(agent) else { return HashSet::new() };
  let mut out = HashSet::new();
  for (tpl, format) in ext.mcp {
    if *tpl == ".mcp.json" && !LOADS_MCP_JSON.contains(&agent) {
      continue;
    }
    let Some(path) = places.expand(tpl) else { continue };
    let Ok(text) = std::fs::read_to_string(&path) else { continue };
    let entries = match format {
      McpFormat::Json => parse_json_mcp(&text),
      McpFormat::Toml => parse_toml_mcp(&text),
      McpFormat::Opencode => parse_opencode_mcp(&text),
    };
    out.extend(entries.into_iter().map(|e| e.name));
  }
  out
}

/// Whether the agent takes client MCP servers at all: a built-in as verified, a custom ACP agent by the spec (stdio is mandatory)
pub fn takes_mcp(agent: &str) -> bool {
  agent_ext(agent).is_none_or(|e| e.shared.mcp)
}

/// Whether the agent can take this transport: http / sse only when advertised
pub fn supports(agent: &str, transport: McpTransport, caps: Option<McpCaps>) -> bool {
  if !takes_mcp(agent) {
    return false;
  }
  match transport {
    McpTransport::Stdio => true,
    McpTransport::Http => caps.is_some_and(|c| c.http),
    McpTransport::Sse => caps.is_some_and(|c| c.sse),
  }
}

static VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\{(?:env:)?([A-Za-z_][A-Za-z0-9_]*)\}").unwrap());

/// `${VAR}` / `${env:VAR}` from the sidecar's environment; an unset variable becomes empty
pub fn expand_vars(s: &str) -> String {
  VAR.replace_all(s, |c: &regex::Captures| std::env::var(&c[1]).unwrap_or_default()).into_owned()
}

/// `{ "K": "v" }` or `[{ "name": "K", "value": "v" }]` into ACP's name / value list, values expanded
fn pairs(v: Option<&Value>) -> Vec<Value> {
  match v {
    Some(Value::Object(m)) => m.iter().map(|(k, v)| json!({ "name": k, "value": expand_vars(v.as_str().unwrap_or(&v.to_string())) })).collect(),
    Some(Value::Array(a)) => a
      .iter()
      .filter_map(|p| {
        let name = p.get("name")?.as_str()?;
        Some(json!({ "name": name, "value": expand_vars(p.get("value").and_then(Value::as_str).unwrap_or("")) }))
      })
      .collect(),
    _ => vec![],
  }
}

/// One server as an ACP `McpServer`; Err names why it cannot be sent
pub async fn to_acp(s: &SharedServer, root: Option<&Path>) -> Result<Value> {
  let str_of = |k: &str| s.spec.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(expand_vars);
  match s.transport {
    McpTransport::Stdio => {
      let command = str_of("command").ok_or_else(|| anyhow!("no command"))?;
      // ACP wants an absolute path: bare names go through PATH like an agent CLI, relative paths from the project root
      let resolved = if Path::new(&command).is_absolute() {
        Some(command.clone())
      } else if command.contains('/') || command.contains('\\') {
        root.map(|r| r.join(&command).to_string_lossy().into_owned())
      } else {
        resolve_command(&command, &[], Os::current(), &ProcessEnv).await
      };
      let command = resolved.ok_or_else(|| anyhow!("`{command}` not found on PATH"))?;
      let args: Vec<String> = string_list(s.spec.get("args")).iter().map(|a| expand_vars(a)).collect();
      Ok(json!({ "name": s.name, "command": command, "args": args, "env": pairs(s.spec.get("env")) }))
    }
    McpTransport::Http | McpTransport::Sse => {
      let url = str_of("url").or_else(|| str_of("serverUrl")).or_else(|| str_of("httpUrl")).ok_or_else(|| anyhow!("no url"))?;
      let kind = if s.transport == McpTransport::Sse { "sse" } else { "http" };
      Ok(json!({ "type": kind, "name": s.name, "url": url, "headers": pairs(s.spec.get("headers")) }))
    }
  }
}

/// What one session gets, and a log line naming what was left out (names only, never values)
pub async fn session_servers(places: &Places, agent: &str, caps: Option<McpCaps>) -> (Vec<Value>, Option<String>) {
  if !takes_mcp(agent) {
    return (vec![], None);
  }
  // Blocking reads: an agent's own config (~/.claude.json) can run to megabytes
  let (p, a) = (places.clone(), agent.to_owned());
  let Ok((servers, native)) = tokio::task::spawn_blocking(move || {
    let servers = all_servers(&p);
    let native = if servers.is_empty() { HashSet::new() } else { native_names(&a, &p) };
    (servers, native)
  })
  .await
  else {
    return (vec![], None);
  };
  if servers.is_empty() {
    return (vec![], None);
  }
  let (mut out, mut sent, mut skipped) = (vec![], vec![], vec![]);
  for (_, s, shadowed) in servers {
    if shadowed || !s.enabled {
      continue;
    }
    if native.contains(&s.name) {
      skipped.push(format!("{} (in the agent's own config)", s.name));
    } else if !supports(agent, s.transport, caps) {
      skipped.push(format!("{} (transport not advertised)", s.name));
    } else {
      match to_acp(&s, places.root.as_deref()).await {
        Ok(v) => {
          sent.push(s.name.clone());
          out.push(v);
        }
        Err(e) => skipped.push(format!("{} ({e})", s.name)),
      }
    }
  }
  let mut line = format!("shared MCP: {}", if sent.is_empty() { "none sent".into() } else { sent.join(", ") });
  if !skipped.is_empty() {
    line.push_str(&format!("; skipped {}", skipped.join(", ")));
  }
  (out, Some(line))
}

/// The project file holding `name`: `.mcp.json`, or the legacy `.agents/mcp.json` for a server only it declares
pub fn project_file_of(places: &Places, name: &str) -> Option<std::path::PathBuf> {
  let current = places.mcp_file(SharedScope::Project)?;
  if !read_servers(&current).iter().any(|s| s.name == name)
    && let Some(legacy) = places.legacy_mcp_file().filter(|l| read_servers(l).iter().any(|s| s.name == name))
  {
    return Some(legacy);
  }
  Some(current)
}

/// Read a shared file for editing: strict JSON only, so a hand-written file with comments is never rewritten
fn load_for_edit(path: &Path) -> Result<Map<String, Value>> {
  match std::fs::read_to_string(path) {
    Err(_) => Ok(Map::new()),
    Ok(text) if text.trim().is_empty() => Ok(Map::new()),
    Ok(text) => match serde_json::from_str::<Value>(&text) {
      Ok(Value::Object(m)) => Ok(m),
      _ => bail!("{} is not plain JSON (comments?); edit it by hand", path.display()),
    },
  }
}

fn save(path: &Path, data: &Map<String, Value>) -> Result<()> {
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
  std::fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&Value::Object(data.clone()))?))?;
  std::fs::rename(&tmp, path)?;
  Ok(())
}

fn servers_mut(data: &mut Map<String, Value>) -> Result<&mut Map<String, Value>> {
  data.entry("mcpServers").or_insert_with(|| Value::Object(Map::new())).as_object_mut().ok_or_else(|| anyhow!("mcpServers is not an object"))
}

fn is_server(m: &Map<String, Value>) -> bool {
  ["command", "url", "serverUrl", "httpUrl"].iter().any(|k| m.contains_key(*k))
}

/// Pasted JSON: `{ "mcpServers": {…} }`, a map of servers, or one server (then `name` names it). Existing names are refused
pub fn add(path: &Path, pasted: &str, name: Option<&str>) -> Result<Vec<String>> {
  let parsed = parse_json_loose(pasted).ok_or_else(|| anyhow!("not JSON"))?;
  let Value::Object(obj) = parsed else { bail!("expected a JSON object") };
  let incoming: Map<String, Value> = if let Some(Value::Object(m)) = obj.get("mcpServers") {
    m.clone()
  } else if is_server(&obj) {
    let name = name.map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| anyhow!("a single server needs a name"))?;
    Map::from_iter([(name.to_owned(), Value::Object(obj))])
  } else {
    obj
  };
  if incoming.is_empty() || !incoming.values().all(|v| v.as_object().is_some_and(is_server)) {
    bail!("no server with a command or url");
  }
  let mut data = load_for_edit(path)?;
  let servers = servers_mut(&mut data)?;
  if let Some(dup) = incoming.keys().find(|k| servers.contains_key(*k)) {
    bail!("`{dup}` already exists");
  }
  let names: Vec<String> = incoming.keys().cloned().collect();
  servers.extend(incoming);
  save(path, &data)?;
  Ok(names)
}

pub fn toggle(path: &Path, name: &str, enabled: bool) -> Result<()> {
  let mut data = load_for_edit(path)?;
  let server = servers_mut(&mut data)?.get_mut(name).and_then(Value::as_object_mut).ok_or_else(|| anyhow!("`{name}` not found"))?;
  server.remove("enabled");
  if enabled {
    server.remove("disabled");
  } else {
    server.insert("disabled".into(), Value::Bool(true));
  }
  save(path, &data)
}

pub fn remove(path: &Path, name: &str) -> Result<()> {
  let mut data = load_for_edit(path)?;
  if servers_mut(&mut data)?.remove(name).is_none() {
    bail!("`{name}` not found");
  }
  save(path, &data)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn places(t: &Path) -> Places {
    let root = t.join("repo");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    Places { home: t.join("home"), config: t.join("home/.config"), root: Some(root) }
  }

  #[tokio::test]
  async fn merges_filters_and_converts() {
    let t = tempfile::tempdir().unwrap();
    let p = places(t.path());
    std::fs::create_dir_all(p.home.join(".agents")).unwrap();
    std::fs::write(
      p.home.join(".agents/mcp.json"),
      r#"{ "mcpServers": {
        "docs": { "command": "/bin/echo", "args": ["${ACPIRA_TEST_MCP_ARG}"], "env": { "K": "${env:ACPIRA_TEST_MCP_ARG}" } },
        "web": { "type": "http", "url": "https://x/mcp", "headers": { "Authorization": "Bearer t" } },
        "old": { "url": "https://y/sse", "type": "sse" },
        "off": { "command": "/bin/true", "disabled": true },
        "dup": { "command": "/bin/true" }
      } }"#,
    )
    .unwrap();
    let project_command = std::env::current_exe().unwrap();
    add(&p.mcp_file(SharedScope::Project).unwrap(), &json!({ "command": project_command }).to_string(), Some("dup")).unwrap();
    unsafe { std::env::set_var("ACPIRA_TEST_MCP_ARG", "v1") };
    let caps = Some(McpCaps { http: true, sse: false });
    let (sent, log) = session_servers(&p, "codex", caps).await;
    let names: Vec<&str> = sent.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["dup", "docs", "web"]);
    assert_eq!(sent[0]["command"], json!(project_command));
    assert_eq!(sent[1]["args"], json!(["v1"]));
    assert_eq!(sent[1]["env"], json!([{ "name": "K", "value": "v1" }]));
    assert_eq!(
      sent[2],
      json!({ "type": "http", "name": "web", "url": "https://x/mcp", "headers": [{ "name": "Authorization", "value": "Bearer t" }] })
    );
    assert!(log.unwrap().contains("old (transport not advertised)"));
    // Pi takes no client MCP; a name the agent's own config declares is skipped
    assert!(session_servers(&p, "pi", caps).await.0.is_empty());
    std::fs::create_dir_all(p.home.join(".codex")).unwrap();
    std::fs::write(p.home.join(".codex/config.toml"), "[mcp_servers.docs]\ncommand = \"x\"\n").unwrap();
    let (sent, _) = session_servers(&p, "codex", caps).await;
    assert!(!sent.iter().any(|v| v["name"] == "docs"));

    // The project's servers live in `.mcp.json`: Claude loads that file itself, Grok gets them over ACP
    let root = p.root.clone().unwrap();
    assert!(root.join(".mcp.json").exists());
    let names = |v: &[Value]| v.iter().map(|x| x["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert!(!names(&session_servers(&p, "claude", caps).await.0).contains(&"dup".to_owned()));
    assert!(names(&session_servers(&p, "grok", caps).await.0).contains(&"dup".to_owned()));
    // The legacy project file is still read, and edits go to the file holding the server
    std::fs::create_dir_all(root.join(".agents")).unwrap();
    std::fs::write(
      root.join(".agents/mcp.json"),
      r#"{ "mcpServers": { "legacy": { "command": "/bin/true" }, "dup": { "command": "/bin/false" } } }"#,
    )
    .unwrap();
    let project: Vec<_> = all_servers(&p).into_iter().filter(|(s, _, _)| *s == SharedScope::Project).map(|(_, s, _)| s).collect();
    assert_eq!(project.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["dup", "legacy"]);
    assert_eq!(project[0].spec["command"], json!(project_command));
    assert_eq!(project_file_of(&p, "legacy").unwrap(), root.join(".agents/mcp.json"));
    assert_eq!(project_file_of(&p, "dup").unwrap(), root.join(".mcp.json"));
  }

  #[test]
  fn edits_keep_other_keys_and_refuse_comments() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("mcp.json");
    assert_eq!(add(&f, r#"{ "mcpServers": { "a": { "command": "x" }, "b": { "url": "u" } } }"#, None).unwrap(), ["a", "b"]);
    assert!(add(&f, r#"{ "a": { "command": "y" } }"#, None).is_err());
    toggle(&f, "a", false).unwrap();
    assert!(!read_servers(&f)[0].enabled);
    toggle(&f, "a", true).unwrap();
    assert!(read_servers(&f)[0].enabled);
    remove(&f, "b").unwrap();
    assert_eq!(read_servers(&f).len(), 1);
    std::fs::write(&f, "{ // mine\n \"mcpServers\": {} }").unwrap();
    assert!(toggle(&f, "a", true).is_err());
  }
}
