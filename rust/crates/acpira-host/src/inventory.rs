//! Read-only scan of an agent's extension points (mirror of src/host/inventory.ts)

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use acpira_shared::inventory::*;

use crate::agent_ext::{AgentExt, McpFormat};
use crate::util::now_iso;

pub struct ScanEnv {
  pub home: String,
  pub cwd: String,
}

pub struct ScanInput {
  pub agent: String,
  pub ext: Option<&'static AgentExt>,
  pub binary: Option<String>,
  pub runtime: Option<AgentRuntimeInfo>,
  pub adapter: Option<AdapterInfo>,
  pub health: Option<AgentHealth>,
}

pub async fn scan_inventory(input: ScanInput, env: &ScanEnv) -> AgentInventory {
  let mut out = AgentInventory {
    agent: input.agent,
    binary: input.binary,
    runtime: input.runtime,
    adapter: input.adapter,
    health: input.health,
    steer: input.ext.is_some_and(|e| e.steer),
    config: vec![],
    mcp: vec![],
    skills: vec![],
    rules: vec![],
    scanned_at: now_iso(),
  };
  let Some(ext) = input.ext else { return out };
  for p in ext.config {
    out.config.push(file_info(p, env).await);
  }
  for (p, format) in ext.mcp {
    out.mcp.extend(read_mcp(p, *format, env).await);
  }
  let mut seen = HashSet::new();
  for d in ext.skills {
    for s in read_skills(d, env).await {
      if seen.insert(s.path.clone()) {
        out.skills.push(s);
      }
    }
  }
  let mut seen = HashSet::new();
  for (p, dir) in ext.rules {
    for r in read_rules(p, *dir, env).await {
      if seen.insert(r.path.clone()) {
        out.rules.push(r);
      }
    }
  }
  out
}

fn config_home(env: &ScanEnv) -> PathBuf {
  if cfg!(windows)
    && let Ok(a) = std::env::var("APPDATA")
    && !a.is_empty()
  {
    return PathBuf::from(a);
  }
  match std::env::var("XDG_CONFIG_HOME") {
    Ok(x) if !x.is_empty() => PathBuf::from(x),
    _ => Path::new(&env.home).join(".config"),
  }
}

/// `~/x` → home; `$CONFIG/x` → XDG config home; relative → workspace; absolute stays
pub fn expand_path(template: &str, env: &ScanEnv) -> String {
  let p = if let Some(rest) = template.strip_prefix("~/") {
    Path::new(&env.home).join(rest)
  } else if let Some(rest) = template.strip_prefix("$CONFIG/") {
    config_home(env).join(rest)
  } else if Path::new(template).is_absolute() {
    PathBuf::from(template)
  } else {
    Path::new(&env.cwd).join(template)
  };
  p.to_string_lossy().into_owned()
}

pub fn scope_of(template: &str) -> InventoryScope {
  if template.starts_with("~/") || template.starts_with("$CONFIG/") || Path::new(template).is_absolute() {
    InventoryScope::User
  } else {
    InventoryScope::Project
  }
}

async fn file_info(template: &str, env: &ScanEnv) -> InventoryFile {
  let path = expand_path(template, env);
  match tokio::fs::metadata(&path).await {
    Ok(m) => InventoryFile { path, scope: scope_of(template), exists: m.is_file(), size: m.is_file().then_some(m.len()) },
    Err(_) => InventoryFile { path, scope: scope_of(template), exists: false, size: None },
  }
}

struct McpEntry {
  name: String,
  transport: McpTransport,
  target: String,
  enabled: bool,
}

async fn read_mcp(template: &str, format: McpFormat, env: &ScanEnv) -> Vec<InventoryMcp> {
  let path = expand_path(template, env);
  let Ok(text) = tokio::fs::read_to_string(&path).await else { return vec![] };
  let scope = scope_of(template);
  let entries = match format {
    McpFormat::Toml => parse_toml_mcp(&text),
    McpFormat::Opencode => parse_opencode_mcp(&text),
    McpFormat::Json => parse_json_mcp(&text),
  };
  entries
    .into_iter()
    .map(|e| InventoryMcp { name: e.name, transport: e.transport, target: e.target, source: path.clone(), scope, enabled: e.enabled })
    .collect()
}

