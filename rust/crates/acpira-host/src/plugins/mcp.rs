//! A plugin's `mcp` contribution (`docs/dev/plugins.md`): entries of `.mcp.json`'s shape, sent to local sessions after
//! the shared servers (a shared entry of the same name shadows one), run from the plugin's frozen copy

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::plugins::host::Active;
use crate::plugins::manifest;
use crate::shared_config::mcp::SharedServer;

/// The placeholders an entry may use for the frozen copy and the plugin's writable folder
pub const DIR_VAR: &str = "${ACPIRA_PLUGIN_DIR}";
pub const DATA_VAR: &str = "${ACPIRA_PLUGIN_DATA}";

/// The servers of every plugin in effect, rewritten to run from its frozen copy
pub fn servers(active: &[Arc<Active>], data_of: &dyn Fn(&str) -> std::path::PathBuf) -> Vec<SharedServer> {
  let mut out = vec![];
  for a in active {
    let Some(mcp) = &a.manifest.contributes.mcp else { continue };
    let data = data_of(&a.id);
    let _ = std::fs::create_dir_all(&data);
    for (name, entry) in mcp {
      if let Some(spec) = entry.as_object() {
        out.push(SharedServer::from_spec(name, &rewrite(spec, &a.dir, &data)));
      }
    }
  }
  out
}

/// One entry with the placeholders filled in, a command naming a file of the plugin made absolute, and both folders in
/// the server's environment
pub fn rewrite(spec: &Map<String, Value>, dir: &Path, data: &Path) -> Map<String, Value> {
  let (d, w) = (dir.to_string_lossy(), data.to_string_lossy());
  let fill = |s: &str| s.replace(DIR_VAR, &d).replace(DATA_VAR, &w);
  let mut out = spec.clone();
  if let Some(command) = spec.get("command").and_then(Value::as_str) {
    let command = match manifest::program_in(dir, strip_dir(command)) {
      Some(p) => p.to_string_lossy().into_owned(),
      None => fill(command),
    };
    out.insert("command".into(), json!(command));
  }
  if let Some(args) = spec.get("args").and_then(Value::as_array) {
    out.insert("args".into(), json!(args.iter().map(|a| a.as_str().map(|s| json!(fill(s))).unwrap_or_else(|| a.clone())).collect::<Vec<_>>()));
  }
  let ours = [("ACPIRA_PLUGIN_DIR", d.to_string()), ("ACPIRA_PLUGIN_DATA", w.to_string())];
  let env = match spec.get("env") {
    Some(Value::Array(list)) => {
      let mut list: Vec<Value> = list.iter().map(|p| fill_pair(p, &fill)).collect();
      list.extend(ours.iter().map(|(k, v)| json!({ "name": k, "value": v })));
      Value::Array(list)
    }
    other => {
      let mut map: Map<String, Value> = other.and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().map(|s| json!(fill(s))).unwrap_or_else(|| v.clone()))).collect()).unwrap_or_default();
      for (k, v) in ours {
        map.insert(k.into(), json!(v));
      }
      Value::Object(map)
    }
  };
  out.insert("env".into(), env);
  out
}

fn fill_pair(p: &Value, fill: &dyn Fn(&str) -> String) -> Value {
  let mut p = p.clone();
  if let Some(v) = p.get("value").and_then(Value::as_str).map(fill) {
    p["value"] = json!(v);
  }
  p
}

/// `${ACPIRA_PLUGIN_DIR}/x` as the plugin-relative `x`
pub fn strip_dir(command: &str) -> &str {
  command.strip_prefix(DIR_VAR).map(|r| r.trim_start_matches(['/', '\\'])).unwrap_or(command)
}

/// The dynamic check: every entry starts from the frozen copy, answers `initialize` and `tools/list` within `limit`,
/// and is ended again
pub async fn check(mcp: &Map<String, Value>, dir: &Path, data: &Path, limit: Duration) -> Result<(), String> {
  let _ = std::fs::create_dir_all(data);
  for (name, entry) in mcp {
    let Some(spec) = entry.as_object() else { continue };
    let server = SharedServer::from_spec(name, &rewrite(spec, dir, data));
    let acp = crate::shared_config::mcp::to_acp(&server, Some(dir)).await.map_err(|e| format!("mcp `{name}`: {e}"))?;
    match tokio::time::timeout(limit, acpira_agent::mcp::probe(&acp, dir)).await {
      Ok(Ok(_)) => {}
      Ok(Err(e)) => return Err(format!("mcp `{name}`: {e}")),
      Err(_) => return Err(format!("mcp `{name}`: no answer to initialize and tools/list within {} s", limit.as_secs())),
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_entry_runs_from_the_frozen_copy() {
    let Ok(dir) = tempfile::tempdir() else { return };
    let _ = std::fs::write(dir.path().join("server.py"), "");
    let data = dir.path().join("data");
    let spec = json!({ "command": "server.py", "args": ["--db", "${ACPIRA_PLUGIN_DATA}/rates.db"], "env": { "MODE": "${ACPIRA_PLUGIN_DIR}" } });
    let out = rewrite(spec.as_object().unwrap_or(&Map::new()), dir.path(), &data);
    let (d, w) = (dir.path().to_string_lossy().into_owned(), data.to_string_lossy().into_owned());
    assert_eq!(out["command"], json!(dir.path().join("server.py").to_string_lossy()));
    assert_eq!(out["args"], json!(["--db", format!("{w}/rates.db")]));
    assert_eq!(out["env"], json!({ "MODE": d, "ACPIRA_PLUGIN_DIR": d, "ACPIRA_PLUGIN_DATA": w }));
    // A program on PATH stays as it is; the list form of env gets both folders too
    let spec = json!({ "command": "uvx", "args": ["${ACPIRA_PLUGIN_DIR}/srv"], "env": [{ "name": "K", "value": "v" }] });
    let out = rewrite(spec.as_object().unwrap_or(&Map::new()), dir.path(), &data);
    assert_eq!((out["command"].as_str(), out["args"][0].as_str()), (Some("uvx"), Some(format!("{d}/srv").as_str())));
    assert_eq!(out["env"].as_array().map(Vec::len), Some(3));
    assert_eq!(strip_dir("${ACPIRA_PLUGIN_DIR}/bin/x"), "bin/x");
  }
}
