//! The terminals the pages open: the desktop tool panel's terminal tabs. Each is the user's login shell on a pseudo
//! terminal (portable-pty: openpty on Unix, ConPTY on Windows) owned by the engine, so it outlives a page reload and the
//! output so far comes back with the page (`replay`). Modelled on VS Code's terminal: output is decoded as one UTF-8
//! stream and sent in 5 ms batches, and stops being read while a page has more than `HIGH_WATER` characters it has not
//! rendered yet (`terminalAck`), so a flood cannot outrun the page; the program then waits on its writes as it would in
//! any terminal.
//!
//! Per terminal a reader thread (pty reads block) feeds a bounded channel, a writer thread takes the input (a write blocks
//! while the program does not read), and a waiter thread reaps the shell. The batching runs as a task, and one ticker
//! serves every terminal: the foreground process names the tab, and a terminal no page viewed for `ORPHAN_GRACE` ends.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Notify, mpsc, oneshot};

use acpira_shared::protocol::{HostMsg, TerminalExit};

/// Where a page's terminal messages go (its bridge's direct post)
pub type Sink = Arc<dyn Fn(HostMsg) + Send + Sync>;

/// Output arriving this soon after the first chunk goes out with it in one message (VS Code's TerminalDataBufferer)
const BATCH: Duration = Duration::from_millis(5);
/// A batch stops growing here, so one message stays small
const BATCH_MAX: usize = 64 * 1024;
/// Flow control in UTF-16 units, VS Code's values: reading stops above HIGH_WATER unacknowledged, resumes under LOW_WATER
const HIGH_WATER: u64 = 100_000;
const LOW_WATER: u64 = 5_000;
/// A page that stopped acknowledging (a lost message, a page stuck elsewhere) does not hold the terminal for good
const ACK_VALVE: Duration = Duration::from_secs(10);
/// The output kept for a page that comes back; trimmed once a quarter over, so the copy stays rare
const SCROLLBACK: usize = 1 << 20;
/// Output the shell wrote just before it ended is waited for this long, so the exit follows it
const EXIT_DRAIN: Duration = Duration::from_millis(100);
/// A terminal no page viewed this long is ended (a closed window; a reload comes back well within it)
const ORPHAN_GRACE: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(1);
/// What a page that has not laid the terminal out yet gets
const DEFAULT_SIZE: (u16, u16) = (80, 24);
/// A shell still there this long after its SIGHUP (it ignores the signal) is killed outright
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_secs(2);

pub struct TerminalHub {
  terms: Mutex<Hub>,
  /// SSH hosts: a terminal in an `ssh://` folder runs its shell there (the helper's `pty`)
  remote: std::sync::OnceLock<Arc<crate::remote::connections::Connections>>,
  views: AtomicU64,
  /// What runs instead of the user's login shell (tests)
  program: Option<Vec<String>>,
  grace: Duration,
  tick: Duration,
  me: Weak<TerminalHub>,
}

#[derive(Default)]
struct Hub {
  terms: HashMap<String, Arc<Term>>,
  /// The ticker runs while there are terminals; set and cleared under this lock, so a terminal opened as the ticker
  /// stops still gets one
  ticking: bool,
}

impl TerminalHub {
  pub fn new() -> Arc<TerminalHub> {
    Self::with(None, ORPHAN_GRACE, TICK)
  }

  fn with(program: Option<Vec<String>>, grace: Duration, tick: Duration) -> Arc<TerminalHub> {
    Arc::new_cyclic(|me| TerminalHub { terms: Default::default(), remote: Default::default(), views: AtomicU64::new(0), program, grace, tick, me: me.clone() })
  }

  /// Where terminals in folders on SSH hosts connect
  pub fn set_remote(&self, connections: Arc<crate::remote::connections::Connections>) {
    let _ = self.remote.set(connections);
  }

  /// An id for a page's bridge: what it opens, acknowledges and lets go of is kept per page
  pub fn next_view(&self) -> u64 {
    self.views.fetch_add(1, Ordering::Relaxed) + 1
  }

  /// View terminal `id`, starting it (a login shell in `cwd`) when it does not run. The page gets the terminal's state
  /// and output so far first, then its output as it comes; a page already viewing it starts over the same way
  pub fn open(&self, view: u64, sink: Sink, id: &str, cwd: Option<&str>, size: Option<(u16, u16)>) {
    let term = {
      let mut hub = self.terms.lock();
      match hub.terms.get(id) {
        Some(t) => t.clone(),
        None => match self.spawn(id, cwd, size.unwrap_or(DEFAULT_SIZE)) {
          Ok(t) => {
            hub.terms.insert(id.to_owned(), t.clone());
            if !hub.ticking {
              hub.ticking = true;
              self.start_ticker();
            }
            t
          }
          Err(e) => {
            sink(HostMsg::TerminalState { id: id.to_owned(), title: None, cols: None, rows: None, replay: None, exit: None, error: Some(format!("{e:#}")) });
            return;
          }
        },
      }
    };
    if let Some((cols, rows)) = size {
      term.resize(cols, rows);
    }
    term.attach(view, sink);
  }

