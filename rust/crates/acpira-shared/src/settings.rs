//! Settings view and sanitation (mirror of packages/shared/settings.ts)

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::{LANGUAGE_TAGS, Language, Locale};
use crate::transcript::AgentId;

pub type HiddenMap = BTreeMap<AgentId, BTreeMap<String, Vec<String>>>;
pub type KeybindingMap = BTreeMap<String, String>;
/// Explicit per-agent overrides for running an agent on this machine in an SSH host folder (`true` = run here, `false` =
/// run on the host; absent = automatic: on for installed external ACPs, off for the built-in agent and uninstalled CLIs)
pub type SshLocalMap = BTreeMap<AgentId, bool>;

pub const UI_FONT_SIZE: (i64, i64, i64) = (10, 20, 13);
pub const CODE_FONT_SIZE: (i64, i64, i64) = (9, 20, 12);
pub const MIN_COMPACT_AT_TOKENS: i64 = 10_000;
/// `compactAtTokens` value meaning "the model's own window": each session compacts near the context size its agent reports
pub const COMPACT_AT_MODEL: i64 = 0;
/// Share of all logical cores every agent of an engine may use together, in percent; 100 lifts the cap (Windows only)
pub const AGENT_CPU_CAP: (i64, i64, i64) = (10, 100, 80);
/// The `proxy` setting's default: the local 127.0.0.1:7890 proxy while it listens, the inherited environment otherwise
pub const DEFAULT_PROXY: &str = "auto";
/// How the desktop app opens file links and plan documents: the system's default application or an editor's command line
pub const FILE_OPENERS: [&str; 7] = ["system", "code", "cursor", "windsurf", "zed", "subl", "idea"];
pub const DEFAULT_FILE_OPENER: &str = "system";
/// What the desktop app's window centres on: the conversation, or the files and pages of the session's project
pub const DESKTOP_LAYOUTS: [&str; 2] = ["chat", "files"];
pub const DEFAULT_DESKTOP_LAYOUT: &str = "chat";
/// How the desktop tool panel scopes its open files, browser tabs and terminals
pub const PANEL_SCOPES: [&str; 3] = ["project", "session", "global"];
pub const DEFAULT_PANEL_SCOPE: &str = "project";

