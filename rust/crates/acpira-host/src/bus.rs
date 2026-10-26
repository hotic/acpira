//! Engine-to-engine push bus, Codex's model (one engine per shell, a router socket between them). Every engine keeps its
//! own sessions; the one holding a session's lease (`store::session_lease`) owns it and pushes its view to the engines
//! that show it read-only, which send their pages' actions on it back to the owner instead of asking for a takeover.
//! Without the bus (an engine from before it, a router being re-elected) those engines keep polling the record
//!
//! Transport: `<home>/run/bus.sock` (a named pipe named after the home on Windows), one JSON object per line. The
//! engine that locks `run/bus.lock` first serves the socket (the router) and every engine, the router's own included,
//! connects to it as a client. When the router's engine ends, the others race for the lock again and reconnect; each
//! client then announces its state afresh (the sessions it owns, the ones it subscribes to), and owners answer every
//! new subscription with a whole view
//!
//! What travels follows the sync model (docs/dev/host-architecture.md, "Sync model"): session facts from their owner
//! (views, subagent transcripts, the list entries of the sessions it runs), and a `touch` whenever a user fact in the
//! shared store changed (unread marks, categories, the list), so every other engine re-reads it at once

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use acpira_rpc::rpc::BoxFuture;
use acpira_shared::session_patch::{LastTurnParts, ViewParts, ViewPartsData, common_prefix, splice_array};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::sync::{mpsc, oneshot};

use crate::store::transcript_store::LogFn;

// Views of one session pushed within this window go out as one frame (a streamed reply changes many times a second)
const BATCH: Duration = Duration::from_millis(16);
// Between attempts to reach or become the router
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
const RETRY: Duration = Duration::from_millis(200);
// A Unix socket path longer than this does not bind on macOS (104 bytes with the terminator)
const SOCKET_PATH_MAX: usize = 100;

/// One step of a session's view on the bus: the turns after the first `keep` the receiver holds, then the live agent turn
/// as its head plus the blocks after the first `keep_blocks` of the live turn the receiver holds. `keep` 0 with
/// `keep_blocks` 0 is the whole view
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewFrame {
  pub head: Box<RawValue>,
  pub keep: usize,
  pub turns: Vec<Box<RawValue>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last: Option<LastFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFrame {
  pub head: Box<RawValue>,
  pub keep_blocks: usize,
  pub blocks: Vec<Box<RawValue>>,
}

/// The frame that turns `prev` (what the receivers hold; None: nothing) into `next`
pub fn view_frame(prev: Option<&ViewPartsData>, next: &ViewPartsData) -> ViewFrame {
  let prev = prev.filter(|p| p.id == next.id && p.epoch == next.epoch);
  // The live agent turn travels as parts; every turn ahead of it whole
  let body = match next.last {
    Some(_) => &next.turns[..next.turns.len().saturating_sub(1)],
    None => &next.turns[..],
  };
  let keep = prev.map_or(0, |p| common_prefix(&p.turns, body));
  let last = next.last.as_ref().map(|nl| {
    // The same live turn as before (every turn ahead of it unchanged, and it is still the last): only its changed blocks
    let kb = match prev {
      Some(p) if keep == body.len() && p.turns.len() == next.turns.len() => p.last.as_ref().map_or(0, |pl| common_prefix(&pl.blocks, &nl.blocks)),
      _ => 0,
    };
    LastFrame { head: nl.head.clone(), keep_blocks: kb, blocks: nl.blocks[kb..].to_vec() }
  });
  ViewFrame { head: next.head.clone(), keep, turns: body[keep..].to_vec(), last }
}

/// The receiver's side of `view_frame`: what it holds plus `f`, as every turn and the live turn's parts. None when `f`
/// refers to more than `prev` holds (a frame for a state this receiver never saw); the receiver then asks for a whole view
pub fn apply_frame(prev: Option<&ViewPartsData>, f: &ViewFrame) -> Option<(Vec<Box<RawValue>>, Option<LastTurnParts>)> {
  let held: &[Box<RawValue>] = prev.map_or(&[], |p| &p.turns);
  if f.keep > held.len() {
    return None;
  }
  let mut turns: Vec<Box<RawValue>> = held[..f.keep].to_vec();
  turns.extend(f.turns.iter().cloned());
  let last = match &f.last {
    None => None,
    Some(l) => {
      let mut blocks = match l.keep_blocks {
        0 => vec![],
        kb => {
          let pl = prev.and_then(|p| p.last.as_ref())?;
          pl.blocks.get(..kb)?.to_vec()
        }
      };
      blocks.extend(l.blocks.iter().cloned());
      turns.push(splice(&l.head, "blocks", &blocks)?);
      Some(LastTurnParts { head: l.head.clone(), blocks })
    }
  };
  Some((turns, last))
}

/// `head` with its empty `key` array filled with `items`: spliced as text, or through a parse when the key is not unique
pub fn splice(head: &RawValue, key: &str, items: &[Box<RawValue>]) -> Option<Box<RawValue>> {
  splice_array(head, key, items).or_else(|| {
    let mut v: Value = serde_json::from_str(head.get()).ok()?;
    let parsed: Result<Vec<Value>, _> = items.iter().map(|i| serde_json::from_str(i.get())).collect();
    // A head that is not an object (another engine's frame) gives None instead of a panic in `IndexMut`
    v.as_object_mut()?.insert(key.to_owned(), Value::Array(parsed.ok()?));
    serde_json::value::to_raw_value(&v).ok()
  })
}

/// What the bus asks of its engine (the session manager)
pub trait BusPeer: Send + Sync {
  /// Owner: another engine started showing `session` (send a whole view now) or the last one stopped
  fn bus_watch(&self, session: &str, on: bool);
  /// Follower: a view step for a session this engine subscribed to
  fn bus_view(&self, session: &str, frame: ViewFrame);
  /// Follower: the owner of a subscribed session let it go or left the bus
  fn bus_gone(&self, session: &str);
  /// Owner: a page of another engine took an action on `session` (a `WebviewMsg`)
  fn bus_act(&self, session: String, msg: Value) -> BoxFuture<Result<(), String>>;
  /// The sessions this engine owns, announced again after every (re)connect
  fn bus_owned(&self) -> Vec<String>;
  /// The sessions this engine shows read-only, subscribed again after every (re)connect
  fn bus_wanted(&self) -> Vec<String>;
  /// The subagents this engine's pages observe on sessions shown read-only, subscribed again after every (re)connect
  fn bus_wanted_nodes(&self) -> Vec<(String, String)>;
  /// The list entries (`SessionSummary` with its state) of the sessions this engine runs, announced again after every
  /// (re)connect
  fn bus_own_live(&self) -> Vec<Value>;
  /// Another engine's list entries, keyed by its connection on the router (empty: it left or runs nothing)
  fn bus_live(&self, peer: u64, summaries: Vec<Value>);
  /// Another engine changed a user fact in the shared store (`unread`, `categories`, `index`): read it again
  fn bus_touch(&self, what: &str);
  /// Owner: another engine started observing a subagent of `session` (send its transcript now) or the last one stopped
  fn bus_node_watch(&self, session: &str, node: &str, on: bool);
  /// Follower: a subagent transcript of a session another engine owns, whole
  fn bus_node_view(&self, session: &str, node: &str, rev: i64, running: bool, turns: Box<RawValue>);
  /// The router went away: what came from other engines (list entries, watches of subagents) is void until the next
  /// router replays it
  fn bus_reset(&self);
}

/// One line on the bus. A flat object rather than a tagged enum: tagged enums buffer their content, which raw JSON
/// values cannot pass through, and unknown kinds or fields from a newer engine are simply skipped
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
  t: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  session: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  sessions: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  on: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  view: Option<ViewFrame>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  id: Option<u64>,
  // Set by the router: the client an `act` came from, the one a `done` goes to
  #[serde(default, skip_serializing_if = "Option::is_none")]
  peer: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  msg: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  error: Option<String>,
  // `live`: an engine's list entries, as plain JSON (an entry a newer engine shapes differently is skipped on its own
  // instead of making the whole frame unreadable)
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  summaries: Vec<Value>,
  // `touch`: which user fact changed
  #[serde(default, skip_serializing_if = "Option::is_none")]
  what: Option<String>,
  // `nsub` / `nunsub` / `nwatch` / `nview`: the subagent (`SubagentNode.id`) of `session`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  node: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  rev: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  running: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  turns: Option<Box<RawValue>>,
}

