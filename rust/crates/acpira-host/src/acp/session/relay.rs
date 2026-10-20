//! Summoned children (cross-harness subagents, see `relay/`): the session runs a persona's CLI as a child of its own.
//! The child gets its own agent process whose client handlers feed the parent session, so its updates, permission and
//! question requests route by native session id into a `session` subagent node exactly like a native child's.
//!
//! One `ask_agent` call = one round = one node. A thread is the sequence of rounds talking to the same native session;
//! its process stays up between rounds (and is resumed by session id after a restart). The call waits for the round
//! up to a deadline, then answers "still working" and the round goes on; a call with the thread and no prompt waits again.
//!
//! Ownership: a thread's process is registered on the session the moment it is spawned, so a failed open, a cancel or
//! the session closing always reaches it. The session's close hands every child process to the close future (and to the
//! hard-kill list of a host shutdown); a process that finishes spawning after the close is ended at once

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, watch};

use acpira_shared::subagents::{RelayMode, SubagentDuty, SubagentHarness, SubagentPersona, SubagentState};
use acpira_shared::transcript::{AgentBlock, DiffKind, Draft, SessionStatus, ToolCallBlock, ToolContent, ToolKind, ToolStatus, Turn};

use crate::acp::session::attachments::{prepare_prompt, prompt_caps_of};
use crate::acp::session::hooks::{Reach, edit_paths, row_reach};
use crate::acp::session::prompt::Origin;
use crate::acp::session::updates::current_turn_index;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::runtime_info_of;
use crate::acp::transcript::subagent_tree::{RelayOpen, RouteCtx};
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::process::{AgentProcess, ClientHandlers};
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::handoff::{self, Level, Pack, PackFile, Recipe, Scope};
use crate::hooks::snapshot::{Snapshot, in_work_tree};
use crate::i18n::t;
use crate::relay::roster::Roster;
use crate::relay::wire::{AskArgs, AskCallerArgs, HubReply};
use crate::relay::{MAX_DEPTH, MAX_RUNNING};

/// How long a call waits for the round when the MCP side names no deadline
const DEFAULT_WAIT: Duration = Duration::from_secs(50);
const MAX_WAIT: Duration = Duration::from_secs(30 * 60);
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
/// Automatic messages delivering finished rounds to the root, per prompt of the user's own
const MAX_WAKES: u32 = 6;
const TITLE_MAX: usize = 48;
/// Files a round's change note names, at most (the rest are counted)
const NOTE_FILES: usize = 20;
/// The handoff pack's name as an attachment
const PACK_NAME: &str = "handoff-pack.md";
/// What the brief says of an attached pack
const PACK_LINE: &str = "Attached is the handoff pack of the caller's uncommitted changes: their diff and, where they are small, the \
changed files in full. Start from it, and open other files only for what it does not show.";
/// Mode ids that mean "read only" across the built-in CLIs: codex-acp `read-only`, Claude / OpenCode / Grok `plan`
const READ_ONLY_MODES: [&str; 3] = ["read-only", "plan", "ask"];
/// Modes that skip the CLI's own approvals: Claude `bypassPermissions`, Codex `agent-full-access`, Devin `bypass`,
/// Antigravity / Kimi `yolo`; a mode the agent marks `_meta.kind` `full_access` counts too
const BYPASS_MODES: [&str; 4] = ["bypassPermissions", "agent-full-access", "bypass", "yolo"];
const MODEL_KEYS: [&str; 1] = ["model"];
const EFFORT_KEYS: [&str; 3] = ["thought_level", "reasoning_effort", "effort"];
const MODE_KEYS: [&str; 1] = ["mode"];

/// The final word of a round: the reply text for the caller, or why there is none
type RoundEnd = Option<std::result::Result<String, String>>;

/// Where a round's result stands: read by a call, owed to its caller, delivered in an automatic message, or voided (the
/// user stopped, the turn it belonged to was edited away, its caller's round ended without it)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owed {
  Running,
  Unread,
  /// Sent with the automatic message being claimed now; back to unread should that message not go out
  Delivering,
  Read,
  Delivered,
  Void,
}

/// One round in the delivery ledger: its node, its caller (None = the root, else the summoned thread that sent it), and
/// the caller's turn that sent it (rounds sent in one turn are delivered together, once all of them ended): the root's
/// turn index, or a summoned caller's follow-up count within its round
struct Owing {
  node: String,
  owner: Option<String>,
  cohort: usize,
  state: Owed,
}

/// One thread's process and native session
pub(crate) struct RelayProc {
  pub proc: Arc<AgentProcess>,
  /// The native session id; empty while the session is still being opened
  pub peer: String,
  /// The session's modes and config options as last known: the open / load / resume answer, then every change
  opened: Value,
  /// The CLI mode the session opened in: a work round after a consult switches back to it
  start_mode: Option<String>,
  /// The mode the previous round ran in (None = not confirmed: the next round applies it again)
  mode: Option<RelayMode>,
  /// The persona's model / effort as last applied: a persona edited since gets them applied again
  applied: (Option<String>, Option<String>),
}

/// Per-session relay state (in `Core`)
pub(crate) struct Relays {
  pub procs: HashMap<String, RelayProc>,
  /// Rounds in flight, by thread: the call waiting on one watches its end
  pub rounds: HashMap<String, watch::Receiver<RoundEnd>>,
  /// Root `ask_agent` rows seen before their round existed
  calls: Vec<PendingCall>,
  /// Root rows recognized as `ask_agent` by name: claude-agent-acp 0.83.0 names the tool only on the first `tool_call`
  /// (with an empty `rawInput`) and streams the arguments in later updates that omit the unchanged title
  rows: HashSet<String>,
  /// Setup notes of a round (a model the CLI lacks, a refused mode), by node: every read of the round repeats them
  notes: HashMap<String, Vec<String>>,
  /// Starts between spawn and registration: the close waits until they are done
  starting: Arc<watch::Sender<usize>>,
  /// The session closed: no round starts, a process spawned afterwards is ended at once
  closed: bool,
  /// Rounds and whether their results reached their callers (in memory only: the children end with the session, so a
  /// reopened one owes nothing)
  ledger: Vec<Owing>,
  /// Bumped whenever a round ends: a summoned caller waiting to hear from the rounds it sent off looks again
  ended: Arc<watch::Sender<u64>>,
  /// A summoned caller's follow-ups so far in its running round, by thread: the cohort of the rounds it sends now
  follow_ups: HashMap<String, usize>,
  /// Automatic deliveries since the user last prompted (`MAX_WAKES`)
  wakes: u32,
  /// Calls waiting on a thread's round right now: a round ending under one is read by it, never delivered as well
  waiters: HashMap<String, usize>,
  /// Bumped whenever a summoned child opens a question card: a call waiting on its thread answers with the question
  pub(crate) asked: Arc<watch::Sender<u64>>,
  /// Child question cards a caller has been told about (by a waiting call or an automatic message), and the ones the
  /// automatic message being claimed now carries
  told: HashSet<String>,
  telling: Vec<String>,
  /// What each thread was last told of the caller's changes: the pack's base and the files with their line counts, so
  /// its next round hears only what moved since
  packs: HashMap<String, (String, Vec<PackFile>)>,
}

impl Relays {
  fn owing(&mut self, node: &str) -> Option<&mut Owing> {
    self.ledger.iter_mut().find(|o| o.node == node)
  }

  /// A call answered with this round's outcome: nothing is owed for it any more
  fn read(&mut self, node: &str) {
    if let Some(o) = self.owing(node).filter(|o| matches!(o.state, Owed::Running | Owed::Unread)) {
      o.state = Owed::Read;
    }
  }

  /// Rounds whose results nobody will want: the user's stop, an edit removing their turn
  pub(crate) fn void(&mut self, nodes: Option<&[String]>) {
    for o in &mut self.ledger {
      if matches!(o.state, Owed::Running | Owed::Unread | Owed::Delivering) && nodes.is_none_or(|n| n.contains(&o.node)) {
        o.state = Owed::Void;
      }
    }
  }

  /// Finished results owed to one caller whose whole turn's batch has ended
  fn deliverable(&self, owner: Option<&str>) -> Vec<String> {
    let mine = |o: &&Owing| o.owner.as_deref() == owner;
    let running: HashSet<usize> = self.ledger.iter().filter(mine).filter(|o| o.state == Owed::Running).map(|o| o.cohort).collect();
    self.ledger.iter().filter(mine).filter(|o| o.state == Owed::Unread && !running.contains(&o.cohort)).map(|o| o.node.clone()).collect()
  }

  /// Rounds a summoned caller sent off whose results it has not had yet
  fn owes(&self, owner: &str) -> bool {
    self.ledger.iter().any(|o| o.owner.as_deref() == Some(owner) && matches!(o.state, Owed::Running | Owed::Unread))
  }

  /// The automatic message was claimed (`true`: its results are delivered) or did not go out (owed again)
  pub(crate) fn delivered(&mut self, sent: bool) {
    for o in self.ledger.iter_mut().filter(|o| o.state == Owed::Delivering) {
      o.state = if sent { Owed::Delivered } else { Owed::Unread };
    }
    let telling = std::mem::take(&mut self.telling);
    if sent {
      self.told.extend(telling);
    } else {
      self.wakes = self.wakes.saturating_sub(1);
    }
  }

  /// A prompt of the user's own: the automatic deliveries start counting again
  pub(crate) fn user_prompted(&mut self) {
    self.wakes = 0;
  }
}

impl Default for Relays {
  fn default() -> Self {
    Relays {
      procs: HashMap::new(),
      rounds: HashMap::new(),
      calls: vec![],
      rows: HashSet::new(),
      notes: HashMap::new(),
      starting: Arc::new(watch::channel(0).0),
      closed: false,
      ledger: vec![],
      ended: Arc::new(watch::channel(0).0),
      follow_ups: HashMap::new(),
      wakes: 0,
      waiters: HashMap::new(),
      asked: Arc::new(watch::channel(0).0),
      told: HashSet::new(),
      telling: vec![],
      packs: HashMap::new(),
    }
  }
}

/// One start in flight (`Relays::starting`), counted for as long as it lives
struct Starting(Arc<watch::Sender<usize>>);

impl Starting {
  fn enter(count: &Arc<watch::Sender<usize>>) -> Starting {
    count.send_modify(|n| *n += 1);
    Starting(count.clone())
  }
}

impl Drop for Starting {
  fn drop(&mut self) {
    self.0.send_modify(|n| *n = n.saturating_sub(1));
  }
}

/// A root `ask_agent` row waiting for its node, with the arguments that tell which round it started
struct PendingCall {
  tool_call_id: String,
  agent: String,
  thread: Option<String>,
  prompt: String,
  turn_index: usize,
}

/// The client side of a summoned child's process: everything goes to the parent session, which routes by session id
struct RelayHandlers {
  session: Weak<AcpSession>,
  thread: String,
  /// A session/load replays the thread's earlier rounds; those are already in their own nodes
  quiet: Arc<AtomicBool>,
}

/// The routing key a thread's traffic carries into the parent session: two CLIs may mint the same session id (the
/// parent's own included), so the child's real id never reaches the parent's router
pub(crate) fn route_key(thread: &str) -> String {
  format!("relay:{thread}")
}

impl RelayHandlers {
  fn stamp(&self, mut v: Value) -> Value {
    if v.is_object() {
      v["sessionId"] = Value::from(route_key(&self.thread));
    }
    v
  }
}

impl ClientHandlers for RelayHandlers {
  fn on_update(&self, params: Value) {
    if self.quiet.load(Ordering::Relaxed) {
      return;
    }
    if let Some(s) = self.session.upgrade() {
      s.on_update(self.stamp(params));
    }
  }

  fn on_permission(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let req = self.stamp(req);
    Box::pin(async move {
      match s {
        Some(s) => s.on_permission(req, cancel).await,
        None => Ok(json!({ "outcome": { "outcome": "cancelled" } })),
      }
    })
  }

  fn on_elicitation(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let req = self.stamp(req);
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_elicitation(req, cancel).await),
        None => Ok(json!({ "action": "cancel" })),
      }
    })
  }

  fn on_grok_question(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let req = self.stamp(req);
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_grok_question(req, cancel).await),
        None => Ok(json!({ "outcome": "skip_interview" })),
      }
    })
  }

  fn on_stderr(&self, line: &str) {
    if let Some(s) = self.session.upgrade() {
      s.log(&format!("relay {}: stderr: {line}", short(&self.thread)));
    }
  }

  fn on_exit(&self, code: Option<i32>, signal: Option<String>) {
    let Some(s) = self.session.upgrade() else { return };
    s.log(&format!("relay {}: exit code={code:?} signal={signal:?}", short(&self.thread)));
    s.core.lock().relays.procs.remove(&self.thread);
  }
}

fn short(id: &str) -> String {
  id.chars().take(8).collect()
}

