//! `plugin.json`, schema 1 (`docs/dev/plugins.md`): what a plugin contributes, and the static check that reads the
//! manifest and the frozen files alone, without running any of the plugin's code

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const FILE: &str = "plugin.json";
pub const SCHEMA: u32 = 1;
/// Longest id and tool name; `<id>_<tool>` behind a client's `mcp__acpira__` stays inside the 64 characters
/// providers allow for a tool name
pub const NAME_MAX: usize = 24;
/// Default and longest run of a script tool: inside the 50 s every client but Claude gets an answer in (`host_mcp.rs`)
pub const TOOL_TIMEOUT_SECS: f64 = 45.0;

const KEYS: &[&str] = &["schema", "id", "version", "name", "description", "requires", "layer", "check", "contributes"];
const CONTRIBUTION_KEYS: &[&str] = &["tools", "mcp", "hooks", "theme", "prompts", "agent", "runtime"];

/// The hooks a plugin's resident process may declare (`contributes.runtime.hooks`, docs/dev/plugins.md, Runtime). The
/// engine calls only those it implements; a name it does not know yet is a note, so a plugin written for a newer
/// Acpira still loads
pub const RUNTIME_HOOKS: &[&str] = &[
  "prompt",
  "permission",
  "tool.before",
  "tool.after",
  "retry",
  "title",
  "session.mcp",
  "agent.launch",
  "context",
  "model.request",
  "compaction",
  "shell",
  "tools",
];
/// The event families a resident process may subscribe to; `<family>.*` takes every event of one, `*` all of them
/// except the streamed deltas, which are asked for by name (`text.delta`, `thought.delta`)
pub const RUNTIME_EVENTS: &[&str] = &["session", "turn", "tool", "permission", "prompt", "files", "model", "plugin", "usage", "text.delta", "thought.delta"];
/// Longest one hook call may take before it counts as failed
pub const RUNTIME_TIMEOUT_SECS: f64 = 10.0;
pub const LAYERS: &[&str] = &["agents", "tools", "interface", "automation"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
  pub schema: u32,
  pub id: String,
  #[serde(default)]
  pub version: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default)]
  pub description: String,
  /// Programs that must be on PATH
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub requires: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub layer: Option<String>,
  /// A self-test run in the frozen copy once the user confirmed
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub check: Option<Vec<String>>,
  #[serde(default)]
  pub contributes: Contributes,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributes {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tools: Vec<ToolDef>,
  /// `.mcp.json`'s `mcpServers` shape
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mcp: Option<Map<String, Value>>,
  /// `.agents/hooks.json`'s shape
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub hooks: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub theme: Option<Theme>,
  /// A folder of prompt sections, relative to the plugin
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prompts: Option<String>,
  /// One ACP agent, `agents.json`'s entry shape plus `name`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<Value>,
  /// A resident process: the hooks it answers and the events it hears
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub runtime: Option<RuntimeDef>,
}

/// `contributes.runtime`: a process the engine keeps running while the plugin is in effect, spoken to over ndjson
/// JSON-RPC on its stdio (docs/dev/plugins.md, Runtime)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDef {
  pub command: Vec<String>,
  /// Hook names from `RUNTIME_HOOKS`; only these are ever called
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub hooks: Vec<String>,
  /// Event names or `<family>.*` / `*`; only these are ever sent
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub events: Vec<String>,
  /// Seconds one hook call may take, at most `RUNTIME_TIMEOUT_SECS`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<f64>,
}

impl RuntimeDef {
  pub fn timeout(&self) -> std::time::Duration {
    let secs = self.timeout.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(RUNTIME_TIMEOUT_SECS).min(RUNTIME_TIMEOUT_SECS);
    std::time::Duration::from_secs_f64(secs)
  }

  /// Whether the process subscribed to `event` (`session.created`, `text.delta`)
  pub fn hears(&self, event: &str) -> bool {
    let family = event.split('.').next().unwrap_or(event);
    let delta = event.ends_with(".delta");
    self.events.iter().any(|e| e == event || (!delta && (e == "*" || e.strip_suffix(".*").is_some_and(|f| f == family))))
  }
}

