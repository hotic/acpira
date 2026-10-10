//! `bash`: one shell command in the session folder. The command runs in a process group of its own; a timeout, a
//! cancelled turn, or a dropped future kills the whole group. Output streams to the tool card as
//! `_meta.terminal_output_delta` while it runs; the model gets stdout and stderr interleaved, fitted to the budget

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use super::{Action, Ctx, Output, budgeted, num_arg, resolve, str_arg};
use crate::budget::Keep;
use crate::llm::ToolSpec;

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
/// After the command exits, how long output still held by its background children is waited for
const DRAIN_GRACE: Duration = Duration::from_millis(200);
const FLUSH_EVERY: Duration = Duration::from_millis(50);

pub fn shell() -> (String, Vec<&'static str>) {
  #[cfg(windows)]
  {
    ("powershell.exe".into(), vec!["-NoProfile", "-NonInteractive", "-Command"])
  }
  #[cfg(not(windows))]
  {
    let sh = if Path::new("/bin/bash").exists() { "/bin/bash" } else { "/bin/sh" };
    (sh.into(), vec!["-c"])
  }
}

pub fn spec() -> ToolSpec {
  let (sh, _) = shell();
  let name = Path::new(&sh).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(sh);
  ToolSpec {
    name: super::BASH.into(),
    description: format!(
      "Run a shell command ({name}) in the session folder and return its output (stdout and stderr together) and exit code. \
       Non-interactive: commands that wait for input fail. Default timeout {}s, at most {}s.",
      DEFAULT_TIMEOUT_MS / 1000,
      MAX_TIMEOUT_MS / 1000
    ),
    parameters: json!({
      "type": "object",
      "properties": {
        "command": { "type": "string", "description": "The command line" },
        "workdir": { "type": "string", "description": "Directory to run in (default: the session folder)" },
        "timeout": { "type": "integer", "description": "Timeout in milliseconds" },
      },
      "required": ["command"],
    }),
  }
}

pub fn prepare(args: &Value, cwd: &Path) -> Result<Action, String> {
  let command = str_arg(args, "command").or_else(|_| str_arg(args, "cmd"))?.trim().to_owned();
  if command.is_empty() {
    return Err("command is empty".into());
  }
  let workdir = args.get("workdir").or_else(|| args.get("cwd")).and_then(Value::as_str).map(|w| resolve(w, cwd)).unwrap_or_else(|| cwd.to_owned());
  if !workdir.is_dir() {
    return Err(format!("workdir {} is not a directory", workdir.display()));
  }
  let timeout_ms = num_arg(args, "timeout").filter(|t| *t > 0).unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS);
  Ok(Action::Bash { command, workdir, timeout_ms })
}

/// Kills the command's process group when dropped while armed
struct Group(Option<u32>);

impl Group {
  fn disarm(&mut self) {
    self.0 = None;
  }

  fn kill(&mut self) {
    if let Some(pid) = self.0.take() {
      kill_tree(pid);
    }
  }
}

impl Drop for Group {
  fn drop(&mut self) {
    self.kill();
  }
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
  // SAFETY: kill has no memory preconditions; a negative pid addresses the group the child leads
  unsafe {
    libc::kill(-(pid as i32), libc::SIGKILL);
  }
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
  use std::os::windows::process::CommandExt;
  const CREATE_NO_WINDOW: u32 = 0x0800_0000;
  let _ = std::process::Command::new("taskkill")
    .args(["/PID", &pid.to_string(), "/T", "/F"])
    .creation_flags(CREATE_NO_WINDOW)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();
}

/// Decodes a byte stream as UTF-8 without splitting a character across chunks
#[derive(Default)]
struct Utf8Tail(Vec<u8>);

impl Utf8Tail {
  fn push(&mut self, bytes: &[u8]) -> String {
    self.0.extend_from_slice(bytes);
    let valid = match std::str::from_utf8(&self.0) {
      Ok(_) => self.0.len(),
      // An incomplete character at the end waits for the next chunk; anything else is replaced
      Err(e) if e.error_len().is_none() => e.valid_up_to(),
      Err(_) => self.0.len(),
    };
    let out = String::from_utf8_lossy(&self.0[..valid]).into_owned();
    self.0.drain(..valid);
    out
  }