impl Frame {
  fn new(t: &str, session: &str) -> Frame {
    Frame { t: t.into(), session: Some(session.into()), ..Default::default() }
  }

  fn node(t: &str, session: &str, node: &str) -> Frame {
    Frame { node: Some(node.into()), ..Frame::new(t, session) }
  }

  #[expect(clippy::expect_used, reason = "a frame holds strings, numbers and JSON values, all with string keys, which always serialize")]
  fn line(&self) -> Arc<str> {
    Arc::from(serde_json::to_string(self).expect("serializable"))
  }
}

// An owner's stream of one watched session: the view the subscribers hold and the newest one waiting for the batch
#[derive(Default)]
struct Stream {
  sent: Option<ViewParts>,
  latest: Option<ViewParts>,
  armed: bool,
  // A subscriber joined: the next frame is the whole view
  whole: bool,
}

// The socket, lock and log only serve the socket / pipe transport; other platforms build the bus without one
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
struct Inner {
  socket: PathBuf,
  lock: PathBuf,
  log: LogFn,
  peer: parking_lot::Mutex<Option<Weak<dyn BusPeer>>>,
  out: parking_lot::Mutex<Option<mpsc::UnboundedSender<Arc<str>>>>,
  pending: parking_lot::Mutex<HashMap<u64, oneshot::Sender<Result<(), String>>>>,
  next: AtomicU64,
  streams: parking_lot::Mutex<HashMap<String, Stream>>,
  // The owned list last announced on this connection: an unchanged list is not sent again
  announced: parking_lot::Mutex<Option<Vec<String>>>,
  // The list entries last announced on this connection, likewise
  announced_live: parking_lot::Mutex<Option<Vec<Value>>>,
  tasks: parking_lot::Mutex<Vec<tokio::task::AbortHandle>>,
}

impl Drop for Inner {
  fn drop(&mut self) {
    for t in self.tasks.get_mut().drain(..) {
      t.abort();
    }
  }
}

/// This engine's end of the bus
pub struct Bus {
  inner: Arc<Inner>,
}

impl Bus {
  /// `root`: ACPIRA_HOME. Nothing happens until `start`
  pub fn new(root: &Path, log: LogFn) -> Bus {
    let (socket, lock) = paths(root);
    Bus {
      inner: Arc::new(Inner {
        socket,
        lock,
        log,
        peer: Default::default(),
        out: Default::default(),
        pending: Default::default(),
        next: AtomicU64::new(1),
        streams: Default::default(),
        announced: Default::default(),
        announced_live: Default::default(),
        tasks: Default::default(),
      }),
    }
  }

  /// Join the bus (becoming its router when nobody serves it) and keep rejoining until this bus is dropped
  pub fn start(&self, peer: Weak<dyn BusPeer>) {
    *self.inner.peer.lock() = Some(peer);
    #[cfg(any(unix, windows))]
    {
      let task = tokio::spawn(run(Arc::downgrade(&self.inner)));
      self.inner.tasks.lock().push(task.abort_handle());
    }
  }

  /// Leave the bus for good (the engine is shutting down): other engines hear at once that its sessions are gone
  pub fn stop(&self) {
    for t in self.inner.tasks.lock().drain(..) {
      t.abort();
    }
    self.inner.disconnected();
  }

  /// Connected to a router right now
  pub fn connected(&self) -> bool {
    self.inner.out.lock().is_some()
  }