fn clip(s: &str, max: usize) -> String {
  let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
  let mut out: String = line.chars().take(max).collect();
  if line.chars().count() > max {
    out.push('…');
  }
  out
}

/// The task the child receives: the caller's prompt, the persona's standing brief, the read-only rule for a consult, what
/// is handed over of the caller's changes, and how to end: the caller reads only the final reply, so it says what was changed, checked and assumed (a reviewer:
/// its findings by severity and place), which is what the caller weighs before it acts on any of it
fn compose_brief(prompt: &str, persona: &SubagentPersona, mode: RelayMode, handover: Option<&str>) -> String {
  let mut parts = vec![prompt.trim().to_owned()];
  if let Some(b) = persona.brief.as_deref().filter(|b| !b.trim().is_empty()) {
    parts.push(b.trim().to_owned());
  }
  if mode == RelayMode::Consult {
    parts.push("You are consulted for an opinion: read whatever you need, but do not modify any file.".into());
  }
  if let Some(h) = handover {
    parts.push(h.to_owned());
  }
  parts.push(format!(
    "(Another agent working in this same repository asked you this. It reads only your final reply, so make that reply \
self-contained. {} When a decision only it can make blocks you, ask it with ask_caller; otherwise decide yourself.)",
    closing(persona.duty, mode)
  ));
  parts.join("\n\n")
}

/// What the final reply ends with, by what the round was for
fn closing(duty: SubagentDuty, mode: RelayMode) -> &'static str {
  match (duty, mode) {
    (SubagentDuty::Reviewer, _) => {
      "End with your findings, most severe first, each as: severity (high / medium / low), path:line, what is wrong and a \
concrete way it fails; if you found nothing wrong, say so plainly."
    }
    (_, RelayMode::Consult) => "End with what you found, each point with its path:line, and what you assumed.",
    _ => "End with the files you changed (or that you changed none), what you ran to check the work and its result, and what you assumed.",
  }
}

/// The files a round's edit rows name, with the lines their diffs add and remove (repeated edits of a file add up), and
/// whether it also ran something that can write anywhere (a command, an MCP tool, an edit naming no file): the rule the
/// workspace gate's `row_reach` follows
fn round_edits(turns: &[Turn]) -> (Vec<(String, u64, u64)>, bool) {
  let mut files: Vec<(String, u64, u64)> = vec![];
  let mut anywhere = false;
  let mut bump = |path: &str, add: u64, del: u64| match files.iter_mut().find(|f| f.0 == path) {
    Some(f) => {
      f.1 += add;
      f.2 += del;
    }
    None => files.push((path.to_owned(), add, del)),
  };
  for turn in turns.iter().filter_map(Turn::as_agent) {
    for b in &turn.blocks {
      let AgentBlock::ToolCall(tc) = b else { continue };
      if row_reach(tc) == Reach::Any {
        anywhere = true;
      }
      if !matches!(tc.kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move) || tc.status == ToolStatus::Failed {
        continue;
      }
      // `contents` holds every item when there are several, `content` the primary one otherwise
      let items: Vec<&ToolContent> = match &tc.contents {
        Some(list) => list.iter().collect(),
        None => tc.content.iter().collect(),
      };
      let mut counted = false;
      for item in items {
        if let ToolContent::Diff { lines, source: Some(src) } = item {
          let n = |k: DiffKind| lines.iter().filter(|l| l.kind == k).count() as u64;
          bump(&src.path, n(DiffKind::Add), n(DiffKind::Del));
          counted = true;
        }
      }
      if !counted {
        let paths = edit_paths(tc);
        let stat = tc.diff_stat.filter(|_| paths.len() == 1);
        for p in paths {
          bump(&p, stat.map_or(0, |s| s.add), stat.map_or(0, |s| s.del));
        }
      }
    }
  }
  (files, anywhere)
}

/// A path as the caller reads it: relative to the session's folder when inside it
fn shown_path(cwd: &str, path: &str) -> String {
  let p = std::path::Path::new(path);
  let rel = p.strip_prefix(cwd).ok().map(|r| r.to_path_buf()).or_else(|| {
    let top = std::fs::canonicalize(cwd).ok()?;
    let canon = std::fs::canonicalize(p).ok().or_else(|| Some(std::fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?)))?;
    canon.strip_prefix(&top).ok().map(|r| r.to_path_buf())
  });
  match rel {
    Some(r) if !r.as_os_str().is_empty() => r.to_string_lossy().replace('\\', "/"),
    _ => path.to_owned(),
  }
}

/// The round's change note the caller reads after the reply: what its edit rows changed, and, when it also ran something
/// that can write anywhere, the other files whose state changed meanwhile (`changed`: None outside a git work tree,
/// where nothing can be seen)
fn change_note(cwd: &str, edits: &[(String, u64, u64)], anywhere: bool, changed: Option<&[PathBuf]>) -> String {
  let listed: Vec<(String, u64, u64)> = {
    let mut out: Vec<(String, u64, u64)> = vec![];
    for (p, a, d) in edits {
      let shown = shown_path(cwd, p);
      match out.iter_mut().find(|f| f.0 == shown) {
        Some(f) => {
          f.1 += a;
          f.2 += d;
        }
        None => out.push((shown, *a, *d)),
      }
    }
    out
  };
  let extra: Vec<String> = match (anywhere, changed) {
    (true, Some(paths)) => {
      paths.iter().map(|p| shown_path(cwd, &p.to_string_lossy())).filter(|p| !listed.iter().any(|f| &f.0 == p)).collect()
    }
    _ => vec![],
  };
  let names = |list: Vec<String>| {
    let more = list.len().saturating_sub(NOTE_FILES);
    let mut out = list.into_iter().take(NOTE_FILES).collect::<Vec<_>>().join(", ");
    if more > 0 {
      out.push_str(&format!(" and {more} more"));
    }
    out
  };
  let edited = names(listed.iter().map(|(p, a, d)| if a + d > 0 { format!("{p} (+{a} -{d})") } else { p.clone() }).collect());
  match (listed.is_empty(), extra.is_empty()) {
    (true, true) if anywhere && changed.is_none() => {
      "it reported no file edits, and outside a git work tree Acpira cannot see what its commands changed".into()
    }
    (true, true) => "it changed no files".into(),
    (false, true) => format!("files it edited: {edited}"),
    (true, false) => format!("it edited no file itself, but these changed while its commands ran: {}", names(extra)),
    (false, false) => format!("files it edited: {edited}; these also changed while its commands ran: {}", names(extra)),
  }
}

/// The config option whose category or id is one of `keys`
fn find_option<'a>(options: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
  options?.as_array()?.iter().find(|o| {
    let id = o.get("id").and_then(Value::as_str).unwrap_or("");
    let cat = o.get("category").and_then(Value::as_str).unwrap_or("");
    keys.iter().any(|k| *k == id || *k == cat)
  })
}

/// A select's `(value, name)` pairs; grouped selects nest their values one level down
fn option_values(o: &Value) -> Vec<(String, String)> {
  let mut values = vec![];
  for v in o.get("options").and_then(Value::as_array).into_iter().flatten() {
    let inner = v.get("options").and_then(Value::as_array);
    for x in inner.map(|l| l.iter().collect::<Vec<_>>()).unwrap_or_else(|| vec![v]) {
      if let Some(val) = x.get("value").and_then(Value::as_str) {
        values.push((val.to_owned(), x.get("name").and_then(Value::as_str).unwrap_or(val).to_owned()));
      }
    }
  }
  values
}

/// `(config id, chosen value)` for a select whose category or id is one of `keys`, matching `want` by value or name
fn pick_option(options: Option<&Value>, keys: &[&str], want: &str) -> Option<(String, String)> {
  let o = find_option(options, keys)?;
  let id = o.get("id").and_then(Value::as_str).unwrap_or("");
  let want = want.trim().to_lowercase();
  let values = option_values(o);
  let exact = values.iter().find(|(v, n)| v.to_lowercase() == want || n.to_lowercase() == want);
  let loose = || values.iter().find(|(v, n)| v.to_lowercase().contains(&want) || n.to_lowercase().contains(&want));
  exact.or_else(loose).map(|(v, _)| (id.to_owned(), v.clone()))
}

/// The display name of a select's current value
fn current_name(options: Option<&Value>, keys: &[&str]) -> Option<String> {
  let o = find_option(options, keys)?;
  let cur = o.get("currentValue").and_then(Value::as_str)?;
  Some(option_values(o).into_iter().find(|(v, _)| v == cur).map(|(_, n)| n).unwrap_or_else(|| cur.to_owned()))
}

/// The session's mode ids: the `modes` list, else the values of a `mode` select
fn mode_ids(opened: &Value) -> Vec<String> {
  let listed: Vec<String> = opened
    .pointer("/modes/availableModes")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned))
    .collect();
  if !listed.is_empty() {
    return listed;
  }
  find_option(opened.get("configOptions"), &MODE_KEYS).map(|o| option_values(o).into_iter().map(|(v, _)| v).collect()).unwrap_or_default()
}

fn current_mode(opened: &Value) -> Option<String> {
  opened
    .pointer("/modes/currentModeId")
    .and_then(Value::as_str)
    .map(str::to_owned)
    .or_else(|| find_option(opened.get("configOptions"), &MODE_KEYS).and_then(|o| o.get("currentValue")).and_then(Value::as_str).map(str::to_owned))
}

fn is_read_only(mode: &str) -> bool {
  READ_ONLY_MODES.contains(&mode)
}

/// The session's full-access mode ids: a known id, or one whose `_meta.kind` is `full_access` (in the `modes` list or
/// among the values of a `mode` select)
/// Whether the CLI is still in a full-access mode although the round asked for less
fn stuck_in_bypass(opened: &Value, mode: RelayMode) -> bool {
  mode != RelayMode::Bypass && current_mode(opened).is_some_and(|c| bypass_modes(opened).contains(&c))
}

fn bypass_modes(opened: &Value) -> Vec<String> {
  let full = |m: &Value| m.pointer("/_meta/kind").and_then(Value::as_str) == Some("full_access");
  let mut marked: Vec<String> = vec![];
  for m in opened.pointer("/modes/availableModes").and_then(Value::as_array).into_iter().flatten() {
    if full(m)
      && let Some(id) = m.get("id").and_then(Value::as_str)
    {
      marked.push(id.to_owned());
    }
  }
  if let Some(o) = find_option(opened.get("configOptions"), &MODE_KEYS) {
    for v in o.get("options").and_then(Value::as_array).into_iter().flatten() {
      for x in v.get("options").and_then(Value::as_array).map(|l| l.iter().collect::<Vec<_>>()).unwrap_or_else(|| vec![v]) {
        if full(x)
          && let Some(val) = x.get("value").and_then(Value::as_str)
        {
          marked.push(val.to_owned());
        }
      }
    }
  }
  mode_ids(opened).into_iter().filter(|m| BYPASS_MODES.contains(&m.as_str()) || marked.contains(m)).collect()
}

/// One `session/set_config_option`; the answer's option list (model-dependent effort values) replaces the known one
async fn set_config(proc: &AgentProcess, sid: &str, opened: &mut Value, id: &str, value: &str) -> std::result::Result<(), String> {
  let r = proc
    .request("session/set_config_option", json!({ "sessionId": sid, "configId": id, "value": value }))
    .await
    .map_err(|e| e.to_string())?;
  if !opened.is_object() {
    *opened = json!({});
  }
  match r.get("configOptions") {
    Some(list) if list.is_array() => opened["configOptions"] = list.clone(),
    _ => {
      if let Some(o) = opened.get_mut("configOptions").and_then(Value::as_array_mut).and_then(|l| l.iter_mut().find(|o| o.get("id").and_then(Value::as_str) == Some(id))) {
        o["currentValue"] = Value::from(value);
      }
    }
  }
  Ok(())
}

/// The mode a round runs in. A persona with a fixed access (consult, work, bypass) runs in it whatever the call asks:
/// the setting is the user's, and a caller asking for a consult must not bring approval cards back to a bypass persona.
/// Only an inherit persona leaves it to the caller: the call's mode overrides it either way (a caller in plan mode may
/// hand out work, one in work mode may ask for bypass), and without one it takes the caller's own access this round
fn round_mode(persona: RelayMode, asked: Option<&str>, caller: impl FnOnce() -> RelayMode) -> RelayMode {
  if persona != RelayMode::Inherit {
    return persona;
  }
  match asked {
    Some("consult") => RelayMode::Consult,
    Some("work") => RelayMode::Work,
    Some("bypass") => RelayMode::Bypass,
    _ => caller(),
  }
}

