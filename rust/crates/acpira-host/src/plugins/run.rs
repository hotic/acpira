//! Running a script tool (`docs/dev/plugins.md`, Script tools): the command in the plugin's frozen copy, the call as
//! JSON on stdin, stdout as the result

use std::path::Path;

use serde_json::{Value, json};

use crate::plugins::manifest::{self, ToolDef};

/// Output kept from one call; the rest is read and dropped
pub const OUTPUT_MAX_BYTES: usize = 256 * 1024;
/// Stderr kept for an error
const ERROR_TAIL_BYTES: usize = 4 * 1024;

/// Where a call comes from
#[derive(Debug, Clone, Default)]
pub struct CallContext {
  /// The session's folder (on an SSH host for a remote session: only its path)
  pub workspace: String,
  pub session: String,
}

/// A plugin command ready to spawn: the program resolved in the frozen copy `dir` or on PATH, run there with the
/// plugin's folders in its environment. Shared by script tools, checks and resident processes
pub async fn command(command: &[String], dir: &Path, data: &Path, workspace: &str) -> Result<tokio::process::Command, String> {
  let (program, args) = command.split_first().ok_or("the command is empty")?;
  let resolved = match manifest::program_in(dir, program) {
    Some(p) => p.to_string_lossy().into_owned(),
    None if !program.contains(['/', '\\']) => {
      crate::acp::agents::registry::resolve_command(program, &[], crate::platform::command::Os::current(), &crate::acp::agents::launch::ProcessEnv)
        .await
        .ok_or_else(|| format!("`{program}` is not on PATH"))?
    }
    None => return Err(format!("`{program}` is not a file of the plugin")),
  };
  let spec = crate::platform::command::spawn_spec(&resolved, args, crate::platform::command::Os::current(), std::env::var("ComSpec").ok().as_deref());
  let mut cmd = tokio::process::Command::new(&spec.command);
  #[cfg(windows)]
  if spec.verbatim {
    cmd.raw_arg(spec.args.join(" "));
  } else {
    cmd.args(&spec.args);
  }
  #[cfg(not(windows))]
  cmd.args(&spec.args);
  crate::platform::command::no_window(&mut cmd);
  if let Some(path) = crate::acp::agents::login_path::merged() {
    cmd.env("PATH", path);
  }
  let _ = std::fs::create_dir_all(data);
  cmd
    .current_dir(dir)
    .env("ACPIRA_PLUGIN_DIR", dir)
    .env("ACPIRA_PLUGIN_DATA", data)
    .env("ACPIRA_WORKSPACE", workspace)
    // The frozen copy is read-only: no cache files next to the scripts
    .env("PYTHONDONTWRITEBYTECODE", "1");
  Ok(cmd)
}

/// Run `tool` of the plugin frozen at `dir` with `arguments`: its stdout, or the error the model reads
pub async fn call(tool: &ToolDef, dir: &Path, data: &Path, arguments: &Value, ctx: &CallContext) -> Result<String, String> {
  let cmd = command(&tool.command, dir, data, &ctx.workspace).await.map_err(|e| if tool.command.is_empty() { "the tool has no command".to_owned() } else { e })?;
  let input = json!({ "arguments": arguments, "workspace": ctx.workspace, "session": ctx.session }).to_string().into_bytes();
  let out = crate::hooks::run::run_child(cmd, &tool.command.join(" "), tool.timeout(), input, OUTPUT_MAX_BYTES).await?;
  let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
  if out.code != Some(0) {
    let err = String::from_utf8_lossy(&out.stderr);
    let tail = tail(err.trim(), ERROR_TAIL_BYTES);
    let code = out.code.map(|c| format!("exit {c}")).unwrap_or_else(|| "killed".into());
    return Err(if tail.is_empty() { format!("the tool failed ({code})") } else { format!("the tool failed ({code}): {tail}") });
  }
  if out.cut {
    text.push_str(&format!("\n[output cut at {} KB]", OUTPUT_MAX_BYTES / 1024));
  }
  Ok(text)
}

fn tail(s: &str, max: usize) -> &str {
  if s.len() <= max {
    return s;
  }
  let mut at = s.len() - max;
  while !s.is_char_boundary(at) {
    at += 1;
  }
  s.get(at..).unwrap_or_default()
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
  use super::*;

  fn tool(command: &[&str], timeout: Option<f64>) -> ToolDef {
    ToolDef { name: "t".into(), description: "d".into(), input: json!({ "type": "object" }), command: command.iter().map(|s| s.to_string()).collect(), read_only: true, timeout }
  }

  #[tokio::test]
  async fn a_script_reads_its_call_and_its_folders() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::write(
      dir.path().join("t.sh"),
      "read line; echo \"$line\"; echo \"dir=$(basename \"$ACPIRA_PLUGIN_DIR\") data=$(basename \"$ACPIRA_PLUGIN_DATA\") ws=$ACPIRA_WORKSPACE cwd=$(basename \"$PWD\")\"",
    )
    .unwrap();
    let ctx = CallContext { workspace: "/w".into(), session: "s1".into() };
    let out = call(&tool(&["sh", "t.sh"], None), dir.path(), &data, &json!({ "amount": 3 }), &ctx).await.unwrap();
    let first: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(first, json!({ "arguments": { "amount": 3 }, "workspace": "/w", "session": "s1" }));
    let name = dir.path().file_name().unwrap().to_string_lossy();
    assert!(out.contains(&format!("dir={name} data=data ws=/w cwd={name}")), "{out}");
    assert!(data.is_dir());
  }

  #[tokio::test]
  async fn failures_timeouts_and_floods_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let ctx = CallContext::default();
    let err = call(&tool(&["sh", "-c", "echo boom >&2; exit 3"], None), dir.path(), &data, &json!({}), &ctx).await.unwrap_err();
    assert_eq!(err, "the tool failed (exit 3): boom");
    let err = call(&tool(&["sh", "-c", "sleep 30"], Some(0.3)), dir.path(), &data, &json!({}), &ctx).await.unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    let out = call(&tool(&["sh", "-c", "head -c 600000 /dev/zero | tr '\\\\0' x"], None), dir.path(), &data, &json!({}), &ctx).await.unwrap();
    assert!(out.ends_with("[output cut at 256 KB]") && out.len() < OUTPUT_MAX_BYTES + 64, "{}", out.len());
    let err = call(&tool(&["./missing.sh"], None), dir.path(), &data, &json!({}), &ctx).await.unwrap_err();
    assert!(err.contains("not a file of the plugin"), "{err}");
  }
}