impl Contributes {
  /// The contribution kinds present, in manifest order
  pub fn kinds(&self) -> Vec<&'static str> {
    let mut out = vec![];
    if !self.tools.is_empty() {
      out.push("tools");
    }
    if self.mcp.as_ref().is_some_and(|m| !m.is_empty()) {
      out.push("mcp");
    }
    if self.hooks.is_some() {
      out.push("hooks");
    }
    if self.theme.is_some() {
      out.push("theme");
    }
    if self.prompts.is_some() {
      out.push("prompts");
    }
    if self.agent.is_some() {
      out.push("agent");
    }
    if self.runtime.is_some() {
      out.push("runtime");
    }
    out
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
  pub name: String,
  #[serde(default)]
  pub description: String,
  /// The JSON Schema of the arguments
  #[serde(default = "object_schema")]
  pub input: Value,
  pub command: Vec<String>,
  #[serde(default)]
  pub read_only: bool,
  /// Seconds
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<f64>,
}

fn object_schema() -> Value {
  serde_json::json!({ "type": "object" })
}

impl ToolDef {
  pub fn timeout(&self) -> std::time::Duration {
    let secs = self.timeout.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(TOOL_TIMEOUT_SECS).min(TOOL_TIMEOUT_SECS);
    std::time::Duration::from_secs_f64(secs)
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
  #[serde(default, skip_serializing_if = "Map::is_empty")]
  pub tokens: Map<String, Value>,
  /// A style sheet relative to the plugin
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub css: Option<String>,
}

impl Manifest {
  pub fn display_name(&self) -> &str {
    self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.id)
  }

  /// The plugin page's row: the manifest's own pick, else the layer of its first contribution
  pub fn layer(&self) -> &str {
    if let Some(l) = self.layer.as_deref().filter(|l| LAYERS.contains(l)) {
      return LAYERS.iter().find(|x| **x == l).copied().unwrap_or("tools");
    }
    match self.contributes.kinds().first().copied() {
      Some("agent") => "agents",
      Some("theme") => "interface",
      Some("hooks") => "automation",
      _ => "tools",
    }
  }
}

/// The name a script tool is served under: `<id>_<tool>` (ids hold no `_`, so it is unique)
pub fn served_name(id: &str, tool: &str) -> String {
  format!("{id}_{tool}")
}