  /// `binary`: each character stands for one byte (xterm's legacy mouse reports), not for its UTF-8 encoding
  pub fn input(&self, id: &str, data: String, binary: bool) {
    if let Some(t) = self.get(id) {
      let bytes = if binary { data.chars().map(|c| c as u32 as u8).collect() } else { data.into_bytes() };
      // The writer thread is gone once the shell's pty closed; input then goes nowhere, as in any terminal
      let _ = t.input.send(bytes);
    }
  }

  pub fn resize(&self, id: &str, cols: u16, rows: u16) {
    if let Some(t) = self.get(id) {
      t.resize(cols, rows);
    }
  }

  pub fn ack(&self, view: u64, id: &str, chars: u64) {
    if let Some(t) = self.get(id) {
      t.ack(view, chars);
    }
  }

  /// The page stops viewing `id`; with no page left it runs on for `grace`
  pub fn close(&self, view: u64, id: &str) {
    if let Some(t) = self.get(id) {
      t.detach(view);
    }
  }

  /// A page went away: it views nothing anymore
  pub fn detach_view(&self, view: u64) {
    let terms: Vec<_> = self.terms.lock().terms.values().cloned().collect();
    for t in terms {
      t.detach(view);
    }
  }

  /// End terminal `id`: its pages hear nothing more of it, and the same id opens a new shell
  pub fn kill(&self, id: &str) {
    let removed = self.terms.lock().terms.remove(id);
    if let Some(t) = removed {
      t.shutdown();
    }
  }

  /// The engine is going: every shell ends
  pub fn dispose(&self) {
    let terms: Vec<_> = self.terms.lock().terms.drain().map(|(_, t)| t).collect();
    for t in terms {
      t.shutdown();
    }
  }

  fn get(&self, id: &str) -> Option<Arc<Term>> {
    self.terms.lock().terms.get(id).cloned()
  }

  fn spawn(&self, id: &str, cwd: Option<&str>, (cols, rows): (u16, u16)) -> anyhow::Result<Arc<Term>> {
    let (cols, rows) = clamp_size(cols, rows);
    if let Some(folder) = cwd.and_then(acpira_shared::remote::parse_remote_folder) {
      return self.spawn_remote(id, folder, (cols, rows));
    }
    let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
    let cmd = command(self.program.as_deref(), cwd)?;
    let shell = shell_name(&cmd);
    let mut child = pair.slave.spawn_command(cmd)?;
    // The child holds the only slave end: once it and its jobs are gone, the reader sees the end of the output
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let killer = child.clone_killer();
    let pid = child.process_id();
    // Set once the shell is reaped: its pid may then belong to another process, which a kill must not reach
    let reaped = Arc::new(AtomicBool::new(false));
    let waited = reaped.clone();

    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(16);
    std::thread::Builder::new().name("acpira-term-read".into()).spawn(move || {
      let mut buf = vec![0u8; 16 * 1024];
      loop {
        match reader.read(&mut buf) {
          Ok(0) => break,
          Ok(n) => {
            if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
              break;
            }
          }
          Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
          // EIO once the slave closed (Linux), or the pty went away
          Err(_) => break,
        }
      }
    })?;
    let (in_tx, in_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new().name("acpira-term-write".into()).spawn(move || {
      while let Ok(bytes) = in_rx.recv() {
        if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
          break;
        }
      }
    })?;
    let (exit_tx, exit_rx) = oneshot::channel::<TerminalExit>();
    std::thread::Builder::new().name("acpira-term-wait".into()).spawn(move || {
      let exit = match child.wait() {
        Ok(s) => TerminalExit { code: s.exit_code() as i64, signal: s.signal().map(str::to_owned) },
        Err(_) => TerminalExit { code: -1, signal: None },
      };
      waited.store(true, Ordering::Release);
      let _ = exit_tx.send(exit);
    })?;