/// The caller's current access, for an inherit persona: a summoned caller's own round mode; the root's mode by id (a
/// read-only one, a full-access one by id or `_meta.kind`, yolo included) — anything else, or no mode at all, is work
fn caller_access(c: &Core, caller: Option<&str>) -> RelayMode {
  if let Some(th) = caller {
    return c.tree.relay_latest(th).map(|(_, h, ..)| h.mode).filter(|m| *m != RelayMode::Inherit).unwrap_or(RelayMode::Consult);
  }
  let controls = &c.state.controls;
  let Some(id) = controls.mode_id.as_deref() else {
    return if c.perms.auto_approve { RelayMode::Bypass } else { RelayMode::Work };
  };
  let marked = || {
    let select = controls.mode_config_id.as_deref().and_then(|cid| controls.options.iter().find(|o| o.id == cid)).map(|o| o.options.as_slice()).unwrap_or(&[]);
    controls.modes.iter().chain(select).any(|m| m.id == id && m.kind.as_deref() == Some("full_access"))
  };
  if c.perms.auto_approve || BYPASS_MODES.contains(&id) || marked() {
    RelayMode::Bypass
  } else if is_read_only(id) {
    RelayMode::Consult
  } else {
    RelayMode::Work
  }
}

/// The thread's display: persona name and CLI
fn who(persona: &SubagentPersona) -> String {
  format!("{} ({})", persona.name, persona.agent)
}

/// Whether a call (its `agent` and `thread` arguments) is the one that opened a round with this harness / persona name.
/// A call without `thread` usually opens a thread's first round, but a scout's or an executor's continues its idle one
fn call_fits(agent: &str, thread: Option<&str>, h: &SubagentHarness, role: Option<&str>) -> bool {
  let agent = agent.trim();
  match thread.map(str::trim).filter(|t| !t.is_empty()) {
    Some(th) => h.thread == th && h.round > 1,
    None => h.persona.as_deref() == Some(agent) || role.is_some_and(|r| r.eq_ignore_ascii_case(agent)),
  }
}

/// What a finished or running round is shown as, from its node: no roster lookup
fn node_persona(h: &SubagentHarness, name: Option<&str>) -> SubagentPersona {
  SubagentPersona {
    id: h.persona.clone().unwrap_or_default(),
    name: name.map(str::to_owned).unwrap_or_else(|| h.agent.clone()),
    agent: h.agent.clone(),
    model: None,
    effort: None,
    mode: h.mode,
    when: String::new(),
    brief: None,
    enabled: true,
    duty: SubagentDuty::General,
    changes: false,
  }
}

/// The persona's latest conversation this caller started in the session, when it is idle and its process still up: a
/// scout or an executor called without `thread` continues it. Otherwise the call starts fresh rather than fall back to
/// an older conversation (one the caller may have left with `new_thread`): a busy one is busy, reopening one whose
/// process is gone might fail, and one in a CLI the persona no longer runs in is another agent
fn idle_thread(c: &Core, persona: &SubagentPersona, caller: Option<&str>) -> Option<String> {
  let latest = c
    .relays
    .ledger
    .iter()
    .rev()
    .filter(|o| o.owner.as_deref() == caller)
    .filter_map(|o| c.tree.relay_harness(&o.node).map(|(h, _)| h))
    .find(|h| h.persona.as_deref() == Some(persona.id.as_str()))?;
  let idle = c.tree.relay_latest(&latest.thread).is_some_and(|(.., state)| state != SubagentState::Running);
  let alive = c.relays.procs.get(&latest.thread).is_some_and(|rp| rp.proc.alive() && !rp.peer.is_empty());
  (latest.agent == persona.agent && idle && alive).then_some(latest.thread)
}

/// The persona a thread continues with: its current settings, in the CLI the thread was started in
fn thread_persona(roster: &Roster, h: &SubagentHarness, name: Option<&str>) -> Result<SubagentPersona> {
  let label = name.unwrap_or(h.persona.as_deref().unwrap_or(&h.agent));
  let Some(p) = roster.find(h.persona.as_deref().unwrap_or("")) else {
    return Err(anyhow!("{label} is no longer an enabled subagent, so thread {} cannot continue.", h.thread));
  };
  if p.agent != h.agent {
    return Err(anyhow!(
      "{} now runs in {}, but thread {} was started in {}. Leave `thread` out to start a new conversation.",
      p.name,
      p.agent,
      h.thread,
      h.agent
    ));
  }
  Ok(p)
}

impl AcpSession {
  /// One `ask_agent` call from an agent of this session (the root, or a summoned child when `caller` names its thread)
  pub async fn relay_ask(self: Arc<Self>, roster: Arc<Roster>, caller: Option<String>, depth: u32, args: AskArgs, out: mpsc::UnboundedSender<HubReply>) {
    let deadline = args.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    // A new task is sent off unless the call asks to wait: the ledger delivers the reply, to the root in an automatic
    // message, to a summoned caller in a follow-up within its own round (`relay_follow_ups`)
    let wait = args.wait.unwrap_or(false);
    match self.clone().relay_start(&roster, caller, depth, args, None).await {
      Ok(Start::Started { thread, persona }) if !wait => {
        let _ = out.send(HubReply::Done(dispatched_text(&thread, &persona)));
      }
      Ok(Start::Answered { thread, persona }) if !wait => {
        let _ = out.send(HubReply::Done(format!(
          "Your answer went to {} (thread: {thread}). It carries on; Acpira sends you its reply in a message when it is done.",
          who(&persona)
        )));
      }
      Ok(Start::Answered { thread, persona }) => self.relay_wait(&thread, &persona, deadline, &out).await,
      Ok(Start::Started { thread, persona }) | Ok(Start::Wait { thread, persona }) => self.relay_wait(&thread, &persona, deadline, &out).await,
      Ok(Start::Ended(reply)) => {
        let _ = out.send(reply);
      }
      Err(e) => {
        let _ = out.send(HubReply::Error(e.to_string()));
      }
    }
  }

  /// `ask_caller` from the CLI of a summoned thread: the question becomes a card on the thread's running round and travels
  /// like one its CLI asked itself (a call waiting on the thread, the automatic message, the user's card). No answer
  /// within the client's bound settles it skipped, and the child goes on with an assumption it states
  pub async fn relay_ask_caller(self: Arc<Self>, thread: String, args: AskCallerArgs, out: mpsc::UnboundedSender<HubReply>) {
    use acpira_shared::transcript::{Question, QuestionKind, QuestionOption, QuestionOutcome};
    let text = args.question.trim().to_owned();
    if text.is_empty() {
      let _ = out.send(HubReply::Error("`question` is empty: say what you need decided.".into()));
      return;
    }
    let deadline = args.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    let mut options: Vec<QuestionOption> = vec![];
    for label in args.options.iter().map(|o| o.trim()).filter(|o| !o.is_empty()) {
      if !options.iter().any(|o| o.label == label) {
        options.push(QuestionOption { id: label.to_owned(), label: label.to_owned(), description: None });
      }
    }
    let kind = if options.is_empty() { QuestionKind::Text } else { QuestionKind::Single };
    let question = Question { id: "answer".into(), title: None, text, kind, options, other: None, numeric: None, required: None };
    let opened = {
      let mut c = self.core.lock();
      match c.tree.relay_latest(&thread) {
        Some((node, .., SubagentState::Running)) if Self::relay_question(&c, &node).is_none() => Ok(self.open_caller_question(&mut c, &node, vec![question])),
        Some((.., SubagentState::Running)) => Err("You already have a question waiting for an answer; wait for it before asking another."),
        _ => Err("Your round is over, so there is no caller waiting to answer."),
      }
    };
    let (mut rx, block) = match opened {
      Ok(o) => o,
      Err(e) => {
        let _ = out.send(HubReply::Error(e.into()));
        return;
      }
    };
    // With the root idle, the automatic message carries the question now
    self.relay_nudge();
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    let reply = loop {
      tokio::select! {
        r = &mut rx => break r.ok(),
        // A progress line that cannot be written: the tool call was abandoned
        _ = tick.tick() => if out.send(HubReply::Progress("Waiting for an answer".into())).is_err() {
          self.close_question(&block, QuestionOutcome::Cancelled);
          return;
        },
        _ = tokio::time::sleep_until(until) => {
          self.close_question(&block, QuestionOutcome::Skipped);
          let _ = out.send(HubReply::Done(
            "No answer came in time. Go on with the most reasonable assumption, and state it in your final reply.".into(),
          ));
          return;
        }
      }
    };
    let _ = out.send(match reply {
      Some(v) if v.get("cancelled").is_some() || v.is_null() => HubReply::Error("The question was withdrawn: your round is being stopped.".into()),
      Some(v) => match v.get("answers").and_then(|a| a.get("answer")).filter(|_| v.get("skipped") != Some(&Value::Bool(true))) {
        Some(a) => HubReply::Done(format!("Answer: {}", answer_text(a))),
        None => HubReply::Done("The question was skipped. Go on with the most reasonable assumption, and state it in your final reply.".into()),
      },
      None => HubReply::Error("The question was withdrawn: your round is being stopped.".into()),
    });
  }

