//! Version diagnostics for npm-packaged ACP adapters (mirror of src/host/acp/adapterInfo.ts): read off package.json
//! files, the CLIs are never invoked, nothing throws

use std::path::{Path, PathBuf};

use serde_json::Value;

use acpira_shared::inventory::{AdapterInfo, AdapterPart, EnginePart};

use super::agent_registry::AgentDef;

const MAX_UP: usize = 8;

async fn read_package(dir: &Path, name: &str) -> Option<Option<String>> {
  let raw = tokio::fs::read(dir.join("package.json")).await.ok()?;
  let pkg: Value = serde_json::from_slice(&raw).ok()?;
  if pkg.get("name").and_then(Value::as_str) != Some(name) {
    return None;
  }
  Some(pkg.get("version").and_then(Value::as_str).map(str::to_owned))
}

async fn adapter_root(binary: &str, pkg: &str) -> Option<PathBuf> {
  let lower = binary.to_lowercase();
  if cfg!(windows) && (lower.ends_with(".cmd") || lower.ends_with(".ps1") || lower.ends_with(".bat")) {
    let dir = Path::new(binary).parent()?.join("node_modules").join(pkg);
    return read_package(&dir, pkg).await.map(|_| dir);
  }
  let mut dir = match tokio::fs::canonicalize(binary).await {
    Ok(p) => p.parent()?.to_path_buf(),
    Err(_) => Path::new(binary).parent()?.to_path_buf(),
  };
  for _ in 0..MAX_UP {
    if read_package(&dir, pkg).await.is_some() {
      return Some(dir);
    }
    let up = dir.parent()?.to_path_buf();
    if up == dir {
      return None;
    }
    dir = up;
  }
  None
}

async fn engine_version(from: &Path, pkg: &str) -> Option<String> {
  let mut dir = from.to_path_buf();
  for _ in 0..MAX_UP {
    if let Some(v) = read_package(&dir.join("node_modules").join(pkg), pkg).await {
      return v;
    }
    let up = dir.parent()?.to_path_buf();
    if up == dir {
      return None;
    }
    dir = up;
  }
  None
}

pub async fn read_adapter_info(binary: &str, def: &AgentDef) -> Option<AdapterInfo> {
  let spec = def.adapter.as_ref()?;
  let mut out = AdapterInfo::default();
  let root = adapter_root(binary, &spec.package).await;
  if let Some(root) = &root {
    out.adapter = Some(AdapterPart {
      name: spec.package.clone(),
      version: read_package(root, &spec.package).await.flatten(),
      root: Some(root.to_string_lossy().into_owned()),
    });
  }
  if let Some(engine) = &spec.engine {
    // The agent's own env may also set the override
    let over = def
      .env
      .as_ref()
      .and_then(|e| e.get(&engine.override_env).cloned())
      .or_else(|| std::env::var(&engine.override_env).ok())
      .filter(|v| !v.is_empty());
    out.engine = Some(match (over, &root) {
      (Some(o), _) => {
        EnginePart { name: engine.name.clone(), version: None, r#override: Some(o), override_env: Some(engine.override_env.clone()) }
      }
      (None, Some(root)) => {
        EnginePart { name: engine.name.clone(), version: engine_version(root, &engine.package).await, r#override: None, override_env: None }
      }
      (None, None) => EnginePart { name: engine.name.clone(), version: None, r#override: None, override_env: None },
    });
  }
  Some(out)
}