    let term = Arc::new(Term {
      id: id.to_owned(),
      state: Mutex::new(TermState {
        viewers: HashMap::new(),
        scrollback: String::new(),
        title: shell.clone(),
        cols,
        rows,
        exit: None,
        orphaned: Some(Instant::now()),
        paused: None,
        ended: false,
      }),
      input: in_tx,
      master: Mutex::new(Some(pair.master)),
      killer: Mutex::new(killer),
      remote: None,
      pid,
      reaped,
      resume: Notify::new(),
      shell,
    });
    tokio::spawn(pump(term.clone(), out_rx, exit_rx));
    Ok(term)
  }

  /// A shell on the folder's SSH host (`remote::helper` `pty`), driven by one task: the helper's output into the same
  /// pump as a local shell's (a lagging page holds the helper's credit, so the shell waits on its writes there), input
  /// and resizes out to it, the end when the call ends. A host that cannot be reached shows why and ends the terminal
  fn spawn_remote(&self, id: &str, folder: acpira_shared::remote::RemoteFolder, (cols, rows): (u16, u16)) -> anyhow::Result<Arc<Term>> {
    let connections = self.remote.get().cloned().ok_or_else(|| anyhow::anyhow!("this engine has no SSH connections"))?;
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(16);
    let (exit_tx, exit_rx) = oneshot::channel::<TerminalExit>();
    let (ctl, mut ctl_rx) = mpsc::unbounded_channel::<RemoteCtl>();
    let title: Arc<Mutex<Option<String>>> = Arc::default();
    // Input arrives on the same channel type as a local shell's
    let (in_tx, in_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let forward = ctl.clone();
    std::thread::Builder::new().name("acpira-term-remote-in".into()).spawn(move || {
      while let Ok(bytes) = in_rx.recv() {
        if forward.send(RemoteCtl::Input(bytes)).is_err() {
          break;
        }
      }
    })?;
    let (alias, seen, program) = (folder.alias.clone(), title.clone(), self.program.clone());
    tokio::spawn(async move {
      let failed = |why: String| format!("\r\n\x1b[31m{why}\x1b[0m\r\n").into_bytes();
      let client = match connections.ensure(&alias, REMOTE_CONNECT).await {
        Ok(c) => c,
        Err(e) => {
          let _ = out_tx.send(failed(format!("{alias}: {e:#}"))).await;
          let _ = exit_tx.send(TerminalExit { code: -1, signal: None });
          return;
        }
      };
      let mut a = serde_json::json!({ "cwd": folder.path, "cols": cols, "rows": rows });
      if let Some(p) = &program {
        a["program"] = serde_json::json!(p);
      }
      let mut call = client.request("pty", a);
      let pty = call.id();
      let exit = loop {
        tokio::select! {
          ev = call.next() => match ev {
            Some(crate::remote::client::Event::Chunk { stream, bytes }) if stream == "title" => {
              *seen.lock() = Some(format!("{} ({alias})", String::from_utf8_lossy(&bytes)));
            }
            Some(crate::remote::client::Event::Chunk { bytes, .. }) => {
              if out_tx.send(bytes).await.is_err() {
                break None;
              }
            }
            Some(crate::remote::client::Event::Done(Ok(v))) => {
              break Some(TerminalExit { code: v.get("code").and_then(serde_json::Value::as_i64).unwrap_or(-1), signal: v.get("signal").and_then(serde_json::Value::as_str).map(str::to_owned) });
            }
            Some(crate::remote::client::Event::Done(Err(e))) => {
              let _ = out_tx.send(failed(format!("{alias}: {e}"))).await;
              break Some(TerminalExit { code: -1, signal: None });
            }
            None => break Some(TerminalExit { code: -1, signal: None }),
          },
          c = ctl_rx.recv() => match c {
            Some(RemoteCtl::Input(bytes)) => call.stdin(&bytes),
            Some(RemoteCtl::Resize(cols, rows)) => {
              let client = client.clone();
              tokio::spawn(async move {
                let _ = client.call("ptyResize", serde_json::json!({ "pty": pty, "cols": cols, "rows": rows })).await;
              });
            }
            // Killed here: dropping the call ends the shell on the host
            Some(RemoteCtl::Stop) | None => break None,
          },
        }
      };
      drop(call);
      if let Some(exit) = exit {
        let _ = exit_tx.send(exit);
      }
    });
    let shell = folder.alias.clone();
    let term = Arc::new(Term {
      id: id.to_owned(),
      state: Mutex::new(TermState {
        viewers: HashMap::new(),
        scrollback: String::new(),
        title: shell.clone(),
        cols,
        rows,
        exit: None,
        orphaned: Some(Instant::now()),
        paused: None,
        ended: false,
      }),
      input: in_tx,
      master: Mutex::new(None),
      killer: Mutex::new(Box::new(RemoteKiller(ctl.clone()))),
      remote: Some(RemoteTerm { ctl, title }),
      pid: None,
      reaped: Arc::new(AtomicBool::new(false)),
      resume: Notify::new(),
      shell,
    });
    tokio::spawn(pump(term.clone(), out_rx, exit_rx));
    Ok(term)
  }

  fn start_ticker(&self) {
    let (me, period) = (self.me.clone(), self.tick);
    tokio::spawn(async move {
      let mut tick = tokio::time::interval(period);
      tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
      loop {
        tick.tick().await;
        let Some(hub) = me.upgrade() else { return };
        if !hub.tick() {
          return;
        }
      }
    });
  }

  /// One round of the ticker: titles of the terminals in view, the end of those nobody viewed for `grace`. False once
  /// no terminal is left (the ticker stops; the next one opened starts it again)
  fn tick(&self) -> bool {
    let (terms, stale): (Vec<_>, Vec<_>) = {
      let mut hub = self.terms.lock();
      if hub.terms.is_empty() {
        hub.ticking = false;
        return false;
      }
      let stale = hub.terms.iter().filter(|(_, t)| t.orphaned_for().is_some_and(|d| d >= self.grace)).map(|(id, _)| id.clone()).collect();
      (hub.terms.values().cloned().collect(), stale)
    };
    for id in stale {
      self.kill(&id);
    }
    for t in terms {
      t.refresh_title();
    }
    true
  }
}

