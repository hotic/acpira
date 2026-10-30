//! Background commands: `bash` with `background` starts one and returns at once with a job id; `job` reads what it
//! printed since the last check (optionally waiting for more, or for its end) and can stop it. Dev servers, watchers
//! and long builds run while the model goes on working, as Claude Code's background Bash and Codex's `exec_command`
//! sessions allow. Jobs belong to their session; each runs in a process group of its own, killed with `kill`, when the
//! agent shuts down (`Server::close`, also on SIGTERM) or when the session's job table is dropped.
//! An interactive job (`bash` with `interactive`, 2026-10-24) runs in a pseudo terminal (portable-pty, ConPTY on
//! Windows) instead of pipes, so prompts, REPLs and installers that ask behave as they do for a person, and `job` with
//! `input` types into it, as Codex's `write_stdin` does. Its output reaches the model without terminal control
//! sequences, a line redrawn with a carriage return as its last state

use std::collections::BTreeMap;
use std::io::{Read, Write};
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
/// After input, the answer is taken once the job has been quiet this long (or `INPUT_WAIT` passed)
const QUIET: Duration = Duration::from_millis(400);
/// How long the answer to input is waited for when the call gives no wait
const INPUT_WAIT: Duration = Duration::from_secs(3);
/// An interactive job's terminal: wide enough that tools do not wrap their lines early
const TTY_SIZE: (u16, u16) = (160, 48);

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::JOB.into(),
    description: "Check a background command started with bash and background: true. Returns what it printed since the last check and \
                  whether it is still running. wait: seconds to wait for more output or for the command to end (at most 120). \
                  kill: stop it. input (a job started with interactive: true): text typed into it first, then what it printed \
                  in answer; end a line with \\n to press Enter (Ctrl-C is \\u0003, the down arrow \\u001b[B)."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "id": { "type": "string", "description": "The job id bash returned" },
        "wait": { "type": "integer", "description": "Seconds to wait for new output or the end (default 0)" },
        "kill": { "type": "boolean", "description": "Stop the command" },
        "input": { "type": "string", "description": "Text to type into an interactive job, \\n for Enter" },
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
  let input = args.get("input").or_else(|| args.get("chars")).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned);
  Ok(Action::Job { id, wait_s, kill, input })
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
  /// An interactive job's terminal
  tty: Option<Tty>,
}

/// What an interactive job is typed into; the pseudo terminal stays open while the job is kept
struct Tty {
  /// Bytes for the writer thread, which a job that is not reading may block, rather than the runtime
  input: std::sync::mpsc::Sender<Vec<u8>>,
  _master: Box<dyn portable_pty::MasterPty + Send>,
}

/// A session's background commands
#[derive(Default)]
pub struct Jobs {
  jobs: Mutex<BTreeMap<String, Job>>,
  seq: AtomicU64,
}

