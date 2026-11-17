//! Plugin processes: started or swapped as the active set changes, their events pumped, stderr kept for errors, and a crashed one restarted with backoff

use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use acpira_rpc::rpc::{Connection, Inbound};

use super::inbound::PluginInbound;
use super::{INIT_TIMEOUT, PROTOCOL_VERSION, Proc, Runtimes, Want, Worker};
use crate::plugins::host::Active;

impl Runtimes {
  /// Bring the processes in line with the plugins in effect: start what is new or changed (the old one is stopped
  /// once the new one answered `initialize`), stop what is gone. Safe mode passes nothing, so everything stops
  pub async fn sync(self: &Arc<Self>, active: &[Arc<Active>]) {
    let wanted: Vec<Want> = active
      .iter()
      .filter_map(|a| {
        Want::of(&a.id, &a.hash, &a.dir, &a.manifest)
      })
      .collect();
    *self.present.write() = active.iter().map(|a| (a.id.clone(), a.manifest.version.clone())).collect();
    *self.wanted.lock() = wanted;
    if !self.enabled() {
      return;
    }
    self.apply().await;
  }

  /// One pass at a time, on what is wanted when it gets the turn: a pass queued behind another never brings back a
  /// plugin switched off meanwhile
  async fn apply(self: &Arc<Self>) {
    let _one = self.sync.lock().await;
    if !self.enabled() {
      return;
    }
    let wanted = self.wanted.lock().clone();
    let mut stopped = vec![];
    for want in &wanted {
      let current = self.workers.read().get(&want.id).cloned();
      if current.as_ref().is_some_and(|w| w.want.hash == want.hash && !w.conn.is_closed() && w.deps == self.dependencies_of(want)) {
        continue;
      }
      match self.start(want).await {
        Ok(worker) => {
          self.errors.write().remove(&want.id);
          if let Some(old) = self.workers.write().insert(want.id.clone(), worker) {
            stopped.push(old);
          }
        }
        // A new version that does not start leaves the running one in place; with none running, it is tried again
        Err(e) if current.as_ref().is_some_and(|w| !w.conn.is_closed()) => {
          (self.log)(&format!("plugin {}: version {} did not start, the running one stays: {e}", want.id, want.version));
          self.errors.write().insert(want.id.clone(), format!("the new version did not start, the previous one keeps running: {e}"));
        }
        Err(e) => {
          (self.log)(&format!("plugin {}: the runtime did not start: {e}", want.id));
          self.errors.write().insert(want.id.clone(), format!("the runtime did not start: {e}"));
          if let Some(old) = self.workers.write().remove(&want.id) {
            stopped.push(old);
          }
          self.schedule_restart(&want.id);
        }
      }
    }
    let gone: Vec<String> = self.workers.read().keys().filter(|id| !wanted.iter().any(|w| &w.id == *id)).cloned().collect();
    let dropped = {
      let mut live = self.live_providers.write();
      let before = live.len();
      live.retain(|id, _| wanted.iter().any(|w| &w.id == id));
      live.len() != before
    };
    if dropped {
      self.write_providers();
    }
    for id in gone {
      self.errors.write().remove(&id);
      self.restarts.lock().remove(&id);
      if let Some(old) = self.workers.write().remove(&id) {
        stopped.push(old);
      }
    }
    for w in stopped {
      w.stop().await;
    }
  }

  /// Stop every process (the engine is shutting down)
  pub async fn stop_all(&self) {
    self.enabled.store(false, Ordering::Relaxed);
    // After a pass under way: it would otherwise put back a worker it was starting
    let _one = self.sync.lock().await;
    let workers: Vec<Arc<Worker>> = std::mem::take(&mut *self.workers.write()).into_values().collect();
    for w in workers {
      w.stop().await;
    }
  }

  async fn start(self: &Arc<Self>, want: &Want) -> Result<Arc<Worker>, String> {
    self.start_as(want, false).await
  }

