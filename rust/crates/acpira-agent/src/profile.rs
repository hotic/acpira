//! A plugin's specialised agent (`contributes.agent.builtin`, docs/dev/plugins.md): this loop with the plugin's own
//! system prompt, a tool list the agent enforces, and a default model. Launched as `acpira agent --home <root> --profile
//! <frozen copy>`, so every session of that agent is one; the frozen copy never changes, a new version is a new launch.
//!
//! The prompt replaces the built-in base and its variants (and the user's and plugins' overrides of them, which are
//! written for the general agent); the environment block stays, AGENTS.md only when the profile asks for it. A tool
//! outside the list is never offered and refused when the model calls it anyway

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::permission::wildcard;

/// The prefix of a tool entry naming one of the plugin's own script tools
const OWN: &str = "plugin:";

/// Largest prompt file a profile may carry
const MAX_PROMPT: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
  /// The plugin's id: the prompt records it as its variant (`profile:<id>`)
  pub id: String,
  pub dir: PathBuf,
  pub prompt: String,
  /// Tool names as the model sees them, `*` and `?` matching (`read`, `grep`); `plugin:<name>` names one of the
  /// plugin's own script tools, which reach this loop through Acpira's MCP server as `acpira_<id>_<name>`
  pub tools: Vec<String>,
  /// `<source>/<model>` a new session starts on, when configured
  pub model: Option<String>,
  /// The session folder's AGENTS.md files join the prompt
  pub instructions: bool,
  /// The plugin's own skills folder (`contributes.skills`): the only skills its sessions see
  pub skills: Option<PathBuf>,
}

impl Profile {
  /// Read from a plugin's frozen copy (`plugin.json`, then the prompt file it names inside the copy)
  pub fn load(dir: &Path) -> Result<Profile, String> {
    let manifest: Value = std::fs::read(dir.join("plugin.json"))
      .map_err(|e| format!("{}: {e}", dir.join("plugin.json").display()))
      .and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("plugin.json: {e}")))?;
    let id = manifest.get("id").and_then(Value::as_str).ok_or("plugin.json has no id")?.to_owned();
    let spec = manifest.pointer("/contributes/agent/builtin").and_then(Value::as_object).ok_or("plugin.json has no contributes.agent.builtin")?;
    let file = spec.get("prompt").and_then(Value::as_str).ok_or("contributes.agent.builtin.prompt is required")?;
    let path = dir.join(file);
    // The copy is the plugin's whole world: nothing outside it, through `..` or a link
    let inside = path.canonicalize().ok().zip(dir.canonicalize().ok()).is_some_and(|(p, d)| p.starts_with(d));
    if !inside || std::fs::metadata(&path).map_or(true, |m| !m.is_file() || m.len() > MAX_PROMPT) {
      return Err(format!("the prompt {file} is not a file of the plugin of at most {} KB", MAX_PROMPT / 1024));
    }
    let prompt = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
    let tools = spec.get("tools").and_then(Value::as_array).ok_or("contributes.agent.builtin.tools is required")?;
    let tools = tools.iter().filter_map(Value::as_str).map(str::to_owned).collect();
    Ok(Profile {
      id,
      dir: dir.to_path_buf(),
      prompt,
      tools,
      model: spec.get("model").and_then(Value::as_str).map(str::to_owned),
      instructions: spec.get("instructions").and_then(Value::as_bool).unwrap_or(false),
      skills: manifest.pointer("/contributes/skills").and_then(Value::as_str).map(|s| dir.join(s)),
    })
  }

  pub fn allows(&self, tool: &str) -> bool {
    // The history tools are how chunk compaction gives the model its own past back, not a capability
    if tool == crate::tools::HISTORY_GREP || tool == crate::tools::HISTORY_LIST {
      return true;
    }
    self.tools.iter().any(|p| match p.strip_prefix(OWN) {
      Some(name) => wildcard(&format!("{}_{}_{name}", acpira_shared::host_tools::SERVER, self.id), tool),
      None => wildcard(p, tool),
    })
  }

  /// The modes its sessions offer: Agent, and Plan only when the profile lets the model leave it (`exit_plan`)
  pub fn offers_mode(&self, mode: &str) -> bool {
    mode == crate::modes::AGENT || (mode == crate::modes::PLAN && self.allows(crate::tools::EXIT_PLAN))
  }

  pub fn variant(&self) -> String {
    format!("profile:{}", self.id)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn plugin(spec: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("prompt.md"), "You keep the ledger.").unwrap();
    std::fs::write(d.path().join("plugin.json"), format!(r#"{{ "id": "ledger", "contributes": {{ "agent": {{ "builtin": {spec} }} }} }}"#)).unwrap();
    d
  }

  #[test]
  fn a_profile_takes_its_prompt_from_the_copy_and_allows_only_its_tools() {
    let d = plugin(r#"{ "prompt": "prompt.md", "tools": ["read", "ledger_*", "plugin:post*"], "model": "gw/fast" }"#);
    let p = Profile::load(d.path()).unwrap();
    assert_eq!((p.id.as_str(), p.prompt.as_str(), p.model.as_deref(), p.instructions), ("ledger", "You keep the ledger.", Some("gw/fast"), false));
    assert!(p.allows("read") && p.allows("ledger_post") && p.allows(crate::tools::HISTORY_GREP));
    assert!(!p.allows("bash") && !p.allows("write") && !p.allows("other_tool"));
    // The plugin's own script tools, under the name Acpira's MCP server gives them
    assert!(p.allows("acpira_ledger_post_entry") && !p.allows("acpira_other_post") && !p.allows("acpira_ledger_void"));
    assert!(p.offers_mode(crate::modes::AGENT) && !p.offers_mode(crate::modes::PLAN) && !p.offers_mode(crate::modes::RAW));
  }

  #[test]
  fn a_prompt_outside_the_copy_or_no_tool_list_is_refused() {
    let d = plugin(r#"{ "prompt": "../prompt.md", "tools": [] }"#);
    assert!(Profile::load(d.path()).unwrap_err().contains("not a file of the plugin"));
    let d = plugin(r#"{ "prompt": "prompt.md" }"#);
    assert!(Profile::load(d.path()).unwrap_err().contains("tools is required"));
  }
}