pub const SETTING_KEYS: [&str; 31] = [
  "language",
  "defaultAgent",
  "agentOrder",
  "disabledAgents",
  "sessionScope",
  "sessionListPosition",
  "autoCompact",
  "compactAtTokens",
  "hiddenOptions",
  "accountSwitch",
  "theme",
  "uiFontSize",
  "codeFontSize",
  "diffMarkers",
  "fontSmoothing",
  "shareEditorSelection",
  "steerQueued",
  "notifySound",
  "notifySystem",
  "planAutoApprove",
  "sshLocalAgents",
  "subagents",
  "agentCpuCap",
  "proxy",
  "openFilesWith",
  "desktopLayout",
  "panelScope",
  "panelFiles",
  "panelTerminal",
  "panelBrowser",
  "keybindings",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
  pub language: Language,
  pub locale: Locale,
  pub default_agent: AgentId,
  pub agent_order: Vec<AgentId>,
  pub disabled_agents: Vec<AgentId>,
  pub session_scope: String,
  pub session_list_position: String,
  pub auto_compact: bool,
  pub compact_at_tokens: i64,
  pub hidden_options: HiddenMap,
  /// Strategy of the automatic account switch, shared by every agent (`off` by default)
  pub account_switch: String,
  pub theme: String,
  pub ui_font_size: i64,
  pub code_font_size: i64,
  pub diff_markers: String,
  pub font_smoothing: bool,
  pub share_editor_selection: bool,
  /// A queued prompt's send button steers it into the running turn on agents that support `_session/steering`
  pub steer_queued: bool,
  /// A chime when a session needs the user or finishes while they are not looking (on by default)
  #[serde(default = "default_notify_sound")]
  pub notify_sound: bool,
  /// A system notification for the same moments (off by default)
  #[serde(default)]
  pub notify_system: bool,
  /// Agents whose plan mode answers tool permission requests itself (plan approvals still ask)
  #[serde(default)]
  pub plan_auto_approve: Vec<AgentId>,
  /// Explicit per-agent overrides for running an agent on this machine in an SSH host folder (`true` = here, `false` =
  /// host; absent = automatic: on for installed external ACPs whose definition supports local run, off for the built-in
  /// agent and uninstalled CLIs). Empty by default
  #[serde(default, deserialize_with = "deserialize_ssh_local_agents")]
  pub ssh_local_agents: SshLocalMap,
  /// Cross-harness subagents (`~/.acpira/subagents.json`, not a host setting: every window and IDE shares the file)
  #[serde(default)]
  pub subagents: Vec<crate::subagents::SubagentPersona>,
  /// Groups of those personas, summoned together (the same file's `groups`)
  #[serde(default)]
  pub subagent_groups: Vec<crate::subagents::SubagentGroup>,
  /// CPU hard cap over every agent process and its descendants, percent of all cores (`AGENT_CPU_CAP`, Windows job objects)
  #[serde(default = "default_agent_cpu_cap")]
  pub agent_cpu_cap: i64,
  /// Network route of agents, installers and downloads: `auto`, `off` or a proxy URL (`proxy_setting`)
  #[serde(default = "default_proxy")]
  pub proxy: String,
  /// Desktop app: the system's default application or an editor for file links and plans (`FILE_OPENERS`)
  #[serde(default = "default_file_opener")]
  pub open_files_with: String,
  /// Desktop app: the window's layout (`DESKTOP_LAYOUTS`)
  #[serde(default = "default_desktop_layout")]
  pub desktop_layout: String,
  /// Desktop app: how the tool panel scopes its open files, browser tabs and terminals (`PANEL_SCOPES`)
  #[serde(default = "default_panel_scope")]
  pub panel_scope: String,
  /// Desktop app: whether the built-in File Tree, Terminal and Browser modules are enabled in the tool panel
  #[serde(default = "default_panel_files")]
  pub panel_files: bool,
  #[serde(default = "default_panel_files")]
  pub panel_terminal: bool,
  #[serde(default = "default_panel_files")]
  pub panel_browser: bool,
  /// User-customized keyboard shortcuts (missing: default, "": unbound)
  #[serde(default)]
  pub keybindings: KeybindingMap,
}

/// The `sshLocalAgents` setting: explicit `agent -> bool` overrides. A legacy opt-in array (`["acpira", "claude"]`)
/// preserves only an explicit built-in `acpira` opt-in; installed external ACPs now run here automatically by default
pub fn ssh_local_agents(value: &Value) -> SshLocalMap {
  let mut out = SshLocalMap::new();
  if let Some(obj) = value.as_object() {
    for (k, v) in obj {
      let id = k.trim();
      if !id.is_empty()
        && let Some(b) = v.as_bool()
      {
        out.insert(id.to_owned(), b);
      }
    }
  } else if let Some(arr) = value.as_array()
    && arr.iter().filter_map(Value::as_str).any(|s| s.trim() == "acpira")
  {
    out.insert("acpira".into(), true);
  }
  out
}

fn deserialize_ssh_local_agents<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SshLocalMap, D::Error> {
  Ok(ssh_local_agents(&Value::deserialize(d)?))
}

fn default_agent_cpu_cap() -> i64 {
  AGENT_CPU_CAP.2
}

fn default_proxy() -> String {
  DEFAULT_PROXY.into()
}

fn default_file_opener() -> String {
  DEFAULT_FILE_OPENER.into()
}

fn default_desktop_layout() -> String {
  DEFAULT_DESKTOP_LAYOUT.into()
}

fn default_panel_scope() -> String {
  DEFAULT_PANEL_SCOPE.into()
}

fn default_panel_files() -> bool {
  true
}

fn default_notify_sound() -> bool {
  true
}