  async fn relay_start(self: Arc<Self>, roster: &Roster, caller: Option<String>, depth: u32, args: AskArgs, shared: Option<Arc<Option<Pack>>>) -> Result<Start> {
    if depth >= MAX_DEPTH {
      return Err(anyhow!("Summoned agents may not summon further than {MAX_DEPTH} levels deep."));
    }
    let mut thread = args.thread.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
    let prompt = args.prompt.trim().to_owned();
    // A scout or an executor keeps its conversation: a new task without `thread` continues the idle one this caller
    // started (unless the call asks for a fresh start); a reviewer, and any other persona, starts fresh
    if thread.is_none() && !prompt.is_empty() && args.new_thread != Some(true)
      && let Some(p) = roster.find(&args.agent).filter(|p| p.duty.keeps_thread())
    {
      thread = idle_thread(&self.core.lock(), &p, caller.as_deref());
    }
    let latest = thread.as_deref().and_then(|th| self.core.lock().tree.relay_latest(th));
    if thread.is_some() && latest.is_none() {
      return Err(anyhow!("Unknown thread {}. Start a new one by leaving `thread` out.", thread.unwrap_or_default()));
    }
    // Waiting on a thread or reading its last round goes by the node alone: a persona edited or removed since then
    // does not take a finished answer away
    if let Some((node, h, name, _, state)) = &latest {
      let th = thread.clone().unwrap_or_default();
      let shown = node_persona(h, name.as_deref());
      if *state == SubagentState::Running {
        // A child waiting on a question: `answers` (or a prompt, for a single question) answers it, like the user's card
        let open = { Self::relay_question(&self.core.lock(), node) };
        if let Some((block, questions)) = open
          && (args.answers.is_some() || !prompt.is_empty())
        {
          let answers = caller_answers(&questions, args.answers.as_deref(), &prompt).map_err(|e| anyhow!("{} {e}", who(&shown)))?;
          self.answer_questions(&block, &answers, false);
          self.core.lock().relays.told.insert(block);
          return Ok(Start::Answered { thread: th, persona: shown });
        }
        if args.answers.is_some() {
          return Err(anyhow!("{} has no open question now (the user may have answered it already).", who(&shown)));
        }
        if prompt.is_empty() {
          return Ok(Start::Wait { thread: th, persona: shown });
        }
        return Err(busy(&shown, &th));
      }
      if args.answers.is_some() {
        return Err(anyhow!("{} has no open question: its round is over.", who(&shown)));
      }
      if prompt.is_empty() {
        let mut c = self.core.lock();
        c.relays.read(node);
        let end = round_result(&c, node);
        return Ok(Start::Ended(final_reply(end, &th, &shown)));
      }
    }
    // A new round: the thread's persona as it is set now, else the named one
    let persona = match &latest {
      Some((_, h, name, _, _)) => thread_persona(roster, h, name.as_deref())?,
      None if args.agent.trim().is_empty() => return Err(anyhow!("Name the agent to summon (`agent`), or a `thread` to continue.")),
      None => roster.find(&args.agent).ok_or_else(|| {
        let ids: Vec<String> = roster.enabled().into_iter().map(|p| p.id).collect();
        anyhow!("No subagent named {:?}. Available: {}.", args.agent, if ids.is_empty() { "none".into() } else { ids.join(", ") })
      })?,
    };
    if prompt.is_empty() {
      return Err(anyhow!("`prompt` is empty: say what {} should do.", who(&persona)));
    }
    let mode = round_mode(persona.mode, args.mode.as_deref(), || caller_access(&self.core.lock(), caller.as_deref()));
    if self.deps.registry.try_get(&persona.agent).is_none() {
      return Err(anyhow!("{} runs in {:?}, which is not an agent Acpira knows.", persona.name, persona.agent));
    }
    // `changes` decides, else the persona's switch; a thread that was handed the pack keeps hearing what moved since
    let told = thread.as_deref().is_some_and(|th| self.core.lock().relays.packs.contains_key(th));
    let handoff = args.changes.unwrap_or(persona.changes || told).then(|| Handoff {
      scope: caller_scope(&self.core.lock(), caller.as_deref()),
      recipe: handoff::recipe_in(&roster.dir()),
      shared: shared.clone(),
    });
    let title = args.title.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(|x| clip(x, TITLE_MAX)).unwrap_or_else(|| clip(&prompt, TITLE_MAX));
    let round = latest.as_ref().map(|(_, h, ..)| h.round + 1).unwrap_or(1);
    let peer = latest.as_ref().and_then(|(_, _, _, p, _)| p.clone());
    let (tx, rx) = watch::channel::<RoundEnd>(None);
    // Everything that decides whether the round may start, and the round's registration, happen under one lock: two
    // calls on one thread cannot both start a round, and a waiter always finds the round it was told about
    let (node, thread, starting) = {
      let mut c = self.core.lock();
      if c.relays.closed {
        return Err(anyhow!("This Acpira session is closing."));
      }
      if let (Some(th), Some((was, ..))) = (&thread, &latest) {
        let now = c.tree.relay_latest(th);
        if !now.as_ref().is_some_and(|(id, .., state)| id == was && *state != SubagentState::Running) {
          return Err(busy(&persona, th));
        }
      }
      let running = c.tree.relay_running().len();
      if running >= MAX_RUNNING {
        return Err(anyhow!(
          "{running} summoned agents are already running in this session (at most {MAX_RUNNING}). Wait for one first: call ask_agent with its thread and an empty prompt."
        ));
      }
      let parent = caller.as_deref().and_then(|th| c.tree.relay_latest(th)).map(|(id, ..)| id);
      let turn_index = current_turn_index(&c);
      let harness = SubagentHarness { agent: persona.agent.clone(), persona: Some(persona.id.clone()), mode, thread: thread.clone().unwrap_or_default(), round, session_id: peer.clone(), effort: None };
      let model = persona.model.clone();
      let node = c.tree.relay_open(RelayOpen { parent, turn_index, harness, role: persona.name.clone(), title, task: prompt.clone(), model });
      let thread = c.tree.relay_thread_of(&node).unwrap_or_else(|| node.clone());
      // A root row announced before the round existed takes the node now
      let opened = c.tree.relay_latest(&thread).map(|(_, h, ..)| h);
      let pending = opened.and_then(|h| {
        c.relays.calls.iter().position(|p| p.turn_index == turn_index && norm(&p.prompt) == norm(&prompt) && call_fits(&p.agent, p.thread.as_deref(), &h, Some(&persona.name)))
      });
      if let Some(i) = pending {
        let call = c.relays.calls.remove(i);
        let Core { tree, state, .. } = &mut *c;
        tree.relay_link_call(&node, &call.tool_call_id, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      }
      c.relays.rounds.insert(thread.clone(), rx);
      let cohort = match caller.as_deref() {
        None => turn_index,
        Some(th) => c.relays.follow_ups.get(th).copied().unwrap_or(0),
      };
      c.relays.ledger.push(Owing { node: node.clone(), owner: caller.clone(), cohort, state: Owed::Running });
      // Counted from here until its process is registered or ended, so a close right now still waits for it
      let starting = Starting::enter(&c.relays.starting);
      self.touch(&mut c);
      (node, thread, starting)
    };
    let me = self.clone();
    let round = Round { thread: thread.clone(), node, persona: persona.clone(), mode, peer, depth, prompt: prompt.clone(), handoff };
    tokio::spawn(async move {
      let end = me.clone().relay_round(&round, starting).await;
      let reply = {
        let mut c = me.core.lock();
        match &end {
          Ok(stop) => {
            let cancelled = stop == "cancelled" || c.tree.relay_cancel_requested(&round.node);
            c.tree.relay_end(&round.node, if cancelled { SubagentState::Cancelled } else { SubagentState::Completed }, None);
          }
          Err(e) => c.tree.relay_end(&round.node, SubagentState::Failed, Some(e.to_string())),
        }
        let waited = c.relays.waiters.get(&round.thread).is_some_and(|n| *n > 0);
        if let Some(o) = c.relays.owing(&round.node).filter(|o| o.state == Owed::Running) {
          o.state = if waited { Owed::Read } else { Owed::Unread };
        }
        me.drain_terminal(&mut c);
        c.relays.rounds.remove(&round.thread);
        c.relays.ended.send_modify(|n| *n += 1);
        me.touch(&mut c);
        round_result(&c, &round.node)
      };
      let _ = tx.send(Some(reply));
      me.relay_settled();
    });
    Ok(Start::Started { thread, persona })
  }

  /// The question card a summoned node is waiting on: (block id, its questions)
  pub(crate) fn relay_question(c: &Core, node: &str) -> Option<(String, Vec<acpira_shared::transcript::Question>)> {
    c.questions.pending.iter().find(|p| p.node_id.as_deref() == Some(node)).map(|p| (p.block_id.clone(), p.questions.clone()))
  }

  /// A question card opened somewhere: with the session idle, an automatic message may carry a child's question now
  pub(crate) fn relay_nudge(self: &Arc<Self>) {
    let idle = {
      let c = self.core.lock();
      c.status == SessionStatus::Ready && !c.phase.running && !c.relays.closed
    };
    if idle {
      self.flush_queue();
    }
  }

  /// The dialect of the CLI behind a summoned child's routing key (None: not a summoned child)
  pub(crate) fn relay_vendor(&self, session_id: &str) -> Option<crate::acp::vendors::Vendor> {
    let thread = session_id.strip_prefix("relay:")?;
    let c = self.core.lock();
    c.tree.relay_latest(thread).map(|(_, h, ..)| crate::acp::vendors::Vendor::of(&h.agent))
  }

  /// Summoned rounds still running: the session has work in the background even while its own turn is over
  pub(crate) fn relay_background(c: &Core) -> bool {
    !c.relays.rounds.is_empty()
  }

  /// A round ended. With the session idle, what the background held back goes now: the workspace gate over the span
  /// the children extended once the last one is done, then (through the queue's turn) the results owed to the root
  fn relay_settled(self: &Arc<Self>) {
    let (idle, deferred) = {
      let c = self.core.lock();
      let idle = c.status == SessionStatus::Ready && !c.phase.running;
      (idle, idle && !Self::relay_background(&c) && c.hooks.deferred)
    };
    if deferred && self.hooks_after_turn() {
      return;
    }
    if idle {
      self.flush_queue();
    }
  }

  /// The queue is empty and the session free: deliver the finished results owed to the root in one automatic message.
  /// Rounds sent in one turn go together once all of them ended; at most `MAX_WAKES` deliveries per prompt of the user's
  pub(crate) fn relay_wake(self: &Arc<Self>) -> bool {
    let text = {
      let mut c = self.core.lock();
      if c.relays.closed || c.relays.wakes >= MAX_WAKES {
        return false;
      }
      let nodes = c.relays.deliverable(None);
      // Questions of rounds the root sent off, not told yet: their children are blocked, so these go without waiting for
      // the rest of the batch. A thread a call waits on hears it from that call instead
      let asks: Vec<(String, String)> = c
        .relays
        .ledger
        .iter()
        .filter(|o| o.owner.is_none() && o.state == Owed::Running)
        .filter(|o| c.tree.relay_thread_of(&o.node).is_none_or(|th| !c.relays.waiters.contains_key(&th)))
        .filter_map(|o| Self::relay_question(&c, &o.node).map(|(b, _)| (o.node.clone(), b)))
        .filter(|(_, b)| !c.relays.told.contains(b))
        .collect();
      if nodes.is_empty() && asks.is_empty() {
        return false;
      }
      for n in &nodes {
        if let Some(o) = c.relays.owing(n) {
          o.state = Owed::Delivering;
        }
      }
      c.relays.telling = asks.iter().map(|(_, b)| b.clone()).collect();
      c.relays.wakes += 1;
      wake_text(&c, &nodes, &asks.into_iter().map(|(n, _)| n).collect::<Vec<_>>())
    };
    self.log("relay: delivering finished rounds to the root");
    let me = self.clone();
    crate::util::run_prefix(Box::pin(async move { me.prompt_inner(text, vec![], Origin::Subagents, None, None).await }));
    true
  }

  /// Wait for the thread's running round up to the deadline, with progress lines meanwhile
  async fn relay_wait(&self, thread: &str, persona: &SubagentPersona, deadline: Duration, out: &mpsc::UnboundedSender<HubReply>) {
    let rx = {
      let mut c = self.core.lock();
      let rx = c.relays.rounds.get(thread).cloned();
      if rx.is_some() {
        *c.relays.waiters.entry(thread.to_owned()).or_default() += 1;
      }
      rx
    };
    let Some(mut rx) = rx else {
      // The round ended between the call's check and now: its outcome is on the node
      let _ = out.send(self.relay_read(thread, persona));
      return;
    };
    let _waiting = Waiting { session: self, thread };
    let mut asked = self.core.lock().relays.asked.subscribe();
    asked.borrow_and_update();
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    loop {
      if rx.borrow().is_some() {
        let _ = out.send(self.relay_read(thread, persona));
        return;
      }
      // The child asks something: the call answers with the question, so the caller can answer it
      if let Some(text) = self.relay_asking(thread, persona) {
        let _ = out.send(HubReply::Done(text));
        return;
      }
      tokio::select! {
        r = asked.changed() => if r.is_err() { return; },
        r = rx.changed() => if r.is_err() { let _ = out.send(HubReply::Error(format!("{} stopped.", who(persona)))); return; },
        _ = tick.tick() => {
          if out.send(HubReply::Progress(self.relay_activity(thread).unwrap_or_else(|| "Working".into()))).is_err() {
            return;
          }
        }
        _ = tokio::time::sleep_until(until) => {
          let latest = self.relay_activity(thread).map(|a| format!("; latest: {a}")).unwrap_or_default();
          // The caller is owed the reply anyway, unless its entry was settled meanwhile
          let still_owed = {
            let c = self.core.lock();
            c.tree.relay_latest(thread).is_some_and(|(id, ..)| c.relays.ledger.iter().any(|o| o.node == id && o.state == Owed::Running))
          };
          let next = if still_owed {
            "Acpira sends you its reply in a message when it is done: carry on with other work or end your turn, and do not poll."
          } else {
            "Call ask_agent again with this thread and an empty prompt to keep waiting for its reply."
          };
          let _ = out.send(HubReply::Done(format!("{} is still working (thread: {thread}{latest}). {next}", who(persona))));
          return;
        }
      }
    }
  }

  /// The question the thread's running round waits on, as a call answers it (marked told)
  fn relay_asking(&self, thread: &str, persona: &SubagentPersona) -> Option<String> {
    let mut c = self.core.lock();
    let (node, ..) = c.tree.relay_latest(thread)?;
    let (block, questions) = Self::relay_question(&c, &node)?;
    c.relays.told.insert(block);
    Some(question_text(&questions, thread, persona))
  }

  /// The thread's latest round as a call answers it, marking it read
  fn relay_read(&self, thread: &str, persona: &SubagentPersona) -> HubReply {
    let mut c = self.core.lock();
    let end = match c.tree.relay_latest(thread) {
      Some((id, ..)) => {
        c.relays.read(&id);
        round_result(&c, &id)
      }
      None => Err("is gone.".into()),
    };
    final_reply(end, thread, persona)
  }

  fn relay_activity(&self, thread: &str) -> Option<String> {
    let c = self.core.lock();
    c.tree.relay_latest(thread).and_then(|(id, ..)| c.tree.activity(&id))
  }

  /// Run one round: connect the thread (spawn, then resume or create its native session), send the brief, wait for the
  /// prompt's answer. Answers the stop reason; the setup notes the caller should hear (a model the CLI lacks …) stay
  /// with the node, so a later read of the round says them too
  async fn relay_round(self: Arc<Self>, r: &Round, starting: Starting) -> Result<String> {
    let mut notes = vec![];
    let (proc, sid, (model, effort)) = self.relay_connect(r, &mut notes, starting).await?;
    for n in &notes {
      self.log(&format!("relay {}: {n}", short(&r.thread)));
    }
    {
      let mut c = self.core.lock();
      if c.relays.closed {
        return Err(anyhow!("the session closed"));
      }
      if !notes.is_empty() {
        c.relays.notes.insert(r.node.clone(), notes);
      }
      let turn_index = current_turn_index(&c);
      let Core { tree, state, .. } = &mut *c;
      tree.relay_bind(&r.node, &route_key(&r.thread), &sid, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      tree.relay_set_controls(&r.node, model, effort);
      // Cancelled while connecting: the prompt never goes out
      let cancelled = tree.relay_cancel_requested(&r.node);
      self.touch(&mut c);
      if cancelled {
        return Ok("cancelled".into());
      }
    }
    self.core.lock().relays.follow_ups.remove(&r.thread);
    // The work tree before the prompt: a round that runs commands is told what changed meanwhile, not only what its
    // edit rows name
    let watch = [PathBuf::from(&self.cwd)];
    let before = if in_work_tree(&watch[0]).await { Some(Snapshot::take(&watch).await) } else { None };
    let (blocks, handed) = self.relay_blocks(r, &proc).await;
    if let Some(n) = handed {
      self.core.lock().relays.notes.entry(r.node.clone()).or_default().push(n);
    }
    let end = self.relay_prompt(&proc, &sid, blocks).await;
    let end = match end {
      Ok(stop) => self.relay_follow_ups(&proc, &sid, r, stop).await,
      Err(e) => Err(e),
    };
    let (edits, anywhere) = round_edits(self.core.lock().tree.relay_turns(&r.node).unwrap_or(&[]));
    let changed = match &before {
      Some(b) if anywhere => Some(Snapshot::take(&watch).await.changed_since(b)),
      Some(_) => Some(vec![]),
      None => None,
    };
    // A read-only round is told about only when it did change something
    let clean = edits.is_empty() && changed.as_ref().is_none_or(|c| c.is_empty());
    let note = (r.mode != RelayMode::Consult || !clean).then(|| change_note(&self.cwd, &edits, anywhere, changed.as_deref()));
    // Whatever this thread sent off and has not had by now goes unread: its round is over
    let mut c = self.core.lock();
    if let Some(note) = note {
      c.relays.notes.entry(r.node.clone()).or_default().push(note);
    }
    c.relays.follow_ups.remove(&r.thread);
    for o in c.relays.ledger.iter_mut().filter(|o| o.owner.as_deref() == Some(r.thread.as_str())) {
      if matches!(o.state, Owed::Running | Owed::Unread) {
        o.state = Owed::Void;
      }
    }
    end
  }

  async fn relay_prompt(&self, proc: &AgentProcess, sid: &str, blocks: Vec<Value>) -> Result<String> {
    let answer = proc.request("session/prompt", json!({ "sessionId": sid, "prompt": blocks })).await.map_err(anyhow::Error::new)?;
    Ok(answer.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_owned())
  }

  /// The round's prompt: its brief, then what it hands over of the caller's changes. A thread's first handover is the
  /// whole pack, staged like a dropped text file (an embedded resource where the CLI takes them, inline text
  /// otherwise); a later round of the same thread hears in one line what moved since. Answers a note for the caller
  /// when there was nothing to hand over, or the pack had to be cut down to fit
  async fn relay_blocks(&self, r: &Round, proc: &AgentProcess) -> (Vec<Value>, Option<String>) {
    let plain = |handover: Option<&str>| vec![json!({ "type": "text", "text": compose_brief(&r.prompt, &r.persona, r.mode, handover) })];
    let Some(h) = &r.handoff else { return (plain(None), None) };
    let cwd = PathBuf::from(&self.cwd);
    let told = self.core.lock().relays.packs.get(&r.thread).cloned();
    if let Some((base, before)) = told {
      let now = match handoff::file_stats(&cwd, Some(&base), h.scope.as_ref()).await {
        Ok(Some(now)) => now,
        _ => return (plain(None), None),
      };
      let moved = handoff::changed_since(&before, &now);
      self.core.lock().relays.packs.insert(r.thread.clone(), (base, now));
      if moved.is_empty() {
        return (plain(None), None);
      }
      let shown = if moved.len() > NOTE_FILES {
        format!("{} and {} more", moved[..NOTE_FILES].join(", "), moved.len() - NOTE_FILES)
      } else {
        moved.join(", ")
      };
      return (plain(Some(&format!("Changed since your last round: {shown}."))), None);
    }
    let pack = match &h.shared {
      Some(p) => (**p).clone(),
      None => handoff::build(&cwd, None, h.scope.as_ref(), &h.recipe).await.unwrap_or_else(|e| {
        self.log(&format!("relay {}: handoff pack: {e:#}", short(&r.thread)));
        None
      }),
    };
    let Some(pack) = pack else { return (plain(None), Some("there were no uncommitted changes to hand over".into())) };
    self.core.lock().relays.packs.insert(r.thread.clone(), (pack.base.clone(), pack.files.clone()));
    // Staged like a dropped text file in the child's own CLI: the recipe's cap keeps the pack under the attachment limit
    let def = self.deps.registry.try_get(&r.persona.agent).cloned().unwrap_or_default();
    let caps = prompt_caps_of(Some(&proc.init), &def);
    let draft = Draft::Text { name: PACK_NAME.into(), text: pack.text.clone(), context: None };
    let brief = compose_brief(&r.prompt, &r.persona, r.mode, Some(PACK_LINE));
    let prepared = prepare_prompt(&self.id, &brief, &[draft], &self.deps.blobs, Some(caps)).await;
    let cut = match pack.level {
      Level::Full => None,
      Level::Trimmed => Some("some full texts left out"),
      Level::Plain => Some("plain hunks, no full texts"),
      Level::Stat => Some("paths and line counts, with only the smallest diffs"),
    };
    (prepared.blocks, cut.map(|c| format!("the handoff pack was cut down to fit its size cap ({c})")))
  }

  /// The child answered, but rounds it sent off are still out: its round goes on. Once a batch of them has ended (or one
  /// asks something), the results go to its native session as one more prompt, after its answer so far is sealed and an
  /// automatic message opens the next turn of its transcript; at most `MAX_WAKES` follow-ups, then the last answer is
  /// the round's reply
  async fn relay_follow_ups(&self, proc: &AgentProcess, sid: &str, r: &Round, mut stop: String) -> Result<String> {
    let mut sent = 0;
    loop {
      if stop == "cancelled" {
        return Ok(stop);
      }
      let (mut ended, mut asked) = {
        let c = self.core.lock();
        (c.relays.ended.subscribe(), c.relays.asked.subscribe())
      };
      let text = {
        let mut c = self.core.lock();
        if c.relays.closed || c.tree.relay_cancel_requested(&r.node) || !c.relays.owes(&r.thread) || sent >= MAX_WAKES {
          return Ok(stop);
        }
        let nodes = c.relays.deliverable(Some(&r.thread));
        let asks: Vec<(String, String)> = c
          .relays
          .ledger
          .iter()
          .filter(|o| o.owner.as_deref() == Some(r.thread.as_str()) && o.state == Owed::Running)
          .filter_map(|o| Self::relay_question(&c, &o.node).map(|(b, _)| (o.node.clone(), b)))
          .filter(|(_, b)| !c.relays.told.contains(b))
          .collect();
        if nodes.is_empty() && asks.is_empty() {
          c.tree.relay_waiting(&r.node);
          self.touch(&mut c);
          None
        } else {
          for n in &nodes {
            if let Some(o) = c.relays.owing(n) {
              o.state = Owed::Delivered;
            }
          }
          c.relays.told.extend(asks.iter().map(|(_, b)| b.clone()));
          let text = wake_text(&c, &nodes, &asks.into_iter().map(|(n, _)| n).collect::<Vec<_>>());
          sent += 1;
          c.relays.follow_ups.insert(r.thread.clone(), sent as usize);
          c.tree.relay_follow_up(&r.node, &text);
          self.touch(&mut c);
          Some(text)
        }
      };
      match text {
        Some(text) => {
          self.log(&format!("relay {}: follow-up {sent} with the rounds it sent off", short(&r.thread)));
          stop = self.relay_prompt(proc, sid, vec![json!({ "type": "text", "text": text })]).await?;
        }
        None => {
          tokio::select! {
            _ = ended.changed() => {}
            _ = asked.changed() => {}
          }
        }
      }
    }
  }

  /// The thread's live process and native session (with the round's settings applied), started when there is none.
  /// Answers the model and reasoning effort the session really runs
  /// `_starting` keeps the session's close waiting until this returns (the process registered, or ended)
  async fn relay_connect(self: &Arc<Self>, r: &Round, notes: &mut Vec<String>, _starting: Starting) -> Result<(Arc<AgentProcess>, String, (Option<String>, Option<String>))> {
    let thread = r.thread.as_str();
    let wanted = (r.persona.model.clone(), r.persona.effort.clone());
    let live = {
      let c = self.core.lock();
      c.relays
        .procs
        .get(thread)
        .filter(|rp| rp.proc.alive() && !rp.peer.is_empty())
        .map(|rp| (rp.proc.clone(), rp.peer.clone(), rp.opened.clone(), rp.start_mode.clone(), rp.mode, rp.applied.clone()))
    };
    // A live thread keeps its process and session; a persona whose model / effort changed since is applied again, and
    // the mode follows the round
    if let Some((proc, sid, mut opened, start_mode, last, applied)) = live {
      if applied != wanted {
        relay_controls(&proc, &sid, &mut opened, &r.persona, notes).await;
      }
      let mode = if last == Some(r.mode) || relay_mode(&proc, &sid, &mut opened, start_mode.as_deref(), r.mode, &r.persona.agent, notes).await { Some(r.mode) } else { None };
      // Still in full access (a refused step down, or no mode to step down to): the prompt would run without the
      // approvals this round asked for
      let stuck = stuck_in_bypass(&opened, r.mode);
      let controls = running_controls(&opened);
      if let Some(rp) = self.core.lock().relays.procs.get_mut(thread) {
        rp.opened = opened;
        rp.mode = mode;
        rp.applied = wanted;
      }
      if stuck {
        return Err(anyhow!("{} stayed in its full-access mode, so this {:?} round was not sent: {}", r.persona.agent, r.mode, notes.join("; ")));
      }
      return Ok((proc, sid, controls));
    }
    let def = self.deps.registry.get(&r.persona.agent)?.clone();
    let bin = self.deps.registry.resolve_binary(&r.persona.agent).await.ok_or_else(|| anyhow!("{} is not installed", def.name))?;
    let quiet = Arc::new(AtomicBool::new(false));
    let handlers: Arc<dyn ClientHandlers> = Arc::new(RelayHandlers { session: self.me.clone(), thread: thread.to_owned(), quiet: quiet.clone() });
    self.log(&format!("relay {}: spawn {bin} {} for {}", short(thread), def.args.join(" "), r.persona.name));
    let proc = AgentProcess::spawn(&def, &bin, &self.cwd, handlers, None, None).await?;
    // Registered before anything else can fail, so the close or a failed open always ends it
    let closed = {
      let mut c = self.core.lock();
      if !c.relays.closed {
        let rp = RelayProc { proc: proc.clone(), peer: String::new(), opened: Value::Null, start_mode: None, mode: None, applied: (None, None) };
        c.relays.procs.insert(thread.to_owned(), rp);
      }
      c.relays.closed
    };
    if closed {
      proc.kill().await;
      return Err(anyhow!("the session closed"));
    }
    let opened = self.relay_open_session(&proc, r, &quiet, notes).await;
    let kept = {
      let mut c = self.core.lock();
      let closed = c.relays.closed;
      match (&opened, c.relays.procs.get_mut(thread).filter(|rp| Arc::ptr_eq(&rp.proc, &proc))) {
        (Ok((sid, opened, start_mode, mode_ok)), Some(rp)) if !closed => {
          rp.peer = sid.clone();
          rp.opened = opened.clone();
          rp.start_mode = start_mode.clone();
          rp.mode = mode_ok.then_some(r.mode);
          rp.applied = wanted;
          true
        }
        _ => false,
      }
    };
    if !kept {
      // Ended while still registered (a close meanwhile takes it over), then let go
      proc.kill().await;
      let mut c = self.core.lock();
      if c.relays.procs.get(thread).is_some_and(|rp| Arc::ptr_eq(&rp.proc, &proc)) {
        c.relays.procs.remove(thread);
      }
    }
    let (sid, opened, ..) = opened?;
    if !kept {
      return Err(anyhow!("the session closed"));
    }
    Ok((proc, sid, running_controls(&opened)))
  }

  /// Open the thread's native session on a fresh process: resume / load the earlier one (never silently a new one in
  /// its place), else create it; then apply the persona's model, effort and the round's mode. Answers the session id,
  /// what it offers, the mode it opened in and whether the round's mode took
  async fn relay_open_session(&self, proc: &AgentProcess, r: &Round, quiet: &AtomicBool, notes: &mut Vec<String>) -> Result<(String, Value, Option<String>, bool)> {
    let req = self.relay_session_request(proc, &r.persona.agent, &r.thread, r.depth + 1).await;
    let (sid, mut opened) = match &r.peer {
      Some(peer) => self.relay_restore(proc, req, peer, &r.thread, quiet).await?,
      None => {
        let v = proc.request("session/new", req).await.map_err(anyhow::Error::new)?;
        (v.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned(), v)
      }
    };
    if sid.is_empty() {
      return Err(anyhow!("{} answered session/new without a session id", r.persona.agent));
    }
    let start_mode = current_mode(&opened);
    relay_controls(proc, &sid, &mut opened, &r.persona, notes).await;
    let mode_ok = relay_mode(proc, &sid, &mut opened, start_mode.as_deref(), r.mode, &r.persona.agent, notes).await;
    // A CLI that starts in full access and cannot leave it gets no prompt for a round that wants approvals
    if stuck_in_bypass(&opened, r.mode) {
      return Err(anyhow!("{} stayed in its full-access mode, so this {:?} round was not sent: {}", r.persona.agent, r.mode, notes.join("; ")));
    }
    Ok((sid, opened, start_mode, mode_ok))
  }

  /// Reopen the thread's earlier native session; failing that is an error the caller reads, since a fresh session would
  /// have lost everything the thread said so far
  async fn relay_restore(&self, proc: &AgentProcess, req: Value, peer: &str, thread: &str, quiet: &AtomicBool) -> Result<(String, Value)> {
    let caps = proc.caps().clone();
    let mut r = req;
    r["sessionId"] = json!(peer);
    let mut why = vec![];
    if crate::json::truthy(caps.get("sessionCapabilities").and_then(|s| s.get("resume"))) {
      match proc.request("session/resume", r.clone()).await {
        Ok(v) => return Ok((peer.to_owned(), v)),
        Err(e) => why.push(format!("session/resume: {e}")),
      }
    }
    if crate::json::truthy(caps.get("loadSession")) {
      quiet.store(true, Ordering::Relaxed);
      let loaded = proc.request("session/load", r).await;
      quiet.store(false, Ordering::Relaxed);
      match loaded {
        Ok(v) => return Ok((peer.to_owned(), v)),
        Err(e) => why.push(format!("session/load: {e}")),
      }
    }
    if why.is_empty() {
      why.push("the CLI cannot reopen an earlier session".into());
    }
    self.log(&format!("relay {}: earlier conversation not restorable: {}", short(thread), why.join("; ")));
    Err(anyhow!(
      "The earlier conversation of thread {thread} could not be reopened ({}). Leave `thread` out to start a new conversation; it will not remember this one.",
      why.join("; ")
    ))
  }

  /// session/new params for a child: the shared MCP servers for its agent, plus Acpira's own leading back to this
  /// session one level deeper
  async fn relay_session_request(&self, proc: &AgentProcess, agent: &str, thread: &str, depth: u32) -> Value {
    let mut servers = match &self.deps.shared_mcp {
      Some(provider) => provider(agent.to_owned(), self.cwd.clone(), runtime_info_of(&proc.init).mcp).await.0,
      None => vec![],
    };
    // Past the depth limit the entry still leads back here (`ask_caller`, the session tools); the hub lists no personas
    let entry = self.deps.host_mcp.as_ref().and_then(|h| h.entry_for_session(agent, &self.arc(), Some(thread), depth));
    crate::host_mcp::push_entry(&mut servers, entry);
    json!({ "cwd": self.cwd, "mcpServers": servers })
  }

  /// A root `ask_agent` row (after the normalizer applied it): tie it to its round's node, or remember it until the
  /// round exists. A wait call (thread, no prompt) stays a visible "waiting" row
  pub(crate) fn relay_annotate(&self, c: &mut Core, u: &Value, turn_index: usize) {
    let tool_call_id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    // A call ends only after the hub answered it, so one still waiting here never started a round (refused, or the hub
    // was unreachable): a retry with the same arguments must not find this row first. The ending update rarely
    // repeats the tool's name, hence before the name check
    let ended = matches!(u.get("status").and_then(Value::as_str), Some("completed" | "failed"));
    let known = if ended { c.relays.rows.remove(&tool_call_id) } else { c.relays.rows.contains(&tool_call_id) };
    // Only this row's own pending entry: another call still waiting must not keep an adapter that sends the arguments
    // with the ending update alone from tying this one
    if ended && c.relays.calls.iter().any(|p| p.tool_call_id == tool_call_id) {
      c.relays.calls.retain(|p| p.tool_call_id != tool_call_id);
      return;
    }
    if !(known || names_ask_agent(u)) {
      return;
    }
    // The name may come without any input (an AIR client gets `rawInput` only once it is complete), so the row is
    // remembered by its name alone: the refinement carrying the arguments leaves the unchanged title out
    if !ended {
      c.relays.rows.insert(tool_call_id.clone());
    }
    let Some(args) = ask_agent_args(u) else { return };
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_owned();
    let (prompt, agent, thread) = (text("prompt"), text("agent"), Some(text("thread")).filter(|t| !t.is_empty()));
    // No prompt and no thread: the arguments are still streaming in (an empty `rawInput` first), not a wait call
    if prompt.is_empty() && thread.is_none() {
      return;
    }
    if prompt.is_empty() {
      let name = thread.as_deref().and_then(|th| c.tree.relay_latest(th)).and_then(|(_, _, name, ..)| name);
      if let Some(block) = crate::acp::transcript::normalize::find_tool_mut(&mut c.state.turns, &tool_call_id) {
        block.verb_key = Some("verb.awaitSubagent".into());
        block.verb = t("verb.awaitSubagent");
        if let Some(name) = name {
          block.target = Some(name);
          block.target_mono = None;
        }
      }
      return;
    }
    if c.tree.relay_linked(&tool_call_id) {
      return;
    }
    match c.tree.relay_unlinked(&prompt, turn_index, |h, role| call_fits(&agent, thread.as_deref(), h, role)) {
      Some(node) => {
        c.relays.calls.retain(|p| p.tool_call_id != tool_call_id);
        let Core { tree, state, .. } = &mut *c;
        tree.relay_link_call(&node, &tool_call_id, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      }
      None => {
        // Calls of earlier turns never got a round: they are dropped rather than matched against a later one
        c.relays.calls.retain(|p| p.turn_index >= turn_index);
        // A streamed input grows field by field: the pending entry follows the latest arguments
        match c.relays.calls.iter_mut().find(|p| p.tool_call_id == tool_call_id) {
          Some(p) => (p.agent, p.thread, p.prompt, p.turn_index) = (agent, thread, prompt, turn_index),
          None => c.relays.calls.push(PendingCall { tool_call_id, agent, thread, prompt, turn_index }),
        }
      }
    }
  }

  /// The session closes: no round starts from now on. Answers its children's processes, for the caller to end, and the
  /// count of starts still in flight (each ends its own process once it sees the close), for the caller to wait on
  pub(crate) fn relay_close(&self, c: &mut Core) -> (Vec<Arc<AgentProcess>>, watch::Receiver<usize>) {
    c.relays.closed = true;
    c.relays.rounds.clear();
    c.relays.calls.clear();
    c.relays.notes.clear();
    c.relays.ledger.clear();
    (c.relays.procs.drain().map(|(_, rp)| rp.proc).collect(), c.relays.starting.subscribe())
  }

  /// History rewritten at turn_index (an edit, a retry): the summoned rounds of the removed turns are cancelled before
  /// their nodes go, so no child keeps working for a turn that no longer exists
  pub(crate) fn relay_truncate(&self, c: &mut Core, turn_index: usize) {
    let doomed = c.tree.relay_running_from(turn_index);
    self.relay_cancel(c, &doomed);
    // Results owed for the removed turns go with them, finished or not
    let gone: Vec<String> = c.relays.ledger.iter().filter(|o| o.cohort >= turn_index).map(|o| o.node.clone()).collect();
    c.relays.void(Some(&gone));
    c.tree.truncate(turn_index);
  }

  /// The process and native session a summoned node's cancel goes to (None = not summoned, or still connecting)
  pub(crate) fn relay_target(c: &Core, node: &str) -> Option<(Arc<AgentProcess>, String)> {
    let thread = c.tree.relay_thread_of(node)?;
    c.relays.procs.get(&thread).filter(|rp| !rp.peer.is_empty()).map(|rp| (rp.proc.clone(), rp.peer.clone()))
  }

  /// Cancel summoned rounds (and their pending cards): each child's own process gets the `session/cancel`; a round still
  /// connecting sees the request before it sends its prompt
  pub(crate) fn relay_cancel(&self, c: &mut Core, nodes: &[String]) {
    for id in nodes {
      if !c.tree.relay_cancel(id) {
        continue;
      }
      self.cancel_permissions_for(c, id);
      self.cancel_questions_for(c, id);
      if let Some((proc, sid)) = Self::relay_target(c, id) {
        proc.notify("session/cancel", json!({ "sessionId": sid }));
      }
    }
  }
}

/// The model and reasoning effort the session runs, by their option names
fn running_controls(opened: &Value) -> (Option<String>, Option<String>) {
  (current_name(opened.get("configOptions"), &MODEL_KEYS), current_name(opened.get("configOptions"), &EFFORT_KEYS))
}

/// The persona's model, then its effort, picked from the options the model switch answered (effort values depend on
/// the model); what the CLI does not offer or refuses becomes a note
async fn relay_controls(proc: &AgentProcess, sid: &str, opened: &mut Value, persona: &SubagentPersona, notes: &mut Vec<String>) {
  if let Some(m) = &persona.model {
    match pick_option(opened.get("configOptions"), &MODEL_KEYS, m) {
      Some((id, value)) => {
        if let Err(e) = set_config(proc, sid, opened, &id, &value).await {
          notes.push(format!("model {m:?} was refused ({e}); the CLI's default model runs"));
        }
      }
      None => notes.push(format!("{} offers no model matching {m:?}; its default model runs", persona.agent)),
    }
  }
  if let Some(e) = &persona.effort {
    match pick_option(opened.get("configOptions"), &EFFORT_KEYS, e) {
      Some((id, value)) => {
        if let Err(err) = set_config(proc, sid, opened, &id, &value).await {
          notes.push(format!("reasoning effort {e:?} was refused ({err})"));
        }
      }
      None => notes.push(format!("{} offers no reasoning effort {e:?} for this model; its default applies", persona.agent)),
    }
  }
}

/// The round's mode: a consult switches to the CLI's read-only mode where it has one; a bypass round to its full-access
/// mode where it has one (without one the host still answers its permission requests); a work round in a read-only or
/// full-access mode switches to the one the session opened in (or the first ordinary one), as does a consult in a
/// full-access mode on a CLI without a read-only one. Answers false when the CLI refused the switch, so the next round
/// tries again instead of taking the mode as set
async fn relay_mode(proc: &AgentProcess, sid: &str, opened: &mut Value, start: Option<&str>, mode: RelayMode, agent: &str, notes: &mut Vec<String>) -> bool {
  let ids = mode_ids(opened);
  let bypass = bypass_modes(opened);
  let current = current_mode(opened);
  let ordinary = |m: &str| !is_read_only(m) && !bypass.iter().any(|b| b == m);
  // The mode to leave a read-only or full-access one for
  let back = || start.filter(|s| ordinary(s)).map(str::to_owned).or_else(|| ids.iter().find(|m| ordinary(m)).cloned());
  let in_bypass = current.as_deref().is_some_and(|c| bypass.iter().any(|b| b == c));
  let want = match mode {
    RelayMode::Consult => match READ_ONLY_MODES.iter().find(|r| ids.iter().any(|m| m == *r)) {
      Some(m) => (*m).to_owned(),
      None => {
        notes.push(format!("{agent} has no read-only mode; it follows its own permission settings, so nothing stops a write"));
        match back().filter(|_| in_bypass) {
          Some(m) => m,
          None => return true,
        }
      }
    },
    RelayMode::Bypass => match bypass.first() {
      Some(m) if !in_bypass => m.clone(),
      _ => return true,
    },
    // Resolved before a round starts (`round_mode`); taken as work should one ever get here
    RelayMode::Work | RelayMode::Inherit => match current.as_deref() {
      Some(c) if !ordinary(c) => match back() {
        Some(m) => m,
        None => return true,
      },
      _ => return true,
    },
  };
  if current.as_deref() == Some(want.as_str()) {
    return true;
  }
  let listed = opened.pointer("/modes/availableModes").and_then(Value::as_array).is_some_and(|l| l.iter().any(|m| m.get("id").and_then(Value::as_str) == Some(want.as_str())));
  let r = if listed {
    match proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": want })).await {
      Ok(_) => {
        if let Some(m) = opened.get_mut("modes").filter(|m| m.is_object()) {
          m["currentModeId"] = Value::from(want.as_str());
        }
        Ok(())
      }
      Err(e) => Err(e.to_string()),
    }
  } else {
    let id = find_option(opened.get("configOptions"), &MODE_KEYS).and_then(|o| o.get("id")).and_then(Value::as_str).map(str::to_owned);
    match id {
      Some(id) => set_config(proc, sid, opened, &id, &want).await,
      None => return true,
    }
  };
  match r {
    Ok(()) => true,
    Err(e) => {
      notes.push(format!("switching {agent} to its {want} mode was refused ({e})"));
      false
    }
  }
}

/// One call waiting on a thread (`Relays::waiters`), counted for as long as it lives
struct Waiting<'a> {
  session: &'a AcpSession,
  thread: &'a str,
}

impl Drop for Waiting<'_> {
  fn drop(&mut self) {
    let mut c = self.session.core.lock();
    if let Some(n) = c.relays.waiters.get_mut(self.thread) {
      *n = n.saturating_sub(1);
      if *n == 0 {
        c.relays.waiters.remove(self.thread);
      }
    }
  }
}

