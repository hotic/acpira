//! Claude Code ultracode as an effort level past Max.
//!
//! Claude Code 2.1.224 defines it as a level (`/effort low|medium|high|xhigh|max|ultracode`, "ultracode: xhigh + dynamic
//! workflow orchestration"); CLI 2.1.284, the one claude-agent-acp 0.87.0 bundles, turned it into a Tab toggle beside the
//! effort slider that combines with any level (both read 2026-10-07 from the binaries' strings). claude-agent-acp
//! publishes no config option for it: its effort select stops at `max`. Acpira keeps the 2.1.224 presentation, the same
//! one Codex's real `ultra` reasoning_effort gets: the host appends an `ultra` option ("Ultra") to the effort select of
//! Claude sessions and shows it selected while ultracode is on. Picking it turns ultracode on with wire effort `xhigh` (the
//! highest level the model offers when it has no xhigh); picking any other level turns it off. `ultra` itself never goes
//! out as an effort value; ultracode is applied through the programmatic settings tier.
//!
//! Source read 2026-10-07, claude-agent-acp 0.87.0 (`dist/acp-agent.js`, `dist/session-effort.js`):
//! - `_meta.claudeCode.options.settings` (an object) becomes the session's settings (`configuredSettingsObject`). It
//!   REPLACES the `{ modelOverrides, availableModels }` the adapter derives from `CLAUDE_MODEL_CONFIG`, so a request that
//!   sends settings merges those two keys back in (`settings`).
//! - The object is kept as `session.effortSettingsOverride`; every effort apply goes through `effortFlagSettings`, which
//!   carries `ultracode: true` along when the settings ask for it (since CLI 2.1.284 an effort apply without the key turns
//!   ultracode off). The initial effort at session start is applied with the same settings.
//! - `settings` is in `OPTION_REBUILDS_SESSION` and part of `computeSessionFingerprint`: a session/resume or load of a
//!   live session whose fingerprint changed tears the query down and recreates it with `resume: sessionId` (same
//!   transcript and context, live permission mode kept). Switching ultracode mid-session is that resume on the same
//!   process; model, effort and Fast may come back as the adapter's defaults, so the host sets them again
//!   (`session/ultracode.rs`).
//! - Ultracode only works with dynamic workflows on, which the sidecar turns on by default (`claude_workflow`); an
//!   explicit `CLAUDE_CODE_WORKFLOWS=0` or `enableWorkflows: false` in a settings file hides the level
//!   (`workflows_enabled`).
//!
//! Not yet observed on the wire with a real prompt.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use acpira_shared::composer_controls::is_reasoning_control;
use acpira_shared::model_catalog::{Level, option_level};
use acpira_shared::transcript::{ConfigControl, SessionOption, StrMap};

use crate::acp::vendors::claude_workflow::WORKFLOWS_ENV;

/// The option id (and shown value) of the host's level: never a wire value of Claude's effort select
pub const LEVEL_ID: &str = "ultra";

/// Its name, the one Codex's real `ultra` level has, so the webview tints it the same way (`isUltraLevel`)
pub const LEVEL_NAME: &str = "Ultra";

/// claude-agent-acp's effort select id: where a remembered level is looked up before any control is known
pub const EFFORT_ID: &str = "effort";

/// The settings key Claude Code reads
pub const SETTINGS_KEY: &str = "ultracode";

/// The adapter's model-routing variable whose keys the settings object must carry along
pub const MODEL_CONFIG_ENV: &str = "CLAUDE_MODEL_CONFIG";

/// The settings key that turns dynamic workflows off whatever the environment says
const ENABLE_WORKFLOWS: &str = "enableWorkflows";

/// The wire effort Ultra stands for on this select: `xhigh`, else the highest level it offers; None when it offers no
/// level at all (then Ultra is not offered)
pub fn wire_target(control: &ConfigControl) -> Option<String> {
  let levels = control.options.iter().filter(|o| o.id != LEVEL_ID).filter_map(|o| option_level(o).filter(|l| *l > Level::Off).map(|l| (l, o)));
  let levels: Vec<(Level, &SessionOption)> = levels.collect();
  levels.iter().find(|(l, _)| *l == Level::XHigh).or_else(|| levels.iter().max_by_key(|(l, _)| *l)).map(|(_, o)| o.id.clone())
}

/// The effort select Ultra joins: the first reasoning select with a level to map it to (the closest outside sign of the
/// CLI's `ultracodeAvailable`, which wants a model that takes an effort level)
pub fn effort_index(options: &[ConfigControl]) -> Option<usize> {
  options.iter().position(|o| is_reasoning_control(o) && wire_target(o).is_some())
}

