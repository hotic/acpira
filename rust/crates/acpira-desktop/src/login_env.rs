//! An app started from the Dock or Finder gets launchd's environment, not the one the user's shell builds: variables
//! exported in ~/.zshrc (API keys, proxies, what MCP servers reference) are missing. The engine only adds the login PATH
//! itself (acpira-host acp/agents/login_path.rs), so the app reads the whole environment of an interactive login shell
//! once, before it launches the engine, and hands it over (SidecarCommand::env)

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARK: &str = "__ACPIRA_ENV_7f3c__";
const TIMEOUT: Duration = Duration::from_secs(10);

/// Per-process values of the shell that ran the probe, not part of the user's environment
const TRANSIENT: [&str; 6] = ["PWD", "OLDPWD", "SHLVL", "_", "TERM_SESSION_ID", "ACPIRA_RESOLVING_ENVIRONMENT"];

/// `env -0` output between the two marks (rc files may print around it): NUL-separated KEY=VALUE pairs
pub fn parse_env_output(out: &str) -> Option<HashMap<String, String>> {
  let start = out.find(MARK)?;
  let end = out.rfind(MARK)?;
  if end <= start {
    return None;
  }
  let mut env = HashMap::new();
  for pair in out[start + MARK.len()..end].split('\0') {
    let Some(eq) = pair.find('=') else { continue };
    if eq == 0 {
      continue;
    }
    let key = &pair[..eq];
    // ELECTRON_*: what an Electron-launched app would add, kept out for the same reason
    if TRANSIENT.contains(&key) || key.starts_with("ELECTRON_") {
      continue;
    }
    env.insert(key.to_owned(), pair[eq + 1..].to_owned());
  }
  Some(env)
}

/// The login shell's environment, or None when there is no POSIX shell to ask, it fails, or it takes longer than TIMEOUT
/// (a prompt waiting for input, a slow rc file); the app then runs with what it inherited. Blocking: call it off the UI
/// thread
pub fn resolve_login_env(log: &dyn Fn(&str), shell: Option<&str>) -> Option<HashMap<String, String>> {
  if cfg!(windows) {
    return None;
  }
  let shell =
    shell.map(str::to_owned).or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty())).unwrap_or_else(|| "/bin/zsh".into());
  let started = Instant::now();
  let script = format!("printf '%s' '{MARK}'; /usr/bin/env -0; printf '%s' '{MARK}'");
  let mut cmd = Command::new(&shell);
  cmd
    .args(["-i", "-l", "-c", &script])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .env("ACPIRA_RESOLVING_ENVIRONMENT", "1");
  // Its own process group, so a timeout takes rc-file children down with it
  crate::process::detach(&mut cmd);
  let mut child = match cmd.spawn() {
    Ok(c) => c,
    Err(e) => {
      log(&format!("login shell environment unavailable: {e}"));
      return None;
    }
  };
  let pid = child.id();
  let mut stdout = child.stdout.take()?;
  let (tx, rx) = std::sync::mpsc::channel();
  std::thread::spawn(move || {
    let mut buf = Vec::new();
    let _ = stdout.read_to_end(&mut buf);
    let _ = tx.send(buf);
  });
  let out = match rx.recv_timeout(TIMEOUT) {
    Ok(buf) => buf,
    Err(_) => {
      #[cfg(unix)]
      // SAFETY: a signal to the probe's own process group
      unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
      }
      let _ = child.wait();
      log(&format!("login shell environment unavailable: timed out after {} ms", TIMEOUT.as_millis()));
      return None;
    }
  };
  let status = child.wait().ok();
  let env = parse_env_output(&String::from_utf8_lossy(&out));
  match &env {
    Some(e) => log(&format!("login shell environment: {} variables in {} ms ({shell})", e.len(), started.elapsed().as_millis())),
    None => log(&format!(
      "login shell environment unavailable: exit {}, no environment printed",
      status.and_then(|s| s.code()).map_or("?".into(), |c| c.to_string())
    )),
  }
  let _ = pid;
  env
}

#[cfg(test)]
mod tests {
  use super::*;

  // Ported from the Electron app's loginEnv.test.ts
  #[test]
  fn reads_the_pairs_between_the_marks_keeps_values_with_equals_and_newlines_drops_per_process_ones() {
    let body = [
      "PATH=/opt/homebrew/bin:/usr/bin",
      "OPENAI_API_KEY=sk-x=y",
      "MULTI=a\nb",
      "PWD=/tmp",
      "SHLVL=2",
      "_=/usr/bin/env",
      "ELECTRON_RUN_AS_NODE=1",
      "BROKEN",
      "",
    ]
    .join("\0");
    let out = format!("rc banner\n{MARK}{body}{MARK}trailing");
    let want: HashMap<String, String> = [("PATH", "/opt/homebrew/bin:/usr/bin"), ("OPENAI_API_KEY", "sk-x=y"), ("MULTI", "a\nb")]
      .into_iter()
      .map(|(k, v)| (k.into(), v.into()))
      .collect();
    assert_eq!(parse_env_output(&out), Some(want));
  }

  #[test]
  fn gives_up_without_both_marks() {
    assert_eq!(parse_env_output("PATH=/usr/bin"), None);
    assert_eq!(parse_env_output(&format!("{MARK}PATH=/usr/bin")), None);
  }

  #[cfg(unix)]
  #[test]
  fn reads_a_real_shell() {
    let env = resolve_login_env(&|_| {}, Some("/bin/sh")).expect("sh prints its environment");
    assert!(env.contains_key("PATH"));
    assert!(!env.contains_key("ACPIRA_RESOLVING_ENVIRONMENT"));
  }
}