impl Jobs {
  /// Start a command in the background, in a terminal when `interactive`; answers after it printed for a moment or
  /// ended
  pub async fn start(&self, command_line: &str, workdir: &Path, interactive: bool, ctx: &Ctx) -> Output {
    let running = self.jobs.lock().values().filter(|j| j.buf.lock().exit.is_none()).count();
    if running >= MAX_RUNNING {
      return Output::error(format!("{running} background jobs are already running; stop one with job and kill first"));
    }
    if interactive {
      return self.start_tty(command_line, workdir, ctx).await;
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
    self.jobs.lock().insert(id.clone(), Job { command: command_line.to_owned(), pid, buf: buf.clone(), changed: changed.clone(), tty: None });
    let _ = tokio::time::timeout(START_WAIT, wait_for_end(&buf, &changed)).await;
    let mut out = self.report(&id, ctx).unwrap_or_else(|| Output::error("the job vanished"));
    out.model = format!("Started background job {id}. Check it with job (id \"{id}\"), with wait to wait for more output.\n{}", out.model);
    // The host names later checks of this id after the command (`rawOutput.job`)
    out.raw_output = Some(json!({ "job": id }));
    out
  }

  /// `start` for an interactive job: the command under the shell in a pseudo terminal, its output read and its exit
  /// waited for on threads of its own (portable-pty's ends are blocking)
  async fn start_tty(&self, command_line: &str, workdir: &Path, ctx: &Ctx) -> Output {
    let spawned = (|| -> anyhow::Result<_> {
      let (cols, rows) = TTY_SIZE;
      let pair = portable_pty::native_pty_system().openpty(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
      let mut child = pair.slave.spawn_command(tty_command(command_line, workdir))?;
      // The child holds the only slave end: once it and its own children are gone, the reader sees the end
      drop(pair.slave);
      let reader = pair.master.try_clone_reader()?;
      let writer = pair.master.take_writer()?;
      let pid = child.process_id();
      let wait = std::thread::Builder::new().name("acpira-job-wait".into()).spawn(move || child.wait().ok().map(|s| (s.exit_code(), s.signal().is_some())))?;
      Ok((pair.master, reader, writer, pid, wait))
    })();
    let (master, mut reader, mut writer, pid, wait) = match spawned {
      Ok(s) => s,
      Err(e) => return Output::error(format!("Cannot start {} in a terminal: {e}", shell().0)),
    };
    let id = (self.seq.fetch_add(1, Ordering::Relaxed) + 1).to_string();
    let buf = Arc::new(Mutex::new(Buf::default()));
    let changed = Arc::new(Notify::new());
    let reading = {
      let (buf, changed) = (buf.clone(), changed.clone());
      std::thread::Builder::new().name("acpira-job-read".into()).spawn(move || {
        let (mut decode, mut plain) = (Utf8Tail::default(), Plain::default());
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
          match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
              append(&buf, &plain.push(&decode.push(&chunk[..n])));
              changed.notify_waiters();
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // EIO once the slave side closed (Linux)
            Err(_) => break,
          }
        }
        append(&buf, &plain.push(&decode.finish()));
      })
    };
    let (input, typed) = std::sync::mpsc::channel::<Vec<u8>>();
    let writing = std::thread::Builder::new().name("acpira-job-write".into()).spawn(move || {
      while let Ok(bytes) = typed.recv() {
        if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
          break;
        }
      }
    });
    if let Err(e) = reading.and(writing) {
      if let Some(pid) = pid {
        kill_tree(pid);
      }
      return Output::error(format!("Cannot start {} in a terminal: {e}", shell().0));
    }
    {
      let (buf, changed) = (buf.clone(), changed.clone());
      tokio::spawn(async move {
        let status = tokio::task::spawn_blocking(move || wait.join().ok().flatten()).await.ok().flatten();
        // What the terminal still holds arrives on the reader; give it a moment, as for pipes
        tokio::time::sleep(Duration::from_millis(200)).await;
        buf.lock().exit = Some(status.and_then(|(code, signalled)| (!signalled).then_some(code as i32)));
        changed.notify_waiters();
      });
    }
    let tty = Tty { input, _master: master };
    self.jobs.lock().insert(id.clone(), Job { command: command_line.to_owned(), pid, buf: buf.clone(), changed: changed.clone(), tty: Some(tty) });
    let _ = tokio::time::timeout(START_WAIT, wait_for_end(&buf, &changed)).await;
    let mut out = self.report(&id, ctx).unwrap_or_else(|| Output::error("the job vanished"));
    out.model = format!(
      "Started interactive job {id} in a terminal. Type into it with job (id \"{id}\", input; \\n is Enter), check it with job and wait.\n{}",
      out.model
    );
    out.raw_output = Some(json!({ "job": id }));
    out
  }

  /// New output since the last check and the status, after typing `input` into it, waiting up to `wait_s` for more (or
  /// the end), or stopping it
  pub async fn check(&self, id: &str, wait_s: u64, kill: bool, input: Option<&str>, ctx: &Ctx) -> Output {
    let found = self.jobs.lock().get(id).map(|j| (j.buf.clone(), j.changed.clone(), j.pid, j.tty.as_ref().map(|t| t.input.clone())));
    let Some((buf, changed, pid, tty)) = found else {
      let known: Vec<String> = self.jobs.lock().keys().cloned().collect();
      return Output::error(if known.is_empty() { format!("No job {id}: no background job was started") } else { format!("No job {id}; jobs: {}", known.join(", ")) });
    };
    if let Some(text) = input.filter(|_| !kill) {
      let Some(tty) = tty else {
        return Output::error(format!("Job {id} was not started with interactive: true, so it takes no input; start the command again with interactive"));
      };
      if buf.lock().exit.is_some() {
        let mut out = self.report(id, ctx).unwrap_or_else(|| Output::error("the job vanished"));
        out.model = format!("Job {id} has ended; the input was not typed.\n{}", out.model);
        out.is_error = true;
        return out;
      }
      // Enter is a carriage return on a terminal: a program reading lines gets it as \n (the line discipline turns
      // one into the other), one reading keys (a prompt library's select) as the Enter key
      let _ = tty.send(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
      let cap = if wait_s > 0 { Duration::from_secs(wait_s) } else { INPUT_WAIT };
      wait_for_quiet(&buf, &changed, cap).await;
      return self.report(id, ctx).unwrap_or_else(|| Output::error("the job vanished"));
    }
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
    let unread = b.text.get(start..).unwrap_or_default();
    let fresh = if job.tty.is_some() { overstrike(unread) } else { unread.to_owned() };
    b.read = b.dropped + b.text.len();
    let status = match b.exit {
      None => format!("[job {id} is still running: {}]", job.command),
      Some(_) if b.killed => format!("[job {id} was stopped]"),
      Some(Some(0)) => format!("[job {id} exited with code 0]"),
      Some(Some(c)) => format!("[job {id} exited with code {c}]"),
      Some(None) => format!("[job {id} was ended by a signal]"),
    };
    let failed = matches!(b.exit, Some(Some(c)) if c != 0) && !b.killed;
    let running = b.exit.is_none();
    drop(b);
    let mut model = String::new();
    if skipped > 0 {
      model.push_str(&format!("[{skipped} bytes of older output were dropped]\n"));
    }
    model.push_str(if fresh.trim().is_empty() { "(no new output)" } else { fresh.trim_end() });
    let model = budgeted(&model, Keep::HeadTail, ctx);
    // The row's card: only what the command printed, plus how it ended. The row already names the command (the host
    // maps the job id to it), and a check that found nothing new shows no card, so a model polling every second adds
    // a quiet row, not a card per poll
    let mut shown = if fresh.trim().is_empty() { String::new() } else { budgeted(fresh.trim_end(), Keep::HeadTail, ctx) };
    if !running {
      shown = if shown.is_empty() { status.clone() } else { format!("{shown}\n{status}") };
    }
    let content = if shown.is_empty() { vec![] } else { vec![super::text_content(&shown)] };
    Some(Output { model: format!("{model}\n{status}"), is_error: failed, content, raw_output: None })
  }
}

