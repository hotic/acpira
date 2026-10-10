//! Background commands: `bash` with `background` starts one and returns at once with a job id; `job` reads what it
//! printed since the last check (optionally waiting for more, or for its end) and can stop it. Dev servers, watchers
//! and long builds run while the model goes on working, as Claude Code's background Bash and Codex's `exec_command`
//! sessions allow. Jobs belong to their session; each runs in a process group of its own, killed with `kill` or when
//! the session's job table is dropped

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

use super::bash::{Utf8Tail, command, kill_tree, shell};
use super::{Action, Ctx, Output, budgeted, num_arg, str_arg};
use crate::budget::Keep;
use crate::llm::ToolSpec;

/// Output kept per job; older output is dropped from the front
const KEEP_BYTES: usize = 512 * 1024;
/// How long `bash` with `background` waits before answering, so a command that fails at once says so
const START_WAIT: Duration = Duration::from_millis(1500);
pub const MAX_WAIT_S: u64 = 120;
/// Jobs one session may have running
const MAX_RUNNING: usize = 8;

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::JOB.into(),
    description: "Check a background command started with bash and background: true. Returns what it printed since the last check and \
                  whether it is still running. wait: seconds to wait for more output or for the command to end (at most 120). \
                  kill: stop it."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "id": { "type": "string", "description": "The job id bash returned" },
        "wait": { "type": "integer", "description": "Seconds to wait for new output or the end (default 0)" },
        "kill": { "type": "boolean", "description": "Stop the command" },
      },
      "required": ["id"],
    }),
  }
}

pub fn prepare(args: &Value) -> Result<Action, String> {
  let id = match args.get("id") {
    Some(Value::Number(n)) => n.to_string(),
    _ => str_arg(args, "id")?.trim().to_owned(),
  };
  let wait_s = num_arg(args, "wait").unwrap_or(0).min(MAX_WAIT_S);
  let kill = args.get("kill").and_then(Value::as_bool).unwrap_or(false);
  Ok(Action::Job { id, wait_s, kill })
}

#[derive(Default)]
struct Buf {
  text: String,
  /// Bytes dropped from the front of `text` so far
  dropped: usize,
  /// Absolute offset the model has read up to
  read: usize,
  /// Set when the command ended: its exit code, None when a signal ended it
  exit: Option<Option<i32>>,
  killed: bool,
}

struct Job {
  command: String,
  pid: Option<u32>,
  buf: Arc<Mutex<Buf>>,
  changed: Arc<Notify>,
}

/// A session's background commands
#[derive(Default)]
pub struct Jobs {
  jobs: Mutex<BTreeMap<String, Job>>,
  seq: AtomicU64,
}

impl Jobs {
  /// Start a command in the background; answers after it printed for a moment or ended
  pub async fn start(&self, command_line: &str, workdir: &Path, ctx: &Ctx) -> Output {
    let running = self.jobs.lock().values().filter(|j| j.buf.lock().exit.is_none()).count();
    if running >= MAX_RUNNING {
      return Output::error(format!("{running} background jobs are already running; stop one with job and kill first"));
    }
    let mut child = match command(command_line, workdir).spawn() {
      Ok(c) => c,
      Err(e) => return Output::error(format!("Cannot start {}: {e}", shell().0)),
    };
    let id = (self.seq.fetch_add(1, Ordering::Relaxed) + 1).to_string();
    let buf = Arc::new(Mutex::new(Buf::default()));
    let changed = Arc::new(Notify::new());
    let pid = child.id();
    let mut readers = vec![];
    for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|p| Box::new(p) as _)]
      .into_iter()
      .flatten()
    {
      let (buf, changed) = (buf.clone(), changed.clone());
      readers.push(tokio::spawn(async move {
        let mut pipe = pipe;
        let mut decode = Utf8Tail::default();
        let mut chunk = vec![0u8; 16 * 1024];
        while let Ok(n) = pipe.read(&mut chunk).await {
          if n == 0 {
            break;
          }
          let text = decode.push(&chunk[..n]);
          append(&buf, &text);
          changed.notify_waiters();
        }
        append(&buf, &decode.finish());
      }));
    }
    {
      let (buf, changed) = (buf.clone(), changed.clone());
      tokio::spawn(async move {
        let status = child.wait().await;
        // What the pipes still hold, unless a child the command left behind keeps them open
        for r in readers {
          let _ = tokio::time::timeout(Duration::from_millis(200), r).await;
        }
        buf.lock().exit = Some(status.ok().and_then(|s| s.code()));
        changed.notify_waiters();
      });
    }
    self.jobs.lock().insert(id.clone(), Job { command: command_line.to_owned(), pid, buf: buf.clone(), changed: changed.clone() });
    let _ = tokio::time::timeout(START_WAIT, wait_for_end(&buf, &changed)).await;
    let mut out = self.report(&id, ctx).unwrap_or_else(|| Output::error("the job vanished"));
    out.model = format!("Started background job {id}. Check it with job (id \"{id}\"), with wait to wait for more output.\n{}", out.model);
    out
  }

  /// New output since the last check and the status, after waiting up to `wait_s` for more (or the end), or after
  /// stopping it
  pub async fn check(&self, id: &str, wait_s: u64, kill: bool, ctx: &Ctx) -> Output {
    let found = self.jobs.lock().get(id).map(|j| (j.buf.clone(), j.changed.clone(), j.pid));
    let Some((buf, changed, pid)) = found else {
      let known: Vec<String> = self.jobs.lock().keys().cloned().collect();
      return Output::error(if known.is_empty() { format!("No job {id}: no background job was started") } else { format!("No job {id}; jobs: {}", known.join(", ")) });
    };
    if kill {
      if buf.lock().exit.is_none() {
        if let Some(pid) = pid {
          kill_tree(pid);
        }
        buf.lock().killed = true;
        let _ = tokio::time::timeout(Duration::from_secs(5), wait_for_end(&buf, &changed)).await;
      }
    } else if wait_s > 0 {
      let unread = {
        let b = buf.lock();
        b.read < b.dropped + b.text.len()
      };
      if !unread {
        let _ = tokio::time::timeout(Duration::from_secs(wait_s), wait_for_change(&buf, &changed)).await;
      }
    }
    self.report(id, ctx).unwrap_or_else(|| Output::error("the job vanished"))
  }

  fn report(&self, id: &str, ctx: &Ctx) -> Option<Output> {
    let jobs = self.jobs.lock();
    let job = jobs.get(id)?;
    let mut b = job.buf.lock();
    let start = b.read.max(b.dropped) - b.dropped;
    let skipped = b.dropped.saturating_sub(b.read);
    let fresh = b.text[start..].to_owned();
    b.read = b.dropped + b.text.len();
    let status = match b.exit {
      None => format!("[job {id} is still running: {}]", job.command),
      Some(_) if b.killed => format!("[job {id} was stopped]"),
      Some(Some(0)) => format!("[job {id} exited with code 0]"),
      Some(Some(c)) => format!("[job {id} exited with code {c}]"),
      Some(None) => format!("[job {id} was ended by a signal]"),
    };
    let failed = matches!(b.exit, Some(Some(c)) if c != 0) && !b.killed;
    drop(b);
    let mut model = String::new();
    if skipped > 0 {
      model.push_str(&format!("[{skipped} bytes of older output were dropped]\n"));
    }
    model.push_str(if fresh.trim().is_empty() { "(no new output)" } else { fresh.trim_end() });
    let model = format!("{}\n{status}", budgeted(&model, Keep::HeadTail, ctx));
    Some(Output { model, is_error: failed, content: vec![], raw_output: Some(json!({ "job": id })) })
  }
}