  fn finish(&mut self) -> String {
    let out = String::from_utf8_lossy(&self.0).into_owned();
    self.0.clear();
    out
  }
}

/// Windows PowerShell 5.1 writes piped output in the OEM code page (GBK on a Chinese system), which the UTF-8 decoder
/// below turns into mojibake. Switching the console's output code page first makes PowerShell and the native commands
/// it starts (they share its hidden console) write UTF-8; `$OutputEncoding` covers text piped into native commands
#[cfg(windows)]
const UTF8_PRELUDE: &str = "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; ";

pub async fn run(command: &str, workdir: &PathBuf, timeout_ms: u64, ctx: &Ctx) -> Output {
  #[cfg(windows)]
  let full = format!("{UTF8_PRELUDE}{command}");
  #[cfg(windows)]
  let command = full.as_str();
  let (sh, flags) = shell();
  let mut cmd = tokio::process::Command::new(&sh);
  cmd.args(&flags).arg(command).current_dir(workdir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
  cmd.env("PAGER", "cat").env("GIT_PAGER", "cat").env("GIT_TERMINAL_PROMPT", "0");
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  cmd.creation_flags(0x0800_0000 | 0x0000_0200);
  let mut child = match cmd.spawn() {
    Ok(c) => c,
    Err(e) => return Output::error(format!("Cannot start {sh}: {e}")),
  };
  let mut group = Group(child.id());
  let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
  for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|p| Box::new(p) as _)]
    .into_iter()
    .flatten()
  {
    let tx = tx.clone();
    tokio::spawn(async move {
      let mut pipe = pipe;
      let mut buf = vec![0u8; 16 * 1024];
      while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
          break;
        }
      }
    });
  }
  drop(tx);

  let mut all = String::new();
  let mut decode = Utf8Tail::default();
  let mut pending = String::new();
  let flush = |pending: &mut String| {
    if !pending.is_empty() {
      (ctx.progress)(json!({ "_meta": { "terminal_output_delta": { "data": std::mem::take(pending) } } }));
    }
  };
  let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms));
  tokio::pin!(deadline);
  let mut ticker = tokio::time::interval(FLUSH_EVERY);
  let mut status = None;
  let mut timed_out = false;
  let mut pipes_open = true;
  loop {
    tokio::select! {
      chunk = rx.recv(), if pipes_open => match chunk {
        Some(bytes) => {
          let text = decode.push(&bytes);
          all.push_str(&text);
          pending.push_str(&text);
        }
        None => pipes_open = false,
      },
      s = child.wait(), if status.is_none() => {
        status = Some(s);
        break;
      }
      _ = &mut deadline => {
        timed_out = true;
        group.kill();
        status = Some(child.wait().await);
        break;
      }
      _ = ticker.tick() => flush(&mut pending),
    }
  }
  // Output still in the pipes; a background child holding them open is not waited for past the grace
  let grace = tokio::time::sleep(DRAIN_GRACE);
  tokio::pin!(grace);
  while pipes_open {
    tokio::select! {
      chunk = rx.recv() => match chunk {
        Some(bytes) => {
          let text = decode.push(&bytes);
          all.push_str(&text);
          pending.push_str(&text);
        }
        None => pipes_open = false,
      },
      _ = &mut grace => break,
    }
  }
  let rest = decode.finish();
  all.push_str(&rest);
  pending.push_str(&rest);
  flush(&mut pending);
  // A command that exited by itself keeps its background children (a dev server it started)
  if !timed_out {
    group.disarm();
  }

  let code = status.and_then(|s| s.ok()).and_then(|s| s.code());
  let mut model = if all.trim().is_empty() { "(no output)".to_owned() } else { budgeted(all.trim_end(), Keep::HeadTail, ctx) };
  if timed_out {
    model.push_str(&format!("\n[timed out after {}s; the command was killed]", timeout_ms / 1000));
  } else {
    match code {
      Some(0) => {}
      Some(c) => model.push_str(&format!("\n[exit code {c}]")),
      None => model.push_str("\n[killed by a signal]"),
    }
  }
  Output {
    model,
    is_error: timed_out || code != Some(0),
    content: vec![],
    raw_output: Some(json!({ "exitCode": code, "timedOut": timed_out })),
  }
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;
  use std::sync::Arc;

  fn ctx(dir: &Path, seen: Arc<parking_lot::Mutex<String>>) -> Ctx {
    Ctx {
      cwd: dir.to_owned(),
      outputs: dir.join("outputs"),
      call_id: "b1".into(),
      progress: Box::new(move |u| seen.lock().push_str(u["_meta"]["terminal_output_delta"]["data"].as_str().unwrap_or(""))),
    }
  }

  #[tokio::test]
  async fn output_exit_code_and_streaming() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(parking_lot::Mutex::new(String::new()));
    let out = run("echo out; echo err >&2; exit 3", &dir.path().to_owned(), 5_000, &ctx(dir.path(), seen.clone())).await;
    assert!(out.is_error);
    assert!(out.model.contains("out") && out.model.contains("err") && out.model.ends_with("[exit code 3]"), "{}", out.model);
    assert!(seen.lock().contains("out"));
    let quiet = run("true", &dir.path().to_owned(), 5_000, &ctx(dir.path(), seen)).await;
    assert_eq!(quiet.model, "(no output)");
  }

  #[tokio::test]
  async fn a_timeout_kills_the_whole_group() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("child-alive");
    let seen = Arc::new(parking_lot::Mutex::new(String::new()));
    // The grandchild would write the marker after the timeout if it survived the kill
    let cmd = format!("(sleep 1; touch {}) & sleep 30", marker.display());
    let started = std::time::Instant::now();
    let out = run(&cmd, &dir.path().to_owned(), 300, &ctx(dir.path(), seen)).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(out.model.contains("timed out"), "{}", out.model);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!marker.exists(), "the background child was killed with the group");
  }

  #[test]
  fn utf8_is_never_split() {
    let mut d = Utf8Tail::default();
    let bytes = "中文".as_bytes();
    assert_eq!(d.push(&bytes[..2]), "");
    assert_eq!(d.push(&bytes[2..]), "中文");
  }
}