struct Term {
  id: String,
  state: Mutex<TermState>,
  input: std::sync::mpsc::Sender<Vec<u8>>,
  /// Taken (and dropped) when the terminal is killed
  master: Mutex<Option<Box<dyn MasterPty + Send>>>,
  killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
  /// A shell on an SSH host: its task, and the foreground name the helper last sent
  remote: Option<RemoteTerm>,
  #[cfg_attr(not(unix), allow(dead_code, reason = "only Unix reads the shell's pid"))]
  pid: Option<u32>,
  reaped: Arc<AtomicBool>,
  /// Flow control resumed, or the terminal ended: the paused pump goes on
  resume: Notify,
  /// The shell's name: the title while the shell itself is in the foreground (and where none can be told)
  shell: String,
}

struct TermState {
  viewers: HashMap<u64, Viewer>,
  scrollback: String,
  title: String,
  cols: u16,
  rows: u16,
  exit: Option<TerminalExit>,
  /// Since when no page views it (None while one does)
  orphaned: Option<Instant>,
  /// Since when the pump waits for the pages to catch up
  paused: Option<Instant>,
  /// Killed: nothing is posted anymore and the pump stops
  ended: bool,
}

struct Viewer {
  sink: Sink,
  /// Output posted to this page and not acknowledged yet, in UTF-16 units (what the page counts)
  unacked: u64,
}

impl Term {
  /// A later change of the state: the title, or the exit
  fn update(&self, title: Option<String>, exit: Option<TerminalExit>) -> HostMsg {
    HostMsg::TerminalState { id: self.id.clone(), title, cols: None, rows: None, replay: None, exit, error: None }
  }

  /// The page's view starts over with the state and the output so far, posted under the lock so no output can slip
  /// between the replay and what follows it
  fn attach(&self, view: u64, sink: Sink) {
    let mut st = self.state.lock();
    if st.ended {
      return;
    }
    sink(HostMsg::TerminalState {
      id: self.id.clone(),
      title: Some(st.title.clone()),
      cols: Some(st.cols),
      rows: Some(st.rows),
      replay: Some(st.scrollback.clone()),
      exit: st.exit.clone(),
      error: None,
    });
    st.viewers.insert(view, Viewer { sink, unacked: 0 });
    st.orphaned = None;
    self.check_resume(&mut st);
  }

  fn detach(&self, view: u64) {
    let mut st = self.state.lock();
    if st.viewers.remove(&view).is_none() {
      return;
    }
    if st.viewers.is_empty() {
      st.orphaned = Some(Instant::now());
    }
    self.check_resume(&mut st);
  }

  fn ack(&self, view: u64, chars: u64) {
    let mut st = self.state.lock();
    if let Some(v) = st.viewers.get_mut(&view) {
      v.unacked = v.unacked.saturating_sub(chars);
    }
    self.check_resume(&mut st);
  }

  /// Paused output goes on once every page caught up (or none views it, the output then only kept)
  fn check_resume(&self, st: &mut TermState) {
    if st.paused.is_some() && st.viewers.values().all(|v| v.unacked <= LOW_WATER) {
      st.paused = None;
      self.resume.notify_one();
    }
  }

  fn output(&self, data: String) {
    let mut st = self.state.lock();
    if st.ended {
      return;
    }
    keep_tail(&mut st.scrollback, &data, SCROLLBACK);
    if st.viewers.is_empty() {
      return;
    }
    let units = data.encode_utf16().count() as u64;
    for v in st.viewers.values_mut() {
      v.unacked += units;
      (v.sink)(HostMsg::TerminalOutput { id: self.id.clone(), data: data.clone() });
    }
    if st.paused.is_none() && st.viewers.values().any(|v| v.unacked > HIGH_WATER) {
      st.paused = Some(Instant::now());
    }
  }

  /// How long to wait before reading on: None when not paused. A pause past the valve is lifted here, the unanswered
  /// output written off
  fn pause_left(&self) -> Option<Duration> {
    let mut st = self.state.lock();
    let since = st.paused?;
    if st.ended {
      return None;
    }
    let left = ACK_VALVE.saturating_sub(since.elapsed());
    if left.is_zero() {
      for v in st.viewers.values_mut() {
        v.unacked = 0;
      }
      st.paused = None;
      return None;
    }
    Some(left)
  }

  fn exited(&self, exit: TerminalExit) {
    let mut st = self.state.lock();
    if st.ended {
      return;
    }
    st.exit = Some(exit.clone());
    for v in st.viewers.values() {
      (v.sink)(self.update(None, Some(exit.clone())));
    }
  }

  fn resize(&self, cols: u16, rows: u16) {
    let (cols, rows) = clamp_size(cols, rows);
    {
      let mut st = self.state.lock();
      if st.ended || (st.cols, st.rows) == (cols, rows) {
        return;
      }
      st.cols = cols;
      st.rows = rows;
    }
    if let Some(m) = self.master.lock().as_ref() {
      let _ = m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }
    if let Some(r) = &self.remote {
      let _ = r.ctl.send(RemoteCtl::Resize(cols, rows));
    }
  }