impl Jobs {
  /// Kill every job still running (the agent is shutting down)
  pub fn stop_all(&self) {
    for job in self.jobs.lock().values() {
      let mut b = job.buf.lock();
      if b.exit.is_none()
        && let Some(pid) = job.pid
      {
        b.killed = true;
        kill_tree(pid);
      }
    }
  }
}

impl Drop for Jobs {
  fn drop(&mut self) {
    self.stop_all();
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

/// The command line under the shell for a pseudo terminal: the same shell as `bash`, a terminal's environment
fn tty_command(command: &str, workdir: &Path) -> portable_pty::CommandBuilder {
  #[cfg(windows)]
  let full = format!("{}{command}", super::bash::UTF8_PRELUDE);
  #[cfg(windows)]
  let command = full.as_str();
  let (sh, flags) = shell();
  let mut cmd = portable_pty::CommandBuilder::new(&sh);
  // PowerShell's -NonInteractive makes Read-Host fail, the opposite of what the terminal is for
  cmd.args(flags.iter().filter(|f| **f != "-NonInteractive"));
  cmd.arg(command);
  cmd.cwd(workdir);
  cmd.env("PAGER", "cat");
  cmd.env("GIT_PAGER", "cat");
  // Credentials are the user's to type, not the model's
  cmd.env("GIT_TERMINAL_PROMPT", "0");
  cmd.env("TERM", "xterm-256color");
  // Fewer escape sequences to strip; a prompt reads the same without colour
  cmd.env("NO_COLOR", "1");
  // An agent started from a GUI app may have no locale, and a terminal without one takes UTF-8 for bytes
  if ["LANG", "LC_ALL", "LC_CTYPE"].iter().all(|k| cmd.get_env(k).is_none_or(|v| v.is_empty())) {
    cmd.env("LANG", "en_US.UTF-8");
  }
  cmd
}

/// Terminal output without its control sequences (CSI, OSC and the other string sequences, charset picks, other
/// control characters), carried across chunks; carriage returns and backspaces stay for `overstrike`
#[derive(Default)]
struct Plain(Seq);

#[derive(Default, Clone, Copy)]
enum Seq {
  #[default]
  Text,
  /// After ESC
  Escape,
  /// ESC [ up to its final byte
  Csi,
  /// OSC, DCS, SOS, PM or APC up to BEL or ESC \
  Str,
  /// ESC inside a string sequence
  StrEscape,
  /// ESC ( and the like: one more character
  OneMore,
}

impl Plain {
  fn push(&mut self, text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
      self.0 = match (self.0, c) {
        (Seq::Text, '\u{1b}') => Seq::Escape,
        (Seq::Text, c) => {
          if !c.is_control() || matches!(c, '\n' | '\t' | '\r' | '\u{8}') {
            out.push(c);
          }
          Seq::Text
        }
        (Seq::Escape, '[') => Seq::Csi,
        (Seq::Escape, ']' | 'P' | 'X' | '^' | '_') => Seq::Str,
        (Seq::Escape, '(' | ')' | '*' | '+' | '#' | '%') => Seq::OneMore,
        (Seq::Escape | Seq::OneMore, _) => Seq::Text,
        (Seq::Csi, '\u{40}'..='\u{7e}') => Seq::Text,
        (Seq::Csi, _) => Seq::Csi,
        (Seq::Str, '\u{7}') => Seq::Text,
        (Seq::Str, '\u{1b}') => Seq::StrEscape,
        (Seq::Str, _) => Seq::Str,
        (Seq::StrEscape, '\\') => Seq::Text,
        (Seq::StrEscape, _) => Seq::Str,
      };
    }
    out
  }
}

/// Lines as a terminal leaves them: a carriage return goes back to the line's start and what follows overwrites it, a
/// backspace steps back one character, so a progress bar or spinner redrawn in place reads as its last state
fn overstrike(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let (mut line, mut col) = (Vec::<char>::new(), 0usize);
  for c in text.chars() {
    match c {
      '\n' => {
        out.extend(line.drain(..));
        out.push('\n');
        col = 0;
      }
      '\r' => col = 0,
      '\u{8}' => col = col.saturating_sub(1),
      c => {
        if col < line.len() {
          line[col] = c;
        } else {
          line.push(c);
        }
        col += 1;
      }
    }
  }
  out.extend(line);
  out
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

/// Until the job has printed nothing for `QUIET`, ended, or `cap` passed: the answer to what was typed, however many
/// chunks it comes in (the echo of the input arrives first)
async fn wait_for_quiet(buf: &Mutex<Buf>, changed: &Notify, cap: Duration) {
  let until = tokio::time::Instant::now() + cap;
  loop {
    let notified = changed.notified();
    if buf.lock().exit.is_some() {
      return;
    }
    let left = until.saturating_duration_since(tokio::time::Instant::now());
    if left.is_zero() || tokio::time::timeout(QUIET.min(left), notified).await.is_err() {
      return;
    }
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
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "j".into(), progress: std::sync::Arc::new(|_| {}), jobs: Arc::new(Jobs::default()), search: Default::default(), mcp: Default::default(), history: Default::default() }
  }

  #[tokio::test]
  async fn a_background_job_reports_new_output_its_end_and_can_be_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let jobs = Jobs::default();
    let out = jobs.start("echo first; sleep 2; echo second", dir.path(), false, &c).await;
    assert!(out.model.contains("Started background job 1") && out.model.contains("first"), "{}", out.model);
    let out = jobs.check("1", 5, false, None, &c).await;
    assert!(out.model.contains("second") && !out.model.contains("first"), "only what is new: {}", out.model);
    let out = jobs.check("1", 5, false, None, &c).await;
    assert!(out.model.contains("exited with code 0"), "{}", out.model);

    let marker = dir.path().join("marker");
    let out = jobs.start(&format!("sleep 30 && touch {}", marker.display()), dir.path(), false, &c).await;
    assert!(out.model.contains("still running"), "{}", out.model);
    let out = jobs.check("2", 0, true, None, &c).await;
    assert!(out.model.contains("was stopped"), "{}", out.model);
    assert!(jobs.check("9", 0, false, None, &c).await.model.contains("jobs: 1, 2"));

    // Dropping the table stops what still runs
    let jobs = Jobs::default();
    jobs.start("sleep 30", dir.path(), false, &c).await;
    let pid = jobs.jobs.lock()["1"].pid.unwrap();
    drop(jobs);
    tokio::time::sleep(Duration::from_millis(200)).await;
    // SAFETY: signal 0 only checks that the process group exists
    assert_ne!(unsafe { libc::kill(-(pid as i32), 0) }, 0, "the job's group is gone");
  }