impl Drop for Jobs {
  fn drop(&mut self) {
    for job in self.jobs.get_mut().values() {
      if job.buf.lock().exit.is_none()
        && let Some(pid) = job.pid
      {
        kill_tree(pid);
      }
    }
  }
}

fn append(buf: &Mutex<Buf>, text: &str) {
  if text.is_empty() {
    return;
  }
  let mut b = buf.lock();
  b.text.push_str(text);
  if b.text.len() > KEEP_BYTES {
    let mut cut = b.text.len() - KEEP_BYTES;
    while !b.text.is_char_boundary(cut) {
      cut += 1;
    }
    b.text.drain(..cut);
    b.dropped += cut;
  }
}

async fn wait_for_end(buf: &Mutex<Buf>, changed: &Notify) {
  loop {
    let notified = changed.notified();
    if buf.lock().exit.is_some() {
      return;
    }
    notified.await;
  }
}

async fn wait_for_change(buf: &Mutex<Buf>, changed: &Notify) {
  let notified = changed.notified();
  if buf.lock().exit.is_some() {
    return;
  }
  notified.await;
  // Let a burst of output arrive together
  tokio::time::sleep(Duration::from_millis(100)).await;
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;

  fn ctx(dir: &Path) -> Ctx {
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "j".into(), progress: Box::new(|_| {}), jobs: Arc::new(Jobs::default()) }
  }

  #[tokio::test]
  async fn a_background_job_reports_new_output_its_end_and_can_be_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let jobs = Jobs::default();
    let out = jobs.start("echo first; sleep 2; echo second", dir.path(), &c).await;
    assert!(out.model.contains("Started background job 1") && out.model.contains("first"), "{}", out.model);
    let out = jobs.check("1", 5, false, &c).await;
    assert!(out.model.contains("second") && !out.model.contains("first"), "only what is new: {}", out.model);
    let out = jobs.check("1", 5, false, &c).await;
    assert!(out.model.contains("exited with code 0"), "{}", out.model);

    let marker = dir.path().join("marker");
    let out = jobs.start(&format!("sleep 30 && touch {}", marker.display()), dir.path(), &c).await;
    assert!(out.model.contains("still running"), "{}", out.model);
    let out = jobs.check("2", 0, true, &c).await;
    assert!(out.model.contains("was stopped"), "{}", out.model);
    assert!(jobs.check("9", 0, false, &c).await.model.contains("jobs: 1, 2"));

    // Dropping the table stops what still runs
    let jobs = Jobs::default();
    jobs.start("sleep 30", dir.path(), &c).await;
    let pid = jobs.jobs.lock()["1"].pid.unwrap();
    drop(jobs);
    tokio::time::sleep(Duration::from_millis(200)).await;
    // SAFETY: signal 0 only checks that the process group exists
    assert_ne!(unsafe { libc::kill(-(pid as i32), 0) }, 0, "the job's group is gone");
  }
}