/// The shown effort is Ultra (a persisted record, a session about to be rebuilt)
pub fn shown_on(options: &[ConfigControl]) -> bool {
  options.iter().any(|o| is_reasoning_control(o) && o.value.as_deref() == Some(LEVEL_ID))
}

/// What remembered or per-turn settings ask for: Some(true) for Ultra, Some(false) for any other level, None when they
/// name no effort. The effort id comes from the controls when they are known
pub fn requested(config: &StrMap, options: &[ConfigControl]) -> Option<bool> {
  let id = effort_index(options).map(|i| options[i].id.as_str()).unwrap_or(EFFORT_ID);
  config.get(id).filter(|v| !v.is_empty()).map(|v| v == LEVEL_ID)
}

/// The value to send for `value` on control `id`: Ultra becomes its wire target, anything else goes as is. None when
/// `value` is Ultra and the select has nothing to map it to
pub fn wire_value(options: &[ConfigControl], id: &str, value: &str) -> Option<String> {
  if value != LEVEL_ID {
    return Some(value.to_owned());
  }
  options.iter().find(|o| o.id == id && is_reasoning_control(o)).and_then(wire_target)
}

/// Put Ultra on the effort select after the options changed and set what it shows. A select without the Ultra option
/// is fresh agent truth: its value is noted in `wire` first. `on` shows Ultra; otherwise `level` (a pick still to be
/// applied by a rebuild) or the agent's own value shows. Without `available`, or on any other reasoning select, an
/// earlier Ultra is taken away again
pub fn place(options: &mut [ConfigControl], available: bool, on: bool, level: Option<&str>, wire: &mut Option<String>) {
  let at = if available { effort_index(options) } else { None };
  for (i, control) in options.iter_mut().enumerate() {
    if !is_reasoning_control(control) {
      continue;
    }
    let ours = control.options.iter().any(|o| o.id == LEVEL_ID);
    if Some(i) != at {
      if ours {
        control.options.retain(|o| o.id != LEVEL_ID);
        if control.value.as_deref() == Some(LEVEL_ID) {
          control.value = wire.clone();
        }
      }
      continue;
    }
    if !ours {
      *wire = control.value.clone();
      control.options.push(SessionOption { id: LEVEL_ID.into(), name: LEVEL_NAME.into(), ..Default::default() });
    }
    let picked = level.filter(|l| *l != LEVEL_ID && control.options.iter().any(|o| o.id == *l));
    if on {
      control.value = Some(LEVEL_ID.into());
    } else if let Some(l) = picked {
      control.value = Some(l.to_owned());
    } else if wire.is_some() || control.value.as_deref() == Some(LEVEL_ID) {
      control.value = wire.clone();
    }
  }
}

/// The settings object: `ultracode: true` plus the `CLAUDE_MODEL_CONFIG` keys the adapter would otherwise have used
/// (a malformed variable adds nothing; the adapter reports it itself)
pub fn settings(model_config: Option<&str>) -> Value {
  let mut out = Map::new();
  out.insert(SETTINGS_KEY.into(), Value::Bool(true));
  if let Some(Value::Object(parsed)) = model_config.and_then(|raw| serde_json::from_str::<Value>(raw).ok()) {
    for key in ["modelOverrides", "availableModels"] {
      if let Some(v) = parsed.get(key).filter(|v| !v.is_null()) {
        out.insert(key.into(), v.clone());
      }
    }
  }
  Value::Object(out)
}

/// Ask for ultracode in a session/new, resume or load request, merging into whatever `_meta` already holds
pub fn with_ultracode(mut req: Value, model_config: Option<&str>) -> Value {
  req["_meta"]["claudeCode"]["options"]["settings"] = settings(model_config);
  req
}

/// The CLI's settings files that can carry `enableWorkflows`, lowest precedence first: user, project, local
pub fn settings_files(config_dir: &Path, cwd: &Path) -> Vec<PathBuf> {
  vec![config_dir.join("settings.json"), cwd.join(".claude").join("settings.json"), cwd.join(".claude").join("settings.local.json")]
}

/// Dynamic workflows are on: the sidecar defaults `CLAUDE_CODE_WORKFLOWS` to 1, an explicit false-ish value turns them
/// off, and the highest-precedence settings file that names `enableWorkflows` wins over either
pub fn workflows_enabled(env_value: Option<&str>, settings: &[Value]) -> bool {
  if let Some(explicit) = settings.iter().rev().find_map(|s| s.get(ENABLE_WORKFLOWS).and_then(Value::as_bool)) {
    return explicit;
  }
  !env_value.map(|v| v.trim().to_ascii_lowercase()).is_some_and(|v| matches!(v.as_str(), "0" | "false" | "no" | "off"))
}