/// What one round runs with
struct Round {
  thread: String,
  node: String,
  persona: SubagentPersona,
  mode: RelayMode,
  /// The thread's native session from its earlier rounds, to resume
  peer: Option<String>,
  depth: u32,
  /// The caller's task, which `compose_brief` wraps
  prompt: String,
  handoff: Option<Handoff>,
}

/// What a round hands over of its caller's changes (`ask_agent` `changes`, or the persona's switch)
struct Handoff {
  /// None: every uncommitted change of the work tree
  scope: Option<Scope>,
  recipe: Recipe,
  /// The pack a group call built once for all its members, so each gets the same one
  shared: Option<Arc<Option<Pack>>>,
}

/// Which changes a caller hands over: the files its own edit rows name, and every uncommitted change once it ran
/// something that can write anywhere (a command, an MCP tool, a summoned round that was allowed to edit: the rule the
/// workspace gate's `row_reach` follows, with read-only rounds left out). A caller that changed nothing itself hands
/// over the whole work tree's uncommitted changes, which are then someone else's (the user's own work, say). The
/// paths it read go along for a recipe that lists them
fn caller_scope(c: &Core, caller: Option<&str>) -> Option<Scope> {
  // A summoned caller's rounds so far: its earlier ones' edits are just as uncommitted
  let rounds: Vec<&[Turn]> = match caller {
    None => vec![&c.state.turns],
    Some(th) => c.tree.relay_thread_turns(th),
  };
  // An `ask_agent` row with no round behind it (this very call, one still waiting, one refused) changed nothing
  let asking = |tc: &ToolCallBlock| {
    c.relays.rows.contains(&tc.id)
      || c.relays.calls.iter().any(|p| p.tool_call_id == tc.id)
      || is_ask_name(&tc.verb)
      || tc.target.as_deref().is_some_and(is_ask_name)
  };
  let mut scope = Scope::default();
  for turn in rounds.into_iter().flatten().filter_map(Turn::as_agent) {
    for b in &turn.blocks {
      let AgentBlock::ToolCall(tc) = b else { continue };
      if let Some(h) = tc.subagent_id.as_deref().and_then(|n| c.tree.relay_harness(n)).map(|(h, _)| h) {
        scope.anywhere |= h.mode != RelayMode::Consult;
        continue;
      }
      if asking(tc) {
        continue;
      }
      match row_reach(tc) {
        Reach::Any => scope.anywhere = true,
        Reach::Edits => scope.files.extend(edit_paths(tc)),
        Reach::None => scope.read.extend(tc.locations.iter().flatten().map(|l| l.path.clone())),
      }
    }
  }
  (scope.anywhere || !scope.files.is_empty()).then_some(scope)
}

