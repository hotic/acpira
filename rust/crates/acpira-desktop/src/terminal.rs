//! runInTerminal (logins, installs, keychain unlocks): the system's own terminal runs the line the engine resolved. macOS
//! opens a .command script in Terminal, which runs it under the user's login shell as VS Code's integrated terminal
//! would, and needs no Automation permission (an AppleScript `do script` does). Linux hands the same script to the
//! desktop's terminal. Windows starts the engine's prepared invocation (`powershell.exe -NoExit -EncodedCommand …`,
//! environment edits folded in) in a console of its own

use std::process::{Command, Stdio};

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalRequest {
  pub title: String,
  pub command: String,
  pub args: Vec<String>,
  /// A null value removes a variable
  pub env: Option<Map<String, Value>>,
}

fn quote(s: &str) -> String {
  let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./=:@%+-".contains(c));
  if plain { s.to_owned() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

fn env_name(name: &str) -> bool {
  let mut chars = name.chars();
  matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The POSIX script for a request. It removes itself and its folder first (the shell keeps reading the open file), names
/// the window, applies the environment edits and runs the line; the window stays open on its output
pub fn terminal_script(r: &TerminalRequest) -> String {
  let mut lines = vec![
    "#!/bin/sh".to_owned(),
    "rm -f \"$0\"; rmdir \"$(dirname \"$0\")\" 2>/dev/null".to_owned(),
    format!("printf '\\033]0;%s\\007' {}", quote(&r.title)),
  ];
  for (name, value) in r.env.iter().flatten() {
    // A name no shell can set never reaches the script
    if !env_name(name) {
      continue;
    }
    match value {
      Value::Null => lines.push(format!("unset {name}")),
      Value::String(v) => lines.push(format!("export {name}={}", quote(v))),
      other => lines.push(format!("export {name}={}", quote(&other.to_string()))),
    }
  }
  lines.push(std::iter::once(&r.command).chain(&r.args).map(|s| quote(s)).collect::<Vec<_>>().join(" "));
  format!("{}\n", lines.join("\n"))
}

/// The terminal command for a script on Linux: the Debian alternative most desktops provide, then plain xterm
const LINUX_TERMINALS: [(&str, &str); 2] = [("x-terminal-emulator", "-e"), ("xterm", "-e")];

pub fn run_in_terminal(r: &TerminalRequest, log: &dyn Fn(&str)) {
  if cfg!(windows) {
    let mut cmd = Command::new(&r.command);
    cmd.args(&r.args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    crate::process::detach(&mut cmd);
    match cmd.spawn() {
      Ok(c) => crate::process::reap(c),
      Err(e) => log(&format!("terminal failed: {e}")),
    }
    return;
  }
  let file = match write_script(r) {
    Ok(f) => f,
    Err(e) => {
      log(&format!("terminal failed: {e}"));
      return;
    }
  };
  if cfg!(target_os = "macos") {
    let mut cmd = Command::new("open");
    cmd.arg("-a").arg("Terminal").arg(&file).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match cmd.spawn() {
      Ok(c) => crate::process::reap(c),
      Err(e) => log(&format!("terminal failed: {e}")),
    }
    return;
  }
  for (bin, flag) in LINUX_TERMINALS {
    let mut cmd = Command::new(bin);
    cmd.arg(flag).arg("sh").arg(&file).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    crate::process::detach(&mut cmd);
    if let Ok(c) = cmd.spawn() {
      crate::process::reap(c);
      return;
    }
  }
  log(&format!("terminal failed: no terminal emulator found for {}", r.title));
}

// A private temp folder holding run.command (macOS) or run.sh, mode 0700
fn write_script(r: &TerminalRequest) -> std::io::Result<std::path::PathBuf> {
  let dir = std::env::temp_dir().join(format!("acpira-term-{}", uuid::Uuid::new_v4().simple()));
  std::fs::create_dir(&dir)?;
  let file = dir.join(if cfg!(target_os = "macos") { "run.command" } else { "run.sh" });
  std::fs::write(&file, terminal_script(r))?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))?;
  }
  Ok(file)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  // Ported from the Electron app's terminal.test.ts
  #[test]
  fn quotes_the_line_and_applies_the_environment_edits_before_it() {
    let env = json!({ "CLAUDE_CONFIG_DIR": "/tmp/x y", "DROP_ME": null, "BAD NAME": "x" });
    let script = terminal_script(&TerminalRequest {
      title: "Claude's login".into(),
      command: "/opt/bin/claude".into(),
      args: vec!["auth".into(), "login".into(), "a b".into()],
      env: env.as_object().cloned(),
    });
    assert_eq!(
      script.split('\n').collect::<Vec<_>>(),
      [
        "#!/bin/sh",
        "rm -f \"$0\"; rmdir \"$(dirname \"$0\")\" 2>/dev/null",
        "printf '\\033]0;%s\\007' 'Claude'\\''s login'",
        "export CLAUDE_CONFIG_DIR='/tmp/x y'",
        "unset DROP_ME",
        "/opt/bin/claude auth login 'a b'",
        "",
      ]
    );
  }

  // The script really runs under sh: arguments with quotes and spaces arrive whole, and the file and its folder go
  #[cfg(unix)]
  #[test]
  fn runs_under_sh_with_its_arguments_intact_and_removes_itself() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("t");
    std::fs::create_dir(&sub).unwrap();
    let file = sub.join("run.sh");
    let env = json!({ "GONE": null });
    std::fs::write(
      &file,
      terminal_script(&TerminalRequest {
        title: "t".into(),
        command: "printf".into(),
        args: vec!["%s|%s|%s".into(), "it's".into(), "two words".into(), "${HOME}".into()],
        env: env.as_object().cloned(),
      }),
    )
    .unwrap();
    let out = Command::new("sh").arg(&file).env("GONE", "x").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.strip_prefix("\u{1b}]0;t\u{7}").unwrap_or(&text), "it's|two words|${HOME}");
    assert!(!sub.exists());
  }
}
