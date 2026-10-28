//! The first launch: the data folder (ACPIRA_HOME, ~/.acpira) does not exist, so the app starts no engine, which would
//! create it, and the page runs its first-run flow (packages/ui/welcome/) until the user says yes. Until then nothing
//! writes the folder; `acpira agents --json` only reads it. See docs/dev/desktop.md

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::locator::SidecarCommand;

/// The registry's own agent (acpira-host's SELF_AGENT_ID): always there, so the intro does not count it as found
const SELF_AGENT: &str = "acpira";

/// How long the agent probe may take (it reads the login shell's PATH first); a row left empty beats a stuck thread
const PROBE_LIMIT: Duration = Duration::from_secs(20);

/// The folder as the consent card names it: `~/…` under the user's home on Unix, where the shell spells it so; the full
/// path otherwise (Windows, a folder elsewhere)
pub fn display_home(data_home: &Path, user_home: &Path, unix: bool) -> String {
  if unix && let Ok(rest) = data_home.strip_prefix(user_home) {
    let rest = rest.to_string_lossy();
    return if rest.is_empty() { "~".to_owned() } else { format!("~/{rest}") };
  }
  data_home.display().to_string()
}

/// The CLIs `acpira agents --json` found on this machine, as the intro's row lists them: `{ id, name }` for every agent
/// with a resolved binary, the built-in one left out. None when the output is not the command's JSON
pub fn detected_agents(stdout: &str) -> Option<Vec<Value>> {
  let v: Value = serde_json::from_str(stdout).ok()?;
  let found = v["agents"]
    .as_array()?
    .iter()
    .filter(|a| a["id"].as_str().is_some_and(|id| id != SELF_AGENT) && a["binary"].as_str().is_some_and(|b| !b.is_empty()))
    .map(|a| json!({ "id": a["id"], "name": a["name"].as_str().or(a["id"].as_str()) }))
    .collect();
  Some(found)
}

/// Runs `<engine> agents --json` with the engine's environment and reads what it found. The engine resolves each CLI
/// exactly as it will once it runs, and touches nothing on disk. Err says why there is no list
pub fn probe_agents(engine: &SidecarCommand) -> Result<Vec<Value>, String> {
  let mut cmd = Command::new(&engine.command);
  cmd.args(["agents", "--json"]).envs(&engine.env).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(crate::process::CREATE_NO_WINDOW);
  }
  let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", engine.command.display()))?;
  // Read on a thread of its own: a full pipe would otherwise keep the child from exiting
  let mut stdout = child.stdout.take().ok_or("no stdout")?;
  let reader = std::thread::spawn(move || {
    let mut out = String::new();
    let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
    out
  });
  let start = Instant::now();
  let status = loop {
    match child.try_wait() {
      Ok(Some(status)) => break status,
      Ok(None) if start.elapsed() < PROBE_LIMIT => std::thread::sleep(Duration::from_millis(50)),
      Ok(None) => {
        let _ = child.kill();
        let _ = child.wait();
        return Err("agents --json timed out".into());
      }
      Err(e) => return Err(e.to_string()),
    }
  };
  let out = reader.join().unwrap_or_default();
  if !status.success() {
    return Err(format!("agents --json exited with {status}"));
  }
  detected_agents(&out).ok_or_else(|| "agents --json printed no agent list".into())
}

/// The user said yes: the folder, private to the user on Unix, as the engine would make it
pub fn create_home(path: &Path) -> std::io::Result<()> {
  #[cfg(unix)]
  {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
  }
  #[cfg(not(unix))]
  std::fs::create_dir_all(path)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_folder_reads_as_the_shell_spells_it() {
    let home = Path::new("/Users/dev");
    assert_eq!(display_home(Path::new("/Users/dev/.acpira"), home, true), "~/.acpira");
    assert_eq!(display_home(Path::new("/Users/dev/data/acpira"), home, true), "~/data/acpira");
    // Elsewhere, or where no shell says ~ (Windows), the full path
    assert_eq!(display_home(Path::new("/srv/acpira"), home, true), "/srv/acpira");
    assert_eq!(display_home(Path::new("/Users/dev/.acpira"), home, false), "/Users/dev/.acpira");
  }

  #[test]
  fn the_intro_lists_found_clis_without_the_built_in_agent() {
    let out = r#"{ "agents": [
      { "id": "acpira", "name": "Acpira", "binary": "/app/acpira" },
      { "id": "claude", "name": "Claude Code", "binary": "/usr/local/bin/claude" },
      { "id": "codex", "name": "Codex", "binary": null },
      { "id": "gemini", "name": "Gemini CLI", "binary": "" },
      { "id": "kimi", "binary": "/opt/kimi" }
    ] }"#;
    assert_eq!(
      detected_agents(out),
      Some(vec![json!({ "id": "claude", "name": "Claude Code" }), json!({ "id": "kimi", "name": "kimi" })])
    );
    assert_eq!(detected_agents(r#"{ "agents": [] }"#), Some(vec![]));
    assert_eq!(detected_agents("PATH from the login shell adds: /opt/bin"), None);
  }

  #[cfg(unix)]
  #[test]
  fn the_folder_is_made_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("nested/.acpira");
    create_home(&home).unwrap();
    assert_eq!(std::fs::metadata(&home).unwrap().permissions().mode() & 0o777, 0o700);
    // Again is fine (another window said yes first)
    create_home(&home).unwrap();
  }
}