fn s(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned)
}

fn transport_of(kind: Option<&str>, command: Option<&str>, url: Option<&str>) -> McpTransport {
  match kind.map(str::to_lowercase).as_deref() {
    Some("sse") => McpTransport::Sse,
    Some("stdio") => McpTransport::Stdio,
    Some("http" | "streamable-http" | "streamable_http") => McpTransport::Http,
    _ => {
      if command.is_some() {
        McpTransport::Stdio
      } else if url.is_some() {
        McpTransport::Http
      } else {
        McpTransport::Stdio
      }
    }
  }
}

fn parse_json_mcp(text: &str) -> Vec<McpEntry> {
  let Some(Value::Object(data)) = parse_json_loose(text) else { return vec![] };
  let Some(servers) = data.get("mcpServers").and_then(Value::as_object) else { return vec![] };
  servers
    .iter()
    .filter_map(|(name, v)| {
      let v = v.as_object()?;
      let command = s(v.get("command"));
      let url = s(v.get("url")).or_else(|| s(v.get("serverUrl"))).or_else(|| s(v.get("httpUrl")));
      let args: Vec<String> = v
        .get("args")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
      let kind = s(v.get("type")).or_else(|| s(v.get("transport")));
      let target = match &command {
        Some(c) => std::iter::once(c.clone()).chain(args).collect::<Vec<_>>().join(" "),
        None => url.clone().unwrap_or_default(),
      };
      Some(McpEntry {
        name: name.clone(),
        transport: transport_of(kind.as_deref(), command.as_deref(), url.as_deref()),
        target,
        enabled: v.get("enabled") != Some(&Value::Bool(false)) && v.get("disabled") != Some(&Value::Bool(true)),
      })
    })
    .collect()
}

static TOML_HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\[\s*mcp_servers\.(?:"([^"]+)"|([^\]\s.]+))\s*\]$"#).unwrap());
static TOML_KV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w-]*)\s*=\s*(.+?)\s*(?:#.*)?$").unwrap());
static TOML_ARG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""((?:[^"\\]|\\.)*)"|'([^']*)'"#).unwrap());
static TOML_STR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^"((?:[^"\\]|\\.)*)"|^'([^']*)'"#).unwrap());

fn toml_string(v: &str) -> String {
  match TOML_STR.captures(v.trim()) {
    Some(m) => m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or("").replace("\\\"", "\""),
    None => v.trim().to_owned(),
  }
}

/// [mcp_servers.name] tables: enough of TOML for these tables, nothing more
fn parse_toml_mcp(text: &str) -> Vec<McpEntry> {
  struct Cur {
    name: String,
    command: Option<String>,
    args: Vec<String>,
    url: Option<String>,
    kind: Option<String>,
    enabled: bool,
  }
  let mut out = vec![];
  let mut cur: Option<Cur> = None;
  let flush = |cur: &mut Option<Cur>, out: &mut Vec<McpEntry>| {
    if let Some(c) = cur.take() {
      let target = match &c.command {
        Some(cmd) => std::iter::once(cmd.clone()).chain(c.args.clone()).collect::<Vec<_>>().join(" "),
        None => c.url.clone().unwrap_or_default(),
      };
      out.push(McpEntry {
        transport: transport_of(c.kind.as_deref(), c.command.as_deref(), c.url.as_deref()),
        name: c.name,
        target,
        enabled: c.enabled,
      });
    }
  };
  for raw in text.split('\n') {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    if line.starts_with('[') {
      flush(&mut cur, &mut out);
      if let Some(m) = TOML_HEADER.captures(line) {
        cur = Some(Cur {
          name: m.get(1).or(m.get(2)).unwrap().as_str().to_owned(),
          command: None,
          args: vec![],
          url: None,
          kind: None,
          enabled: true,
        });
      }
      continue;
    }
    let Some(c) = cur.as_mut() else { continue };
    let Some(kv) = TOML_KV.captures(line) else { continue };
    let value = &kv[2];
    match &kv[1] {
      "command" => c.command = Some(toml_string(value)).filter(|x| !x.is_empty()),
      "url" => c.url = Some(toml_string(value)).filter(|x| !x.is_empty()),
      "type" | "transport" => c.kind = Some(toml_string(value)),
      "enabled" => c.enabled = value.trim() != "false",
      "args" => {
        c.args =
          TOML_ARG.captures_iter(value).map(|m| m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or("").replace("\\\"", "\"")).collect()
      }
      _ => {}
    }
  }
  flush(&mut cur, &mut out);
  out
}