/// A proxy URL with a supported scheme and a host, trailing slash dropped; a bare `host:port` reads as `http://host:port`
pub fn proxy_url(raw: &str) -> Option<String> {
  let v = raw.trim().trim_end_matches('/');
  let with_scheme = if v.contains("://") { v.to_owned() } else { format!("http://{v}") };
  let (scheme, rest) = with_scheme.split_once("://")?;
  let ok_scheme = matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "socks5" | "socks5h");
  // The authority must carry a host; credentials (`user:pass@`) are allowed, a path is not
  let authority = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
  let ok_host = !authority.is_empty() && !authority.starts_with(':') && !rest.contains('/') && !rest.contains(char::is_whitespace);
  (ok_scheme && ok_host).then_some(with_scheme)
}

/// The `proxy` setting: `auto`, `off` or a normalized proxy URL; anything else reads as `auto`
pub fn proxy_setting(value: &Value) -> String {
  let raw = value.as_str().unwrap_or("").trim();
  match raw.to_ascii_lowercase().as_str() {
    "" | "auto" => DEFAULT_PROXY.into(),
    "off" | "none" | "direct" => "off".into(),
    _ => proxy_url(raw).unwrap_or_else(|| DEFAULT_PROXY.into()),
  }
}

pub const ACCOUNT_SWITCH_STRATEGIES: [&str; 4] = ["off", "earliestReset", "mostRemaining", "listOrder"];
pub const DEFAULT_ACCOUNT_SWITCH: &str = "off";

pub fn is_setting_key(k: &str) -> bool {
  SETTING_KEYS.contains(&k)
}

/// The settings each shell keeps in its own store, since they follow the window's shape: the session list's side and
/// scope (the desktop app always lists every project on the left) and sharing the editor selection (IDE shells only).
/// Every other setting is one value for all shells, in `<ACPIRA_HOME>/settings.json` (2026-10-23), like the transcripts;
/// `subagents` has a file of its own and `agents` is the legacy per-shell agent list agents.json is seeded from
pub const SHELL_SETTING_KEYS: [&str; 3] = ["sessionScope", "sessionListPosition", "shareEditorSelection"];

/// Whether `k` lives in the shared settings file: a setting key outside the shell's own, or an appearance axis
pub fn is_shared_setting(k: &str) -> bool {
  (is_setting_key(k) && !SHELL_SETTING_KEYS.contains(&k) && k != "subagents") || k.starts_with("appearance.")
}

fn one_of(v: &Value, list: &[&str], fallback: &str) -> String {
  v.as_str().filter(|s| list.contains(s)).unwrap_or(fallback).to_owned()
}

fn clamp_size(v: &Value, (min, max, default): (i64, i64, i64)) -> i64 {
  match v.as_f64() {
    Some(n) if n.is_finite() => (js_round(n) as i64).clamp(min, max),
    _ => default,
  }
}

/// Math.round: halves go up
fn js_round(n: f64) -> f64 {
  (n + 0.5).floor()
}

/// Trimmed, non-empty, first occurrence wins; anything that is not an array reads as empty
pub fn id_list(v: &Value) -> Vec<String> {
  let mut out: Vec<String> = vec![];
  for s in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
    let s = s.trim();
    if !s.is_empty() && !out.iter().any(|x| x == s) {
      out.push(s.to_owned());
    }
  }
  out
}

pub fn is_hidden_map(v: &Value) -> bool {
  let Some(m) = v.as_object() else { return false };
  m.values().all(|families| {
    families.as_object().is_some_and(|f| f.values().all(|names| names.as_array().is_some_and(|a| a.iter().all(Value::is_string))))
  })
}

