//! How the session follows its summoned rounds: the questions they wait on, the results owed to the root and the
//! automatic message that delivers them, the root's `ask_agent` rows, and close / truncate / cancel

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tokio::sync::watch;

use acpira_shared::transcript::SessionStatus;

use super::personas::call_fits;
use super::texts::wake_text;
use super::{MAX_WAKES, Owed, PendingCall};
use crate::acp::session::prompt::Origin;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::subagent_tree::RouteCtx;
use crate::acp::transport::process::AgentProcess;
use crate::i18n::t;

/// The `ask_agent` arguments of a root tool update, when it is Acpira's tool (named like `show_image` per adapter)
/// The arguments of an `ask_agent` row. `known`: an earlier update of this row already named the tool, so this one
/// counts even without the name
pub(super) fn names_ask_agent(u: &Value) -> bool {
  let named = |x: Option<&Value>| x.and_then(Value::as_str).is_some_and(is_ask_name);
  named(u.get("title")) || named(u.get("name")) || named(u.get("rawInput").and_then(|r| r.get("tool")))
}

/// Whether a tool name is Acpira's `ask_agent` (bare, or the way an adapter prefixes its server)
pub(super) fn is_ask_name(x: &str) -> bool {
  static ASK: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| acpira_shared::re::compile(r"(?i)(?:^|acpira[_.:/-]{1,2})ask_agent$"));
  ASK.is_match(x.trim())
}

/// The call's arguments, codex-acp's `{ server, tool, arguments }` unwrapped; None while no input arrived yet
pub(super) fn ask_agent_args(u: &Value) -> Option<Map<String, Value>> {
  let raw = u.get("rawInput").and_then(Value::as_object)?;
  Some(raw.get("arguments").and_then(Value::as_object).unwrap_or(raw).clone())
}

impl AcpSession {
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
  pub(super) fn relay_settled(self: &Arc<Self>) {
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