  #[tokio::test]
  async fn an_interactive_job_runs_in_a_terminal_and_takes_typed_answers() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let jobs = Jobs::default();
    let script = r#"[ -t 0 ] && echo "on a terminal"; printf '\033[1;32mready\033[0m\n'; read -p "Name? " n; echo "hello $n"; read -p "Again? [y/n] " a; echo "answer=$a""#;
    // What the job printed, without the status line (which quotes the command)
    let printed = |out: &Output| out.model.split("\n[job ").next().unwrap().to_owned();
    let out = jobs.start(script, dir.path(), true, &c).await;
    let first = printed(&out);
    assert!(first.starts_with("Started interactive job 1") && first.contains("on a terminal") && first.ends_with("Name?"), "{}", out.model);
    assert!(first.contains("\nready\n") && !first.contains('\u{1b}') && !first.contains("[0m"), "escapes are stripped: {first:?}");
    let out = jobs.check("1", 0, false, Some("Ada\n"), &c).await;
    let answer = printed(&out);
    assert!(answer.contains("hello Ada") && answer.contains("Again? [y/n]") && !answer.contains("Name?"), "{}", out.model);
    let out = jobs.check("1", 0, false, Some("y\n"), &c).await;
    assert!(out.model.contains("answer=y") && out.model.contains("exited with code 0"), "{}", out.model);
    let out = jobs.check("1", 0, false, Some("more\n"), &c).await;
    assert!(out.is_error && out.model.contains("has ended"), "{}", out.model);

