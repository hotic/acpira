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
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::sync::watch;

use acpira_shared::subagents::{GROUP_MEMBERS_MAX, RelayMode, SubagentPersona};
use acpira_shared::transcript::{AgentBlock, ToolCallBlock, Turn};

use crate::acp::session::hooks::{Reach, edit_paths, row_reach};
use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::process::{AgentProcess, ClientHandlers};
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::handoff::{Pack, PackFile, Recipe, Scope};
use crate::relay::MAX_RUNNING;
use crate::relay::wire::HubReply;

mod calls;
mod controls;
mod personas;
mod round;
mod texts;
mod tracking;

use self::texts::short;
use self::tracking::is_ask_name;

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
pub(crate) const BYPASS_MODES: [&str; 4] = ["bypassPermissions", "agent-full-access", "bypass", "yolo"];
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
  fn on_remote_tool(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let thread = self.thread.clone();
    Box::pin(async move {
      let Some(s) = s else { return Err(RpcError::new(-32603, "the session is gone")) };
      let me = Arc::downgrade(&s);
      let notify: crate::acp::session::remote::Notify = Arc::new(move |method: &str, params: Value| {
        let proc = me.upgrade().and_then(|s| s.core.lock().relays.procs.get(&thread).map(|r| r.proc.clone()));
        if let Some(p) = proc {
          p.notify(method, params);
        }
      });
      s.on_remote_tool(req, cancel, notify).await
    })
  }

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

/// A round one group call started among others: the group's id goes on the node, and the pack built once for every
/// member that hands over
#[derive(Clone)]
struct Via {
  group: String,
  pack: Option<Arc<Option<Pack>>>,
}

// A whole group can always run at once in an idle session
const _: () = assert!(GROUP_MEMBERS_MAX <= MAX_RUNNING);

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

#[cfg(test)]
mod tests {
  use super::controls::{bypass_modes, current_name, mode_ids, pick_option, round_mode};
  use super::personas::call_fits;
  use super::texts::{caller_answers, change_note, compose_brief, round_edits};
  use super::tracking::{ask_agent_args, names_ask_agent};
  use super::*;
  use crate::util::random_uuid;
  use acpira_shared::subagents::{SubagentDuty, SubagentHarness};
  use acpira_shared::transcript::{AgentTurn, DiffLine, DiffSource, DiffStat, Location, ToolCallBlock};
  use acpira_shared::transcript::{DiffKind, ToolContent, ToolKind, ToolStatus};
  use std::path::PathBuf;

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
      group: None,
    };
    assert!(call_fits("codex-review", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(call_fits("Codex Review", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(!call_fits("claude-planner", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    // A scout's or an executor's call without a thread may continue its idle conversation
    assert!(call_fits("codex-review", None, &h("codex-review", "n1", 2), Some("Codex Review")));
    assert!(call_fits("", Some("n1"), &h("codex-review", "n1", 2), None));
    assert!(!call_fits("", Some("n2"), &h("codex-review", "n1", 2), None));
    // A call naming a group fits the rounds that group started
    let member = SubagentHarness { group: Some("reviewers".into()), ..h("codex-review", "n3", 1) };
    assert!(call_fits("reviewers", None, &member, Some("Codex Review")));
    assert!(!call_fits("reviewers", None, &h("codex-review", "n1", 1), Some("Codex Review")));
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
