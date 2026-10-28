//! Commands run here, each in its own process group; the groups still running die with the helper

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::Out;
use super::files::host_path;
use crate::remote::wire::{CHUNK, WireError};

/// Process groups this helper started and has not seen end; all of them die with the helper
static GROUPS: std::sync::LazyLock<Mutex<HashSet<u32>>> = std::sync::LazyLock::new(Mutex::default);

fn kill_group(pid: u32) {
  #[cfg(unix)]
  // SAFETY: kill has no memory preconditions; a negative pid addresses the group the child leads
  unsafe {
    libc::kill(-(pid as i32), libc::SIGKILL);
  }
  #[cfg(not(unix))]
  let _ = pid;
}

pub fn kill_all_groups() {
  for pid in GROUPS.lock().drain() {
    kill_group(pid);
  }
}

/// Kills its group when dropped, unless the command ended by itself (its background children then live on, as a
/// dev server started by a build script does)
struct Group(Option<u32>);

impl Group {
  fn new(pid: Option<u32>) -> Group {
    if let Some(pid) = pid {
      GROUPS.lock().insert(pid);
    }
    Group(pid)
  }

  fn disarm(&mut self) {
    if let Some(pid) = self.0.take() {
      GROUPS.lock().remove(&pid);
    }
  }
}

impl Drop for Group {
  fn drop(&mut self) {
    if let Some(pid) = self.0.take() {
      GROUPS.lock().remove(&pid);
      kill_group(pid);
    }
  }
}

/// Run a command: `argv` as given, or `command` under `sh -c`; output streamed as `out` / `err` chunks, stdin from
/// `stdin` frames when asked for. The answer: exit code, signal, whether the timeout killed it
pub(super) async fn exec(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let argv: Vec<String> = match (a.get("argv").and_then(Value::as_array), a.get("command").and_then(Value::as_str)) {
    (Some(v), _) => v.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
    (None, Some(c)) => vec!["/bin/sh".into(), "-c".into(), c.to_owned()],
    _ => return Err(WireError::new("invalid", "exec needs argv or command")),
  };
  let Some((program, rest)) = argv.split_first() else { return Err(WireError::new("invalid", "empty argv")) };
  let cwd = match a.get("cwd").and_then(Value::as_str) {
    Some(c) => host_path(c)?,
    None => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into())),
  };
  let mut cmd = tokio::process::Command::new(program);
  crate::platform::command::no_window(&mut cmd);
  cmd.args(rest).current_dir(&cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
  let want_stdin = a.get("stdin").and_then(Value::as_bool).unwrap_or(false);
  cmd.stdin(if want_stdin { Stdio::piped() } else { Stdio::null() });
  if let Some(env) = a.get("env").and_then(Value::as_object) {
    for (k, v) in env {
      match v.as_str() {
        Some(v) => cmd.env(k, v),
        None => cmd.env_remove(k),
      };
    }
  }
  #[cfg(unix)]
  cmd.process_group(0);
  let mut child = cmd.spawn().map_err(|e| WireError::io(&e, program))?;
  let mut group = Group::new(child.id());
  if let (Some(mut pipe), Some(mut rx)) = (child.stdin.take(), stdin_rx) {
    tokio::spawn(async move {
      while let Some(Some(bytes)) = rx.recv().await {
        if pipe.write_all(&bytes).await.is_err() {
          break;
        }
      }
      // EOF or the request ended: dropping the pipe closes it
    });
  }
  let (tx, mut rx) = mpsc::channel::<(&'static str, Vec<u8>)>(16);
  for (name, pipe) in [("out", child.stdout.take().map(|p| Box::new(p) as Box<dyn AsyncRead + Unpin + Send>)), ("err", child.stderr.take().map(|p| Box::new(p) as _))] {
    let Some(mut pipe) = pipe else { continue };
    let tx = tx.clone();
    tokio::spawn(async move {
      let mut buf = vec![0u8; CHUNK];
      while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 || tx.send((name, buf[..n].to_vec())).await.is_err() {
          break;
        }
      }
    });
  }
  drop(tx);
  let timeout = a.get("timeoutMs").and_then(Value::as_u64).map(Duration::from_millis);
  let deadline = tokio::time::sleep(timeout.unwrap_or(Duration::from_secs(365 * 24 * 3600)));
  tokio::pin!(deadline);
  let mut status = None;
  let mut timed_out = false;
  let mut open = true;
  while status.is_none() || open {
    tokio::select! {
      c = rx.recv(), if open => match c {
        Some((s, bytes)) => out.chunks(id, s, &bytes).await,
        None => open = false,
      },
      s = child.wait(), if status.is_none() => { status = Some(s); }
      _ = &mut deadline, if !timed_out && status.is_none() => {
        timed_out = true;
        drop(std::mem::replace(&mut group, Group(None)));
      }
      // A background child holding the pipes open is not waited for past a short grace once the command ended
      _ = tokio::time::sleep(Duration::from_millis(500)), if status.is_some() && open => { open = false; }
    }
  }
  if !timed_out {
    group.disarm();
  }
  let status = status.and_then(Result::ok);
  #[cfg(unix)]
  let signal = {
    use std::os::unix::process::ExitStatusExt;
    status.and_then(|s| s.signal())
  };
  #[cfg(not(unix))]
  let signal: Option<i32> = None;
  Ok(json!({ "code": status.and_then(|s| s.code()), "signal": signal, "timedOut": timed_out }))
}
