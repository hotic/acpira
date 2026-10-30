//! `plugin.json`, schema 1 (`docs/dev/plugins.md`): what a plugin contributes, and the static check that reads the
//! manifest and the frozen files alone, without running any of the plugin's code

use std::collections::BTreeMap;
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

const KEYS: &[&str] = &["schema", "id", "version", "name", "description", "requires", "layer", "check", "contributes", "dependencies", "optionalDependencies", "exports"];
const CONTRIBUTION_KEYS: &[&str] = &["tools", "mcp", "hooks", "theme", "style", "prompts", "agent", "runtime", "skills", "commands", "providers", "ui", "schedules"];

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
  "websearch",
  "command",
];
/// The event families a resident process may subscribe to; `<family>.*` takes every event of one, `*` all of them
/// except the streamed deltas, which are asked for by name (`text.delta`, `thought.delta`)
pub const RUNTIME_EVENTS: &[&str] = &["session", "turn", "tool", "permission", "files", "model", "plugin", "usage", "text.delta", "thought.delta"];
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
  /// Other plugins this one needs, by id, with an npm-style version range (`^1.2`, `~0.3`, `*`): it calls only these
  /// and the user enables them first. npm packages belong in `package.json`
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub dependencies: BTreeMap<String, String>,
  /// Plugins it uses when they are there (`ctx.plugins.get`)
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub optional_dependencies: BTreeMap<String, String>,
  /// The module other plugins import as `@acpira-plugin/<id>` once copied into theirs (`acpira plugin vendor`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub exports: Option<String>,
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
  /// A stylesheet of the plugin laid over the page whatever the skin, through `STYLE_HOOKS` only; every plugin's in
  /// effect applies, after the skin's
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub style: Option<String>,
  /// A folder of prompt sections, relative to the plugin
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prompts: Option<String>,
  /// One ACP agent, `agents.json`'s entry shape plus `name`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<Value>,
  /// A resident process: the hooks it answers and the events it hears
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub runtime: Option<RuntimeDef>,
  /// A folder of skills (`<name>/SKILL.md`), relative to the plugin: every agent is offered them
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub skills: Option<String>,
  /// Slash commands every session lists: a prompt template, or the runtime's `command` hook
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub commands: Vec<CommandDef>,
  /// Model sources for Acpira's built-in agent, `providers.json`'s source shape plus `keyEnv` (`provider_of`). Kept
  /// raw so the check can name what is wrong with each
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub providers: Vec<Value>,
  /// Interface pieces: an HTML file shown in a sandboxed frame in one of the page's slots
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub ui: Vec<UiDef>,
  /// Jobs the engine starts on a cron schedule, answered by the runtime (`schedule/<id>`)
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub schedules: Vec<ScheduleDef>,
}

/// `contributes.schedules[]`: `cron` is five fields (minute hour day-of-month month day-of-week) in the machine's zone
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDef {
  pub id: String,
  pub cron: String,
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub description: String,
  /// A run missed while no engine ran (asleep, closed): `once` runs it when one starts again (the default), `skip`
  /// waits for the next time
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub catch_up: Option<String>,
  /// Seconds one run may take, at most `SCHEDULE_TIMEOUT_MAX_SECS`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<f64>,
}

/// The longest a scheduled run may take
pub const SCHEDULE_TIMEOUT_MAX_SECS: f64 = 3600.0;
/// When a scheduled run declares no timeout
pub const SCHEDULE_TIMEOUT_SECS: f64 = 600.0;
pub const CATCH_UP: &[&str] = &["once", "skip"];

impl ScheduleDef {
  pub fn timeout(&self) -> std::time::Duration {
    let secs = self.timeout.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(SCHEDULE_TIMEOUT_SECS).min(SCHEDULE_TIMEOUT_MAX_SECS);
    std::time::Duration::from_secs_f64(secs)
  }

  pub fn catches_up(&self) -> bool {
    self.catch_up.as_deref() != Some("skip")
  }
}

/// `contributes.ui[]`: `html` is a file of the plugin, at most `UI_HTML_MAX_BYTES`, for the slot `slot`
/// (`acpira_shared::plugins::UI_SLOTS`); a `message.actions` piece is a native button calling `method` instead
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDef {
  pub id: String,
  pub slot: String,
  #[serde(default)]
  pub title: String,
  #[serde(default)]
  pub html: String,
  /// A component module (`.tsx`, `.jsx`, `.ts`, `.js`) whose default export the frame renders with React, compiled by
  /// the engine (`build.rs`) in place of an `html` file
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub entry: Option<String>,
  /// The piece's box in CSS pixels, along the sides the slot does not size (`slot_box`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub width: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub height: Option<u32>,
  /// `message.actions`: the runtime method a click calls with `{ session, turn, text }`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub method: Option<String>,
}

/// Which sides of a piece's box its slot leaves to the plugin (`width`, `height`): a slot sizes the rest itself
pub fn slot_box(slot: &str) -> (bool, bool) {
  match slot {
    "composer.above" | "composer.toolbar" | "statusbar" => (true, true),
    "session.panel" => (true, false),
    "sidebar" | "start" => (false, true),
    _ => (false, false),
  }
}

/// Largest interface document; it travels to every page
pub const UI_HTML_MAX_BYTES: u64 = 256 * 1024;