/// A sanitized value per key, as JSON (the webview receives it inside SettingsView)
pub fn sanitize_setting(key: &str, value: &Value) -> Value {
  match key {
    "language" => Value::from(one_of(value, &LANGUAGE_TAGS, "auto")),
    "defaultAgent" => Value::from(value.as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("acpira")),
    "autoCompact" | "panelFiles" | "panelTerminal" | "panelBrowser" => Value::from(value.as_bool().unwrap_or(true)),
    "fontSmoothing" => Value::from(value.as_bool().unwrap_or(false)),
    "shareEditorSelection" => Value::from(value.as_bool().unwrap_or(true)),
    "steerQueued" => Value::from(value.as_bool().unwrap_or(false)),
    "notifySound" => Value::from(value.as_bool().unwrap_or(true)),
    "notifySystem" => Value::from(value.as_bool().unwrap_or(false)),
    "compactAtTokens" => {
      let n = value.as_f64().filter(|n| n.is_finite()).map(|n| js_round(n) as i64).unwrap_or(300_000);
      Value::from(if n <= COMPACT_AT_MODEL { COMPACT_AT_MODEL } else { n.max(MIN_COMPACT_AT_TOKENS) })
    }
    "uiFontSize" => Value::from(clamp_size(value, UI_FONT_SIZE)),
    "codeFontSize" => Value::from(clamp_size(value, CODE_FONT_SIZE)),
    "agentCpuCap" => Value::from(clamp_size(value, AGENT_CPU_CAP)),
    "proxy" => Value::from(proxy_setting(value)),
    "theme" => Value::from(one_of(value, &["auto", "light", "dark"], "auto")),
    "diffMarkers" => Value::from(one_of(value, &["color", "signs"], "color")),
    "sessionScope" => Value::from(one_of(value, &["workspace", "all"], "workspace")),
    "sessionListPosition" => Value::from(one_of(value, &["hidden", "left", "right"], "hidden")),
    "hiddenOptions" => {
      if !is_hidden_map(value) {
        return Value::Object(Default::default());
      }
      // Built-in model visibility belongs to providers.json, not the legacy menu filter.
      let mut hidden = value.clone();
      if let Some(agents) = hidden.as_object_mut()
        && let Some(controls) = agents.get_mut("acpira").and_then(Value::as_object_mut)
      {
        controls.remove("model");
        if controls.is_empty() {
          agents.remove("acpira");
        }
      }
      hidden
    }
    "accountSwitch" => Value::from(one_of(value, &ACCOUNT_SWITCH_STRATEGIES, DEFAULT_ACCOUNT_SWITCH)),
    "openFilesWith" => Value::from(one_of(value, &FILE_OPENERS, DEFAULT_FILE_OPENER)),
    "desktopLayout" => Value::from(one_of(value, &DESKTOP_LAYOUTS, DEFAULT_DESKTOP_LAYOUT)),
    "panelScope" => Value::from(one_of(value, &PANEL_SCOPES, DEFAULT_PANEL_SCOPE)),
    "agentOrder" | "disabledAgents" | "planAutoApprove" => Value::from(id_list(value)),
    "sshLocalAgents" => serde_json::to_value(ssh_local_agents(value)).unwrap_or_else(|_| Value::Object(Default::default())),
    "subagents" => serde_json::to_value(crate::subagents::sanitize_personas(value)).unwrap_or(Value::Array(vec![])),
    "keybindings" => sanitize_keybindings(value),
    _ => Value::Null,
  }
}

const SHORTCUT_DEFAULTS: [(&str, &str); 20] = [
  ("openSettings", "CmdOrCtrl+,"),
  ("openShortcuts", "CmdOrCtrl+/"),
  ("openFolder", "CmdOrCtrl+O"),
  ("quickOpen", "CmdOrCtrl+P"),
  ("toggleSidebar", "CmdOrCtrl+B"),
  ("back", "CmdOrCtrl+["),
  ("forward", "CmdOrCtrl+]"),
  ("closeTab", "CmdOrCtrl+W"),
  ("focusComposer", "Ctrl+L"),
  ("cycleMode", "Shift+Tab"),
  ("openModelPicker", "CmdOrCtrl+Shift+M"),
  ("invertSteerSend", "CmdOrCtrl+Enter"),
  ("findInChat", "CmdOrCtrl+F"),
  ("findNext", "CmdOrCtrl+G"),
  ("findPrev", "CmdOrCtrl+Shift+G"),
  ("newSession", "CmdOrCtrl+N"),
  ("searchSessions", "CmdOrCtrl+K"),
  ("nextSession", "Ctrl+Tab"),
  ("prevSession", "Ctrl+Shift+Tab"),
  ("nextAttention", "Alt+CmdOrCtrl+A"),
];