  /// The sessions this engine owns now (sent only when the list changed)
  pub fn announce(&self, mut owned: Vec<String>) {
    owned.sort();
    {
      let mut last = self.inner.announced.lock();
      if !self.connected() || last.as_ref() == Some(&owned) {
        return;
      }
      *last = Some(owned.clone());
    }
    self.inner.send(&Frame { t: "own".into(), sessions: owned, ..Default::default() });
  }

  /// The list entries of the sessions this engine runs now (sent only when they changed), so every other engine's list
  /// shows them as this one does: running, waiting or failed, renamed, pinned, filed, on another model
  pub fn announce_live(&self, summaries: Vec<Value>) {
    {
      let mut last = self.inner.announced_live.lock();
      if !self.connected() || last.as_ref() == Some(&summaries) {
        return;
      }
      *last = Some(summaries.clone());
    }
    self.inner.send(&Frame { t: "live".into(), summaries, ..Default::default() });
  }

  /// A user fact in the shared store changed here (`unread`, `categories`, `index`): every other engine reads it again
  pub fn touch(&self, what: &str) {
    self.inner.send(&Frame { t: "touch".into(), what: Some(what.into()), ..Default::default() });
  }

  /// A page here observes a subagent of a session another engine owns: its owner pushes the transcript
  pub fn node_subscribe(&self, session: &str, node: &str) {
    self.inner.send(&Frame::node("nsub", session, node));
  }

  pub fn node_unsubscribe(&self, session: &str, node: &str) {
    self.inner.send(&Frame::node("nunsub", session, node));
  }

  /// Owner: a watched subagent's transcript, whole (as a page here gets it)
  pub fn node_publish(&self, session: &str, node: &str, rev: i64, running: bool, turns: Box<RawValue>) {
    self.inner.send(&Frame { rev: Some(rev), running: Some(running), turns: Some(turns), ..Frame::node("nview", session, node) });
  }

  pub fn subscribe(&self, session: &str) {
    self.inner.send(&Frame::new("sub", session));
  }

  pub fn unsubscribe(&self, session: &str) {
    self.inner.send(&Frame::new("unsub", session));
  }

  /// Another engine shows this owned session: its views go out through `publish`
  pub fn watching(&self, session: &str) -> bool {
    self.inner.streams.lock().contains_key(session)
  }

  /// The newest view of a watched session; sent after a short batch window, as a step from the one sent before
  pub fn publish(&self, session: &str, parts: &ViewParts) {
    let arm = {
      let mut streams = self.inner.streams.lock();
      let Some(s) = streams.get_mut(session) else { return };
      s.latest = Some(parts.clone());
      !std::mem::replace(&mut s.armed, true)
    };
    if arm {
      let me = Arc::downgrade(&self.inner);
      let session = session.to_owned();
      tokio::spawn(async move {
        tokio::time::sleep(BATCH).await;
        if let Some(inner) = me.upgrade() {
          inner.flush(&session);
        }
      });
    }
  }

  /// Run a page's action on a session another engine owns; its error (or the bus being down) comes back as text
  pub async fn act(&self, session: &str, msg: Value) -> Result<(), String> {
    let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    self.inner.pending.lock().insert(id, tx);
    if !self.inner.send(&Frame { id: Some(id), msg: Some(msg), ..Frame::new("act", session) }) {
      self.inner.pending.lock().remove(&id);
      return Err("bus disconnected".into());
    }
    rx.await.unwrap_or_else(|_| Err("bus disconnected".into()))
  }
}

impl Inner {
  fn peer(&self) -> Option<Arc<dyn BusPeer>> {
    self.peer.lock().as_ref().and_then(|p| p.upgrade())
  }

  fn send(&self, f: &Frame) -> bool {
    let out = self.out.lock();
    out.as_ref().is_some_and(|tx| tx.send(f.line()).is_ok())
  }

  fn flush(&self, session: &str) {
    let frame = {
      let mut streams = self.streams.lock();
      let Some(s) = streams.get_mut(session) else { return };
      s.armed = false;
      let Some(next) = s.latest.take() else { return };
      let prev = if s.whole { None } else { s.sent.as_ref().map(|p| &*p.0) };
      let frame = view_frame(prev, &next.0);
      s.whole = false;
      s.sent = Some(next);
      frame
    };
    self.send(&Frame { view: Some(frame), ..Frame::new("view", session) });
  }

