//! `plugin.json`, schema 1 (`docs/dev/plugins.md`): what a plugin contributes, and the static check that reads the
//! manifest and the frozen files alone, without running any of the plugin's code

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod validate;

pub use self::validate::{check, inside, parse, program_in, read, safe_css_value, version_meets, version_req};

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
  "transcript",
  "context",
  "model.request",
  "compaction",
  "shell",
  "tools",
  "websearch",
  "command",
  "menu",
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
/// (`acpira_shared::plugins::UI_SLOTS`); a piece of a `BUTTON_SLOTS` slot is a native button calling `method` instead
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
  /// A button slot (`message.actions`, `prompt.actions`, `selection.actions`): the runtime method a click calls with `{ session, turn, text }`
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
  /// The button opens a menu of these instead of running the command, like the mode chip: picks are kept per session
  /// by the engine and handed to the plugin's hooks as `menus`; the chip shows the first group's pick
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub menu: Vec<MenuItemDef>,
}

/// One row of a command button's menu (`MENU_KINDS`): `radio` is one choice of its `group`, `toggle` a switch of its
/// own, `action` calls the runtime's `menu` hook and keeps nothing, `separator` a rule
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemDef {
  #[serde(default)]
  pub id: String,
  pub kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub group: Option<String>,
  #[serde(default)]
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  /// The group's choice, or a toggle that starts on, until the user picks otherwise
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub default: bool,
}

pub const MENU_KINDS: &[&str] = &["radio", "toggle", "action", "separator"];
/// Most rows a menu may have, separators included
pub const MENU_MAX_ITEMS: usize = 16;
/// Longest row label and description, in characters
pub const MENU_LABEL_MAX_CHARS: usize = 24;
pub const MENU_DESCRIPTION_MAX_CHARS: usize = 48;

impl CommandDef {
  /// `input` as it applies (see there)
  pub fn takes_draft(&self) -> bool {
    match self.input.as_deref() {
      Some(i) => i == "draft",
      None => self.draft || self.prompt.as_deref().is_some_and(|p| p.contains("$ARGUMENTS")),
    }
  }

  /// The menu's state in a session: each group's choice (by its name) and each toggle (by its id), from what the user
  /// picked there where the manifest still has it, else the defaults. A group without a default starts at its first
  pub fn menu_picks(&self, stored: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut out = Map::new();
    for item in &self.menu {
      match (item.kind.as_str(), item.group.as_deref()) {
        ("radio", Some(group)) => {
          let kept = stored.and_then(|s| s.get(group)).and_then(Value::as_str).filter(|id| self.menu.iter().any(|i| i.kind == "radio" && i.group.as_deref() == Some(group) && i.id == *id));
          match kept {
            Some(id) => {
              out.insert(group.to_owned(), Value::from(id));
            }
            None if item.default || !out.contains_key(group) => {
              out.insert(group.to_owned(), Value::from(item.id.as_str()));
            }
            None => {}
          }
        }
        ("toggle", _) => {
          let on = stored.and_then(|s| s.get(&item.id)).and_then(Value::as_bool).unwrap_or(item.default);
          out.insert(item.id.clone(), Value::from(on));
        }
        _ => {}
      }
    }
    out
  }

  /// The picks after the user chose `id`: a radio becomes its group's choice, a toggle flips; None for an action or an
  /// id the menu lacks
  pub fn menu_pick(&self, current: &Map<String, Value>, id: &str) -> Option<Map<String, Value>> {
    let item = self.menu.iter().find(|i| i.id == id && i.kind != "separator")?;
    let mut next = current.clone();
    match (item.kind.as_str(), item.group.as_deref()) {
      ("radio", Some(group)) => next.insert(group.to_owned(), Value::from(id)),
      ("toggle", _) => next.insert(id.to_owned(), Value::from(!current.get(id).and_then(Value::as_bool).unwrap_or(item.default))),
      _ => return None,
    };
    Some(next)
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
  use acpira_shared::providers::{INPUT_FIELD, KEY_ENV_FIELD, PLUGIN_FIELD, Provider};
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
  // A model entry that omits `input` leaves image support to the catalogue, while an explicit `input` list stays as declared
  if let Some(raw_models) = raw.get("models").and_then(Value::as_array) {
    for (m, raw_m) in p.models.iter_mut().zip(raw_models) {
      if raw_m.get("input").is_none() && !m.input_guessed() {
        m.estimated.push(INPUT_FIELD.to_owned());
      }
    }
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

#[cfg(test)]
mod tests;