enum Start {
  /// A new round is running
  Started { thread: String, persona: SubagentPersona },
  /// The call answered the running round's question
  Answered { thread: String, persona: SubagentPersona },
  /// A wait call on a thread whose round is running
  Wait { thread: String, persona: SubagentPersona },
  Ended(HubReply),
}

fn norm(s: &str) -> String {
  s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn busy(persona: &SubagentPersona, thread: &str) -> anyhow::Error {
  anyhow!("{} is still working on the previous round of thread {thread}. Call ask_agent with this thread and an empty prompt to wait for it.", who(persona))
}

/// A finished round as the caller reads it, the same live and later: the reply (and the round's notes: its setup and
/// what it changed), or why there is none (with the same notes: a stopped round may have changed files already)
fn round_result(c: &Core, node: &str) -> std::result::Result<String, String> {
  let Some((state, reply, result)) = c.tree.relay_outcome(node) else { return Err("is gone.".into()) };
  let notes = c.relays.notes.get(node).filter(|n| !n.is_empty()).map(|n| format!("\n\n(Acpira: {}.)", n.join("; "))).unwrap_or_default();
  match state {
    SubagentState::Completed if notes.is_empty() => Ok(reply),
    SubagentState::Completed => Ok(format!("{}{notes}", reply.trim())),
    SubagentState::Cancelled if reply.trim().is_empty() => Err(format!("was cancelled.{notes}")),
    SubagentState::Cancelled => Err(format!("was cancelled. Its last words:\n\n{}{notes}", reply.trim())),
    SubagentState::Failed | SubagentState::Disconnected => {
      Err(format!("failed: {}{notes}", result.as_deref().map(str::trim).filter(|r| !r.is_empty()).unwrap_or("no reason given")))
    }
    SubagentState::Running => Err("is still working.".into()),
  }
}

fn final_reply(end: std::result::Result<String, String>, thread: &str, persona: &SubagentPersona) -> HubReply {
  match end {
    Ok(reply) => HubReply::Done(done_text(&reply, thread, persona)),
    Err(e) => HubReply::Error(format!("{} {e}", who(persona))),
  }
}

/// What a call that sent a task off answers: the thread, and that the reply comes by itself
fn dispatched_text(thread: &str, persona: &SubagentPersona) -> String {
  format!(
    "Sent to {} (thread: {thread}). Carry on with other work that does not depend on it, or end your turn if there is none: \
Acpira sends you its reply in a message when it is done. Do not poll, and do not do this task yourself meanwhile.",
    who(persona)
  )
}

/// The automatic message delivering finished rounds (each one as a call would have read it, under its persona and
/// thread) and the questions of rounds still running
fn wake_text(c: &Core, nodes: &[String], asking: &[String]) -> String {
  let mut parts = vec![];
  if !nodes.is_empty() {
    parts.push(if nodes.len() == 1 {
      "(Acpira) A summoned agent you sent off has finished. Its reply:".to_owned()
    } else {
      format!("(Acpira) The {} summoned agents you sent off have finished. Their replies:", nodes.len())
    });
  }
  for node in asking {
    let Some((h, name)) = c.tree.relay_harness(node) else { continue };
    let Some((_, questions)) = AcpSession::relay_question(c, node) else { continue };
    let persona = node_persona(&h, name.as_deref());
    parts.push(format!("## {} asks\n\n{}", who(&persona), question_text(&questions, &h.thread, &persona)));
  }
  for node in nodes {
    let Some((h, name)) = c.tree.relay_harness(node) else { continue };
    let persona = node_persona(&h, name.as_deref());
    let text = match final_reply(round_result(c, node), &h.thread, &persona) {
      HubReply::Done(t) | HubReply::Error(t) => t,
      _ => continue,
    };
    parts.push(format!("## {}\n\n{text}", who(&persona)));
  }
  parts.join("\n\n")
}

/// A child's open question as its caller reads it: every question with its options, and how to answer
fn question_text(questions: &[acpira_shared::transcript::Question], thread: &str, persona: &SubagentPersona) -> String {
  let mut out = vec![format!("{} is waiting for an answer before it goes on (thread: {thread}):", who(persona))];
  for (i, q) in questions.iter().enumerate() {
    let title = q.title.as_deref().filter(|t| !t.trim().is_empty() && t.trim() != q.text.trim()).map(|t| format!("{}: ", t.trim())).unwrap_or_default();
    let many = if q.kind == acpira_shared::transcript::QuestionKind::Multiple { " (one or more)" } else { "" };
    out.push(format!("{}. {title}{}{many}", i + 1, q.text.trim()));
    for o in &q.options {
      out.push(match o.description.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("   - {}: {}", o.label, d.trim()),
        None => format!("   - {}", o.label),
      });
    }
    if !q.options.is_empty() && q.other != Some(false) {
      out.push("   - (or an answer of your own)".into());
    }
  }
  let how = if questions.len() == 1 {
    "pass the answer as `prompt`, or as `answers` with one entry"
  } else {
    "pass `answers`, one entry per question in this order"
  };
  out.push(format!(
    "Answer with ask_agent, thread {thread:?}: {how} (an option's label, or your own words). The user sees the same question and may answer first."
  ));
  out.join("\n")
}