  #[cfg_attr(not(any(unix, windows)), allow(dead_code))]
  fn receive(self: &Arc<Self>, line: &str) {
    let f: Frame = match serde_json::from_str(line) {
      Ok(f) => f,
      Err(e) => {
        (self.log)(&format!("bus: unreadable frame: {e}"));
        return;
      }
    };
    let Some(peer) = self.peer() else { return };
    match f.t.as_str() {
      "live" => {
        if let Some(from) = f.peer {
          peer.bus_live(from, f.summaries);
        }
        return;
      }
      "touch" => {
        if let Some(what) = &f.what {
          peer.bus_touch(what);
        }
        return;
      }
      "nwatch" | "nview" => {
        let (Some(session), Some(node)) = (f.session.as_deref(), f.node.as_deref()) else { return };
        if f.t == "nwatch" {
          peer.bus_node_watch(session, node, f.on == Some(true));
        } else if let (Some(rev), Some(turns)) = (f.rev, f.turns) {
          peer.bus_node_view(session, node, rev, f.running == Some(true), turns);
        }
        return;
      }
      _ => {}
    }
    let Some(session) = f.session.clone() else {
      if f.t == "done" {
        self.done(f);
      }
      return;
    };
    match f.t.as_str() {
      "watch" => {
        let on = f.on == Some(true);
        if on {
          self.streams.lock().entry(session.clone()).or_default().whole = true;
        } else {
          self.streams.lock().remove(&session);
        }
        peer.bus_watch(&session, on);
      }
      "view" => {
        if let Some(v) = f.view {
          peer.bus_view(&session, v);
        }
      }
      "gone" => peer.bus_gone(&session),
      "act" => {
        let (id, from) = (f.id, f.peer);
        let run = peer.bus_act(session, f.msg.unwrap_or(Value::Null));
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
          let result = run.await;
          if let Some(inner) = me.upgrade() {
            inner.send(&Frame { t: "done".into(), id, peer: from, error: result.err(), ..Default::default() });
          }
        });
      }
      "done" => self.done(f),
      _ => {}
    }
  }

  #[cfg_attr(not(any(unix, windows)), allow(dead_code))]
  fn done(&self, f: Frame) {
    if let Some(tx) = f.id.and_then(|id| self.pending.lock().remove(&id)) {
      let _ = tx.send(f.error.map_or(Ok(()), Err));
    }
  }

  // Joined a router: announce everything afresh
  #[cfg_attr(not(any(unix, windows)), allow(dead_code))]
  fn connected(&self, tx: mpsc::UnboundedSender<Arc<str>>) {
    *self.out.lock() = Some(tx);
    *self.announced.lock() = None;
    *self.announced_live.lock() = None;
    if let Some(peer) = self.peer() {
      let mut owned = peer.bus_owned();
      owned.sort();
      *self.announced.lock() = Some(owned.clone());
      self.send(&Frame { t: "own".into(), sessions: owned, ..Default::default() });
      for s in peer.bus_wanted() {
        self.send(&Frame::new("sub", &s));
      }
      for (s, n) in peer.bus_wanted_nodes() {
        self.send(&Frame::node("nsub", &s, &n));
      }
      let live = peer.bus_own_live();
      *self.announced_live.lock() = Some(live.clone());
      if !live.is_empty() {
        self.send(&Frame { t: "live".into(), summaries: live, ..Default::default() });
      }
    }
  }

  // The router went away: watches end with it (the next router sends them again) and pending actions fail
  fn disconnected(&self) {
    *self.out.lock() = None;
    self.streams.lock().clear();
    for (_, tx) in self.pending.lock().drain() {
      let _ = tx.send(Err("bus disconnected".into()));
    }
    if let Some(peer) = self.peer() {
      peer.bus_reset();
    }
  }
}

/// `<root>/run/bus.sock` and its lock, or a short path under the temp dir when that would not bind. On Windows the socket
/// is a named pipe whose name comes from the root (`\\.\pipe\acpira-bus-<hash>`), and the lock stays under `run/`
fn paths(root: &Path) -> (PathBuf, PathBuf) {
  let run = root.join("run");
  if cfg!(windows) {
    return (PathBuf::from(format!(r"\\.\pipe\acpira-bus-{}", root_key(root))), run.join("bus.lock"));
  }
  let socket = run.join("bus.sock");
  if socket.as_os_str().len() <= SOCKET_PATH_MAX {
    return (socket, run.join("bus.lock"));
  }
  let key = root_key(root);
  let tmp = std::env::temp_dir();
  (tmp.join(format!("acpira-bus-{key}.sock")), tmp.join(format!("acpira-bus-{key}.lock")))
}

fn root_key(root: &Path) -> String {
  use sha2::Digest;
  let digest = sha2::Sha256::digest(root.to_string_lossy().as_bytes());
  digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Reaching the router: connected, nobody serves it (elect one), or it is there but cannot be used now (try again)
#[cfg(any(unix, windows))]
enum Reach<S> {
  Joined(S),
  Absent,
  #[cfg_attr(unix, allow(dead_code, reason = "only the Windows pipe can be busy"))]
  Busy,
}

#[cfg(unix)]
async fn reach(socket: &Path) -> Reach<tokio::net::UnixStream> {
  match tokio::net::UnixStream::connect(socket).await {
    Ok(s) => Reach::Joined(s),
    Err(_) => Reach::Absent,
  }
}

/// The router's pipe, trusted only when the process serving it runs as this user (another user could create the name
/// first). Every instance taken for a moment (the router creates the next one right after each connection) is busy
#[cfg(windows)]
async fn reach(socket: &Path) -> Reach<tokio::net::windows::named_pipe::NamedPipeClient> {
  const ERROR_PIPE_BUSY: i32 = 231;
  match tokio::net::windows::named_pipe::ClientOptions::new().open(socket) {
    Ok(pipe) if acpira_shared::win_pipe::served_by_this_user(&pipe) => Reach::Joined(pipe),
    Ok(_) => Reach::Busy,
    Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => Reach::Busy,
    Err(_) => Reach::Absent,
  }
}

#[cfg(any(unix, windows))]
async fn run(me: Weak<Inner>) {
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
  let mut said = false;
  loop {
    let Some(inner) = me.upgrade() else { return };
    let (socket, lock, log) = (inner.socket.clone(), inner.lock.clone(), inner.log.clone());
    drop(inner);
    match reach(&socket).await {
      Reach::Joined(stream) => {
        if !said {
          log(&format!("bus: joined {}", socket.display()));
          said = true;
        }
        let (rd, mut wr) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::unbounded_channel::<Arc<str>>();
        let writer = tokio::spawn(async move {
          while let Some(line) = rx.recv().await {
            if wr.write_all(line.as_bytes()).await.is_err() || wr.write_all(b"\n").await.is_err() {
              break;
            }
          }
        });
        if let Some(inner) = me.upgrade() {
          inner.connected(tx);
        }
        let mut lines = BufReader::new(rd).lines();
        while let Ok(Some(line)) = lines.next_line().await {
          let Some(inner) = me.upgrade() else { break };
          inner.receive(&line);
        }
        writer.abort();
        if let Some(inner) = me.upgrade() {
          inner.disconnected();
        }
      }
      Reach::Absent => {
        if let Some(router) = elect(&socket, &lock) {
          let Some(inner) = me.upgrade() else { return };
          log(&format!("bus: routing on {}", socket.display()));
          let task = tokio::spawn(route(router.0, router.1, socket));
          inner.tasks.lock().push(task.abort_handle());
          continue;
        }
      }
      Reach::Busy => {}
    }
    tokio::time::sleep(RETRY).await;
  }
}

/// The router's lock: whoever holds it serves the bus
#[cfg(any(unix, windows))]
fn take_lock(lock: &Path) -> Option<std::fs::File> {
  if let Some(dir) = lock.parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  let mut options = std::fs::OpenOptions::new();
  options.create(true).truncate(false).read(true).write(true);
  #[cfg(unix)]
  std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
  let file = options.open(lock).ok()?;
  file.try_lock().ok()?;
  Some(file)
}

/// Become the router when nobody holds its lock: the lock lives as long as the listener (the router task)
#[cfg(unix)]
fn elect(socket: &Path, lock: &Path) -> Option<(std::fs::File, tokio::net::UnixListener)> {
  use std::os::unix::fs::PermissionsExt;
  if let Some(dir) = socket.parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  let file = take_lock(lock)?;
  // Holding the lock, a socket file left here belongs to a router that died
  let _ = std::fs::remove_file(socket);
  let listener = tokio::net::UnixListener::bind(socket).ok()?;
  let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));
  Some((file, listener))
}