const PANEL_SHORTCUT_DEFAULTS: [(&str, &str); 4] = [
  ("toggleToolPanel", "Alt+CmdOrCtrl+B"),
  ("toggleTerminal", "Ctrl+`"),
  ("openChanges", "CmdOrCtrl+Shift+D"),
  ("toggleBrowser", "CmdOrCtrl+Shift+B"),
];

pub fn normalize_accelerator(raw: &str) -> Option<String> {
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    return None;
  }
  let mut parts: Vec<&str> = trimmed.split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
  let key_raw = parts.pop()?;
  let key = if key_raw.len() == 1 {
    let ch = key_raw.chars().next()?;
    if ch.is_ascii_alphabetic() {
      ch.to_ascii_uppercase().to_string()
    } else if ch.is_ascii_digit() || matches!(ch, '`' | '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | ',' | '.' | '/') {
      ch.to_string()
    } else {
      return None;
    }
  } else {
    let upper = key_raw.to_ascii_uppercase();
    if upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n)) {
      upper
    } else {
      match key_raw.to_ascii_lowercase().as_str() {
        "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        "space" => "Space".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "escape" | "esc" => "Escape".into(),
        "up" | "arrowup" => "Up".into(),
        "down" | "arrowdown" => "Down".into(),
        "left" | "arrowleft" => "Left".into(),
        "right" | "arrowright" => "Right".into(),
        _ => return None,
      }
    }
  };
  let mut ctrl = false;
  let mut alt = false;
  let mut shift = false;
  let mut cmd_or_ctrl = false;
  for m in parts {
    match m.to_ascii_lowercase().as_str() {
      "ctrl" | "control" => ctrl = true,
      "alt" | "option" => alt = true,
      "shift" => shift = true,
      "cmdorctrl" | "commandorcontrol" | "cmd" | "command" | "meta" => cmd_or_ctrl = true,
      _ => return None,
    }
  }
  let is_fn = key.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n));
  let has_primary = cmd_or_ctrl || ctrl || alt;
  let allowed_shift_only = shift && (key == "Tab" || key == "Enter" || is_fn);
  if !has_primary && !allowed_shift_only && !is_fn {
    return None;
  }
  let mut out = vec![];
  if ctrl {
    out.push("Ctrl");
  }
  if alt {
    out.push("Alt");
  }
  if shift {
    out.push("Shift");
  }
  if cmd_or_ctrl {
    out.push("CmdOrCtrl");
  }
  out.push(&key);
  Some(out.join("+"))
}

pub fn sanitize_keybindings(value: &Value) -> Value {
  let Some(obj) = value.as_object() else { return Value::Object(Default::default()) };
  let mut out = serde_json::Map::new();
  for (id, raw) in obj {
    let Some(default) = SHORTCUT_DEFAULTS.iter().chain(PANEL_SHORTCUT_DEFAULTS.iter()).find(|(k, _)| k == id).map(|(_, d)| *d) else {
      continue;
    };
    let Some(s) = raw.as_str() else { continue };
    let trimmed = s.trim();
    if trimmed.is_empty() {
      out.insert(id.clone(), Value::String(String::new()));
    } else if let Some(norm) = normalize_accelerator(trimmed)
      && norm != default
    {
      out.insert(id.clone(), Value::String(norm));
    }
  }
  Value::Object(out)
}

