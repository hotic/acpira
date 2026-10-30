//! Pseudo terminals here: the user's login shell in a folder, by the request that opened it

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::Out;
use super::files::host_path;
use crate::remote::wire::WireError;

/// The pseudo terminals open here, by the request that opened them (`ptyResize` names it)
pub(super) static PTYS: std::sync::LazyLock<Mutex<HashMap<u64, Box<dyn portable_pty::MasterPty + Send>>>> = std::sync::LazyLock::new(Mutex::default);

/// Ends a terminal's shell when its request goes before the shell did (a cancel, the helper's exit), and forgets its pty
struct PtyGuard {
  id: u64,
  killer: Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}

impl Drop for PtyGuard {
  fn drop(&mut self) {
    if let Some(k) = self.killer.as_mut() {
      let _ = k.kill();
    }
    PTYS.lock().remove(&self.id);
  }
}

/// The user's login shell on a pseudo terminal in `cwd`, as the engine opens one locally (`terminal::command`). Output goes
/// out as `data` chunks (the credit window holds the shell back when the engine does not keep up), the foreground
/// process's name as `title` chunks when it changes; `stdin` frames are its input. The answer: its exit
pub(super) async fn pty(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  use portable_pty::{PtySize, native_pty_system};
  let cwd = match a.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
    Some(c) => Some(host_path(c)?.to_string_lossy().into_owned()),
    None => None,
  };
  let (cols, rows) = crate::terminal::clamp_size(a.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16, a.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16);
  let program: Option<Vec<String>> = a.get("program").and_then(|p| serde_json::from_value(p.clone()).ok());
  let failed = |e: anyhow::Error| WireError::new("io", format!("{e:#}"));
  let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(failed)?;
  let cmd = crate::terminal::command(program.as_deref(), cwd.as_deref()).map_err(failed)?;
  let shell = crate::terminal::shell_name(&cmd);
  let mut child = pair.slave.spawn_command(cmd).map_err(failed)?;
  drop(pair.slave);
  let mut reader = pair.master.try_clone_reader().map_err(failed)?;
  let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(failed)?));
  let mut guard = PtyGuard { id, killer: Some(child.clone_killer()) };
  PTYS.lock().insert(id, pair.master);
  out.chunks(id, "title", shell.as_bytes()).await;
  let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
  std::thread::spawn(move || {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
      match reader.read(&mut buf) {
        Ok(0) => break,
        Ok(n) => {
          if tx.blocking_send(buf[..n].to_vec()).is_err() {
            break;
          }
        }
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
        Err(_) => break,
      }
    }
  });
  if let Some(mut input) = stdin_rx {
    tokio::spawn(async move {
      while let Some(Some(bytes)) = input.recv().await {
        let w = writer.clone();
        let wrote = tokio::task::spawn_blocking(move || {
          let mut w = w.lock();
          w.write_all(&bytes).and_then(|_| w.flush())
        })
        .await;
        if !matches!(wrote, Ok(Ok(()))) {
          break;
        }
      }
    });
  }
  let mut wait = tokio::task::spawn_blocking(move || child.wait());
  let mut tick = tokio::time::interval(Duration::from_secs(1));
  let mut title = shell.clone();
  let status = loop {
    tokio::select! {
      data = rx.recv() => match data {
        Some(bytes) => out.chunks(id, "data", &bytes).await,
        // The output ended before the shell: wait for its exit
        None => break wait.await,
      },
      status = &mut wait => {
        // Output the shell wrote just before it ended still goes out ahead of the exit
        let drain = tokio::time::sleep(Duration::from_millis(100));
        tokio::pin!(drain);
        loop {
          tokio::select! {
            data = rx.recv() => match data {
              Some(bytes) => out.chunks(id, "data", &bytes).await,
              None => break,
            },
            () = &mut drain => break,
          }
        }
        break status;
      }
      _ = tick.tick() => {
        let now = pty_foreground(id).unwrap_or_else(|| shell.clone());
        if now != title {
          title = now;
          out.chunks(id, "title", title.as_bytes()).await;
        }
      }
    }
  };
  // Reaped: its pid may be another process's now, which the guard must not signal
  guard.killer = None;
  let (code, signal) = match status {
    Ok(Ok(s)) => (s.exit_code() as i64, s.signal().map(str::to_owned)),
    _ => (-1, None),
  };
  Ok(json!({ "code": code, "signal": signal }))
}

/// The name of the process in a terminal's foreground
#[cfg(unix)]
fn pty_foreground(id: u64) -> Option<String> {
  let leader = PTYS.lock().get(&id).and_then(|m| m.process_group_leader())?;
  crate::terminal::process_name(leader)
}

#[cfg(not(unix))]
fn pty_foreground(_id: u64) -> Option<String> {
  None
}