  /// `probe`: the activation check, which only proves the process starts and speaks the protocol; it is told so, and
  /// a definition then runs neither `migrate` nor `activate` (no data changes for a version not yet confirmed)
  pub(super) async fn start_as(self: &Arc<Self>, want: &Want, probe: bool) -> Result<Arc<Worker>, String> {
    let data = self.data_dir(&want.id);
    // A `main` module runs compiled (`build.rs`); the output is rebuilt when missing, from the frozen copy alone
    let command = match want.def.main.as_deref() {
      Some(main) => {
        let root = self.data_root.parent().unwrap_or(&self.data_root);
        let out = crate::plugins::build::output_dir(root, &want.id, &want.hash);
        let (dir, main) = (want.dir.clone(), main.to_owned());
        let boot = tokio::task::spawn_blocking(move || crate::plugins::build::runtime(&dir, &main, &out)).await.map_err(|e| e.to_string())??;
        vec!["node".to_owned(), boot.to_string_lossy().into_owned()]
      }
      None => want.def.command.clone(),
    };
    let mut cmd = crate::plugins::run::command(&command, &want.dir, &data, "").await?;
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    let (mut child, job) = crate::platform::windows_process::spawn(&mut cmd).await.map_err(|e| format!("{e}"))?;
    #[cfg(not(windows))]
    let mut child = cmd.spawn().map_err(|e| format!("{e}"))?;
    let pid = child.id();
    let stdin = child.stdin.take().ok_or("stdin is not piped")?;
    let stdout = child.stdout.take().ok_or("stdout is not piped")?;
    let stderr = child.stderr.take().ok_or("stderr is not piped")?;
    let tail = self.pipe_stderr(&want.id, stderr);
    let inbound: Arc<dyn Inbound> = Arc::new(PluginInbound { id: want.id.clone(), kv: self.kv(&want.id), runtimes: Arc::downgrade(self), want: want.clone() });
    let log = self.log.clone();
    let id = want.id.clone();
    let conn = Connection::start(stdout, stdin, Arc::new(move || inbound.clone()), Arc::new(move |l: &str| log(&format!("plugin {id}: {l}"))));
    let proc = Proc {
      child: parking_lot::Mutex::new(Some(child)),
      pid,
      #[cfg(windows)]
      job,
    };
    let deps = self.dependencies_of(want);
    let mut init = json!({
      "protocolVersion": PROTOCOL_VERSION,
      "plugin": { "id": want.id, "version": want.version, "snapshot": want.hash, "dir": want.dir, "data": data },
      "engine": { "name": "acpira", "version": acpira_shared::version::VERSION },
      "hooks": want.hooks(),
      "events": want.def.events,
      "methods": want.def.methods,
      "tools": want.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
      "schedules": want.schedules.iter().map(|j| j.id.clone()).collect::<Vec<_>>(),
      "dependencies": deps,
    });
    if probe {
      init["probe"] = json!(true);
    }
    let answer = match tokio::time::timeout(INIT_TIMEOUT, conn.request("initialize", init)).await {
      Ok(Ok(v)) => v,
      Ok(Err(e)) => {
        conn.close();
        proc.stop().await;
        return Err(tail.explain(format!("initialize failed: {}", e.message)).await);
      }
      Err(_) => {
        conn.close();
        proc.stop().await;
        return Err(tail.explain(format!("no answer to initialize within {} s", INIT_TIMEOUT.as_secs())).await);
      }
    };
    if answer.get("protocolVersion").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
      conn.close();
      proc.stop().await;
      return Err(tail.explain(format!("it speaks protocol {}, this Acpira speaks {PROTOCOL_VERSION}", answer.get("protocolVersion").map(Value::to_string).unwrap_or_else(|| "none".into()))).await);
    }
    // The answer may narrow what it handles, never widen what the user confirmed
    let hooks = match answer.get("hooks").and_then(Value::as_array) {
      Some(list) => want.hooks().into_iter().filter(|h| list.iter().any(|v| v.as_str() == Some(h.as_str()))).collect(),
      None => want.hooks(),
    };
    let (tx, rx) = mpsc::channel(EVENT_QUEUE);
    let worker = Arc::new(Worker {
      want: want.clone(),
      conn,
      proc,
      hooks,
      failures: AtomicU32::new(0),
      tripped: Default::default(),
      events: tx,
      dropped: Arc::new(AtomicU64::new(0)),
      stopping: AtomicBool::new(false),
      deps,
    });
    self.pump_events(&worker, rx);
    self.watch_exit(&worker);
    Ok(worker)
  }

  /// Deliver queued events in order; the first one after a drop says how many were lost
  fn pump_events(&self, w: &Arc<Worker>, mut rx: mpsc::Receiver<Value>) {
    let conn = w.conn.clone();
    let dropped = w.dropped.clone();
    tokio::spawn(async move {
      while let Some(mut ev) = rx.recv().await {
        // A process that stops reading its stdin holds the events here, in the bounded queue, where the overflow is
        // dropped and counted, instead of in the connection's unbounded one
        while conn.backlog() > EVENT_BACKLOG && !conn.is_closed() {
          tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if conn.is_closed() {
          break;
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0
          && let Some(o) = ev.as_object_mut()
        {
          o.insert("dropped".into(), json!(lost));
        }
        conn.notify("event", ev);
      }
    });
  }

  /// A process that exits while its plugin is in effect is started again after a growing pause
  fn watch_exit(self: &Arc<Self>, w: &Arc<Worker>) {
    let me = Arc::downgrade(self);
    let weak = Arc::downgrade(w);
    let conn = w.conn.clone();
    tokio::spawn(async move {
      conn.closed().await;
      let (Some(me), Some(w)) = (me.upgrade(), weak.upgrade()) else { return };
      if w.stopping.load(Ordering::Relaxed) {
        return;
      }
      let id = w.want.id.clone();
      let still = me.workers.read().get(&id).is_some_and(|cur| Arc::ptr_eq(cur, &w));
      if !still {
        return;
      }
      // The reason goes in before the worker leaves, so nobody sees it stopped without one
      me.errors.write().insert(id.clone(), "the runtime exited; starting it again".into());
      me.workers.write().remove(&id);
      w.proc.stop().await;
      (me.log)(&format!("plugin {id}: the runtime exited"));
      me.schedule_restart(&id);
    });
  }

  fn schedule_restart(self: &Arc<Self>, id: &str) {
    let pause = {
      let mut r = self.restarts.lock();
      let e = r.entry(id.to_owned()).or_insert((0, Instant::now()));
      // A process that ran ten minutes without trouble starts over from the first pause
      if e.1.elapsed() > Duration::from_secs(600) {
        e.0 = 0;
      }
      e.1 = Instant::now();
      let pause = RESTART_FIRST.saturating_mul(1 << e.0.min(6)).min(RESTART_MAX);
      e.0 += 1;
      pause
    };
    let me = Arc::downgrade(self);
    let id = id.to_owned();
    tokio::spawn(async move {
      tokio::time::sleep(pause).await;
      let Some(me) = me.upgrade() else { return };
      if !me.enabled() || me.workers.read().contains_key(&id) {
        return;
      }
      let wanted = me.wanted.lock().iter().any(|w| w.id == id);
      if wanted {
        me.apply().await;
      }
    });
  }

  /// A process's stderr goes to the engine log, a bounded number of lines a minute
  fn pipe_stderr(&self, id: &str, stderr: tokio::process::ChildStderr) -> StderrTail {
    let log = self.log.clone();
    let id = id.to_owned();
    let tail = StderrTail::default();
    let keep = tail.clone();
    tokio::spawn(async move {
      let mut lines = BufReader::new(stderr).lines();
      let mut window = (Instant::now(), 0u32);
      while let Ok(Some(line)) = lines.next_line().await {
        keep.push(&line);
        if window.0.elapsed() > Duration::from_secs(60) {
          window = (Instant::now(), 0);
        }
        window.1 += 1;
        if window.1 <= STDERR_LINES_PER_MIN {
          log(&format!("plugin {id}: {line}"));
        } else if window.1 == STDERR_LINES_PER_MIN + 1 {
          log(&format!("plugin {id}: more stderr this minute left out"));
        }
      }
      keep.closed.notify_one();
    });
    tail
  }
}

/// Events waiting for one process
const EVENT_QUEUE: usize = 1024;

/// The pause before a process that exited is started again: doubling from the first, at most the last
const RESTART_FIRST: Duration = Duration::from_secs(1);

const RESTART_MAX: Duration = Duration::from_secs(60);

/// Stderr lines of one process kept in the engine log per minute
const STDERR_LINES_PER_MIN: u32 = 60;

/// The stderr lines a failed start's error carries (the first ones, which name the error, and the last ones), and how
/// long it waits for a dying process to finish writing
const STDERR_HEAD_LINES: usize = 8;

const STDERR_TAIL_LINES: usize = 12;

const STDERR_TAIL_WAIT: Duration = Duration::from_millis(500);

/// What a process wrote to stderr, its first and last lines. A process that dies on start says why there ("the plugin
/// failed to load: ReferenceError …", `build.rs`, then the stack), and the activation check returns its error to the
/// model that wrote the plugin, which can fix only what it can read (docs/dev/plugins.md, Activation)
#[derive(Clone, Default)]
struct StderrTail {
  lines: Arc<parking_lot::Mutex<StderrLines>>,
  closed: Arc<tokio::sync::Notify>,
}

#[derive(Default)]
struct StderrLines {
  head: Vec<String>,
  tail: std::collections::VecDeque<String>,
  /// Lines between the two, left out
  skipped: usize,
}

impl StderrTail {
  fn push(&self, line: &str) {
    let mut l = self.lines.lock();
    let line: String = line.chars().take(500).collect();
    if l.head.len() < STDERR_HEAD_LINES {
      l.head.push(line);
      return;
    }
    if l.tail.len() == STDERR_TAIL_LINES {
      l.tail.pop_front();
      l.skipped += 1;
    }
    l.tail.push_back(line);
  }

  /// `error`, followed by what the process wrote once it closed stderr (or the wait passed)
  async fn explain(&self, error: String) -> String {
    let _ = tokio::time::timeout(STDERR_TAIL_WAIT, self.closed.notified()).await;
    let l = self.lines.lock();
    if l.head.is_empty() {
      return error;
    }
    let skipped = (l.skipped > 0).then(|| format!("… {} lines left out", l.skipped));
    let text: Vec<&str> = l.head.iter().map(String::as_str).chain(skipped.as_deref()).chain(l.tail.iter().map(String::as_str)).collect();
    format!("{error}; its stderr:\n{}", text.join("\n"))
  }
}

/// Lines the connection to a process may hold unwritten before its events wait in their bounded queue
const EVENT_BACKLOG: usize = 256;
