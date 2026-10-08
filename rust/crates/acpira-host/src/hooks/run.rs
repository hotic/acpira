//! Running one hook command: the payload on stdin, the verdict read the way Claude Code reads its hooks

use std::path::Path;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::hooks::Hook;

/// What a hook decided
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
  Pass,
  /// `permissionDecision: deny` (before an edit) or `decision: block` (after a turn), with the reason the agent is given
  Block(String),
  /// The hook could not give an answer (spawn failure, timeout, an exit code other than 0 / 2): reported, never blocking
  Broken(String),
}

/// Stdout / stderr kept for the reason and the log; a hook printing more is cut
const OUTPUT_MAX_BYTES: usize = 64 * 1024;

/// The verdict of a finished hook process, Claude Code's rules: exit 2 blocks with stderr as the reason, exit 0 is read
/// as JSON (`hookSpecificOutput.permissionDecision`, then `decision`), anything else is a broken hook
pub fn verdict_of(code: Option<i32>, stdout: &str, stderr: &str) -> Verdict {
  let reason_or = |text: &str, fallback: &str| {
    let t = text.trim();
    if t.is_empty() { fallback.to_owned() } else { t.to_owned() }
  };
  match code {
    Some(2) => Verdict::Block(reason_or(stderr, "blocked by hook (exit 2)")),
    Some(0) => {
      let Ok(out) = serde_json::from_str::<Value>(stdout.trim()) else { return Verdict::Pass };
      let specific = out.get("hookSpecificOutput");
      if specific.and_then(|s| s.get("permissionDecision")).and_then(Value::as_str) == Some("deny") {
        let reason = specific.and_then(|s| s.get("permissionDecisionReason")).and_then(Value::as_str).unwrap_or("");
        return Verdict::Block(reason_or(reason, "denied by hook"));
      }
      if out.get("decision").and_then(Value::as_str) == Some("block") {
        return Verdict::Block(reason_or(out.get("reason").and_then(Value::as_str).unwrap_or(""), "blocked by hook"));
      }
      Verdict::Pass
    }
    Some(n) => Verdict::Broken(format!("exit {n}: {}", reason_or(stderr, "no output"))),
    None => Verdict::Broken(format!("killed: {}", reason_or(stderr, "no output"))),
  }
}

fn clip(bytes: &[u8]) -> String {
  let end = bytes.len().min(OUTPUT_MAX_BYTES);
  String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The platform shell running `command` (hooks are shell source, like Claude Code's `command` hooks)
fn shell(command: &str) -> tokio::process::Command {
  #[cfg(windows)]
  {
    let mut c = tokio::process::Command::new(std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into()));
    // Shell source, not argv: CRT escaping would corrupt its quotes
    c.args(["/d", "/s", "/c"]).raw_arg(format!("\"{command}\""));
    c
  }
  #[cfg(not(windows))]
  {
    let mut c = tokio::process::Command::new("/bin/sh");
    c.args(["-c", command]);
    c
  }
}

/// Run `hook` in `root` with `payload` on stdin. `env` adds variables (the session and agent ids)
pub async fn run(hook: &Hook, root: &Path, payload: &Value, env: &[(&str, String)]) -> Verdict {
  let mut cmd = shell(&hook.run);
  cmd
    .current_dir(root)
    .env("ACPIRA_PROJECT_DIR", root)
    // Scripts written for Claude Code find the project the same way
    .env("CLAUDE_PROJECT_DIR", root)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
  for (k, v) in env {
    cmd.env(k, v);
  }
  // A process group of its own: a timed-out hook's children go with it
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  let spawned = crate::platform::windows_process::spawn(&mut cmd).await;
  #[cfg(not(windows))]
  let spawned = cmd.spawn().map(|child| (child, ()));
  let (mut child, _job) = match spawned {
    Ok(c) => c,
    Err(e) => return Verdict::Broken(format!("could not start `{}`: {e}", hook.run)),
  };
  // The payload goes in while the output is read, both under the timeout: a hook that never reads a payload larger than
  // the pipe buffer (or fills its stdout before reading) must not stall the write past it
  let stdin = child.stdin.take();
  let input = payload.to_string().into_bytes();
  let feed = async move {
    if let Some(mut stdin) = stdin {
      // A hook that never reads its stdin closes the pipe early: that is not a failure
      let _ = stdin.write_all(&input).await;
      let _ = stdin.shutdown().await;
    }
  };
  #[cfg(unix)]
  let pid = child.id();
  let finished = async move { tokio::join!(feed, child.wait_with_output()).1 };
  match tokio::time::timeout(hook.timeout, finished).await {
    Ok(Ok(out)) => verdict_of(out.status.code(), &clip(&out.stdout), &clip(&out.stderr)),
    Ok(Err(e)) => Verdict::Broken(format!("`{}` failed: {e}", hook.run)),
    Err(_) => {
      // The future owned the child and dropped it (kill_on_drop); the group still holds its descendants
      #[cfg(unix)]
      if let Some(pid) = pid {
        // SAFETY: signalling a process group this hook created
        unsafe {
          libc::killpg(pid as i32, libc::SIGKILL);
        }
      }
      Verdict::Broken(format!("`{}` timed out after {}s", hook.run, hook.timeout.as_secs_f64()))
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn claude_code_hook_answers_are_understood() {
    let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"read README first"}}"#;
    assert_eq!(verdict_of(Some(0), deny, ""), Verdict::Block("read README first".into()));
    let block = r#"{"decision":"block","reason":"gate failed"}"#;
    assert_eq!(verdict_of(Some(0), block, ""), Verdict::Block("gate failed".into()));
    assert_eq!(verdict_of(Some(2), "", " no tests \n"), Verdict::Block("no tests".into()));
    // An allow with extra context, an empty object and plain text are all passes
    let context = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"note"}}"#;
    for out in [context, "{}", "", "all good"] {
      assert_eq!(verdict_of(Some(0), out, ""), Verdict::Pass, "{out}");
    }
    assert!(matches!(verdict_of(Some(1), "", "Traceback"), Verdict::Broken(m) if m.contains("Traceback")));
    assert!(matches!(verdict_of(None, "", ""), Verdict::Broken(_)));
  }

  // A hook that never reads stdin while a payload larger than any pipe buffer waits there: the timeout still ends it
  #[tokio::test]
  async fn the_timeout_covers_a_payload_the_hook_never_reads() {
    #[cfg(windows)]
    let command = "ping -n 30 127.0.0.1 >nul".to_owned();
    #[cfg(not(windows))]
    let command = "sleep 30".to_owned();
    let hook = Hook { run: command, timeout: std::time::Duration::from_millis(500) };
    let payload = serde_json::json!({ "blob": "x".repeat(4 << 20) });
    let dir = std::env::temp_dir();
    let t0 = std::time::Instant::now();
    let verdict = tokio::time::timeout(std::time::Duration::from_secs(20), run(&hook, &dir, &payload, &[])).await.expect("the hook's own timeout");
    assert!(matches!(&verdict, Verdict::Broken(m) if m.contains("timed out")), "{verdict:?}");
    assert!(t0.elapsed() < std::time::Duration::from_secs(10), "{:?}", t0.elapsed());
  }
}