  /// The foreground process as the title, posted when it changed and a page views the terminal
  fn refresh_title(&self) {
    if self.state.lock().viewers.is_empty() {
      return;
    }
    let title = self.foreground().unwrap_or_else(|| self.shell.clone());
    let mut st = self.state.lock();
    if st.ended || st.exit.is_some() || st.title == title {
      return;
    }
    st.title = title.clone();
    for v in st.viewers.values() {
      (v.sink)(self.update(Some(title.clone()), None));
    }
  }

  #[cfg(unix)]
  fn foreground(&self) -> Option<String> {
    if let Some(r) = &self.remote {
      return r.title.lock().clone();
    }
    let pid = self.master.lock().as_ref()?.process_group_leader()?;
    process_name(pid)
  }

  #[cfg(not(unix))]
  fn foreground(&self) -> Option<String> {
    self.remote.as_ref().and_then(|r| r.title.lock().clone())
  }

  fn orphaned_for(&self) -> Option<Duration> {
    self.state.lock().orphaned.map(|at| at.elapsed())
  }

  /// SIGHUP to the shell (it passes it on to its jobs), as closing a terminal window does, and SIGKILL when it ignores
  /// that; the pty closes with the last process holding it
  fn shutdown(&self) {
    {
      let mut st = self.state.lock();
      st.ended = true;
      st.viewers.clear();
    }
    self.resume.notify_one();
    if !self.reaped.load(Ordering::Acquire) {
      let _ = self.killer.lock().kill();
      #[cfg(unix)]
      if let Some(pid) = self.pid {
        let reaped = self.reaped.clone();
        let _ = std::thread::Builder::new().name("acpira-term-kill".into()).spawn(move || {
          let start = Instant::now();
          while start.elapsed() < KILL_GRACE {
            std::thread::sleep(Duration::from_millis(100));
            if reaped.load(Ordering::Acquire) {
              return;
            }
          }
          // SAFETY: the shell is not reaped yet, so the pid is still its own
          unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        });
      }
    }
    drop(self.master.lock().take());
  }
}

/// How long a terminal in a folder on an SSH host waits for the host's connection
const REMOTE_CONNECT: Duration = Duration::from_secs(30);

struct RemoteTerm {
  ctl: mpsc::UnboundedSender<RemoteCtl>,
  title: Arc<Mutex<Option<String>>>,
}

enum RemoteCtl {
  Input(Vec<u8>),
  Resize(u16, u16),
  Stop,
}

/// A remote shell's kill: its task drops the helper call, which ends the shell on the host
struct RemoteKiller(mpsc::UnboundedSender<RemoteCtl>);

impl std::fmt::Debug for RemoteKiller {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("RemoteKiller")
  }
}

impl ChildKiller for RemoteKiller {
  fn kill(&mut self) -> std::io::Result<()> {
    let _ = self.0.send(RemoteCtl::Stop);
    Ok(())
  }

  fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
    Box::new(RemoteKiller(self.0.clone()))
  }
}

/// The terminal's output into its pages: chunks gathered for `BATCH` after the first, paused while a page lags, the exit
/// posted after the output before it
async fn pump(term: Arc<Term>, mut rx: mpsc::Receiver<Vec<u8>>, mut exit_rx: oneshot::Receiver<TerminalExit>) {
  let mut utf8 = Utf8Stream::default();
  let (mut eof, mut exited) = (false, false);
  loop {
    while let Some(left) = term.pause_left() {
      let _ = tokio::time::timeout(left, term.resume.notified()).await;
    }
    if term.state.lock().ended {
      return;
    }
    tokio::select! {
      chunk = rx.recv(), if !eof => match chunk {
        Some(first) => {
          let mut out = utf8.push(&first);
          let deadline = tokio::time::Instant::now() + BATCH;
          while out.len() < BATCH_MAX {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
              Ok(Some(more)) => out.push_str(&utf8.push(&more)),
              Ok(None) => {
                eof = true;
                out.push_str(&utf8.finish());
                break;
              }
              Err(_) => break,
            }
          }
          if !out.is_empty() {
            term.output(out);
          }
        }
        None => {
          eof = true;
          let tail = utf8.finish();
          if !tail.is_empty() {
            term.output(tail);
          }
        }
      },
      exit = &mut exit_rx, if !exited => {
        exited = true;
        let deadline = tokio::time::Instant::now() + EXIT_DRAIN;
        let mut out = String::new();
        while !eof {
          match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(more)) => out.push_str(&utf8.push(&more)),
            Ok(None) => {
              eof = true;
              out.push_str(&utf8.finish());
            }
            Err(_) => break,
          }
        }
        if !out.is_empty() {
          term.output(out);
        }
        term.exited(exit.unwrap_or(TerminalExit { code: -1, signal: None }));
      }
    }
    if eof && exited {
      return;
    }
  }
}

