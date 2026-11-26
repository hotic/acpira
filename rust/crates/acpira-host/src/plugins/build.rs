//! Thin adapter binding `acpira-plugin-build` (isolated oxc compiler crate) to the host's manifest and SDK.

use std::path::{Path, PathBuf};

pub use acpira_plugin_build::{BUILD, BUILD_VERSION, DOC_MAX_BYTES, SDK_SPECIFIER, UI_EXTERNALS, VENDORED_SCOPE, compiled_source, ui};

/// Where the compiled output of snapshot `hash` of `id` lives under the plugins root
pub fn output_dir(root: &Path, id: &str, hash: &str) -> PathBuf {
  acpira_plugin_build::output_dir(root, id, hash, crate::plugins::sdk::SDK)
}

/// Compile `main` of the frozen copy `dir` for a resident process into `out`
pub fn runtime(dir: &Path, main: &str, out: &Path) -> Result<PathBuf, String> {
  acpira_plugin_build::runtime(dir, main, out, crate::plugins::sdk::SDK)
}

/// Check that every compiled source of `manifest` compiles, writing the output the engine will load
pub fn check(dir: &Path, manifest: &crate::plugins::manifest::Manifest, out: &Path) -> Vec<String> {
  let compiles = |rel: &str| compiled_source(rel) && crate::plugins::manifest::inside(dir, rel).is_some_and(|p| p.is_file());
  let mut errors = vec![];
  if let Some(main) = manifest.contributes.runtime.as_ref().and_then(|r| r.main.as_deref()).filter(|m| compiles(m))
    && let Err(e) = runtime(dir, main, out)
  {
    errors.push(format!("runtime.main: {e}"));
  }
  for u in &manifest.contributes.ui {
    if let Some(entry) = u.entry.as_deref().filter(|e| compiles(e))
      && let Err(e) = ui(dir, entry, &u.id, out)
    {
      errors.push(format!("ui `{}`: {e}", u.id));
    }
  }
  if let Some(e) = manifest.exports.as_deref().filter(|e| compiles(e))
    && let Err(e) = acpira_plugin_build::exports_compile(dir, e)
  {
    errors.push(format!("exports: {e}"));
  }
  errors
}