    // Ctrl-C reaches the foreground program through the terminal
    jobs.start("sleep 30", dir.path(), true, &c).await;
    let out = jobs.check("2", 5, false, Some("\u{3}"), &c).await;
    let out = if out.model.contains("still running") { jobs.check("2", 5, false, None, &c).await } else { out };
    assert!(out.model.contains("ended by a signal") || out.model.contains("exited with code 130"), "{}", out.model);

    // A plain background job takes no input
    jobs.start("sleep 30", dir.path(), false, &c).await;
    let out = jobs.check("3", 0, false, Some("x\n"), &c).await;
    assert!(out.is_error && out.model.contains("interactive: true"), "{}", out.model);
  }

  #[test]
  fn terminal_output_reads_as_its_last_state() {
    let mut plain = Plain::default();
    // A sequence split across chunks is still taken out whole
    let text = plain.push("\u{1b}[3") + &plain.push("2mok\u{1b}[0m \u{1b}]0;title\u{7}done\u{1b}(B!");
    assert_eq!(text, "ok done!");
    assert_eq!(plain.push("a\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\b"), "alinkb");
    assert_eq!(overstrike("progress 10%\rprogress 100%\nnext\r\n"), "progress 100%\nnext\n");
    assert_eq!(overstrike("abc\u{8}\u{8}XY"), "aXY");
    assert_eq!(overstrike("⠋ loading\r⠙ loading\rdone     "), "done     ");
  }
}