fn parse_opencode_mcp(text: &str) -> Vec<McpEntry> {
  let Some(Value::Object(data)) = parse_json_loose(text) else { return vec![] };
  let Some(servers) = data.get("mcp").and_then(Value::as_object) else { return vec![] };
  servers
    .iter()
    .filter_map(|(name, v)| {
      let v = v.as_object()?;
      let command = match v.get("command") {
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")),
        other => s(other),
      };
      let url = s(v.get("url"));
      let transport = match v.get("type").and_then(Value::as_str) {
        Some("local") => McpTransport::Stdio,
        Some("remote") => McpTransport::Http,
        _ => transport_of(None, command.as_deref(), url.as_deref()),
      };
      Some(McpEntry {
        name: name.clone(),
        transport,
        target: command.or(url).unwrap_or_default(),
        enabled: v.get("enabled") != Some(&Value::Bool(false)),
      })
    })
    .collect()
}

static TRAILING_COMMA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",(\s*[}\]])").unwrap());

/// JSON with `//` and `/* */` comments and trailing commas; strict JSON first
pub fn parse_json_loose(text: &str) -> Option<Value> {
  if let Ok(v) = serde_json::from_str(text) {
    return Some(v);
  }
  let b: Vec<char> = text.chars().collect();
  let mut out = String::with_capacity(text.len());
  let mut i = 0;
  while i < b.len() {
    let c = b[i];
    if c == '"' {
      let mut j = i + 1;
      while j < b.len() && b[j] != '"' {
        if b[j] == '\\' {
          j += 1;
        }
        j += 1;
      }
      out.extend(&b[i..(j + 1).min(b.len())]);
      i = j + 1;
    } else if c == '/' && b.get(i + 1) == Some(&'/') {
      while i < b.len() && b[i] != '\n' {
        i += 1;
      }
    } else if c == '/' && b.get(i + 1) == Some(&'*') {
      let rest: String = b[i + 2..].iter().collect();
      i = match rest.find("*/") {
        Some(end) => i + 2 + rest[..end].chars().count() + 2,
        None => b.len(),
      };
    } else {
      out.push(c);
      i += 1;
    }
  }
  serde_json::from_str(&TRAILING_COMMA.replace_all(&out, "$1")).ok()
}

async fn read_skills(template: &str, env: &ScanEnv) -> Vec<InventorySkill> {
  let dir = expand_path(template, env);
  let Ok(mut rd) = tokio::fs::read_dir(&dir).await else { return vec![] };
  let scope = scope_of(template);
  let mut found = vec![];
  while let Ok(Some(d)) = rd.next_entry().await {
    let name = d.file_name().to_string_lossy().into_owned();
    let Ok(ft) = d.file_type().await else { continue };
    if ft.is_dir() || ft.is_symlink() {
      let path = Path::new(&dir).join(&name).join("SKILL.md").to_string_lossy().into_owned();
      if let Ok(text) = tokio::fs::read_to_string(&path).await {
        let fm = parse_frontmatter(&text);
        found.push(InventorySkill { name: fm.0.unwrap_or(name), description: fm.1, path, scope });
      }
    } else if ft.is_file() && name.to_lowercase().ends_with(".md") {
      let path = Path::new(&dir).join(&name).to_string_lossy().into_owned();
      if let Ok(text) = tokio::fs::read_to_string(&path).await {
        let fm = parse_frontmatter(&text);
        if let Some(n) = fm.0 {
          found.push(InventorySkill { name: n, description: fm.1, path, scope });
        }
      }
    }
  }
  found.sort_by(|a, b| locale_compare(&a.name, &b.name));
  found
}