/// The pipe's instances admit this user alone and no remote client; the first one claims the name, so a pipe of the
/// same name that someone else created first makes the election fail rather than serve beside it
#[cfg(windows)]
fn pipe_instance(name: &Path, first: bool) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
  let sa = acpira_shared::win_pipe::OwnerOnly::new()?;
  let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
  options.first_pipe_instance(first).reject_remote_clients(true);
  // SAFETY: the security attributes outlive the call that copies them into the pipe
  unsafe { options.create_with_security_attributes_raw(name, sa.as_ptr()) }
}

#[cfg(windows)]
fn elect(socket: &Path, lock: &Path) -> Option<(std::fs::File, tokio::net::windows::named_pipe::NamedPipeServer)> {
  let file = take_lock(lock)?;
  let server = pipe_instance(socket, true).ok()?;
  Some((file, server))
}

/// The router's side of one engine's connection, until it closes
#[cfg(any(unix, windows))]
fn serve_client<S>(router: &Arc<parking_lot::Mutex<Router>>, id: u64, stream: S, conns: &mut tokio::task::JoinSet<()>)
where
  S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
{
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
  let (rd, mut wr) = tokio::io::split(stream);
  let (tx, mut rx) = mpsc::unbounded_channel::<Arc<str>>();
  router.lock().join(id, tx);
  let r = router.clone();
  conns.spawn(async move {
    let writer = tokio::spawn(async move {
      while let Some(line) = rx.recv().await {
        if wr.write_all(line.as_bytes()).await.is_err() || wr.write_all(b"\n").await.is_err() {
          break;
        }
      }
    });
    let mut lines = BufReader::new(rd).lines();
    while let Ok(Some(line)) = lines.next_line().await {
      let Ok(f) = serde_json::from_str::<Frame>(&line) else { continue };
      r.lock().handle(id, f, Arc::from(line));
    }
    r.lock().leave(id);
    writer.abort();
  });
  // Reap finished connections
  while conns.try_join_next().is_some() {}
}

#[cfg(unix)]
async fn route(_lock: std::fs::File, listener: tokio::net::UnixListener, _socket: PathBuf) {
  let router = Arc::new(parking_lot::Mutex::new(Router::default()));
  let mut next = 0u64;
  // Connection tasks end with the router (its engine dropping the bus aborts this task, and the set aborts them)
  let mut conns = tokio::task::JoinSet::new();
  loop {
    let Ok((stream, _)) = listener.accept().await else { continue };
    next += 1;
    serve_client(&router, next, stream, &mut conns);
  }
}

/// A pipe instance serves one connection: the next one is created before the connected one is handed over, so an engine
/// arriving meanwhile finds it
#[cfg(windows)]
async fn route(_lock: std::fs::File, mut server: tokio::net::windows::named_pipe::NamedPipeServer, socket: PathBuf) {
  let router = Arc::new(parking_lot::Mutex::new(Router::default()));
  let mut next = 0u64;
  let mut conns = tokio::task::JoinSet::new();
  loop {
    if server.connect().await.is_err() {
      // This instance broke before a client got it: replace it
      match pipe_instance(&socket, false) {
        Ok(s) => server = s,
        Err(_) => tokio::time::sleep(RETRY).await,
      }
      continue;
    }
    let fresh = loop {
      match pipe_instance(&socket, false) {
        Ok(s) => break s,
        Err(_) => tokio::time::sleep(RETRY).await,
      }
    };
    let connected = std::mem::replace(&mut server, fresh);
    next += 1;
    serve_client(&router, next, connected, &mut conns);
  }
}

// The router runs on Unix sockets and Windows named pipes (its tests run everywhere)
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
struct Client {
  tx: mpsc::UnboundedSender<Arc<str>>,
  owned: HashSet<String>,
  subs: HashSet<String>,
  // The subagents (session, node) it observes on sessions it shows read-only
  nodes: HashSet<(String, String)>,
  // Its last `live` frame as relayed (stamped with its id), replayed to every engine that joins later
  live: Option<Arc<str>>,
}

#[derive(Default)]
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
struct Router {
  clients: HashMap<u64, Client>,
}

#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
impl Router {
  fn owner(&self, session: &str) -> Option<u64> {
    self.clients.iter().find(|(_, c)| c.owned.contains(session)).map(|(id, _)| *id)
  }

  fn subscribers(&self, session: &str, except: u64) -> Vec<u64> {
    self.clients.iter().filter(|(id, c)| **id != except && c.subs.contains(session)).map(|(id, _)| *id).collect()
  }

  fn node_subscribers(&self, key: &(String, String), except: u64) -> Vec<u64> {
    self.clients.iter().filter(|(id, c)| **id != except && c.nodes.contains(key)).map(|(id, _)| *id).collect()
  }

  fn node_watch(&self, owner: u64, key: &(String, String), on: bool) {
    self.send(owner, Frame { on: Some(on), ..Frame::node("nwatch", &key.0, &key.1) }.line());
  }

  fn send(&self, to: u64, line: Arc<str>) {
    if let Some(c) = self.clients.get(&to) {
      let _ = c.tx.send(line);
    }
  }