/// Whether a session belongs to the workspace shown
pub fn in_workspace(cwd: &str, workspace: &str) -> bool {
  cwd == workspace
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn legacy_model_hiding_does_not_override_builtin_provider_switches() {
    use serde_json::json;
    let legacy = json!({ "acpira": { "model": ["[\"1\",\"Qwen3.8-Flash-Next\"]"], "effort": ["Low"] }, "codex": { "model": ["GPT"] } });
    assert_eq!(sanitize_setting("hiddenOptions", &legacy), json!({ "acpira": { "effort": ["Low"] }, "codex": { "model": ["GPT"] } }));
    assert!(legacy["acpira"]["model"].is_array());
    assert_eq!(sanitize_setting("hiddenOptions", &json!({ "acpira": { "model": ["Qwen"] } })), json!({}));
    assert_eq!(sanitize_setting("hiddenOptions", &json!({ "codex": { "model": [1] } })), json!({}));
  }

  #[test]
  fn the_agent_cpu_cap_is_a_whole_percentage_between_10_and_100_and_80_otherwise() {
    assert_eq!(sanitize_setting("agentCpuCap", &Value::from(60)), Value::from(60));
    assert_eq!(sanitize_setting("agentCpuCap", &Value::from(72.6)), Value::from(73));
    assert_eq!(sanitize_setting("agentCpuCap", &Value::from(0)), Value::from(10));
    assert_eq!(sanitize_setting("agentCpuCap", &Value::from(250)), Value::from(100));
    assert_eq!(sanitize_setting("agentCpuCap", &Value::from("50")), Value::from(80));
    assert!(is_setting_key("agentCpuCap"));
  }

  #[test]
  fn the_proxy_setting_is_auto_off_or_a_normalized_url() {
    let p = |v: Value| sanitize_setting("proxy", &v);
    assert_eq!(p(Value::Null), Value::from("auto"));
    assert_eq!(p(Value::from(" AUTO ")), Value::from("auto"));
    assert_eq!(p(Value::from("Direct")), Value::from("off"));
    assert_eq!(p(Value::from("http://127.0.0.1:7897/")), Value::from("http://127.0.0.1:7897"));
    assert_eq!(p(Value::from("127.0.0.1:7890")), Value::from("http://127.0.0.1:7890"));
    assert_eq!(p(Value::from("socks5h://u:p@proxy:1080")), Value::from("socks5h://u:p@proxy:1080"));
    for bad in ["ftp://x:21", "http://:8080", "http://host:1/path", "not a proxy"] {
      assert_eq!(p(Value::from(bad)), Value::from("auto"), "{bad}");
    }
    assert_eq!(p(Value::from(7890)), Value::from("auto"));
    assert!(is_setting_key("proxy"));
  }

  #[test]
  fn files_open_with_a_listed_editor_or_the_system_default() {
    assert_eq!(sanitize_setting("openFilesWith", &Value::from("zed")), Value::from("zed"));
    for bad in [Value::from("vim"), Value::from("Code"), Value::Null, Value::from(1)] {
      assert_eq!(sanitize_setting("openFilesWith", &bad), Value::from("system"), "{bad}");
    }
    assert!(is_setting_key("openFilesWith"));
  }

  #[test]
  fn the_desktop_layout_is_chat_or_files_and_shared() {
    assert_eq!(sanitize_setting("desktopLayout", &Value::from("files")), Value::from("files"));
    for bad in [Value::from("editor"), Value::from("Files"), Value::Null, Value::from(true)] {
      assert_eq!(sanitize_setting("desktopLayout", &bad), Value::from("chat"), "{bad}");
    }
    assert!(is_shared_setting("desktopLayout"));
    assert_eq!(sanitize_setting("panelFiles", &Value::from(false)), Value::from(false));
    assert_eq!(sanitize_setting("panelFiles", &Value::Null), Value::from(true));
    assert!(is_shared_setting("panelFiles"));
  }

  #[test]
  fn ssh_local_agents_accepts_override_map_and_migrates_legacy_array() {
    use serde_json::json;
    assert_eq!(sanitize_setting("sshLocalAgents", &Value::Null), json!({}));
    assert_eq!(
      sanitize_setting("sshLocalAgents", &json!({ " claude ": false, "acpira": true, "bad": "yes", "": true })),
      json!({ "acpira": true, "claude": false }),
    );
    // Legacy opt-in arrays: installed external ACPs default on now, so only an explicit built-in opt-in carries over
    assert_eq!(sanitize_setting("sshLocalAgents", &json!(["claude", "antigravity"])), json!({}));
    assert_eq!(sanitize_setting("sshLocalAgents", &json!(["acpira", "antigravity"])), json!({ "acpira": true }));
  }
}