/// `[a-z0-9][a-z0-9-]*`, at most `NAME_MAX`
pub fn valid_id(id: &str) -> bool {
  let b = id.as_bytes();
  !b.is_empty() && b.len() <= NAME_MAX && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// `[a-z][a-z0-9_]*`, at most `NAME_MAX`
pub fn valid_tool_name(name: &str) -> bool {
  let b = name.as_bytes();
  !b.is_empty() && b.len() <= NAME_MAX && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// What the static check found: errors keep the plugin from being activated, notes are shown beside it
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Findings {
  pub errors: Vec<String>,
  pub notes: Vec<String>,
}

impl Findings {
  pub fn ok(&self) -> bool {
    self.errors.is_empty()
  }

  /// One block of text for a model or the page
  pub fn text(&self) -> String {
    let mut lines: Vec<String> = self.errors.iter().map(|e| format!("error: {e}")).collect();
    lines.extend(self.notes.iter().map(|n| format!("note: {n}")));
    lines.join("\n")
  }
}

/// Read `plugin.json` from `dir`: the manifest, or why it cannot be one
pub fn read(dir: &Path) -> Result<(Manifest, Findings), String> {
  let path = dir.join(FILE);
  let bytes = std::fs::read(&path).map_err(|e| format!("{FILE}: {e}"))?;
  parse(&bytes)
}

pub fn parse(bytes: &[u8]) -> Result<(Manifest, Findings), String> {
  let raw: Value = serde_json::from_slice(bytes).map_err(|e| format!("{FILE}: invalid JSON at line {}, column {}", e.line(), e.column()))?;
  let Some(obj) = raw.as_object() else { return Err(format!("{FILE}: expected an object")) };
  let schema = obj.get("schema").and_then(Value::as_u64);
  if schema != Some(SCHEMA as u64) {
    return Err(format!("{FILE}: schema {} is not one this Acpira reads (it reads {SCHEMA})", obj.get("schema").map(Value::to_string).unwrap_or_else(|| "missing".into())));
  }
  let mut findings = Findings::default();
  for k in obj.keys().filter(|k| !KEYS.contains(&k.as_str())) {
    findings.notes.push(format!("unknown key `{k}` ignored"));
  }
  if let Some(c) = obj.get("contributes").and_then(Value::as_object) {
    for k in c.keys().filter(|k| !CONTRIBUTION_KEYS.contains(&k.as_str())) {
      findings.notes.push(format!("unknown contribution `{k}` ignored"));
    }
  }
  let manifest: Manifest = serde_json::from_value(raw).map_err(|e| format!("{FILE}: {e}"))?;
  Ok((manifest, findings))
}

/// The static check of a manifest against the folder it sits in (the source folder or its frozen copy). `which`
/// resolves a program on PATH
pub fn check(m: &Manifest, folder_id: &str, dir: &Path, which: &dyn Fn(&str) -> bool, findings: &mut Findings) {
  let err = |f: &mut Findings, s: String| f.errors.push(s);
  if !valid_id(&m.id) {
    err(findings, format!("id `{}` must be lowercase letters, digits and `-`, at most {NAME_MAX} characters", m.id));
  }
  if m.id != folder_id {
    err(findings, format!("id `{}` must be the folder's name `{folder_id}`", m.id));
  }
  if m.description.trim().is_empty() {
    findings.notes.push("no description".into());
  }
  if let Some(l) = &m.layer
    && !LAYERS.contains(&l.as_str())
  {
    findings.notes.push(format!("layer `{l}` is not one of {}", LAYERS.join(", ")));
  }
  if m.contributes.kinds().is_empty() {
    err(findings, "contributes nothing".into());
  }
  for r in &m.requires {
    if !which(r) {
      err(findings, format!("requires `{r}`, which is not on PATH"));
    }
  }
  if let Some(check) = &m.check {
    command_exists(check, dir, which, "check", findings);
  }
  let mut seen = std::collections::HashSet::new();
  for t in &m.contributes.tools {
    let at = format!("tool `{}`", t.name);
    if !valid_tool_name(&t.name) {
      err(findings, format!("{at}: the name must be lowercase letters, digits and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !seen.insert(t.name.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if t.description.trim().is_empty() {
      err(findings, format!("{at}: a description is needed, the model picks tools by it"));
    }
    if t.input.get("type").and_then(Value::as_str) != Some("object") {
      err(findings, format!("{at}: `input` must be a JSON Schema of type object"));
    }
    if t.timeout.is_some_and(|s| s > TOOL_TIMEOUT_SECS) {
      findings.notes.push(format!("{at}: timeout cut to {TOOL_TIMEOUT_SECS} s"));
    }
    command_exists(&t.command, dir, which, &at, findings);
  }
  if let Some(theme) = &m.contributes.theme {
    if let Some(css) = &theme.css {
      file_inside(dir, css, "theme css", findings);
    }
    for k in theme.tokens.keys() {
      if !THEME_TOKENS.contains(&k.as_str()) {
        err(findings, format!("theme token `{k}` is not one a theme may set"));
      }
    }
    for (k, v) in &theme.tokens {
      if !v.as_str().is_some_and(safe_css_value) {
        err(findings, format!("theme token `{k}`: the value must be a CSS value without `;`, braces or `<`"));
      }
    }
  }
  if let Some(p) = &m.contributes.prompts
    && !inside(dir, p).is_some_and(|d| d.is_dir())
  {
    err(findings, format!("prompts folder `{p}` not found in the plugin"));
  }
  if let Some(agent) = &m.contributes.agent {
    match agent.get("command").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) {
      None => err(findings, "agent: `command` is needed".into()),
      Some(command) => command_exists(&[crate::plugins::mcp::strip_dir(command).to_owned()], dir, which, "agent", findings),
    }
  }
  if let Some(hooks) = &m.contributes.hooks {
    if !hooks.is_object() {
      err(findings, "hooks: expected the object of .agents/hooks.json".into());
    } else {
      match crate::hooks::parse(dir, &hooks.to_string()) {
        Err(e) => err(findings, format!("hooks: {e}")),
        Ok(c) => {
          if !c.unknown.is_empty() {
            err(findings, format!("hooks: unknown keys {}", c.unknown.join(", ")));
          }
          for p in &c.context {
            if !p.is_file() {
              err(findings, format!("hooks: context file {} not found in the plugin", p.strip_prefix(dir).unwrap_or(p).display()));
            }
          }
        }
      }
    }
  }
  if let Some(rt) = &m.contributes.runtime {
    command_exists(&rt.command, dir, which, "runtime", findings);
    if rt.hooks.is_empty() && rt.events.is_empty() {
      err(findings, "runtime: declares no `hooks` and no `events`, so it would never be called".into());
    }
    for h in rt.hooks.iter().filter(|h| !RUNTIME_HOOKS.contains(&h.as_str())) {
      findings.notes.push(format!("runtime: hook `{h}` is not one this Acpira calls"));
    }
    for e in &rt.events {
      let family = e.strip_suffix(".*").unwrap_or(e);
      if e != "*" && !RUNTIME_EVENTS.iter().any(|k| *k == e || k.split('.').next() == Some(family)) {
        findings.notes.push(format!("runtime: event `{e}` is not one this Acpira sends"));
      }
    }
    if rt.timeout.is_some_and(|s| s > RUNTIME_TIMEOUT_SECS) {
      findings.notes.push(format!("runtime: timeout cut to {RUNTIME_TIMEOUT_SECS} s"));
    }
  }
  if let Some(mcp) = &m.contributes.mcp {
    for (name, entry) in mcp {
      if name == acpira_shared::host_tools::SERVER {
        err(findings, format!("mcp: the name `{name}` is Acpira's own"));
      }
      if !entry.is_object() {
        err(findings, format!("mcp `{name}`: expected an entry of .mcp.json"));
      } else if let Some(command) = entry.get("command").and_then(Value::as_str) {
        let program = crate::plugins::mcp::strip_dir(command).to_owned();
        command_exists(&[program], dir, which, &format!("mcp `{name}`"), findings);
      } else if entry.get("url").and_then(Value::as_str).is_none() {
        err(findings, format!("mcp `{name}`: needs a `command` or a `url`"));
      }
    }
  }
}

/// A command's program: a file of the plugin when it names one (`convert.py`, `bin/tool`), else a program on PATH
fn command_exists(command: &[String], dir: &Path, which: &dyn Fn(&str) -> bool, at: &str, findings: &mut Findings) {
  let Some(program) = command.first().filter(|p| !p.trim().is_empty()) else {
    findings.errors.push(format!("{at}: `command` is empty"));
    return;
  };
  if program_in(dir, program).is_some() || (!program.contains(['/', '\\']) && which(program)) {
    return;
  }
  findings.errors.push(format!("{at}: `{program}` is neither a file of the plugin nor on PATH"));
}

/// `program` as a file inside the plugin folder, when it names one
pub fn program_in(dir: &Path, program: &str) -> Option<std::path::PathBuf> {
  inside(dir, program).filter(|p| p.is_file())
}

fn file_inside(dir: &Path, rel: &str, what: &str, findings: &mut Findings) {
  if !inside(dir, rel).is_some_and(|p| p.is_file()) {
    findings.errors.push(format!("{what} `{rel}` not found in the plugin"));
  }
}

/// A relative path that stays inside `dir` (no `..`, not absolute)
pub fn inside(dir: &Path, rel: &str) -> Option<std::path::PathBuf> {
  let p = Path::new(rel);
  if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
    return None;
  }
  Some(dir.join(p))
}

/// The tokens a theme may set: the palette and type roles of `packages/ui/styles/tokens.css`, not the sizes the
/// layout is built on
pub const THEME_TOKENS: &[&str] = &[
  "--fg-1",
  "--fg-2",
  "--fg-3",
  "--fg-strong",
  "--action-fg",
  "--bg-0",
  "--bg-1",
  "--bg-2",
  "--bg-code",
  "--line",
  "--line-strong",
  "--conversation-line",
  "--conversation-line-focus",
  "--chip",
  "--chip-hover",
  "--hover",
  "--active",
  "--focus",
  "--brand",
  "--brand-fg",
  "--ok",
  "--warn",
  "--danger",
  "--diff-add",
  "--diff-del",
  "--font",
  "--font-mono",
];

/// No way out of a declaration: a value cannot end it, open a block or start markup
pub fn safe_css_value(v: &str) -> bool {
  !v.trim().is_empty() && !v.contains([';', '{', '}', '<', '>']) && !v.contains("/*")
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn manifest(v: Value) -> (Manifest, Findings) {
    parse(v.to_string().as_bytes()).unwrap()
  }

  #[test]
  fn a_script_tool_manifest_passes_and_names_its_tool() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("convert.py"), "print(1)").unwrap();
    let (m, mut f) = manifest(json!({
      "schema": 1, "id": "currency", "version": "0.1.0", "description": "x",
      "contributes": { "tools": [{ "name": "convert", "description": "Convert", "input": { "type": "object" }, "command": ["python3", "convert.py"], "readOnly": true }] }
    }));
    check(&m, "currency", dir.path(), &|p| p == "python3", &mut f);
    assert!(f.ok(), "{f:?}");
    assert_eq!(m.layer(), "tools");
    assert_eq!(served_name(&m.id, &m.contributes.tools[0].name), "currency_convert");
    assert_eq!(m.contributes.tools[0].timeout(), std::time::Duration::from_secs(45));
  }

  #[test]
  fn the_static_check_names_every_problem_without_running_anything() {
    let dir = tempfile::tempdir().unwrap();
    let (m, mut f) = manifest(json!({
      "schema": 1, "id": "Bad_Id", "extra": 1, "requires": ["nope"],
      "contributes": {
        "tools": [{ "name": "Go", "command": ["../escape.sh"], "input": { "type": "string" } }, { "name": "Go", "command": [] }],
        "theme": { "tokens": { "--ctl-w": "1px", "--bg-0": "red;}" }, "css": "missing.css" },
        "mcp": { "acpira": {} },
        "widgets": []
      }
    }));
    check(&m, "other", dir.path(), &|_| false, &mut f);
    let text = f.text();
    for needle in ["id `Bad_Id`", "folder's name", "requires `nope`", "tool `Go`: the name", "declared twice", "a description is needed", "type object", "`../escape.sh`", "`command` is empty", "`--ctl-w`", "`--bg-0`", "missing.css", "Acpira's own", "unknown key `extra`", "unknown contribution `widgets`"] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
  }

  #[test]
  fn an_unknown_schema_is_refused_whole() {
    assert!(parse(br#"{"schema":2,"id":"x"}"#).unwrap_err().contains("schema 2"));
    assert!(parse(b"{").unwrap_err().contains("invalid JSON"));
  }

  #[test]
  fn ids_and_tool_names() {
    assert!(valid_id("currency") && valid_id("a-1") && !valid_id("a_b") && !valid_id("-a") && !valid_id(""));
    assert!(valid_tool_name("convert_rate") && !valid_tool_name("1x") && !valid_tool_name("a-b"));
    assert!(inside(Path::new("/p"), "a/b.css").is_some() && inside(Path::new("/p"), "../x").is_none() && inside(Path::new("/p"), "/etc/x").is_none());
  }
}