  /// A new connection: it hears every other engine's list entries at once
  fn join(&mut self, id: u64, tx: mpsc::UnboundedSender<Arc<str>>) {
    for c in self.clients.values() {
      if let Some(line) = &c.live {
        let _ = tx.send(line.clone());
      }
    }
    self.clients.insert(id, Client { tx, owned: HashSet::new(), subs: HashSet::new(), nodes: HashSet::new(), live: None });
  }

  fn broadcast(&self, from: u64, line: Arc<str>) {
    for (id, c) in &self.clients {
      if *id != from {
        let _ = c.tx.send(line.clone());
      }
    }
  }

  fn watch(&self, owner: u64, session: &str, on: bool) {
    self.send(owner, Frame { on: Some(on), ..Frame::new("watch", session) }.line());
  }

  fn handle(&mut self, from: u64, f: Frame, line: Arc<str>) {
    let session = f.session.clone().unwrap_or_default();
    match f.t.as_str() {
      "own" => {
        let now: HashSet<String> = f.sessions.into_iter().collect();
        let Some(c) = self.clients.get_mut(&from) else { return };
        let before = std::mem::replace(&mut c.owned, now.clone());
        for s in now.difference(&before) {
          if !self.subscribers(s, from).is_empty() {
            self.watch(from, s, true);
          }
        }
        // Subagents observed on a session that just found its owner: that owner pushes them
        let watched: HashSet<(String, String)> =
          self.clients.iter().filter(|(id, _)| **id != from).flat_map(|(_, c)| c.nodes.iter().filter(|k| now.contains(&k.0) && !before.contains(&k.0)).cloned()).collect();
        for key in &watched {
          self.node_watch(from, key, true);
        }
        for s in before.difference(&now) {
          for sub in self.subscribers(s, from) {
            self.send(sub, Frame::new("gone", s).line());
          }
        }
      }
      "sub" => {
        if let Some(c) = self.clients.get_mut(&from) {
          c.subs.insert(session.clone());
        }
        // Every new subscriber gets a whole view: the owner starts its stream over
        if let Some(o) = self.owner(&session).filter(|o| *o != from) {
          self.watch(o, &session, true);
        }
      }
      "unsub" => {
        if let Some(c) = self.clients.get_mut(&from) {
          c.subs.remove(&session);
        }
        if let Some(o) = self.owner(&session).filter(|o| *o != from)
          && self.subscribers(&session, o).is_empty()
        {
          self.watch(o, &session, false);
        }
      }
      "view" => {
        for sub in self.subscribers(&session, from) {
          self.send(sub, line.clone());
        }
      }
      "act" => match self.owner(&session).filter(|o| *o != from) {
        Some(o) => self.send(o, Frame { peer: Some(from), ..f }.line()),
        None => {
          let error = Some("no engine holds this session".to_owned());
          self.send(from, Frame { t: "done".into(), id: f.id, error, ..Default::default() }.line());
        }
      },
      "done" => {
        if let Some(to) = f.peer {
          self.send(to, Frame { peer: None, ..f }.line());
        }
      }
      "live" => {
        let line = Frame { t: "live".into(), peer: Some(from), summaries: f.summaries, ..Default::default() }.line();
        let Some(c) = self.clients.get_mut(&from) else { return };
        c.live = Some(line.clone());
        self.broadcast(from, line);
      }
      "touch" => self.broadcast(from, line),
      "nsub" | "nunsub" | "nview" => {
        let Some(node) = f.node.clone() else { return };
        let key = (session.clone(), node);
        match f.t.as_str() {
          "nsub" => {
            if let Some(c) = self.clients.get_mut(&from) {
              c.nodes.insert(key.clone());
            }
            // Every new observer gets the transcript whole: the owner sends it again
            if let Some(o) = self.owner(&session).filter(|o| *o != from) {
              self.node_watch(o, &key, true);
            }
          }
          "nunsub" => {
            if let Some(c) = self.clients.get_mut(&from) {
              c.nodes.remove(&key);
            }
            if let Some(o) = self.owner(&session).filter(|o| *o != from)
              && self.node_subscribers(&key, o).is_empty()
            {
              self.node_watch(o, &key, false);
            }
          }
          _ => {
            for sub in self.node_subscribers(&key, from) {
              self.send(sub, line.clone());
            }
          }
        }
      }
      _ => {}
    }
  }