/// A caller's answers as the card's would be: by question order, an option's label or id (any case) picks it, anything
/// else is the question's own text; one question may take the prompt instead
fn caller_answers(
  questions: &[acpira_shared::transcript::Question],
  given: Option<&[String]>,
  prompt: &str,
) -> std::result::Result<acpira_shared::transcript::QuestionAnswers, String> {
  use acpira_shared::transcript::QuestionKind;
  let fallback = [prompt.to_owned()];
  let given = match given {
    Some(g) if !g.is_empty() => g,
    _ if questions.len() == 1 && !prompt.trim().is_empty() => &fallback[..],
    _ => return Err(format!("asks {} questions: pass `answers`, one entry per question.", questions.len())),
  };
  let mut out = acpira_shared::transcript::QuestionAnswers::new();
  for (q, a) in questions.iter().zip(given) {
    let a = a.trim();
    if a.is_empty() {
      continue;
    }
    let pick = |x: &str| q.options.iter().find(|o| o.label.trim().eq_ignore_ascii_case(x.trim()) || o.id.eq_ignore_ascii_case(x.trim())).map(|o| o.id.clone());
    let closed = !q.options.is_empty() && q.other == Some(false);
    let one = |x: &str| -> std::result::Result<String, String> {
      match pick(x) {
        Some(id) => Ok(id),
        None if closed => Err(format!(
          "only takes one of its options for {:?}: {}.",
          q.text.trim(),
          q.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>().join(", ")
        )),
        None => Ok(x.trim().to_owned()),
      }
    };
    let value = if q.kind == QuestionKind::Multiple && pick(a).is_none() {
      let items = a.split([',', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(one).collect::<std::result::Result<Vec<_>, _>>()?;
      Value::from(items)
    } else if q.kind == QuestionKind::Multiple {
      Value::from(vec![one(a)?])
    } else {
      Value::from(one(a)?)
    };
    out.insert(q.id.clone(), value);
  }
  Ok(out)
}

/// An answer as `ask_caller` reports it: the text, or the picks joined
fn answer_text(a: &Value) -> String {
  match a {
    Value::String(s) => s.trim().to_owned(),
    Value::Array(l) => l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
    other => other.to_string(),
  }
}

fn done_text(reply: &str, thread: &str, persona: &SubagentPersona) -> String {
  let body = if reply.trim().is_empty() { "(no reply text)" } else { reply.trim() };
  format!("{body}\n\n---\nthread: {thread} — {}. Pass this thread to ask_agent to follow up in the same conversation.", who(persona))
}

/// The `ask_agent` arguments of a root tool update, when it is Acpira's tool (named like `show_image` per adapter)
/// The arguments of an `ask_agent` row. `known`: an earlier update of this row already named the tool, so this one
/// counts even without the name
fn names_ask_agent(u: &Value) -> bool {
  let named = |x: Option<&Value>| x.and_then(Value::as_str).is_some_and(is_ask_name);
  named(u.get("title")) || named(u.get("name")) || named(u.get("rawInput").and_then(|r| r.get("tool")))
}

/// Whether a tool name is Acpira's `ask_agent` (bare, or the way an adapter prefixes its server)
fn is_ask_name(x: &str) -> bool {
  static ASK: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(?:^|acpira[_.:/-]{1,2})ask_agent$").unwrap());
  ASK.is_match(x.trim())
}

/// The call's arguments, codex-acp's `{ server, tool, arguments }` unwrapped; None while no input arrived yet
fn ask_agent_args(u: &Value) -> Option<Map<String, Value>> {
  let raw = u.get("rawInput").and_then(Value::as_object)?;
  Some(raw.get("arguments").and_then(Value::as_object).unwrap_or(raw).clone())
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::transcript::{AgentTurn, DiffLine, DiffSource, DiffStat, Location, ToolCallBlock};
  use crate::util::random_uuid;

  #[test]
  fn a_fixed_persona_ignores_the_calls_mode_and_an_inherit_one_takes_it() {
    use RelayMode::*;
    for fixed in [Consult, Work, Bypass] {
      for asked in [None, Some("consult"), Some("work"), Some("bypass")] {
        assert_eq!(round_mode(fixed, asked, || panic!("a fixed persona never asks the caller")), fixed, "{fixed:?} asked {asked:?}");
      }
    }
    // The call overrides the caller's access either way
    assert_eq!(round_mode(Inherit, Some("work"), || Consult), Work);
    assert_eq!(round_mode(Inherit, Some("bypass"), || Work), Bypass);
    assert_eq!(round_mode(Inherit, Some("consult"), || Bypass), Consult);
    // Without one (or with an unknown one) it is the caller's access
    assert_eq!(round_mode(Inherit, None, || Bypass), Bypass);
    assert_eq!(round_mode(Inherit, Some("yolo"), || Work), Work);
  }

  #[test]
  fn full_access_modes_are_known_ids_or_marked_full_access() {
    let claude = json!({ "modes": { "currentModeId": "default", "availableModes": [
      { "id": "default" }, { "id": "plan" }, { "id": "bypassPermissions", "_meta": { "kind": "full_access" } },
    ] } });
    assert_eq!(bypass_modes(&claude), ["bypassPermissions"]);
    let marked = json!({ "modes": { "availableModes": [{ "id": "agent" }, { "id": "danger", "_meta": { "kind": "full_access" } }] } });
    assert_eq!(bypass_modes(&marked), ["danger"]);
    let select = json!({ "configOptions": [{ "id": "mode", "category": "mode", "currentValue": "agent", "options": [
      { "value": "read-only" }, { "value": "agent" }, { "value": "agent-full-access" },
    ] }] });
    assert_eq!(bypass_modes(&select), ["agent-full-access"]);
    assert!(bypass_modes(&json!({ "modes": { "availableModes": [{ "id": "agent" }, { "id": "plan" }] } })).is_empty());
  }

  #[test]
  fn picks_a_select_value_by_value_or_name_including_grouped_ones() {
    let options = json!([
      { "id": "model", "category": "model", "currentValue": "gpt-5.6", "options": [{ "value": "gpt-6-astra", "name": "GPT-6 Astra" }, { "value": "gpt-5.6", "name": "GPT-5.6 Terra" }] },
      { "id": "x", "category": "thought_level", "options": [{ "group": "g", "name": "G", "options": [{ "value": "max", "name": "Max" }] }] },
      { "id": "mode", "category": "mode", "options": [{ "value": "build", "name": "Build" }, { "value": "plan", "name": "Plan" }] },
    ]);
    assert_eq!(pick_option(Some(&options), &["model"], "GPT-6 Astra"), Some(("model".into(), "gpt-6-astra".into())));
    assert_eq!(pick_option(Some(&options), &["model"], "terra"), Some(("model".into(), "gpt-5.6".into())));
    assert_eq!(pick_option(Some(&options), &["thought_level"], "max"), Some(("x".into(), "max".into())));
    assert_eq!(pick_option(Some(&options), &["mode"], "plan"), Some(("mode".into(), "plan".into())));
    assert_eq!(pick_option(Some(&options), &["model"], "claude"), None);
    assert_eq!(current_name(Some(&options), &["model"]).as_deref(), Some("GPT-5.6 Terra"));
    assert_eq!(mode_ids(&json!({ "configOptions": options })), ["build", "plan"]);
  }

  #[test]
  fn recognizes_ask_agent_rows_across_adapter_namings() {
    let claude = json!({ "toolCallId": "t", "title": "mcp__acpira__ask_agent", "rawInput": { "agent": "codex-review", "prompt": "look" } });
    let codex = json!({ "toolCallId": "t", "title": "mcp.acpira.ask_agent", "rawInput": { "server": "acpira", "tool": "ask_agent", "arguments": { "prompt": "look" } } });
    let other = json!({ "toolCallId": "t", "title": "mcp__acpira__show_image", "rawInput": {} });
    assert!(names_ask_agent(&claude) && names_ask_agent(&codex) && !names_ask_agent(&other));
    assert_eq!(ask_agent_args(&claude).unwrap()["agent"], "codex-review");
    assert_eq!(ask_agent_args(&codex).unwrap()["prompt"], "look");
    // claude-agent-acp 0.83.0 refinements carry the grown rawInput without the title
    let refinement = json!({ "toolCallId": "t", "rawInput": { "agent": "codex-review", "prompt": "look" } });
    assert!(!names_ask_agent(&refinement));
    // ...and to an AIR client its first tool_call names the tool with no rawInput at all
    let first = json!({ "toolCallId": "t", "title": "mcp__acpira__ask_agent", "status": "pending" });
    assert!(names_ask_agent(&first) && ask_agent_args(&first).is_none());
  }

  #[test]
  fn a_call_fits_only_the_round_it_opened() {
    let h = |persona: &str, thread: &str, round: u32| SubagentHarness {
      agent: "codex".into(),
      persona: Some(persona.into()),
      mode: RelayMode::Consult,
      thread: thread.into(),
      round,
      session_id: None,
      effort: None,
    };
    assert!(call_fits("codex-review", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(call_fits("Codex Review", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(!call_fits("claude-planner", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    // A scout's or an executor's call without a thread may continue its idle conversation
    assert!(call_fits("codex-review", None, &h("codex-review", "n1", 2), Some("Codex Review")));
    assert!(call_fits("", Some("n1"), &h("codex-review", "n1", 2), None));
    assert!(!call_fits("", Some("n2"), &h("codex-review", "n1", 2), None));
  }

  #[test]
  fn caller_answers_map_labels_to_option_ids_by_question_order() {
    use acpira_shared::transcript::{Question, QuestionKind, QuestionOption};
    let opt = |id: &str, label: &str| QuestionOption { id: id.into(), label: label.into(), description: None };
    let q = |id: &str, kind, other| Question {
      id: id.into(),
      title: None,
      text: format!("{id}?"),
      kind,
      options: vec![opt("a", "Alpha, first"), opt("b", "Beta")],
      other,
      numeric: None,
      required: None,
    };
    let qs = [q("one", QuestionKind::Single, Some(false)), q("many", QuestionKind::Multiple, None)];
    let got = caller_answers(&qs, Some(&["alpha, FIRST".into(), "beta, my own".into()]), "").unwrap();
    assert_eq!(got["one"], "a");
    assert_eq!(got["many"], json!(["b", "my own"]));
    // A closed question takes only its options; several questions need `answers`
    assert!(caller_answers(&qs, Some(&["gamma".into()]), "").unwrap_err().contains("Alpha, first, Beta"));
    assert!(caller_answers(&qs, None, "Beta").unwrap_err().contains("2 questions"));
    assert_eq!(caller_answers(&qs[..1], None, " beta ").unwrap()["one"], "b");
  }

  #[test]
  fn the_brief_carries_the_standing_rules_and_the_read_only_line_for_a_consult() {
    let p = SubagentPersona {
      id: "r".into(),
      name: "R".into(),
      agent: "codex".into(),
      model: None,
      effort: None,
      mode: RelayMode::Consult,
      when: String::new(),
      brief: Some("Three severity levels.".into()),
      enabled: true,
      duty: SubagentDuty::General,
      changes: false,
    };
    let consult = compose_brief(" review the diff ", &p, RelayMode::Consult, None);
    assert!(consult.starts_with("review the diff\n\nThree severity levels."));
    assert!(consult.contains("do not modify any file"));
    assert!(!compose_brief("fix it", &p, RelayMode::Work, None).contains("do not modify"));
    // How the reply ends follows what the round is for: findings by severity and place for a reviewer, sources for a
    // consult, changes, checks and assumptions for work
    assert!(consult.contains("each point with its path:line, and what you assumed"));
    let work = compose_brief("fix it", &p, RelayMode::Work, None);
    assert!(work.contains("End with the files you changed (or that you changed none), what you ran to check the work"));
    let reviewer = SubagentPersona { duty: SubagentDuty::Reviewer, ..p };
    let review = compose_brief("review", &reviewer, RelayMode::Work, None);
    assert!(review.contains("severity (high / medium / low), path:line") && !review.contains("files you changed"));
    assert!(review.ends_with("otherwise decide yourself.)"));
    // What is handed over sits after the persona's brief and before how the reply ends
    let handed = compose_brief("review", &reviewer, RelayMode::Consult, Some(PACK_LINE));
    let at = handed.find(PACK_LINE).unwrap();
    assert!(at > handed.find("Three severity levels.").unwrap() && at < handed.find("severity (high / medium / low)").unwrap(), "{handed}");
  }

  fn row(kind: ToolKind, paths: &[&str], f: impl FnOnce(&mut ToolCallBlock)) -> AgentBlock {
    let mut tc = ToolCallBlock {
      observation: None,
      id: random_uuid(),
      kind,
      verb: String::new(),
      verb_key: None,
      mcp_server: None,
      target: None,
      target_mono: None,
      locations: Some(paths.iter().map(|p| Location { path: (*p).to_owned(), line: None }).collect()),
      read_range: None,
      status: ToolStatus::Completed,
      background: None,
      async_task: None,
      started_at: None,
      ended_at: None,
      meta: None,
      diff_stat: None,
      todo_entries: None,
      content: None,
      contents: None,
      subagent_id: None,
    };
    f(&mut tc);
    AgentBlock::ToolCall(tc)
  }

  fn diff(path: &str, add: usize, del: usize) -> ToolContent {
    let line = |kind| DiffLine { kind, text: "x".into(), old_line: None, new_line: None };
    let mut lines: Vec<DiffLine> = (0..add).map(|_| line(DiffKind::Add)).collect();
    lines.extend((0..del).map(|_| line(DiffKind::Del)));
    lines.push(line(DiffKind::Ctx));
    ToolContent::Diff { lines, source: Some(DiffSource { path: path.into(), old_text: String::new(), new_text: String::new(), omitted: false }) }
  }

  #[test]
  fn a_rounds_edit_rows_add_up_per_file_and_commands_mark_it_as_reaching_anywhere() {
    let turns = |blocks| vec![Turn::Agent(AgentTurn { blocks, ..Default::default() })];
    let (files, anywhere) = round_edits(&turns(vec![
      row(ToolKind::Edit, &["/w/a.rs"], |t| t.content = Some(diff("/w/a.rs", 2, 1))),
      row(ToolKind::Edit, &["/w/a.rs"], |t| t.content = Some(diff("/w/a.rs", 3, 0))),
      // `contents` holds every item when there are several; `content` repeats the first and is not counted again
      row(ToolKind::Edit, &["/w/b.rs", "/w/c.rs"], |t| {
        t.content = Some(diff("/w/b.rs", 1, 0));
        t.contents = Some(vec![diff("/w/b.rs", 1, 0), diff("/w/c.rs", 0, 4)]);
      }),
      // No diff: the row's own stat, when it names one file
      row(ToolKind::Edit, &["/w/d.rs"], |t| t.diff_stat = Some(DiffStat { add: 7, del: 2 })),
      // A failed edit changed nothing
      row(ToolKind::Edit, &["/w/e.rs"], |t| {
        t.status = ToolStatus::Failed;
        t.content = Some(diff("/w/e.rs", 9, 9));
      }),
      row(ToolKind::Read, &["/w/f.rs"], |_| {}),
    ]));
    let want: Vec<(String, u64, u64)> =
      vec![("/w/a.rs".into(), 5, 1), ("/w/b.rs".into(), 1, 0), ("/w/c.rs".into(), 0, 4), ("/w/d.rs".into(), 7, 2)];
    assert_eq!(files, want);
    assert!(!anywhere, "edits naming their files and reads stay within what the rows say");
    let (_, anywhere) = round_edits(&turns(vec![row(ToolKind::Execute, &[], |_| {})]));
    assert!(anywhere);
  }

  #[test]
  fn the_change_note_names_edited_files_relative_to_the_folder_and_what_commands_changed_besides() {
    let edits = vec![("/w/src/a.rs".to_owned(), 5, 1), ("/elsewhere/b.rs".to_owned(), 0, 0)];
    assert_eq!(change_note("/w", &edits, false, Some(&[])), "files it edited: src/a.rs (+5 -1), /elsewhere/b.rs");
    assert_eq!(change_note("/w", &[], false, Some(&[])), "it changed no files");
    assert_eq!(change_note("/w", &[], true, Some(&[])), "it changed no files");
    assert_eq!(
      change_note("/w", &[], true, None),
      "it reported no file edits, and outside a git work tree Acpira cannot see what its commands changed"
    );
    // What the snapshot saw besides the edit rows; a file the rows already name is not repeated
    let seen = [PathBuf::from("/w/src/a.rs"), PathBuf::from("/w/gen/out.txt")];
    assert_eq!(
      change_note("/w", &edits[..1], true, Some(&seen)),
      "files it edited: src/a.rs (+5 -1); these also changed while its commands ran: gen/out.txt"
    );
    assert_eq!(change_note("/w", &[], true, Some(&seen[1..])), "it edited no file itself, but these changed while its commands ran: gen/out.txt");
    // Without commands the snapshot is not consulted
    assert_eq!(change_note("/w", &edits[..1], false, Some(&seen)), "files it edited: src/a.rs (+5 -1)");
    let many: Vec<(String, u64, u64)> = (0..NOTE_FILES + 3).map(|i| (format!("/w/f{i}"), 1, 0)).collect();
    assert!(change_note("/w", &many, false, Some(&[])).ends_with("f19 (+1 -0) and 3 more"));
  }
}