/// An approximation of String.prototype.localeCompare for plain names: case-insensitive first, then exact
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
  a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

static FRONT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^---\r?\n((?s:.*?))\r?\n---").unwrap());
static FM_KV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w-]*):\s*(.*)$").unwrap());
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^"((?:[^"\\]|\\.)*)"$|^'([^']*)'$"#).unwrap());

/// (name, description) from `key: value` lines between the first two `---` fences
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
  let Some(m) = FRONT.captures(text) else { return (None, None) };
  let lines: Vec<&str> = m[1].split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
  let mut map = Map::new();
  let mut i = 0;
  while i < lines.len() {
    if let Some(kv) = FM_KV.captures(lines[i]) {
      let mut value = kv[2].trim().to_owned();
      if matches!(value.as_str(), ">" | "|" | ">-" | "|-") {
        let folded = value.starts_with('>');
        let mut block = vec![];
        while i + 1 < lines.len() && lines[i + 1].starts_with(char::is_whitespace) && !lines[i + 1].trim().is_empty() {
          i += 1;
          block.push(lines[i].trim());
        }
        value = block.join(if folded { " " } else { "\n" });
      }
      let unq = match QUOTED.captures(&value) {
        Some(q) => q.get(1).or(q.get(2)).map(|x| x.as_str()).unwrap_or("").replace("\\\"", "\""),
        None => value,
      };
      map.insert(kv[1].to_owned(), Value::from(unq));
    }
    i += 1;
  }
  (map.get("name").and_then(Value::as_str).map(str::to_owned), map.get("description").and_then(Value::as_str).map(str::to_owned))
}

async fn read_rules(template: &str, dir: bool, env: &ScanEnv) -> Vec<InventoryFile> {
  if !dir {
    return vec![file_info(template, env).await];
  }
  let path = expand_path(template, env);
  let scope = scope_of(template);
  let Ok(mut rd) = tokio::fs::read_dir(&path).await else { return vec![] };
  let mut names = vec![];
  while let Ok(Some(d)) = rd.next_entry().await {
    let n = d.file_name().to_string_lossy().into_owned();
    let lower = n.to_lowercase();
    if d.file_type().await.is_ok_and(|t| t.is_file()) && (lower.ends_with(".md") || lower.ends_with(".mdc")) {
      names.push(n);
    }
  }
  names.sort();
  let mut out = vec![];
  for n in names {
    let p = Path::new(&path).join(&n);
    let size = tokio::fs::metadata(&p).await.map(|m| m.len()).ok();
    out.push(InventoryFile { path: p.to_string_lossy().into_owned(), scope, exists: true, size });
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_mcp_and_frontmatter() {
    let t = parse_toml_mcp(
      "[mcp_servers.fs]\ncommand = \"npx\"\nargs = [\"-y\", 'srv']\n[mcp_servers.\"web x\"]\nurl = \"https://x\"\nenabled = false\n",
    );
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].target, "npx -y srv");
    assert_eq!((t[1].name.as_str(), t[1].transport, t[1].enabled), ("web x", McpTransport::Http, false));
    let j = parse_json_mcp("{ // c\n \"mcpServers\": { \"a\": { \"url\": \"u\", \"type\": \"sse\" }, } }");
    assert_eq!(j[0].transport, McpTransport::Sse);
    assert_eq!(parse_frontmatter("---\nname: x\ndescription: >\n  a\n  b\n---\n"), (Some("x".into()), Some("a b".into())));
  }
}
