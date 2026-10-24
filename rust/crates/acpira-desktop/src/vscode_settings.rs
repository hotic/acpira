//! The acpira.* settings the extension left in VS Code's user settings.json (or Cursor's): while no engine has created the
//! shared <ACPIRA_HOME>/settings.json yet, the app's hello carries their shared keys so the first engine seeds the file
//! from them. settings.json is JSONC: comments and trailing commas are allowed

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Not taken over: the column settings are the desktop app's own (always on the left, over every project), and `agents`
/// is the legacy per-machine agent list, which the engine seeds into ~/.acpira/agents.json once. Settings Sync carries
/// it between machines, so the Mac's settings.json can hold another OS's commands (Windows paths, seen 2026-10-23)
const NOT_IMPORTED: [&str; 3] = ["sessionListPosition", "sessionScope", "agents"];

/// The user settings files the extension's hosts write, in the order they are tried. `platform` is Node's naming
pub fn ide_settings_paths(platform: &str, env: &dyn Fn(&str) -> Option<String>, home: &Path) -> Vec<PathBuf> {
  let base = match platform {
    "darwin" => home.join("Library").join("Application Support"),
    "win32" => env("APPDATA").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming")),
    _ => env("XDG_CONFIG_HOME").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".config")),
  };
  ["Code", "Cursor"].iter().map(|app| base.join(app).join("User").join("settings.json")).collect()
}

/// JSONC to JSON: comments become whitespace and a comma before a closing bracket goes; strings are copied untouched
pub fn strip_jsonc(text: &str) -> String {
  let s: Vec<char> = text.strip_prefix('\u{feff}').unwrap_or(text).chars().collect();
  let n = s.len();
  let starts = |j: usize, p: &str| p.chars().enumerate().all(|(k, c)| s.get(j + k) == Some(&c));
  // The next index from j past whitespace and comments
  let skip = |from: usize| -> usize {
    let mut j = from;
    loop {
      while j < n && s[j].is_whitespace() {
        j += 1;
      }
      if starts(j, "//") {
        j = (j..n).find(|&k| s[k] == '\n').map_or(n, |e| e + 1);
        continue;
      }
      if starts(j, "/*") {
        j = (j + 2..n.saturating_sub(1)).find(|&k| s[k] == '*' && s[k + 1] == '/').map_or(n, |e| e + 2);
        continue;
      }
      return j;
    }
  };
  let mut out = String::with_capacity(text.len());
  let mut i = 0;
  while i < n {
    let c = s[i];
    if c == '"' {
      let mut j = i + 1;
      while j < n && s[j] != '"' {
        j += if s[j] == '\\' { 2 } else { 1 };
      }
      out.extend(&s[i..(j + 1).min(n)]);
      i = j + 1;
    } else if c == '/' && matches!(s.get(i + 1), Some('/' | '*')) {
      out.push(' ');
      i = skip(i);
    } else if c == ',' && matches!(s.get(skip(i + 1)), Some('}' | ']')) {
      i += 1;
    } else {
      out.push(c);
      i += 1;
    }
  }
  out
}

/// The acpira.* keys of a parsed settings file, without the prefix, minus the ones not taken over
pub fn acpira_keys(settings: &Value) -> Map<String, Value> {
  let mut out = Map::new();
  let Some(obj) = settings.as_object() else { return out };
  for (key, value) in obj {
    let Some(name) = key.strip_prefix("acpira.") else { continue };
    if !name.is_empty() && !NOT_IMPORTED.contains(&name) {
      out.insert(name.to_owned(), value.clone());
    }
  }
  out
}

/// The first settings file that exists and parses, as the keys to import and where they came from
pub fn read_ide_settings(paths: &[PathBuf], log: &dyn Fn(&str)) -> Option<(PathBuf, Map<String, Value>)> {
  for path in paths {
    let Ok(text) = std::fs::read_to_string(path) else { continue };
    match serde_json::from_str::<Value>(&strip_jsonc(&text)) {
      Ok(v) => return Some((path.clone(), acpira_keys(&v))),
      Err(e) => log(&format!("settings import: {} does not parse ({e})", path.display())),
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  // Ported from the Electron app's vscodeSettings.test.ts
  #[test]
  fn reads_jsonc_comments_and_trailing_commas_go_strings_stay_as_written() {
    let text = "\u{feff}{
      // a comment
      \"acpira.defaultAgent\": \"claude\", /* inline */
      \"acpira.note\": \"keep // this, and /* this */ too\",
      \"acpira.escaped\": \"a \\\"quoted\\\" word,\",
      \"acpira.list\": [1, 2,],
      \"editor.fontSize\": 13,
    }";
    assert_eq!(
      serde_json::from_str::<Value>(&strip_jsonc(text)).unwrap(),
      json!({
        "acpira.defaultAgent": "claude",
        "acpira.note": "keep // this, and /* this */ too",
        "acpira.escaped": "a \"quoted\" word,",
        "acpira.list": [1, 2],
        "editor.fontSize": 13,
      })
    );
  }

  #[test]
  fn takes_the_acpira_keys_without_the_prefix_minus_the_column_settings_and_the_agent_list() {
    let v = json!({ "acpira.defaultAgent": "kimi", "acpira.appearance.font": "inter", "acpira.sessionScope": "workspace", "acpira.sessionListPosition": "right",
      "acpira.agents": { "codex": { "command": "C:\\codex.cmd" } }, "acpira.": 1, "editor.tabSize": 2 });
    assert_eq!(Value::Object(acpira_keys(&v)), json!({ "defaultAgent": "kimi", "appearance.font": "inter" }));
    assert!(acpira_keys(&json!([])).is_empty());
    assert!(acpira_keys(&Value::Null).is_empty());
  }

  #[test]
  fn looks_where_each_os_keeps_the_user_settings_vscode_before_cursor() {
    let none = |_: &str| None;
    assert_eq!(
      ide_settings_paths("darwin", &none, Path::new("/Users/u")),
      [
        PathBuf::from("/Users/u/Library/Application Support/Code/User/settings.json"),
        PathBuf::from("/Users/u/Library/Application Support/Cursor/User/settings.json"),
      ]
    );
    let xdg = |k: &str| (k == "XDG_CONFIG_HOME").then(|| "/cfg".to_owned());
    assert_eq!(ide_settings_paths("linux", &xdg, Path::new("/home/u"))[0], PathBuf::from("/cfg/Code/User/settings.json"));
    assert_eq!(ide_settings_paths("linux", &none, Path::new("/home/u"))[0], PathBuf::from("/home/u/.config/Code/User/settings.json"));
    let appdata = |k: &str| (k == "APPDATA").then(|| "C:\\AppData".to_owned());
    assert_eq!(
      ide_settings_paths("win32", &appdata, Path::new("C:\\Users\\u"))[0],
      PathBuf::from("C:\\AppData").join("Code").join("User").join("settings.json")
    );
  }
}
