//! Version diagnostics for npm-packaged ACP adapters: read off package.json
//! files, the CLIs are never invoked, nothing throws

use std::path::{Path, PathBuf};

use serde_json::Value;

use acpira_shared::inventory::{AdapterInfo, AdapterPart, EnginePart};

use crate::acp::agents::registry::{AgentDef, NativeLayout};

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

/// The engine package's directory and version, searched from the adapter root up the way node resolves it
async fn find_engine(from: &Path, pkg: &str) -> Option<(PathBuf, Option<String>)> {
  let mut dir = from.to_path_buf();
  for _ in 0..MAX_UP {
    let candidate = dir.join("node_modules").join(pkg);
    if let Some(v) = read_package(&candidate, pkg).await {
      return Some((candidate, v));
    }
    let up = dir.parent()?.to_path_buf();
    if up == dir {
      return None;
    }
    dir = up;
  }
  None
}

/// Where a layout's native binary may live for one platform (node's `process.platform` / `process.arch` names)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCandidates {
  /// The platform package to report as missing
  pub package: String,
  /// Paths relative to a `node_modules` directory on the engine's resolution chain
  pub in_node_modules: Vec<String>,
  /// A path relative to the engine package itself
  pub in_engine: Option<String>,
}

/// None for a platform the layout has no binary for (the engine then fails on its own terms, nothing to check)
pub fn native_candidates(layout: NativeLayout, platform: &str, arch: &str) -> Option<NativeCandidates> {
  if !matches!(arch, "x64" | "arm64") {
    return None;
  }
  let exe = if platform == "win32" { ".exe" } else { "" };
  match layout {
    NativeLayout::ClaudeSdk => {
      let base = format!("@anthropic-ai/claude-agent-sdk-{platform}-{arch}");
      let paths = match platform {
        // claude-agent-acp tries glibc first and musl second (the other way round on musl); either one works
        "linux" => vec![format!("{base}/claude"), format!("{base}-musl/claude")],
        "darwin" | "win32" => vec![format!("{base}/claude{exe}")],
        _ => return None,
      };
      Some(NativeCandidates { package: base, in_node_modules: paths, in_engine: None })
    }
    NativeLayout::CodexVendor => {
      let cpu = if arch == "x64" { "x86_64" } else { "aarch64" };
      let triple = match platform {
        "linux" => format!("{cpu}-unknown-linux-musl"),
        "darwin" => format!("{cpu}-apple-darwin"),
        "win32" => format!("{cpu}-pc-windows-msvc"),
        _ => return None,
      };
      let package = format!("@openai/codex-{platform}-{arch}");
      let rel = format!("vendor/{triple}/bin/codex{exe}");
      Some(NativeCandidates { in_node_modules: vec![format!("{package}/{rel}")], in_engine: Some(rel), package })
    }
  }
}

/// This machine in node's terms
fn node_target() -> (&'static str, &'static str) {
  let platform = match std::env::consts::OS {
    "macos" => "darwin",
    "windows" => "win32",
    other => other,
  };
  let arch = match std::env::consts::ARCH {
    "x86_64" => "x64",
    "aarch64" => "arm64",
    other => other,
  };
  (platform, arch)
}

async fn is_file(p: &Path) -> bool {
  tokio::fs::metadata(p).await.is_ok_and(|m| m.is_file())
}

/// The missing platform package, or None when a binary is there (or the platform has none to look for)
pub async fn missing_native(engine_dir: &Path, layout: NativeLayout, platform: &str, arch: &str) -> Option<String> {
  let c = native_candidates(layout, platform, arch)?;
  if let Some(rel) = &c.in_engine
    && is_file(&engine_dir.join(rel)).await
  {
    return None;
  }
  // Node's lookup from inside the engine package: every ancestor's node_modules
  let mut dir = engine_dir.to_path_buf();
  for _ in 0..MAX_UP + 4 {
    for rel in &c.in_node_modules {
      if is_file(&dir.join("node_modules").join(rel)).await {
        return None;
      }
    }
    let Some(up) = dir.parent().map(Path::to_path_buf) else { break };
    if up == dir {
      break;
    }
    dir = up;
  }
  Some(c.package)
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
    let blank = EnginePart { name: engine.name.clone(), version: None, r#override: None, override_env: None, native_missing: None };
    out.engine = Some(match (over, &root) {
      (Some(o), _) => EnginePart { r#override: Some(o), override_env: Some(engine.override_env.clone()), ..blank },
      (None, Some(root)) => match find_engine(root, &engine.package).await {
        Some((dir, version)) => {
          let (platform, arch) = node_target();
          let native_missing = match engine.native {
            Some(layout) => missing_native(&dir, layout, platform, arch).await,
            None => None,
          };
          EnginePart { version, native_missing, ..blank }
        }
        None => blank,
      },
      (None, None) => blank,
    });
  }
  Some(out)
}