// Run on a Windows machine (cross-built test binary); PowerShell is the shell there
#[cfg(all(test, windows))]
mod windows_tests {
  use super::*;
  use std::sync::Arc;

  fn ctx(dir: &Path) -> Ctx {
    let seen = Arc::new(parking_lot::Mutex::new(String::new()));
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "b1".into(), progress: Box::new(move |u| seen.lock().push_str(u.to_string().as_str())) }
  }

  #[tokio::test]
  async fn chinese_output_is_decoded_from_powershell_and_native_commands() {
    let dir = tempfile::tempdir().unwrap();
    let out = run("Write-Output '中文输出'; cmd /c echo 原生命令", &dir.path().to_owned(), 15_000, &ctx(dir.path())).await;
    assert!(out.model.contains("中文输出") && out.model.contains("原生命令"), "{:?}", out.model);
  }

  #[tokio::test]
  async fn a_cancelled_command_takes_its_children_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("child-alive");
    // The child would create the marker after the cancel if it survived
    let cmd = format!(
      "Start-Process -NoNewWindow powershell -ArgumentList '-NoProfile','-Command','Start-Sleep 3; New-Item -ItemType File \"{}\"'; Start-Sleep 30",
      marker.display()
    );
    let ctx = ctx(dir.path());
    let cut = tokio::time::timeout(Duration::from_millis(1500), run(&cmd, &dir.path().to_owned(), 60_000, &ctx)).await;
    assert!(cut.is_err(), "the command was still running when the turn was cancelled");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!marker.exists(), "the child process outlived the cancel");
  }
}