  fn leave(&mut self, id: u64) {
    let Some(c) = self.clients.remove(&id) else { return };
    // Its sessions stop showing as running anywhere else
    if c.live.is_some() {
      self.broadcast(id, Frame { t: "live".into(), peer: Some(id), ..Default::default() }.line());
    }
    for key in &c.nodes {
      if let Some(o) = self.owner(&key.0)
        && self.node_subscribers(key, o).is_empty()
      {
        self.node_watch(o, key, false);
      }
    }
    for s in &c.owned {
      for sub in self.subscribers(s, id) {
        self.send(sub, Frame::new("gone", s).line());
      }
    }
    for s in &c.subs {
      if let Some(o) = self.owner(s)
        && self.subscribers(s, o).is_empty()
      {
        self.watch(o, s, false);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::session_patch::{encode_turns, raw};
  use acpira_shared::transcript::Turn;
  use serde_json::json;

  fn turn(v: Value) -> Turn {
    serde_json::from_value(v).unwrap()
  }

  fn user(id: &str) -> Turn {
    turn(json!({ "role": "user", "id": id, "text": id }))
  }

  fn agent(texts: &[&str]) -> Turn {
    turn(json!({ "role": "agent", "blocks": texts.iter().map(|t| json!({ "type": "text", "markdown": t })).collect::<Vec<_>>() }))
  }

  fn parts(rev: i64, mut turns: Vec<Turn>) -> ViewPartsData {
    let mut last = turns.pop();
    let before: Vec<&Turn> = turns.iter().collect();
    let (turns, last) = encode_turns(&before, last.as_mut());
    ViewPartsData { id: "s".into(), epoch: 1, rev, head: raw(&json!({ "id": "s", "rev": rev, "turns": [] })), turns, last }
  }

  // What the receiver rebuilds from a frame against what it held, as plain JSON per turn
  fn rebuilt(prev: Option<&ViewPartsData>, f: &ViewFrame) -> Vec<Value> {
    let (turns, _) = apply_frame(prev, f).expect("applies");
    turns.iter().map(|t| serde_json::from_str(t.get()).unwrap()).collect()
  }

  fn plain(p: &ViewPartsData) -> Vec<Value> {
    p.turns.iter().map(|t| serde_json::from_str(t.get()).unwrap()).collect()
  }

  #[test]
  fn a_streamed_chunk_sends_only_the_changed_blocks_and_rebuilds_the_same_view() {
    let a = parts(1, vec![user("u1"), agent(&["a", "b"])]);
    let b = parts(2, vec![user("u1"), agent(&["a", "b", "c"])]);
    let whole = view_frame(None, &a);
    assert_eq!((whole.keep, whole.turns.len()), (0, 1), "the user turn whole, the live turn as parts");
    let (turns, last) = apply_frame(None, &whole).unwrap();
    let held = ViewPartsData { turns, last, ..parts(1, vec![]) };
    assert_eq!(plain(&held), plain(&a));
    let step = view_frame(Some(&a), &b);
    assert_eq!(step.keep, 1);
    assert!(step.turns.is_empty());
    let l = step.last.as_ref().unwrap();
    assert_eq!((l.keep_blocks, l.blocks.len()), (2, 1));
    assert_eq!(rebuilt(Some(&held), &step), plain(&b));
  }

  #[test]
  fn a_new_turn_after_the_live_one_keeps_the_finished_turn_and_sends_the_rest() {
    let a = parts(1, vec![user("u1"), agent(&["a"])]);
    let b = parts(2, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x"])]);
    let step = view_frame(Some(&a), &b);
    assert_eq!(step.keep, 2);
    assert_eq!(step.turns.len(), 1);
    assert_eq!(step.last.as_ref().unwrap().keep_blocks, 0);
    assert_eq!(rebuilt(Some(&a), &step), plain(&b));
  }

  #[test]
  fn a_rewind_and_a_frame_beyond_what_is_held_are_handled() {
    let a = parts(1, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["b"])]);
    let b = parts(2, vec![user("u1"), agent(&["a"])]);
    let step = view_frame(Some(&a), &b);
    assert_eq!(rebuilt(Some(&a), &step), plain(&b));
    // Nothing held: a step that keeps turns cannot apply, the receiver asks for the whole view
    assert!(apply_frame(None, &step).is_none());
    // Another epoch is another instance of the session: the whole view
    let other = ViewPartsData { epoch: 2, ..parts(3, vec![user("u1")]) };
    assert_eq!(view_frame(Some(&a), &other).keep, 0);
  }

  #[test]
  fn a_head_with_a_nested_empty_key_still_splices() {
    let head = raw(&json!({ "role": "agent", "blocks": [], "meta": { "blocks": [] } }));
    let out = splice(&head, "blocks", &[raw(&json!({ "type": "text", "markdown": "x" }))]).unwrap();
    let v: Value = serde_json::from_str(out.get()).unwrap();
    assert_eq!(v["blocks"][0]["markdown"], "x");
    assert_eq!(v["meta"]["blocks"], json!([]));
  }

  #[test]
  fn frames_skip_unknown_fields_and_kinds() {
    let f: Frame = serde_json::from_str(r#"{"t":"later","session":"s","novel":1}"#).unwrap();
    assert_eq!(f.t, "later");
    let mut r = Router::default();
    r.handle(1, f, Arc::from("{}"));
  }

  #[cfg(unix)]
  #[test]
  fn long_homes_get_a_short_socket_path() {
    let (s, l) = paths(Path::new("/h"));
    assert_eq!(s, Path::new("/h/run/bus.sock"));
    assert_eq!(l, Path::new("/h/run/bus.lock"));
    let deep = PathBuf::from("/").join("x".repeat(120));
    let (s, _) = paths(&deep);
    assert!(s.as_os_str().len() <= SOCKET_PATH_MAX + 40 && s.starts_with(std::env::temp_dir()));
  }

  #[cfg(windows)]
  #[test]
  fn the_bus_is_a_pipe_named_after_the_home() {
    let (s, l) = paths(Path::new(r"C:\h"));
    assert!(s.to_string_lossy().starts_with(r"\\.\pipe\acpira-bus-"), "{}", s.display());
    assert_ne!(s, paths(Path::new(r"C:\other")).0);
    assert_eq!(l, Path::new(r"C:\h\run\bus.lock"));
  }

  #[test]
  fn the_router_routes_watches_views_actions_and_departures() {
    let mut r = Router::default();
    let mut rx = HashMap::new();
    for id in [1u64, 2, 3] {
      let (tx, rx1) = mpsc::unbounded_channel();
      r.join(id, tx);
      rx.insert(id, rx1);
    }
    let mut take = |id: u64| -> Vec<Value> {
      let mut out = vec![];
      while let Ok(l) = rx.get_mut(&id).unwrap().try_recv() {
        out.push(serde_json::from_str(&l).unwrap());
      }
      out
    };
    let frame = |v: Value| -> (Frame, Arc<str>) {
      let s = v.to_string();
      (serde_json::from_str(&s).unwrap(), Arc::from(s))
    };
    // 2 subscribes before anyone owns the session; 1 then announces it and is asked to stream
    let (f, l) = frame(json!({ "t": "sub", "session": "s" }));
    r.handle(2, f, l);
    let (f, l) = frame(json!({ "t": "own", "sessions": ["s"] }));
    r.handle(1, f, l);
    assert_eq!(take(1), vec![json!({ "t": "watch", "session": "s", "on": true })]);
    // 3 joins: another whole view is asked for
    let (f, l) = frame(json!({ "t": "sub", "session": "s" }));
    r.handle(3, f, l);
    assert_eq!(take(1).len(), 1);
    // A view goes to both subscribers as sent
    let (f, l) = frame(json!({ "t": "view", "session": "s", "view": { "head": {}, "keep": 0, "turns": [] } }));
    r.handle(1, f, l);
    assert_eq!(take(2).len(), 1);
    assert_eq!(take(3).len(), 1);
    // An action reaches the owner with its sender; the answer goes back to that sender only
    let (f, l) = frame(json!({ "t": "act", "session": "s", "id": 7, "msg": { "type": "stop" } }));
    r.handle(3, f, l);
    let got = take(1);
    assert_eq!(got[0]["peer"], 3);
    let (f, l) = frame(json!({ "t": "done", "id": 7, "peer": 3 }));
    r.handle(1, f, l);
    assert_eq!(take(3), vec![json!({ "t": "done", "id": 7 })]);
    assert!(take(2).is_empty());
    // Nobody owns another session: an action on it is answered with an error at once
    let (f, l) = frame(json!({ "t": "act", "session": "x", "id": 8 }));
    r.handle(2, f, l);
    assert!(take(2)[0]["error"].is_string());
    // 2 and 3 leave the session: the owner stops streaming once the last one has gone
    let (f, l) = frame(json!({ "t": "unsub", "session": "s" }));
    r.handle(2, f, l);
    assert!(take(1).is_empty());
    r.leave(3);
    assert_eq!(take(1), vec![json!({ "t": "watch", "session": "s", "on": false })]);
    // The owner leaves while 2 subscribes again: 2 hears the session is gone
    let (f, l) = frame(json!({ "t": "sub", "session": "s" }));
    r.handle(2, f, l);
    take(1);
    r.leave(1);
    assert_eq!(take(2), vec![json!({ "t": "gone", "session": "s" })]);
  }

  fn take(rx: &mut mpsc::UnboundedReceiver<Arc<str>>) -> Vec<Value> {
    let mut out = vec![];
    while let Ok(l) = rx.try_recv() {
      out.push(serde_json::from_str(&l).unwrap());
    }
    out
  }

  fn send(r: &mut Router, from: u64, v: Value) {
    let line = v.to_string();
    r.handle(from, serde_json::from_str(&line).unwrap(), Arc::from(line));
  }

  // List entries (title, state …) reach every other engine stamped with their sender, a late joiner hears them at once,
  // and an engine leaving clears its entries everywhere. A `touch` goes to everyone else, once
  #[test]
  fn list_entries_are_relayed_replayed_and_cleared_and_touches_broadcast() {
    let mut r = Router::default();
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    r.join(1, tx1);
    r.join(2, tx2);
    let entry = json!({ "id": "s", "title": "t", "state": "working" });
    send(&mut r, 1, json!({ "t": "live", "summaries": [entry] }));
    assert!(take(&mut rx1).is_empty(), "not echoed to its sender");
    assert_eq!(take(&mut rx2), vec![json!({ "t": "live", "peer": 1, "summaries": [entry] })]);
    // A third engine joining later gets the entries already announced
    let (tx3, mut rx3) = mpsc::unbounded_channel();
    r.join(3, tx3);
    assert_eq!(take(&mut rx3), vec![json!({ "t": "live", "peer": 1, "summaries": [entry] })]);
    send(&mut r, 2, json!({ "t": "touch", "what": "unread" }));
    assert!(take(&mut rx2).is_empty());
    assert_eq!(take(&mut rx1), vec![json!({ "t": "touch", "what": "unread" })]);
    assert_eq!(take(&mut rx3), vec![json!({ "t": "touch", "what": "unread" })]);
    // The sender leaves: everyone hears its entries are gone
    r.leave(1);
    assert_eq!(take(&mut rx2), vec![json!({ "t": "live", "peer": 1 })]);
    assert_eq!(take(&mut rx3), vec![json!({ "t": "live", "peer": 1 })]);
  }

  // Observing a subagent of a session another engine owns: the owner is asked for it by every new observer, its
  // transcripts reach the observers of that subagent only, it is told when the last observer goes (unsubscribed or gone),
  // and a session that changes hands is asked of its new owner
  #[test]
  fn subagent_observers_are_routed_to_the_owner_and_back() {
    let mut r = Router::default();
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let (tx3, mut rx3) = mpsc::unbounded_channel();
    r.join(1, tx1);
    r.join(2, tx2);
    r.join(3, tx3);
    send(&mut r, 1, json!({ "t": "own", "sessions": ["s"] }));
    send(&mut r, 2, json!({ "t": "nsub", "session": "s", "node": "c1" }));
    assert_eq!(take(&mut rx1), vec![json!({ "t": "nwatch", "session": "s", "node": "c1", "on": true })]);
    send(&mut r, 3, json!({ "t": "nsub", "session": "s", "node": "c2" }));
    take(&mut rx1);
    let view = json!({ "t": "nview", "session": "s", "node": "c1", "rev": 3, "running": true, "turns": [] });
    send(&mut r, 1, view.clone());
    assert_eq!(take(&mut rx2), vec![view]);
    assert!(take(&mut rx3).is_empty(), "another subagent's observer");
    // The last observer of c1 goes: the owner stops; 3 leaving ends c2 the same way
    send(&mut r, 2, json!({ "t": "nunsub", "session": "s", "node": "c1" }));
    assert_eq!(take(&mut rx1), vec![json!({ "t": "nwatch", "session": "s", "node": "c1", "on": false })]);
    send(&mut r, 2, json!({ "t": "nsub", "session": "s", "node": "c1" }));
    take(&mut rx1);
    r.leave(3);
    assert_eq!(take(&mut rx1), vec![json!({ "t": "nwatch", "session": "s", "node": "c2", "on": false })]);
    // The owner leaves and another engine opens the session: it is asked for what 2 still observes
    r.leave(1);
    let (tx4, mut rx4) = mpsc::unbounded_channel();
    r.join(4, tx4);
    send(&mut r, 4, json!({ "t": "own", "sessions": ["s"] }));
    assert!(take(&mut rx4).contains(&json!({ "t": "nwatch", "session": "s", "node": "c1", "on": true })));
  }
}