/// The shell as a terminal emulator starts it: the user's login shell (PowerShell on Windows) in `cwd` (home when
/// omitted), the variables a terminal sets, none of the engine's own host's
pub(crate) fn command(program: Option<&[String]>, cwd: Option<&str>) -> anyhow::Result<CommandBuilder> {
  if let Some(cwd) = cwd {
    // portable-pty silently substitutes home for a missing cwd; reject invalid folders on both local and SSH starts.
    anyhow::ensure!(!acpira_shared::remote::is_remote_folder(cwd), "invalid SSH terminal folder: {cwd}");
    anyhow::ensure!(
      Path::new(cwd).is_absolute() && Path::new(cwd).is_dir(),
      "terminal folder must be an existing absolute directory: {cwd}"
    );
  }
  let mut cmd = match program {
    Some([bin, args @ ..]) => {
      let mut c = CommandBuilder::new(bin);
      c.args(args);
      c
    }
    _ => default_shell(),
  };
  match cwd {
    Some(dir) => cmd.cwd(dir),
    None => cmd.cwd(crate::store::data_dir::home_dir()),
  }
  // What the engine inherited from the app or terminal that started it: Electron's switches, VS Code's terminal hooks,
  // another emulator's identity (tools pick image protocols and key handling by it)
  let inherited: Vec<String> = cmd.iter_full_env_as_str().map(|(k, _)| k.to_owned()).collect();
  for key in inherited {
    if key.starts_with("ELECTRON_") || key.starts_with("VSCODE_") || INHERITED_TERMINAL_VARS.contains(&key.as_str()) {
      cmd.env_remove(key);
    }
  }
  cmd.env("TERM", "xterm-256color");
  cmd.env("COLORTERM", "truecolor");
  cmd.env("TERM_PROGRAM", "Acpira");
  cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
  // A GUI app starts without a locale, and a shell without one takes UTF-8 for bytes (VS Code's detectLocale does the same)
  if ["LANG", "LC_ALL", "LC_CTYPE"].iter().all(|k| cmd.get_env(k).is_none_or(|v| v.is_empty())) {
    cmd.env("LANG", "en_US.UTF-8");
  }
  // The proxy setting, as for an agent; a definition of the user's own in the shell's rc files still wins
  for (k, v) in crate::net_proxy::terminal_env() {
    match v {
      Some(v) => cmd.env(k, v),
      None => cmd.env_remove(k),
    }
  }
  Ok(cmd)
}

const INHERITED_TERMINAL_VARS: &[&str] = &[
  "TERM_SESSION_ID",
  "ITERM_SESSION_ID",
  "ITERM_PROFILE",
  "LC_TERMINAL",
  "LC_TERMINAL_VERSION",
  "COLORFGBG",
  "WEZTERM_PANE",
  "WEZTERM_EXECUTABLE",
  "KITTY_WINDOW_ID",
  "GHOSTTY_RESOURCES_DIR",
  "WT_SESSION",
];

#[cfg(not(windows))]
fn default_shell() -> CommandBuilder {
  CommandBuilder::new_default_prog()
}

/// Windows PowerShell, which every Windows since 7 has; cmd.exe (COMSPEC) only without it
#[cfg(windows)]
fn default_shell() -> CommandBuilder {
  let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
  let ps = Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
  if ps.is_file() { CommandBuilder::new(ps) } else { CommandBuilder::new_default_prog() }
}

pub(crate) fn shell_name(cmd: &CommandBuilder) -> String {
  let path = if cmd.is_default_prog() { cmd.get_shell() } else { cmd.get_argv().first().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default() };
  program_name(&path)
}

/// `/bin/zsh` → `zsh`, `-zsh` (a login shell's argv0) → `zsh`, `powershell.exe` → `powershell`
fn program_name(path: &str) -> String {
  let base = path.rsplit(['/', '\\']).next().unwrap_or(path).trim_start_matches('-');
  let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
  base.to_owned()
}

#[cfg(target_os = "macos")]
pub(crate) fn process_name(pid: i32) -> Option<String> {
  let mut buf = [0u8; 256];
  // SAFETY: proc_name writes at most `buffersize` bytes into the buffer and returns how many it wrote
  let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
  (n > 0).then(|| program_name(&String::from_utf8_lossy(&buf[..n as usize])))
}

#[cfg(target_os = "linux")]
pub(crate) fn process_name(pid: i32) -> Option<String> {
  let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
  Some(program_name(comm.trim_end()))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
pub(crate) fn process_name(_pid: i32) -> Option<String> {
  None
}

/// A page asks for its laid-out size; a hidden or collapsed one can measure nothing, which a pty must not take
pub(crate) fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
  (cols.clamp(2, 1000), rows.clamp(1, 1000))
}

/// Bytes into text across chunk edges: a character split between two reads waits for its rest, a byte that can never be
/// UTF-8 turns into U+FFFD
#[derive(Default)]
struct Utf8Stream {
  tail: Vec<u8>,
}

impl Utf8Stream {
  fn push(&mut self, bytes: &[u8]) -> String {
    let joined;
    let mut rest: &[u8] = if self.tail.is_empty() {
      bytes
    } else {
      let mut b = std::mem::take(&mut self.tail);
      b.extend_from_slice(bytes);
      joined = b;
      &joined
    };
    let mut out = String::with_capacity(rest.len());
    loop {
      match std::str::from_utf8(rest) {
        Ok(s) => {
          out.push_str(s);
          return out;
        }
        Err(e) => {
          let (valid, after) = rest.split_at(e.valid_up_to());
          // SAFETY: `valid_up_to` is where the valid prefix ends
          out.push_str(unsafe { std::str::from_utf8_unchecked(valid) });
          match e.error_len() {
            Some(n) => {
              out.push('\u{FFFD}');
              rest = &after[n..];
            }
            None => {
              self.tail = after.to_vec();
              return out;
            }
          }
        }
      }
    }
  }