/// `workflows_enabled` for an agent: the definition's env first, then the sidecar's own (the child sees them in that
/// order of precedence), and the settings files under its `CLAUDE_CONFIG_DIR` (or `~/.claude`) and `cwd`
pub fn workflows_enabled_for(def_env: Option<&acpira_shared::transcript::StrMap>, cwd: &str) -> bool {
  let var = |key: &str| def_env.and_then(|e| e.get(key).cloned()).or_else(|| std::env::var(key).ok());
  let config_dir =
    var("CLAUDE_CONFIG_DIR").filter(|d| !d.trim().is_empty()).map(PathBuf::from).unwrap_or_else(|| crate::store::data_dir::home_dir().join(".claude"));
  let files: Vec<Value> = settings_files(&config_dir, Path::new(cwd))
    .iter()
    .filter_map(|p| std::fs::read_to_string(p).ok())
    .filter_map(|text| serde_json::from_str(&text).ok())
    .collect();
  workflows_enabled(var(WORKFLOWS_ENV).as_deref(), &files)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn effort(levels: &[&str], value: &str) -> ConfigControl {
    ConfigControl {
      id: "effort".into(),
      name: "Effort".into(),
      category: Some("thought_level".into()),
      kind: None,
      options: levels.iter().map(|l| SessionOption { id: (*l).into(), name: (*l).into(), ..Default::default() }).collect(),
      value: Some(value.into()),
    }
  }

  fn named(id: &str, category: Option<&str>) -> ConfigControl {
    ConfigControl { id: id.into(), name: id.into(), category: category.map(str::to_owned), kind: None, options: vec![], value: None }
  }

  fn ids(c: &ConfigControl) -> Vec<&str> {
    c.options.iter().map(|o| o.id.as_str()).collect()
  }

  #[test]
  fn ultra_maps_to_xhigh_else_the_highest_level() {
    assert_eq!(wire_target(&effort(&["low", "high", "xhigh", "max"], "low")).as_deref(), Some("xhigh"));
    assert_eq!(wire_target(&effort(&["low", "medium", "high", "max"], "low")).as_deref(), Some("max"));
    assert_eq!(wire_target(&effort(&["high", "low"], "low")).as_deref(), Some("high"));
    // Nothing on the scale (or only off): no Ultra
    assert_eq!(wire_target(&effort(&["on", "off"], "on")), None);
    assert_eq!(wire_target(&effort(&[], "")), None);
    assert_eq!(wire_value(&[effort(&["low", "xhigh"], "low")], "effort", LEVEL_ID).as_deref(), Some("xhigh"));
    assert_eq!(wire_value(&[effort(&["low", "xhigh"], "low")], "effort", "low").as_deref(), Some("low"));
    assert_eq!(wire_value(&[effort(&["on"], "on")], "effort", LEVEL_ID), None);
  }

  #[test]
  fn ultra_is_the_last_effort_option_and_shows_while_on() {
    let mut opts = vec![named("model", Some("model")), effort(&["low", "high", "xhigh", "max"], "high"), named("fast", Some("model_config"))];
    let mut wire = None;
    place(&mut opts, true, false, None, &mut wire);
    assert_eq!(ids(&opts[1]), ["low", "high", "xhigh", "max", LEVEL_ID]);
    assert_eq!(opts[1].options.last().unwrap().name, LEVEL_NAME);
    assert_eq!(opts[1].value.as_deref(), Some("high"));
    assert_eq!(wire.as_deref(), Some("high"));
    // On: Ultra shows over the agent's value, which stays noted; placing again changes nothing
    place(&mut opts, true, true, None, &mut wire);
    place(&mut opts, true, true, None, &mut wire);
    assert_eq!(opts[1].value.as_deref(), Some(LEVEL_ID));
    assert_eq!(ids(&opts[1]).iter().filter(|id| **id == LEVEL_ID).count(), 1);
    assert_eq!(wire.as_deref(), Some("high"));
    assert!(shown_on(&opts));
    // The agent reports xhigh (a fresh select): noted, Ultra still shows
    opts[1] = effort(&["low", "high", "xhigh", "max"], "xhigh");
    place(&mut opts, true, true, None, &mut wire);
    assert_eq!((opts[1].value.as_deref(), wire.as_deref()), (Some(LEVEL_ID), Some("xhigh")));
    // Off with a level still to be applied: that level shows; without one, the agent's value
    place(&mut opts, true, false, Some("low"), &mut wire);
    assert_eq!(opts[1].value.as_deref(), Some("low"));
    place(&mut opts, true, false, None, &mut wire);
    assert_eq!(opts[1].value.as_deref(), Some("xhigh"));
  }

  #[test]
  fn no_ultra_without_a_level_or_with_workflows_off() {
    let mut wire = None;
    let mut plain = vec![named("model", Some("model"))];
    place(&mut plain, true, true, None, &mut wire);
    assert!(!shown_on(&plain));
    let mut toggle = vec![effort(&["on", "off"], "on")];
    place(&mut toggle, true, true, None, &mut wire);
    assert_eq!(ids(&toggle[0]), ["on", "off"]);
    // An Ultra placed earlier goes away again, and the agent's value shows
    let mut opts = vec![effort(&["low", "xhigh"], "low")];
    place(&mut opts, true, true, None, &mut wire);
    place(&mut opts, false, true, None, &mut wire);
    assert_eq!(ids(&opts[0]), ["low", "xhigh"]);
    assert_eq!(opts[0].value.as_deref(), Some("low"));
  }

  #[test]
  fn settings_ask_for_ultra_by_the_effort_value() {
    let config: StrMap = [("effort".to_owned(), LEVEL_ID.to_owned())].into();
    assert_eq!(requested(&config, &[]), Some(true));
    let config: StrMap = [("effort".to_owned(), "high".to_owned())].into();
    assert_eq!(requested(&config, &[]), Some(false));
    assert_eq!(requested(&StrMap::new(), &[]), None);
    // A select under another id is looked up by its own id
    let mut other = effort(&["low", "high"], "low");
    other.id = "thought_level".into();
    let config: StrMap = [("thought_level".to_owned(), LEVEL_ID.to_owned())].into();
    assert_eq!(requested(&config, &[other]), Some(true));
  }

  #[test]
  fn settings_carry_the_model_config_keys_along() {
    assert_eq!(settings(None), json!({ "ultracode": true }));
    let raw = r#"{ "modelOverrides": { "opus": "arn:x" }, "availableModels": ["opus"], "other": 1 }"#;
    assert_eq!(settings(Some(raw)), json!({ "ultracode": true, "modelOverrides": { "opus": "arn:x" }, "availableModels": ["opus"] }));
    assert_eq!(settings(Some("not json")), json!({ "ultracode": true }));
    assert_eq!(settings(Some("[1]")), json!({ "ultracode": true }));
  }

  #[test]
  fn the_settings_merge_into_the_existing_meta() {
    let req = json!({ "cwd": "/w", "_meta": { "claudeCode": { "options": { "thinking": { "type": "adaptive" } }, "emitRawSDKMessages": [] } } });
    let req = with_ultracode(req, None);
    assert_eq!(req["_meta"]["claudeCode"]["options"]["thinking"]["type"], "adaptive");
    assert_eq!(req["_meta"]["claudeCode"]["emitRawSDKMessages"], json!([]));
    assert_eq!(req["_meta"]["claudeCode"]["options"]["settings"]["ultracode"], true);
    let bare = with_ultracode(json!({ "cwd": "/w" }), None);
    assert_eq!(bare["_meta"]["claudeCode"]["options"]["settings"], json!({ "ultracode": true }));
  }

  #[test]
  fn workflows_follow_the_environment_unless_a_settings_file_decides() {
    assert!(workflows_enabled(None, &[]));
    assert!(workflows_enabled(Some("1"), &[]));
    assert!(!workflows_enabled(Some("0"), &[]));
    assert!(!workflows_enabled(Some(" false "), &[]));
    assert!(!workflows_enabled(Some("1"), &[json!({ "enableWorkflows": false })]));
    // The later (higher-precedence) file wins
    assert!(workflows_enabled(Some("1"), &[json!({ "enableWorkflows": false }), json!({ "enableWorkflows": true })]));
    assert!(workflows_enabled(Some("0"), &[json!({ "enableWorkflows": true })]));
    assert!(!workflows_enabled(Some("0"), &[json!({ "other": true })]));
  }

  #[test]
  fn workflows_read_the_config_dir_and_project_files() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    let env: acpira_shared::transcript::StrMap =
      [("CLAUDE_CONFIG_DIR".to_owned(), config.to_string_lossy().into_owned()), (WORKFLOWS_ENV.to_owned(), "1".to_owned())].into();
    let cwd = project.to_string_lossy().into_owned();
    assert!(workflows_enabled_for(Some(&env), &cwd));
    std::fs::write(config.join("settings.json"), r#"{ "enableWorkflows": false }"#).unwrap();
    assert!(!workflows_enabled_for(Some(&env), &cwd));
    std::fs::write(project.join(".claude").join("settings.local.json"), r#"{ "enableWorkflows": true }"#).unwrap();
    assert!(workflows_enabled_for(Some(&env), &cwd));
  }
}