/// `contributes.commands[]`: `/name` in the composer of every session. `prompt` is sent in its place with `$ARGUMENTS`
/// replaced by what follows the name (appended when the template has no `$ARGUMENTS`); without one the runtime's
/// `command` hook answers. With `button` it is also a button of that label in the composer's toolbar, run with the draft
/// as its arguments: sent like `/name <draft>`, or with `draft` its text put in place of the draft for the user to send
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDef {
  pub name: String,
  #[serde(default)]
  pub description: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub hint: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prompt: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub button: Option<String>,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub draft: bool,
  /// The button's glyph, one of `BUTTON_ICONS`; absent: a bolt, or a wand for a `draft` command
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon: Option<String>,
  /// The button's look, one of `BUTTON_LOOKS`; absent: `brand`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub look: Option<String>,
  /// Whether the button takes the draft as its arguments: `draft` or `none`. Absent: it does when `draft` is set or the
  /// template has `$ARGUMENTS`, so a fixed prompt never sweeps up what the user is still writing
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input: Option<String>,
}

impl CommandDef {
  /// `input` as it applies (see there)
  pub fn takes_draft(&self) -> bool {
    match self.input.as_deref() {
      Some(i) => i == "draft",
      None => self.draft || self.prompt.as_deref().is_some_and(|p| p.contains("$ARGUMENTS")),
    }
  }
}

/// Largest stylesheet a theme's `css` or a `style` may be: a longer one is refused by the check and cut where served
pub const CSS_MAX_BYTES: u64 = 64 * 1024;

/// Longest composer button label, in characters: a chip beside the mode, not a sentence
pub const BUTTON_MAX_CHARS: usize = 16;
/// The glyphs a command button may carry (`plugins/CommandButtons.tsx` draws each; a contract test keeps the two lists equal)
pub const BUTTON_ICONS: &[&str] = &[
  "bolt", "wand", "sparkles", "git-commit", "git-branch", "git-pull-request", "check", "play", "rocket", "bug", "test", "search",
  "eye", "file", "list", "message", "languages", "refresh", "terminal", "package", "star", "heart", "flame", "bot",
];
/// `brand`: a tint of the theme's brand colour; `solid`: the mode chip's fill; `outline`: a hairline; `icon`: the glyph
/// alone, the label in its tooltip
pub const BUTTON_LOOKS: &[&str] = &["brand", "solid", "outline", "icon"];

/// `contributes.runtime`: a process the engine keeps running while the plugin is in effect, spoken to over ndjson
/// JSON-RPC on its stdio (docs/dev/plugins.md, Runtime)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDef {
  /// The program and its arguments; empty with `main`
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub command: Vec<String>,
  /// A module (`.ts`, `.js`, …) whose default export is the plugin's definition (`definePlugin`), compiled by the
  /// engine and run with Node in place of a `command`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub main: Option<String>,
  /// Hook names from `RUNTIME_HOOKS`; only these are ever called
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub hooks: Vec<String>,
  /// Event names or `<family>.*` / `*`; only these are ever sent
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub events: Vec<String>,
  /// Methods the pages and other plugins may call (`plugin/call`, `pluginCall`), sent to the process as `rpc/<name>`
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub methods: Vec<String>,
  /// Seconds one hook call may take, at most `RUNTIME_TIMEOUT_SECS`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<f64>,
  /// `"agent"`: the hooks are asked only in sessions of the plugin's own agent (`contributes.agent`), so a specialised
  /// agent's guards never touch another agent's work
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub scope: Option<String>,
}