  /// The output ended: an unfinished character is lost
  fn finish(&mut self) -> String {
    if std::mem::take(&mut self.tail).is_empty() { String::new() } else { "\u{FFFD}".into() }
  }
}

/// Append `add`, keeping about the last `max` bytes once a quarter over; the kept part starts on a line when one starts
/// soon after the cut, so a replay rarely opens inside an escape sequence
fn keep_tail(buf: &mut String, add: &str, max: usize) {
  buf.push_str(add);
  if buf.len() <= max + max / 4 {
    return;
  }
  let mut cut = buf.len() - max;
  while !buf.is_char_boundary(cut) {
    cut += 1;
  }
  if let Some(nl) = buf.get(cut..).and_then(|t| t.find('\n')).filter(|&i| i < max / 16) {
    cut += nl + 1;
  }
  buf.drain(..cut);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn utf8_stream_joins_split_characters_and_replaces_bad_bytes() {
    let mut s = Utf8Stream::default();
    let text = "终端 ok";
    let bytes = text.as_bytes();
    // Split inside the first character's three bytes
    assert_eq!(s.push(&bytes[..1]), "");
    assert_eq!(s.push(&bytes[1..4]), "终");
    assert_eq!(s.push(&bytes[4..]), "端 ok");
    assert_eq!(s.push(&[b'a', 0xff, b'b']), "a\u{FFFD}b");
    assert_eq!(s.push(&[0xe7, 0xbb]), "");
    assert_eq!(s.finish(), "\u{FFFD}");
    assert_eq!(s.finish(), "");
  }

  #[test]
  fn keep_tail_trims_to_a_line_start() {
    let mut buf = String::new();
    keep_tail(&mut buf, "aaaa\nbbbb\n", 16);
    assert_eq!(buf, "aaaa\nbbbb\n");
    keep_tail(&mut buf, "cccc\ndddd\neeee\n", 16);
    // 25 bytes > 16 + 4: cut at 9 (inside "bbbb"), on to the next line start
    assert_eq!(buf, "cccc\ndddd\neeee\n");
    let mut wide = String::new();
    keep_tail(&mut wide, &"终".repeat(10), 8);
    assert!(wide.len() <= 10 && wide.chars().all(|c| c == '终'));
  }

  #[test]
  fn program_names() {
    assert_eq!(program_name("/bin/zsh"), "zsh");
    assert_eq!(program_name("-zsh"), "zsh");
    assert_eq!(program_name(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"), "powershell");
  }

  #[cfg(unix)]
  mod pty {
    use super::super::*;

    type Log = Arc<Mutex<Vec<HostMsg>>>;

    fn hub(script: &str) -> Arc<TerminalHub> {
      TerminalHub::with(Some(vec!["/bin/sh".into(), "-c".into(), script.into()]), Duration::from_millis(300), Duration::from_millis(50))
    }

    fn sink() -> (Log, Sink) {
      let log: Log = Default::default();
      let l = log.clone();
      (log, Arc::new(move |m| l.lock().push(m)))
    }

    fn output(log: &Log) -> String {
      log
        .lock()
        .iter()
        .map(|m| match m {
          HostMsg::TerminalOutput { data, .. } => data.as_str(),
          HostMsg::TerminalState { replay: Some(r), .. } => r.as_str(),
          _ => "",
        })
        .collect()
    }

    async fn until(what: &str, mut cond: impl FnMut() -> bool) {
      let start = Instant::now();
      while !cond() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
      }
    }

    fn exit_of(log: &Log) -> Option<TerminalExit> {
      log.lock().iter().find_map(|m| match m {
        HostMsg::TerminalState { exit: Some(e), .. } => Some(e.clone()),
        _ => None,
      })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn input_reaches_the_shell_and_the_exit_follows_the_output() {
      let h = hub("printf ready; read line; printf 'got:%s' \"$line\"; exit 3");
      let (log, s) = sink();
      h.open(1, s, "t1", None, Some((100, 30)));
      assert!(matches!(&log.lock()[0], HostMsg::TerminalState { cols: Some(100), rows: Some(30), replay: Some(_), .. }));
      until("the prompt", || output(&log).contains("ready")).await;
      h.input("t1", "hello\r".into(), false);
      until("the exit", || exit_of(&log).is_some()).await;
      assert!(output(&log).contains("got:hello"));
      assert_eq!(exit_of(&log), Some(TerminalExit { code: 3, signal: None }));
      // The exit is posted after the output before it
      let last = log.lock().iter().rposition(|m| matches!(m, HostMsg::TerminalOutput { .. })).unwrap();
      let exit = log.lock().iter().position(|m| matches!(m, HostMsg::TerminalState { exit: Some(_), .. })).unwrap();
      assert!(exit > last);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_coming_back_gets_the_output_so_far_and_the_size() {
      let h = hub("printf 'line one\\n'; stty size; read x; stty size; sleep 5");
      let (log, s) = sink();
      h.open(1, s, "t2", None, Some((90, 20)));
      until("the size", || output(&log).contains("20 90")).await;
      h.close(1, "t2");
      let (again, s2) = sink();
      h.open(2, s2, "t2", None, None);
      match &again.lock()[0] {
        HostMsg::TerminalState { replay: Some(r), cols, rows, title, .. } => {
          assert!(r.contains("line one") && r.contains("20 90"), "{r:?}");
          assert_eq!((*cols, *rows), (Some(90), Some(20)));
          assert!(title.is_some());
        }
        m => panic!("{m:?}"),
      }
      h.resize("t2", 120, 40);
      h.input("t2", "\r".into(), false);
      until("the new size", || output(&again).contains("40 120")).await;
      // The page that left heard nothing after it left
      assert!(!output(&log).contains("40 120"));
      h.kill("t2");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn output_pauses_for_a_page_that_does_not_keep_up() {
      let h = hub("i=0; while [ $i -lt 4000 ]; do echo 0123456789012345678901234567890123456789012345678901234567890123456789; i=$((i+1)); done; echo done");
      let (log, s) = sink();
      h.open(1, s, "t3", None, None);
      until("a pause", || h.get("t3").is_some_and(|t| t.state.lock().paused.is_some())).await;
      tokio::time::sleep(Duration::from_millis(200)).await;
      let units = |log: &Log| -> u64 {
        log.lock().iter().map(|m| if let HostMsg::TerminalOutput { data, .. } = m { data.encode_utf16().count() as u64 } else { 0 }).sum()
      };
      let held = units(&log);
      assert!(held > HIGH_WATER && held < HIGH_WATER + 2 * BATCH_MAX as u64, "{held}");
      assert!(!output(&log).contains("done"));
      // Acknowledging all of it lets the rest through
      let mut acked = 0;
      until("the rest", || {
        let now = units(&log);
        if now > acked {
          h.ack(1, "t3", now - acked);
          acked = now;
        }
        output(&log).contains("done")
      })
      .await;
      h.kill("t3");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_ends_the_shell_and_an_unviewed_terminal_ends_on_its_own() {
      let h = hub("echo pid:$$; sleep 30");
      let (log, s) = sink();
      h.open(1, s, "t4", None, None);
      until("the pid", || output(&log).contains("pid:")).await;
      let text = output(&log);
      let pid: i32 = text.split("pid:").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
      h.kill("t4");
      assert!(h.get("t4").is_none());
      // SAFETY: signal 0 only checks that the process exists
      until("the shell to end", || unsafe { libc::kill(pid, 0) } != 0).await;
      let n = log.lock().len();
      h.input("t4", "x".into(), false);
      tokio::time::sleep(Duration::from_millis(50)).await;
      assert_eq!(log.lock().len(), n);

      let (_log, s) = sink();
      h.open(7, s, "t5", None, None);
      h.detach_view(7);
      until("the orphan to end", || h.get("t5").is_none()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_title_follows_the_foreground_process() {
      let h = TerminalHub::with(Some(vec!["/bin/sh".into(), "-c".into(), "exec sleep 5".into()]), Duration::from_secs(60), Duration::from_millis(50));
      let (log, s) = sink();
      h.open(1, s, "t6", None, None);
      assert!(matches!(&log.lock()[0], HostMsg::TerminalState { title: Some(t), .. } if t == "sh"));
      if cfg!(any(target_os = "macos", target_os = "linux")) {
        until("the title", || log.lock().iter().any(|m| matches!(m, HostMsg::TerminalState { title: Some(t), replay: None, .. } if t == "sleep"))).await;
      }
      h.dispose();
      assert!(h.get("t6").is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_invalid_starting_folder_is_reported_without_starting_a_shell() {
      let h = hub("echo must-not-start");
      let dir = tempfile::tempdir().unwrap();
      let file = dir.path().join("file");
      std::fs::write(&file, "file").unwrap();
      for cwd in [
        dir.path().join("missing").to_string_lossy().into_owned(),
        file.to_string_lossy().into_owned(),
        ".".into(),
        "".into(),
        "ssh://host/project/../elsewhere".into(),
        "ssh://-option/project".into(),
      ] {
        let (log, s) = sink();
        h.open(1, s, "invalid", Some(&cwd), None);
        assert!(matches!(&log.lock()[0], HostMsg::TerminalState { error: Some(_), .. }), "cwd: {cwd}");
        assert!(h.get("invalid").is_none(), "cwd: {cwd}");
      }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_shell_that_cannot_start_says_why() {
      let h = TerminalHub::with(Some(vec!["/nonexistent/acpira-shell".into()]), Duration::from_secs(60), Duration::from_millis(50));
      let (log, s) = sink();
      h.open(1, s, "t7", None, None);
      assert!(matches!(&log.lock()[0], HostMsg::TerminalState { error: Some(_), .. }));
      assert!(h.get("t7").is_none());
    }
  }
}