/// `runtime.scope` of a plugin whose hooks serve its own agent only
pub const SCOPE_AGENT: &str = "agent";

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
    if self.style.is_some() {
      out.push("style");
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
    if self.skills.is_some() {
      out.push("skills");
    }
    if !self.commands.is_empty() {
      out.push("commands");
    }
    if !self.providers.is_empty() {
      out.push("providers");
    }
    if !self.ui.is_empty() {
      out.push("ui");
    }
    if !self.schedules.is_empty() {
      out.push("schedules");
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
  /// The script run per call; empty: the runtime answers it (`tools` of its definition)
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
  /// `dark` / `light`: the skin fixes the page's scheme (its tokens are written for it); absent: the host's
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub base: Option<String>,
  /// How much of the backdrop the surfaces let through (`GLASS`); only with a backdrop
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub glass: Option<String>,
  /// An HTML document relative to the plugin, run in a sandboxed frame behind the page (desktop)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub backdrop: Option<String>,
  /// A scene script relative to the plugin, drawn by Acpira's own glyph renderer (`sdk::backdrop_html`) as the
  /// backdrop; the renderer is the running Acpira's, not a copy in the plugin. Excludes `backdrop`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub scene: Option<String>,
}

pub const THEME_BASES: &[&str] = &["dark", "light"];
pub const GLASS: &[&str] = &["veil", "calm", "frost"];
/// Largest backdrop document; it travels to every page with the theme
pub const BACKDROP_MAX_BYTES: u64 = 256 * 1024;
/// Largest scene script; the renderer around it is Acpira's
pub const SCENE_MAX_BYTES: u64 = 64 * 1024;
/// Largest box a sized interface piece may ask for, in CSS pixels
pub const UI_SIZE_MAX: u32 = 480;

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
      Some("agent" | "providers") => "agents",
      Some("theme" | "style" | "ui") => "interface",
      Some("hooks" | "runtime") => "automation",
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

/// `[a-z][a-z0-9_-]*`, at most `NAME_MAX`
pub fn valid_command_name(name: &str) -> bool {
  let b = name.as_bytes();
  !b.is_empty() && b.len() <= NAME_MAX && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

/// An environment variable name: `[A-Z_][A-Z0-9_]*`
fn valid_env_name(name: &str) -> bool {
  let b = name.as_bytes();
  !b.is_empty() && !b[0].is_ascii_digit() && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}

/// One contributed model source as the built-in agent reads it, tagged with `plugin`; the reason when it cannot be
/// used. The id follows plugin ids' rules (no `/`, which would break `<provider>/<model>` picks), the format is one
/// the agent speaks, and every model has an id. A key comes from the vault under the source's id or from the
/// variable `keyEnv` names; the key itself never goes in the file
pub fn provider_of(raw: &Value, plugin: &str) -> Result<acpira_shared::providers::Provider, String> {
  use acpira_shared::providers::{KEY_ENV_FIELD, PLUGIN_FIELD, Provider};
  let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
  let at = format!("provider `{id}`");
  if !valid_id(id) {
    return Err(format!("{at}: the id must be lowercase letters, digits and `-`, at most {NAME_MAX} characters"));
  }
  for k in ["apiKey", "key"] {
    if raw.get(k).is_some() {
      return Err(format!("{at}: `{k}` is not allowed, name the variable that holds it in `{KEY_ENV_FIELD}`"));
    }
  }
  let mut p: Provider = serde_json::from_value(raw.clone()).map_err(|e| format!("{at}: {e}"))?;
  // A credential in a header would land in the file like a key; a token goes through `model.request` instead
  if let Some(h) = p.headers.keys().find(|h| matches!(h.to_ascii_lowercase().as_str(), "authorization" | "proxy-authorization" | "x-api-key" | "api-key" | "x-goog-api-key")) {
    return Err(format!("{at}: header `{h}` is a credential; name its variable in `{KEY_ENV_FIELD}` or send it from the `model.request` hook"));
  }
  if p.api_format().is_none() {
    return Err(format!("{at}: format `{}` is not openai-chat, openai-responses or anthropic", p.format));
  }
  if p.base_url.trim().is_empty() {
    return Err(format!("{at}: `baseUrl` is needed"));
  }
  if p.models.is_empty() || p.models.iter().any(|m| m.id.trim().is_empty()) {
    return Err(format!("{at}: `models` needs at least one model, each with an `id`"));
  }
  if let Some(env) = p.extra.get(KEY_ENV_FIELD)
    && !env.as_str().is_some_and(valid_env_name)
  {
    return Err(format!("{at}: `{KEY_ENV_FIELD}` must be an environment variable name"));
  }
  if p.name.trim().is_empty() {
    p.name = id.to_owned();
  }
  p.extra.insert(PLUGIN_FIELD.into(), Value::String(plugin.to_owned()));
  Ok(p)
}

/// `[a-z][a-z0-9_.-]*`, at most 48 characters: a method a runtime offers (`rates.refresh`)
pub fn valid_method_name(name: &str) -> bool {
  let b = name.as_bytes();
  !b.is_empty() && b.len() <= 48 && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(c))
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
    if !t.command.is_empty() {
      command_exists(&t.command, dir, which, &at, findings);
    } else if m.contributes.runtime.is_none() {
      err(findings, format!("{at}: needs a `command`, or a runtime whose definition answers it (`tools` in `definePlugin`)"));
    }
  }
  if let Some(css) = &m.contributes.style {
    css_file(dir, css, "style css", findings);
  }
  if let Some(theme) = &m.contributes.theme {
    if let Some(css) = &theme.css {
      css_file(dir, css, "theme css", findings);
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
    if let Some(b) = theme.base.as_deref().filter(|b| !THEME_BASES.contains(b)) {
      err(findings, format!("theme base `{b}` is not one of {}", THEME_BASES.join(", ")));
    }
    if let Some(g) = theme.glass.as_deref().filter(|g| !GLASS.contains(g)) {
      err(findings, format!("theme glass `{g}` is not one of {}", GLASS.join(", ")));
    }
    for (what, file, max) in [("backdrop", &theme.backdrop, BACKDROP_MAX_BYTES), ("scene", &theme.scene, SCENE_MAX_BYTES)] {
      let Some(b) = file else { continue };
      match inside(dir, b).and_then(|p| std::fs::metadata(p).ok()).filter(std::fs::Metadata::is_file) {
        None => err(findings, format!("theme {what} `{b}` not found in the plugin")),
        Some(meta) if meta.len() > max => err(findings, format!("theme {what} `{b}` is over {} KB", max / 1024)),
        Some(_) => {}
      }
    }
    if theme.backdrop.is_some() && theme.scene.is_some() {
      err(findings, "theme: `backdrop` and `scene` exclude each other (a scene is drawn by Acpira's renderer, a backdrop draws itself)".into());
    }
    if theme.backdrop.is_some() || theme.scene.is_some() {
      if theme.base.is_none() {
        findings.notes.push("theme: a backdrop without `base` follows the host's light or dark scheme; its tokens must suit both".into());
      }
    } else if theme.glass.is_some() {
      findings.notes.push("theme: `glass` only applies with a `backdrop` or a `scene`".into());
    }
  }
  if let Some(p) = &m.contributes.prompts
    && !inside(dir, p).is_some_and(|d| d.is_dir())
  {
    err(findings, format!("prompts folder `{p}` not found in the plugin"));
  }
  if let Some(p) = &m.contributes.skills
    && !inside(dir, p).is_some_and(|d| d.is_dir())
  {
    err(findings, format!("skills folder `{p}` not found in the plugin"));
  }
  let mut names = std::collections::HashSet::new();
  // A `main` definition answers its commands in code (`commands` of `definePlugin`) without listing the hook
  let hooked = m.contributes.runtime.as_ref().is_some_and(|r| r.main.is_some() || r.hooks.iter().any(|h| h == "command"));
  for c in &m.contributes.commands {
    let at = format!("command `/{}`", c.name);
    if !valid_command_name(&c.name) {
      err(findings, format!("{at}: the name must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !names.insert(c.name.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if c.prompt.as_deref().is_none_or(|p| p.trim().is_empty()) && !hooked {
      err(findings, format!("{at}: needs a `prompt`, or a runtime that answers the `command` hook"));
    }
    match c.button.as_deref().map(str::trim) {
      Some(b) if b.is_empty() || b.chars().count() > BUTTON_MAX_CHARS => err(findings, format!("{at}: `button` is the label, 1 to {BUTTON_MAX_CHARS} characters")),
      None if c.draft || c.icon.is_some() || c.look.is_some() || c.input.is_some() => err(findings, format!("{at}: `draft`, `icon`, `look` and `input` need a `button`")),
      _ => {}
    }
    if let Some(i) = c.icon.as_deref().filter(|i| !BUTTON_ICONS.contains(i)) {
      err(findings, format!("{at}: icon `{i}` is not one of {}", BUTTON_ICONS.join(", ")));
    }
    if let Some(l) = c.look.as_deref().filter(|l| !BUTTON_LOOKS.contains(l)) {
      err(findings, format!("{at}: look `{l}` is not one of {}", BUTTON_LOOKS.join(", ")));
    }
    if let Some(i) = c.input.as_deref().filter(|i| !["draft", "none"].contains(i)) {
      err(findings, format!("{at}: input `{i}` is not `draft` or `none`"));
    }
  }
  let mut pieces = std::collections::HashSet::new();
  for u in &m.contributes.ui {
    let at = format!("ui `{}`", u.id);
    if !valid_command_name(&u.id) {
      err(findings, format!("{at}: the id must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !pieces.insert(u.id.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if !acpira_shared::plugins::UI_SLOTS.contains(&u.slot.as_str()) {
      err(findings, format!("{at}: slot `{}` is not one of {}", u.slot, acpira_shared::plugins::UI_SLOTS.join(", ")));
    }
    if u.slot == "message.actions" {
      // A button under each reply: one frame per message would cost a document per turn
      match u.method.as_deref() {
        None => err(findings, format!("{at}: a `message.actions` piece names the runtime `method` its button calls, instead of `html`")),
        Some(method) if !m.contributes.runtime.as_ref().is_some_and(|r| r.methods.iter().any(|x| x == method)) => {
          err(findings, format!("{at}: method `{method}` is not one of the runtime's `methods`"))
        }
        Some(_) => {}
      }
      if !u.html.is_empty() || u.entry.is_some() {
        err(findings, format!("{at}: a `message.actions` piece is a button, not a document; drop `html` and `entry`"));
      }
      if u.title.trim().is_empty() {
        err(findings, format!("{at}: the button needs a `title`"));
      }
      continue;
    }
    if u.method.is_some() {
      err(findings, format!("{at}: only a `message.actions` piece calls a `method`"));
    }
    match (u.html.is_empty(), u.entry.as_deref()) {
      (false, Some(_)) => err(findings, format!("{at}: `html` and `entry` exclude each other")),
      (true, None) => err(findings, format!("{at}: needs an `html` file or a component `entry`")),
      (false, None) => match inside(dir, &u.html).and_then(|p| std::fs::metadata(p).ok()).filter(std::fs::Metadata::is_file) {
        None => err(findings, format!("{at}: html file `{}` not found in the plugin", u.html)),
        Some(meta) if meta.len() > UI_HTML_MAX_BYTES => err(findings, format!("{at}: `{}` is over {} KB", u.html, UI_HTML_MAX_BYTES / 1024)),
        Some(_) => {}
      },
      (true, Some(entry)) => source_file(dir, entry, &format!("{at}: entry"), findings),
    }
    for (name, v) in [("width", u.width), ("height", u.height)] {
      if v.is_some_and(|v| v == 0 || v > UI_SIZE_MAX) {
        err(findings, format!("{at}: {name} must be 1 to {UI_SIZE_MAX} CSS pixels"));
      }
    }
    let (w, h) = slot_box(&u.slot);
    let missing: Vec<&str> = [(w && u.width.is_none(), "`width`"), (h && u.height.is_none(), "`height`")].into_iter().filter(|(m, _)| *m).map(|(_, n)| n).collect();
    if !missing.is_empty() {
      err(findings, format!("{at}: the slot `{}` needs {}", u.slot, missing.join(" and ")));
    }
  }
  let mut sources = std::collections::HashSet::new();
  for raw in &m.contributes.providers {
    match provider_of(raw, &m.id) {
      Err(e) => err(findings, e),
      Ok(p) if !sources.insert(p.id.clone()) => err(findings, format!("provider `{}` is declared twice", p.id)),
      Ok(_) => {}
    }
  }
  if let Some(agent) = &m.contributes.agent {
    if agent.get("builtin").is_some() {
      // A specialised agent runs Acpira's own loop: the agent reads the profile itself, so it is checked the same way
      if agent.get("command").is_some() {
        err(findings, "agent: `builtin` and `command` exclude each other".into());
      } else if let Err(e) = acpira_agent::profile::Profile::load(dir) {
        err(findings, format!("agent: {e}"));
      }
    } else {
      match agent.get("command").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) {
        None => err(findings, "agent: `command` (another ACP agent) or `builtin` (Acpira's loop with a profile) is needed".into()),
        Some(command) => command_exists(&[crate::plugins::mcp::strip_dir(command).to_owned()], dir, which, "agent", findings),
      }
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
    match (rt.command.is_empty(), rt.main.as_deref()) {
      (false, Some(_)) => err(findings, "runtime: `command` and `main` exclude each other".into()),
      (true, None) => err(findings, "runtime: needs a `command`, or a `main` module whose default export is the plugin's definition".into()),
      (false, None) => command_exists(&rt.command, dir, which, "runtime", findings),
      (true, Some(main)) => {
        source_file(dir, main, "runtime: main", findings);
        if !which("node") {
          err(findings, "runtime: `main` runs with Node 18 or newer, which is not on PATH".into());
        }
      }
    }
    let served = m.contributes.tools.iter().any(|t| t.command.is_empty())
      || !m.contributes.schedules.is_empty()
      || m.contributes.commands.iter().any(|c| c.prompt.is_none());
    if rt.hooks.is_empty() && rt.events.is_empty() && rt.methods.is_empty() && !served {
      err(findings, "runtime: declares no `hooks`, `events`, `methods`, tools, schedules or prompt-less commands, so it would never be called".into());
    }
    for name in rt.methods.iter().filter(|n| !valid_method_name(n)) {
      err(findings, format!("runtime: method `{name}` must be lowercase letters, digits, `_`, `.` and `-`, starting with a letter, at most 48 characters"));
    }
    for h in rt.hooks.iter().filter(|h| !RUNTIME_HOOKS.contains(&h.as_str())) {
      findings.notes.push(format!("runtime: hook `{h}` is not one this Acpira calls"));
    }
    for e in &rt.events {
      // `turn.*` and `turn.ended` both name the `turn` family; the names inside a family are not listed here
      let family = e.strip_suffix(".*").unwrap_or(e).split('.').next().unwrap_or(e);
      if e != "*" && !RUNTIME_EVENTS.iter().any(|k| *k == e || k.split('.').next() == Some(family)) {
        findings.notes.push(format!("runtime: event `{e}` is not one this Acpira sends"));
      }
    }
    match rt.scope.as_deref() {
      None => {}
      Some(SCOPE_AGENT) if m.contributes.agent.is_some() => {}
      Some(SCOPE_AGENT) => err(findings, "runtime: scope `agent` needs the plugin's own `agent`".into()),
      Some(other) => err(findings, format!("runtime: scope `{other}` is not `agent`")),
    }
    if rt.timeout.is_some_and(|s| s > RUNTIME_TIMEOUT_SECS) {
      findings.notes.push(format!("runtime: timeout cut to {RUNTIME_TIMEOUT_SECS} s"));
    }
  }
  let mut jobs = std::collections::HashSet::new();
  for j in &m.contributes.schedules {
    let at = format!("schedule `{}`", j.id);
    if !valid_command_name(&j.id) {
      err(findings, format!("{at}: the id must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !jobs.insert(j.id.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if let Err(e) = crate::plugins::schedule::Cron::parse(&j.cron) {
      err(findings, format!("{at}: cron {e}"));
    }
    if let Some(c) = j.catch_up.as_deref().filter(|c| !CATCH_UP.contains(c)) {
      err(findings, format!("{at}: catchUp `{c}` is not one of {}", CATCH_UP.join(", ")));
    }
    if j.timeout.is_some_and(|t| t > SCHEDULE_TIMEOUT_MAX_SECS) {
      findings.notes.push(format!("{at}: timeout cut to {SCHEDULE_TIMEOUT_MAX_SECS} s"));
    }
    if m.contributes.runtime.is_none() {
      err(findings, format!("{at}: needs a runtime to run it (`schedules` in `definePlugin`, or the SDK's `schedule`)"));
    }
  }
  check_dependencies(m, findings);
  if let Some(e) = &m.exports {
    source_file(dir, e, "exports", findings);
  }
  check_packages(dir, findings);
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

/// `dependencies` / `optionalDependencies`: plugin ids with npm-style ranges. Whether they are installed and in effect is
/// the host's check (`PluginHost::check_dependencies`), against the state
fn check_dependencies(m: &Manifest, findings: &mut Findings) {
  let mut seen = std::collections::HashSet::new();
  for (kind, map) in [("dependencies", &m.dependencies), ("optionalDependencies", &m.optional_dependencies)] {
    for (id, range) in map {
      if !valid_id(id) {
        findings.errors.push(format!("{kind}: `{id}` is not a plugin id; npm packages go in package.json"));
        continue;
      }
      if *id == m.id {
        findings.errors.push(format!("{kind}: a plugin cannot depend on itself"));
      }
      if !seen.insert(id.as_str()) {
        findings.errors.push(format!("`{id}` is in both dependencies and optionalDependencies"));
      }
      if let Err(e) = version_req(range) {
        findings.errors.push(format!("{kind} `{id}`: {e}"));
      }
    }
  }
}

/// `version` satisfies the npm-style `range`. `version` is free text in the manifest: `1` and `1.2` count as `1.0.0`
/// and `1.2.0`, a leading `v` is dropped, anything else satisfies nothing
pub fn version_meets(version: &str, range: &str) -> bool {
  let Some(v) = coerce_version(version) else { return false };
  version_req(range).is_ok_and(|r| r.matches(&v))
}

fn coerce_version(text: &str) -> Option<semver::Version> {
  let t = text.trim().trim_start_matches(['v', 'V']);
  if let Ok(v) = semver::Version::parse(t) {
    return Some(v);
  }
  let parts: Vec<u64> = t.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
  match parts.as_slice() {
    [major] => Some(semver::Version::new(*major, 0, 0)),
    [major, minor] => Some(semver::Version::new(*major, *minor, 0)),
    _ => None,
  }
}

/// An npm range: `^1.2`, `~0.3.1`, `>=1.2 <2` (comparators by space or comma), `1.x`, `*`. A bare version, partial
/// or not, means that version as npm reads it: `1.2.3` exactly, `1.2` any `1.2.x`, `1` any `1.x` (the semver crate
/// alone would read a bare `1.2` as `^1.2`). `||` alternatives and hyphen ranges are refused
pub fn version_req(range: &str) -> Result<semver::VersionReq, String> {
  let r = range.trim();
  if r.is_empty() || r == "latest" || r == "*" || r == "x" {
    return Ok(semver::VersionReq::STAR);
  }
  if r.contains("||") {
    return Err(format!("`{r}`: alternatives (`||`) are not supported; use one range"));
  }
  if r.contains(" - ") {
    return Err(format!("`{r}`: hyphen ranges are not supported; write `>=a <=b`"));
  }
  let bad = || format!("`{r}` is not a version range (`^1.2`, `~0.3`, `>=1 <2`, `1.x`, `*`)");
  // Comparators, an operator written apart from its version (`>= 1.2`) joined back to it
  let mut comparators: Vec<String> = vec![];
  let mut pending = String::new();
  for token in r.split([' ', ',']).filter(|t| !t.is_empty()) {
    if token.chars().all(|c| "<>=^~".contains(c)) {
      pending.push_str(token);
      continue;
    }
    let token = token.trim_start_matches(['v', 'V']).replace(".x", ".*").replace(".X", ".*");
    let op_len = token.find(|c: char| !"<>=^~".contains(c)).unwrap_or(token.len());
    let (op, ver) = token.split_at(op_len);
    let op = format!("{pending}{op}");
    pending.clear();
    let ver = ver.trim_start_matches(['v', 'V']);
    if ver.is_empty() {
      return Err(bad());
    }
    comparators.push(if op.is_empty() { format!("={ver}") } else { format!("{op}{ver}") });
  }
  if !pending.is_empty() || comparators.is_empty() {
    return Err(bad());
  }
  semver::VersionReq::parse(&comparators.join(", ")).map_err(|_| bad())
}

/// Lockfiles that pin what `node_modules` holds
const LOCKFILES: &[&str] = &["package-lock.json", "npm-shrinkwrap.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb"];

/// npm packages: `package.json` lists them, the author installs them (Acpira never runs an install or its scripts), and
/// the lockfile pins them. Native code in them is named, since it runs with the user's rights like the rest
fn check_packages(dir: &Path, findings: &mut Findings) {
  let Ok(bytes) = std::fs::read(dir.join("package.json")) else { return };
  let pkg: Value = match serde_json::from_slice(&bytes) {
    Ok(v) => v,
    Err(e) => {
      findings.errors.push(format!("package.json: {e}"));
      return;
    }
  };
  let listed = ["dependencies", "optionalDependencies"].iter().any(|k| pkg.get(k).and_then(Value::as_object).is_some_and(|d| !d.is_empty()));
  if !listed {
    return;
  }
  if !dir.join("node_modules").is_dir() {
    findings.errors.push("package.json lists dependencies but node_modules is missing: run `npm install --ignore-scripts` in the plugin folder".into());
  }
  if !LOCKFILES.iter().any(|f| dir.join(f).is_file()) {
    findings.errors.push("package.json lists dependencies without a lockfile: install with npm, pnpm, yarn or bun so the versions are pinned".into());
  }
  let mut native = vec![];
  let _ = walk_files(&dir.join("node_modules"), &mut |p| {
    if p.extension().is_some_and(|e| e == "node") && native.len() < 5 {
      native.push(p.strip_prefix(dir).unwrap_or(p).to_string_lossy().replace('\\', "/"));
    }
  });
  if !native.is_empty() {
    findings.notes.push(format!("native code in node_modules ({}): it runs in the plugin's process with your rights", native.join(", ")));
  }
}

fn walk_files(dir: &Path, f: &mut dyn FnMut(&Path)) -> std::io::Result<()> {
  for e in std::fs::read_dir(dir)?.flatten() {
    let t = e.file_type()?;
    if t.is_dir() {
      walk_files(&e.path(), f)?;
    } else if t.is_file() {
      f(&e.path());
    }
  }
  Ok(())
}

/// A module the engine compiles (`runtime.main`, `ui[].entry`, `exports`)
fn source_file(dir: &Path, rel: &str, at: &str, findings: &mut Findings) {
  if !inside(dir, rel).is_some_and(|p| p.is_file()) {
    findings.errors.push(format!("{at} `{rel}` not found in the plugin"));
  } else if !crate::plugins::build::compiled_source(rel) {
    findings.errors.push(format!("{at} `{rel}` must be a .ts, .tsx, .js, .jsx or .mjs module"));
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

/// A stylesheet of the plugin, within `CSS_MAX_BYTES`
fn css_file(dir: &Path, rel: &str, what: &str, findings: &mut Findings) {
  match inside(dir, rel).and_then(|p| p.metadata().ok()).filter(|m| m.is_file()) {
    None => findings.errors.push(format!("{what} `{rel}` not found in the plugin")),
    Some(m) if m.len() > CSS_MAX_BYTES => findings.errors.push(format!("{what} `{rel}` is over {} KB", CSS_MAX_BYTES / 1024)),
    Some(_) => {}
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
    for needle in ["id `Bad_Id`", "folder's name", "requires `nope`", "tool `Go`: the name", "declared twice", "a description is needed", "type object", "`../escape.sh`", "needs a `command`", "`--ctl-w`", "`--bg-0`", "missing.css", "Acpira's own", "unknown key `extra`", "unknown contribution `widgets`"] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
  }

  #[test]
  fn a_skin_names_its_look_and_a_piece_on_the_composer_its_box() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cat.html"), "<p>cat</p>").unwrap();
    let skin = |theme: Value, ui: Value| {
      json!({ "schema": 1, "id": "sea", "version": "1", "description": "x", "contributes": { "theme": theme, "ui": ui } })
    };
    let (m, mut f) = manifest(skin(json!({ "base": "dim", "glass": "glass", "backdrop": "gone.html", "tokens": { "--bg-0": "#000" } }),
      json!([{ "id": "cat", "slot": "composer.above", "title": "Cat", "html": "cat.html" }, { "id": "big", "slot": "page", "title": "Big", "html": "cat.html", "width": 900 }])));
    check(&m, "sea", dir.path(), &|_| false, &mut f);
    let text = f.text();
    for needle in ["base `dim`", "glass `glass`", "`gone.html` not found", "`composer.above` needs `width` and `height`", "width must be 1 to 480"] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
    // A backdrop without a base, and glass without a backdrop, pass with a note
    std::fs::write(dir.path().join("sea.html"), "<p>sea</p>").unwrap();
    let (m, mut f) = manifest(skin(json!({ "backdrop": "sea.html", "tokens": { "--bg-0": "#000" } }), json!([])));
    check(&m, "sea", dir.path(), &|_| false, &mut f);
    assert!(f.ok() && f.notes.iter().any(|n| n.contains("without `base`")), "{f:?}");
    let (m, mut f) = manifest(skin(json!({ "glass": "frost", "tokens": { "--bg-0": "#000" } }), json!([])));
    check(&m, "sea", dir.path(), &|_| false, &mut f);
    assert!(f.ok() && f.notes.iter().any(|n| n.contains("only applies with a `backdrop`")), "{f:?}");
    // A scene is drawn by Acpira's renderer; a plugin gives one or its own document, not both
    std::fs::write(dir.path().join("sea.js"), "const SCENE = {};").unwrap();
    let (m, mut f) = manifest(skin(json!({ "base": "dark", "scene": "sea.js", "tokens": { "--bg-0": "#000" } }), json!([])));
    check(&m, "sea", dir.path(), &|_| false, &mut f);
    assert!(f.ok() && f.notes.is_empty(), "{f:?}");
    let (m, mut f) = manifest(skin(json!({ "base": "dark", "scene": "sea.js", "backdrop": "sea.html" }), json!([])));
    check(&m, "sea", dir.path(), &|_| false, &mut f);
    assert!(f.text().contains("exclude each other"), "{f:?}");
  }

  #[test]
  fn each_slot_asks_for_the_sides_it_does_not_size_and_a_message_action_is_a_button() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("p.html"), "<p>p</p>").unwrap();
    std::fs::write(dir.path().join("run.mjs"), "").unwrap();
    let ui = |ui: Value| {
      json!({ "schema": 1, "id": "kit", "version": "1", "description": "x",
        "contributes": { "ui": ui, "runtime": { "command": ["node", "run.mjs"], "methods": ["summarize"] } } })
    };
    let piece = |slot: &str, extra: Value| {
      let mut v = json!({ "id": slot.replace('.', "-"), "slot": slot, "title": "T", "html": "p.html" });
      v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
      v
    };
    let sized = json!([
      piece("composer.toolbar", json!({ "width": 80, "height": 24 })), piece("statusbar", json!({ "width": 60, "height": 20 })),
      piece("session.panel", json!({ "width": 40 })), piece("sidebar", json!({ "height": 120 })), piece("start", json!({ "height": 90 })),
      piece("panel.tab", json!({})), piece("page", json!({})),
      { "id": "sum", "slot": "message.actions", "title": "Summarize", "method": "summarize" },
    ]);
    let (m, mut f) = manifest(ui(sized));
    check(&m, "kit", dir.path(), &|p| p == "node", &mut f);
    assert!(f.ok(), "{f:?}");
    let bare = json!([
      piece("statusbar", json!({ "width": 60 })), piece("session.panel", json!({})), piece("sidebar", json!({})),
      { "id": "a", "slot": "message.actions", "title": "A", "method": "nope" },
      { "id": "b", "slot": "message.actions", "title": "", "html": "p.html" },
      piece("start", json!({ "height": 90, "method": "summarize" })),
    ]);
    let (m, mut f) = manifest(ui(bare));
    check(&m, "kit", dir.path(), &|p| p == "node", &mut f);
    let text = f.text();
    for needle in [
      "`statusbar` needs `height`", "`session.panel` needs `width`", "`sidebar` needs `height`", "method `nope` is not one of",
      "names the runtime `method`", "not a document", "needs a `title`", "only a `message.actions` piece calls",
    ] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
  }

  #[test]
  fn commands_need_a_prompt_or_the_command_hook_and_skills_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("run.mjs"), "").unwrap();
    let base = |commands: Value, runtime: Value, skills: &str| {
      json!({ "schema": 1, "id": "cmds", "version": "0.1.0", "description": "x", "contributes": { "commands": commands, "runtime": runtime, "skills": skills } })
    };
    let ok = base(json!([{ "name": "review", "prompt": "Review $ARGUMENTS", "button": "Review" }, { "name": "ping-me", "button": "Polish", "draft": true, "icon": "wand", "look": "icon" }]), json!({ "command": ["node", "run.mjs"], "hooks": ["command"] }), "skills");
    let (m, mut f) = manifest(ok);
    check(&m, "cmds", dir.path(), &|p| p == "node", &mut f);
    assert!(f.ok(), "{f:?}");
    assert_eq!(m.contributes.kinds(), ["runtime", "skills", "commands"]);
    let bad = base(json!([{ "name": "Review" }, { "name": "x", "prompt": "a" }, { "name": "x", "prompt": "b" }, { "name": "y", "prompt": "a", "button": " " }, { "name": "z", "prompt": "a", "draft": true }, { "name": "w", "prompt": "a", "button": "W", "icon": "unicorn", "look": "neon" }]), json!({ "command": ["node", "run.mjs"], "events": ["turn.*"] }), "nope");
    let (m, mut f) = manifest(bad);
    check(&m, "cmds", dir.path(), &|p| p == "node", &mut f);
    let text = f.text();
    for needle in ["command `/Review`: the name", "needs a `prompt`", "`/x` is declared twice", "skills folder `nope`", "`/y`: `button` is the label", "`/z`: `draft`, `icon`, `look` and `input` need a `button`", "icon `unicorn`", "look `neon`"] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
  }

  #[test]
  fn providers_need_a_usable_source_and_never_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let base = |providers: Value| json!({ "schema": 1, "id": "gw", "version": "0.1.0", "description": "x", "contributes": { "providers": providers } });
    let good = json!({ "id": "gw-claude", "format": "anthropic", "baseUrl": "http://gw:8080", "keyEnv": "GW_KEY", "models": [{ "id": "claude-x" }] });
    let (m, mut f) = manifest(base(json!([good.clone()])));
    check(&m, "gw", dir.path(), &|_| false, &mut f);
    assert!(f.ok(), "{f:?}");
    assert_eq!((m.contributes.kinds(), m.layer()), (vec!["providers"], "agents"));
    let p = provider_of(&good, "gw").unwrap();
    assert_eq!((p.name.as_str(), p.extra["plugin"].as_str(), p.extra["keyEnv"].as_str()), ("gw-claude", Some("gw"), Some("GW_KEY")));
    let bad = base(json!([
      good,
      { "id": "gw-claude", "format": "anthropic", "baseUrl": "http://x", "models": [{ "id": "m" }] },
      { "id": "a/b", "baseUrl": "http://x", "models": [{ "id": "m" }] },
      { "id": "k", "baseUrl": "http://x", "apiKey": "sk-1", "models": [{ "id": "m" }] },
      { "id": "f", "format": "gemini", "baseUrl": "http://x", "models": [{ "id": "m" }] },
      { "id": "u", "models": [{ "id": "m" }] },
      { "id": "n", "baseUrl": "http://x", "models": [] },
      { "id": "e", "baseUrl": "http://x", "keyEnv": "lower", "models": [{ "id": "m" }] },
      { "id": "h", "baseUrl": "http://x", "headers": { "Authorization": "Bearer sk" }, "models": [{ "id": "m" }] },
    ]));
    let (m, mut f) = manifest(bad);
    check(&m, "gw", dir.path(), &|_| false, &mut f);
    let text = f.text();
    for needle in ["`gw-claude` is declared twice", "`a/b`: the id", "`apiKey` is not allowed", "format `gemini`", "`baseUrl` is needed", "at least one model", "environment variable name", "header `Authorization` is a credential"] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
  }

  #[test]
  fn interface_pieces_need_a_known_slot_and_a_small_html_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("panel.html"), "<p>hi</p>").unwrap();
    std::fs::write(dir.path().join("big.html"), "x".repeat(UI_HTML_MAX_BYTES as usize + 1)).unwrap();
    let base = |ui: Value| json!({ "schema": 1, "id": "look", "version": "0.1.0", "description": "x", "contributes": { "ui": ui } });
    let (m, mut f) = manifest(base(json!([{ "id": "rates", "slot": "panel.tab", "title": "Rates", "html": "panel.html" }])));
    check(&m, "look", dir.path(), &|_| false, &mut f);
    assert!(f.ok(), "{f:?}");
    assert_eq!((m.contributes.kinds(), m.layer()), (vec!["ui"], "interface"));
    let (m, mut f) = manifest(base(json!([
      { "id": "a", "slot": "toolbar", "html": "panel.html" },
      { "id": "a", "slot": "page", "html": "panel.html" },
      { "id": "b", "slot": "page", "html": "../escape.html" },
      { "id": "c", "slot": "page", "html": "big.html" },
    ])));
    check(&m, "look", dir.path(), &|_| false, &mut f);
    let text = f.text();
    for needle in ["slot `toolbar` is not one of", "ui `a` is declared twice", "`../escape.html` not found", "`big.html` is over 256 KB"] {
      assert!(text.contains(needle), "{needle} missing in\n{text}");
    }
  }

  #[test]
  fn an_unknown_schema_is_refused_whole() {
    assert!(parse(br#"{"schema":2,"id":"x"}"#).unwrap_err().contains("schema 2"));
    assert!(parse(b"{").unwrap_err().contains("invalid JSON"));
  }

  #[test]
  fn ranges_read_as_npm_reads_them() {
    let meets = |v: &str, r: &str| version_meets(v, r);
    // A bare partial version pins what it names, never a caret
    assert!(meets("1.2.9", "1.2") && !meets("1.3.0", "1.2") && meets("1.9.0", "1") && !meets("2.0.0", "1"));
    assert!(meets("1.2.3", "1.2.3") && !meets("1.2.4", "1.2.3"));
    // Space-separated comparators, an operator apart from its version, `x` wildcards
    assert!(meets("1.5.0", ">=1.2 <2") && !meets("2.0.0", ">=1.2 <2") && meets("1.5.0", ">= 1.2, < 2"));
    assert!(meets("0.3.4", "0.3.x") && meets("0.3.4", "^0.3") && !meets("0.4.0", "~0.3.1"));
    // Free-form versions in a manifest
    assert!(meets("1", "^1") && meets("v1.2", "~1.2") && !meets("soon", "*") && meets("1.0.0", ""));
    assert!(version_req("1 || 2").is_err() && version_req("1 - 2").is_err() && version_req(">=").is_err());
  }

  #[test]
  fn ids_and_tool_names() {
    assert!(valid_id("currency") && valid_id("a-1") && !valid_id("a_b") && !valid_id("-a") && !valid_id(""));
    assert!(valid_tool_name("convert_rate") && !valid_tool_name("1x") && !valid_tool_name("a-b"));
    assert!(inside(Path::new("/p"), "a/b.css").is_some() && inside(Path::new("/p"), "../x").is_none() && inside(Path::new("/p"), "/etc/x").is_none());
  }
}
