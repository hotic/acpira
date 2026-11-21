//! The turn itself: prompt, settle and cancel. The follow-up queue, usage snapshots, update routing, compaction policy,
//! the automatic account switch and plan execution are further `impl AcpSession` blocks in sibling modules

mod droid;

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::slash_commands::{command_changes, command_name, named_command};
use acpira_shared::transcript::*;
use acpira_shared::turn_errors::is_context_length_error;
use acpira_shared::turn_settings::capture_turn_settings;

use crate::acp::session::attachments::{PreparedPrompt, prepare_prompt, prompt_caps_of};
use crate::acp::session::compaction::{CompactionCompletion, is_compact_command};
use crate::acp::session::edit::{Handoff, HistoryContext, history_context};
use crate::acp::session::errors::{error_text, is_auth, is_session_gone, turn_error_of};
use crate::acp::session::failure::{failure_of, failure_turn_error};
use crate::acp::session::queue::PromptSteerEnd;
use crate::acp::session::turn_usage::{add_wire_usage, prompt_usage_of};
use crate::acp::session::{AcpSession, Core, num};
use crate::acp::transcript::normalize::{activity_of, apply_goal, apply_session_failure, end_turn, fail_turn};
use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::acp::vendors::antigravity::ReplyError;
use crate::acp::vendors::{FailureLog, Vendor, claude_window, droid_usage, goal, kimi_failure, logged_failure, pi_failure, pi_usage};
use crate::i18n::{t, tp};
use crate::limits::TITLE_MAX;
use crate::util::{js_num, now_ms, random_uuid};

/// An already staged payload: the queue flush hands its entry over, an edited turn also marks the user turn
#[derive(Clone)]
pub struct Staged {
  pub prepared: PreparedPrompt,
  pub edited: bool,
  /// The queued entry's id: the user turn keeps it, so the webview's pending bubble for that entry becomes the sent one in place
  pub id: Option<String>,
}

/// Who sent a prompt: the user (or the queue on the user's behalf), the over-threshold /compact, the continue that
/// follows an automatic account switch, the continue a Retry sends after a failed turn that already did some work, or
/// the workspace gate sending what blocked the last turn back to the agent (`hooks.rs`), or a goal control from the dock
/// sent as its `/goal …` command (`session/goal.rs`), or an active goal's next turn (`goal_wake`), or the results
/// summoned agents owe the root (`relay.rs`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
  User,
  Compact,
  Continue,
  Retry,
  Gate,
  Goal,
  GoalContinue,
  Subagents,
}

impl Origin {
  /// Sent into a session something reserved for it (the account switch, the gate): it goes ahead of the queue, and is
  /// dropped when anything else took the session first
  fn reserved(self) -> bool {
    matches!(self, Origin::Continue | Origin::Gate)
  }
}

/// Where `claim` sends a prompt: onto the wire (after an over-budget /compact first), behind the running turn, or nowhere
enum Gate {
  Queue,
  Drop,
  /// Another Acpira engine is running a turn on this session, or its lease could not be taken (`store::session_lease`):
  /// nothing is sent, the reason is shown
  Elsewhere(String),
  /// `rebuild`: an ultracode switch is still to be applied and was reserved for this prompt (`ultracode.rs`)
  Go { compact_first: bool, rebuild: bool },
}

/// The staged payload, plus a fork's history context (or why it could not be built)
struct Staging {
  prepared: PreparedPrompt,
  edited_staged: bool,
  /// The id the user turn takes over from its queued entry
  queued_id: Option<String>,
  fork_history: Option<HistoryContext>,
  fork_error: Option<String>,
}

/// A prompt accepted onto the transcript: its user turn and what settling the answer needs of it
struct Accepted {
  user_turn: UserTurn,
  prepared: PreparedPrompt,
  compacting: bool,
  /// The slash command the text starts with, recorded on the agent turn as a receipt
  command_name: Option<String>,
  before: TurnSettings,
  forked: bool,
  edited_staged: bool,
}

/// The agent turn a prompt opened, and the process generation its answer must still belong to
struct OpenTurn {
  proc: Option<Arc<AgentProcess>>,
  acp_id: Option<String>,
  proc_gen: u64,
  usage_before: u64,
  agent_idx: usize,
  started_at: i64,
}

impl OpenTurn {
  fn live(&self, c: &Core) -> bool {
    c.proc_gen == self.proc_gen && c.status != SessionStatus::Closed
  }
}

/// How the answer settled the turn, and whether it reserved the session for an automatic account switch
struct Settled {
  stop: TurnStop,
  exhausted: bool,
}

impl AcpSession {
  pub(crate) fn caps(&self, c: &Core) -> crate::acp::session::attachments::PromptCaps {
    prompt_caps_of(c.proc.as_ref().map(|p| &p.init), &self.def())
  }

  /// auto: sent by Acpira itself (over-threshold /compact); doesn't change the title and renders as a note line.
  /// running is claimed before staging so a second send meanwhile queues instead of racing onto the wire
  pub fn prompt(
    self: &Arc<Self>,
    text: String,
    drafts: Vec<Draft>,
    auto: bool,
    staged: Option<Staged>,
    plan_id: Option<String>,
  ) -> BoxFuture<()> {
    let me = self.clone();
    let origin = if auto { Origin::Compact } else { Origin::User };
    Box::pin(async move { me.prompt_inner(text, drafts, origin, staged, plan_id).await })
  }

  /// One prompt, phase by phase: claim the session, stage the payload, accept it onto the transcript, compact first
  /// when over budget, open the agent turn, send it, settle the answer and hand over to what follows the turn
  pub(crate) async fn prompt_inner(self: Arc<Self>, text: String, drafts: Vec<Draft>, origin: Origin, staged: Option<Staged>, plan_id: Option<String>) {
    let auto = origin == Origin::Compact;
    let compact_first = match self.claim(&text, &drafts, origin, staged.as_ref()) {
      Gate::Queue => {
        self.enqueue(text, drafts, staged.map(|s| s.prepared)).await;
        return;
      }
      Gate::Drop => {
        if origin.reserved() {
          self.flush_queue();
        }
        return;
      }
      // A reservation (the gate's follow-up, the switch's continue) was let go in `claim`; the queue stays where it is,
      // since every entry it sent would be refused the same way. Owed results stay owed
      Gate::Elsewhere(reason) => {
        if origin == Origin::Subagents {
          self.core.lock().relays.delivered(false);
        }
        self.log(&format!("turn refused: {reason}"));
        self.notify(&reason);
        return;
      }
      Gate::Go { compact_first, rebuild } => {
        // The switch reaches the native session before the prompt does; still staging, so a stop meanwhile drops the
        // prompt in `accept`
        if rebuild && !self.rebuild_before_prompt().await {
          self.park_after_failed_rebuild(text, drafts, origin, staged).await;
          return;
        }
        compact_first
      }
    };
    let mut staging = self.stage_turn(&text, &drafts, staged, origin).await;
    // The workspace hooks: `beforePrompt` may block the prompt or add context behind it, the snapshot is taken before
    // anything reaches the agent, and a first prompt carries the context files in front
    let context = match self.hooks_before_turn(origin, &text, &staging.prepared.attachments).await {
      Ok(context) => context,
      Err(reason) => {
        self.hooks_prompt_blocked(&text, &reason);
        return;
      }
    };
    if context.plugins.dropped {
      self.plugin_dropped();
      return;
    }
    if let Some((plugin, agent)) = context.plugins.redirect {
      self.plugin_redirect(text, drafts, plugin, agent);
      return;
    }
    // A plugin's rewrite reaches the agent; the bubble keeps what the user wrote
    if let Some(rewritten) = context.plugins.text
      && let Some(block) = staging.prepared.blocks.iter_mut().find(|b| b.get("type").and_then(Value::as_str) == Some("text") && b.get("text").and_then(Value::as_str) == Some(text.as_str()))
    {
      block["text"] = Value::String(rewritten);
    }
    staging.prepared.blocks.splice(0..0, context.lead);
    staging.prepared.blocks.extend(context.tail);
    // Read before the turn clock or wire starts, while cancellation can still drop the staged prompt. A pre-send
    // compaction can change the counters, so a baseline taken before it would not belong to this turn.
    let droid_baseline = if self.vendor == Vendor::Droid && !compact_first && !auto && !is_compact_command(&text) {
      let source = {
        let c = self.core.lock();
        c.proc.as_ref().and_then(|p| p.droid_sessions.clone()).zip(c.acp_session_id.clone())
      };
      match source {
        Some((dir, id)) => droid_usage::Baseline::capture(dir, id).await,
        None => None,
      }
    } else {
      None
    };
    let Some(mut accepted) = self.accept(&text, origin, plan_id.as_deref(), staging) else { return };
    if compact_first && !self.compact_before(&accepted.user_turn).await {
      return;
    }
    let turn_id = accepted.user_turn.id.clone();
    let title_before = self.core.lock().state.title.clone();
    let turn = self.open_turn(&text, origin, plan_id.as_deref(), &mut accepted);
    self.emit_turn_started(origin, &text);
    self.plugin_prompt_opened(origin);
    // Titled after this prompt: a plugin may title it otherwise
    if let Some(title) = self.core.lock().state.title.clone().filter(|t| title_before.as_ref() != Some(t)) {
      self.plugin_title(title, "prompt", Some(text.clone()));
    }
    let blocks = std::mem::take(&mut accepted.prepared.blocks);
    // From here on Claude Code has a transcript for this native session to resume
    {
      let mut c = self.core.lock();
      c.unprompted = false;
      // A swap refused in an earlier turn that never settled here (its process closed) is not this turn's
      c.droid_core.blocked = None;
    }
    let mut droid_cut = false;
    let result = match &turn.proc {
      Some(p) => {
        let mut params = self.prompt_params(turn.acp_id.as_deref(), blocks);
        // An agent that rewinds its own history (`edit::rewinds`) learns which user turn this prompt is, to rewind to
        if let Some(id) = turn_id.filter(|_| crate::acp::session::edit::rewinds(&p.init)) {
          params["_meta"]["acpira/turn"] = Value::String(id);
        }
        let send = self.send_prompt(p, params);
        if let Some(model) = accepted.before.config.get("model").filter(|m| m.starts_with("gemini-"))
          && let Some(dir) = p.droid_sessions.as_deref().and_then(|d| d.parent())
          && let Some(id) = turn.acp_id.as_deref()
        {
          // The watcher lives exactly as long as this request; a ready ACP answer always wins.
          tokio::pin!(send);
          tokio::select! {
            biased;
            result = &mut send => result,
            cut = self.watch_droid_cuts(&turn, p, dir, id, model) => {
              droid_cut = cut;
              if cut {
                match tokio::time::timeout(super::lifecycle::CLOSE_GRACE, send).await {
                  Ok(answer) => answer,
                  Err(_) => {
                    self.droid_cancel_unanswered(&turn).await;
                    return;
                  }
                }
              } else {
                send.await
              }
            }
          }
        } else {
          send.await
        }
      }
      None => Err(RpcError::internal("no process")),
    };
    let settled = match result {
      Ok(r) if droid_cut && r["stopReason"] == "cancelled" => {
        let mut c = self.core.lock();
        if !turn.live(&c) { return; }
        self.settle(&mut c, TurnStop::Error, Some(droid::cut_error()));
        Some(Settled { stop: TurnStop::Error, exhausted: false })
      }
      // Droid moved onto a Droid Core model during the turn and was stopped (`droid_core.rs`)
      Ok(_) if self.core.lock().droid_core.blocked.is_some() => self.settle_core_swap(&turn).await,
      Ok(r) => self.prompt_answered(r, &turn, &accepted, auto, droid_baseline).await,
      Err(e) => self.prompt_failed(e, &turn).await,
    };
    self.core.lock().droid_core.blocked = None;
    if accepted.forked {
      self.reclaim_fork_history(&turn);
    }
    if let Some(settled) = settled {
      self.finish_turn(&turn, auto, accepted.compacting, settled);
    }
  }

  /// A fork's first prompt that failed or was stopped before the agent produced anything may never have reached it:
  /// the history is owed again, so a Retry or the next message carries it (`retry_turn` assumes the same of an empty attempt)
  fn reclaim_fork_history(&self, turn: &OpenTurn) {
    let mut c = self.core.lock();
    let Some(a) = agent_turn_mut(&mut c, turn.agent_idx, turn.started_at) else { return };
    let empty = a.blocks.iter().all(|b| match b {
      AgentBlock::Text(x) => x.markdown.trim().is_empty(),
      AgentBlock::Notice(_) | AgentBlock::Goal(_) => true,
      _ => false,
    });
    if empty && matches!(a.stop, Some(TurnStop::Error | TurnStop::Cancelled)) {
      c.lineage.history_pending = true;
      self.touch(&mut c);
      drop(c);
      self.log("fork history owed again: the first prompt ended without output");
    }
  }

  /// The turn's `session/prompt`. On an agent that steers by prompt (`Vendor::steers_by_prompt`) each steer is a prompt
  /// of its own that answers the one before it at once and owns the rest of the turn (`steer_by_prompt`): the turn
  /// follows them to the last one's answer, which carries the usage every earlier answer reported
  async fn send_prompt(&self, p: &AgentProcess, params: Value) -> Result<Value, RpcError> {
    if !self.vendor.steers_by_prompt(&p.init) {
      return p.request("session/prompt", params).await;
    }
    let mut answer = p.request_ordered("session/prompt", params).await;
    let mut earlier = Value::Null;
    loop {
      let (mut r, handoff) = answer?;
      // Under the handoff, so the steer block lands ahead of whatever the steer's own prompt streams
      let Some(next) = self.land_prompt_steer() else {
        drop(handoff);
        add_wire_usage(&mut r, &earlier);
        return Ok(r);
      };
      // A stop that reached the agent ahead of the steer ended only the prompt it stopped: the steer would run on
      if r.get("stopReason").and_then(Value::as_str) == Some("cancelled")
        && let Some(sid) = self.core.lock().acp_session_id.clone()
      {
        p.notify("session/cancel", json!({ "sessionId": sid }));
      }
      drop(handoff);
      add_wire_usage(&mut earlier, &r);
      answer = next.await.unwrap_or_else(|_| Err(RpcError::internal("the steer's prompt went without an answer")));
    }
  }

  /// The `session/prompt` params: the blocks, and the auto-compaction setting as `_meta["acpira/compaction"]`, which
  /// the built-in agent reads to compact inside the turn at that threshold (every other agent ignores it)
  pub(crate) fn prompt_params(&self, acp_id: Option<&str>, blocks: Vec<Value>) -> Value {
    let mut params = json!({ "sessionId": acp_id, "prompt": blocks });
    // Every turn names whose settings it reads: the session's sender may have changed since the last one
    self.with_client_settings(&mut params, &self.agent);
    if let Some(policy) = self.deps.policy.compaction() {
      params["_meta"]["acpira/compaction"] = json!({ "auto": policy.auto, "atTokens": policy.at_tokens });
    }
    params
  }

  /// Where a prompt goes. running is claimed here, before staging, so a second send meanwhile queues
  fn claim(&self, text: &str, drafts: &[Draft], origin: Origin, staged: Option<&Staged>) -> Gate {
    let auto = origin == Origin::Compact;
    let mut c = self.core.lock();
    // Owed results and a goal's next turn go only into a free session, never behind anything (the queue is the
    // user's): taken first, results stay owed and the goal is due again at the next turn's end
    if matches!(origin, Origin::Subagents | Origin::GoalContinue)
      && (c.status != SessionStatus::Ready
        || c.phase.running
        || c.switching
        || c.hooks.gating
        || c.picks.adopt_pending
        || !c.picks.values.is_empty()
        || c.ultracode.rebuilding
        || c.peer.detached
        || c.pending_prompt.is_some()
        || !c.queue.entries.is_empty())
    {
      if origin == Origin::Subagents {
        c.relays.delivered(false);
      }
      Gate::Drop
    } else if origin.reserved() && (c.status != SessionStatus::Ready || c.phase.running) {
      // The switch (or the gate) reserved the session for this prompt; anything else taking it first makes it moot
      self.release_reservation(&mut c, origin);
      Gate::Drop
    } else if c.status == SessionStatus::Starting {
      Gate::Queue
    } else if c.status != SessionStatus::Ready
      || (text.trim().is_empty() && drafts.is_empty() && staged.is_none_or(|s| s.prepared.blocks.is_empty()))
    {
      Gate::Drop
    } else if !origin.reserved()
      && (c.switching
        || c.hooks.gating
        || c.picks.adopt_pending
        || !c.picks.values.is_empty()
        || c.ultracode.rebuilding
        || c.phase.running
        || c.peer.detached
        // A replay being read for a late reply (`late_reply.rs`) would swallow this turn's updates
        || c.late.capture.is_some()
        || (!auto && c.pending_prompt.is_some()))
    {
      Gate::Queue
    } else if !origin.reserved() && AcpSession::reserve_reopen(&mut c) {
      // The process started with what its adapter or account no longer is: the turn waits for the new one
      self.arc().spawn_reopen();
      Gate::Queue
    } else if let Err(reason) = self.lease_turn(&mut c) {
      // A reservation that cannot start must not hold the queue (and its turn's lease) for good
      if origin.reserved() {
        self.release_reservation(&mut c, origin);
      }
      Gate::Elsewhere(reason)
    } else {
      c.switching = false;
      if origin == Origin::Gate {
        c.hooks.gating = false;
      }
      match origin {
        Origin::Subagents => c.relays.delivered(true),
        Origin::User | Origin::Goal => c.relays.user_prompted(),
        _ => {}
      }
      c.peer.idle = false;
      // Whatever goes out now, the goal is due again only when its turn ends with the goal still active
      c.goal_due = false;
      if origin != Origin::Compact {
        // A new attempt may observe another autonomous cycle; late packets from the prior one are ignored.
        c.peer.autonomous_blocked = false;
      }
      // Mid-turn /compact cannot be injected: the next user-facing request is the earliest slot, compact that first
      let compact_first = !auto && !is_compact_command(text) && self.should_auto_compact(&c);
      // A turn the user asked for reads the settings of whoever sent last, fixed until it ends: a send meanwhile waits
      // for the next one. Turns Acpira starts on its own (a /compact ahead of an accepted prompt, a goal's next turn,
      // owed results, the gate's follow-up) go on with what the session reads and leave a pending sender pending
      if matches!(origin, Origin::User | Origin::Retry | Origin::Goal) {
        self.adopt_sender();
      }
      c.phase.running = true;
      c.compaction.auto_eligible = false;
      c.phase.staging = true;
      c.phase.staging_aborted = false;
      if origin == Origin::User {
        self.bump(&mut c);
      } else {
        self.touch(&mut c);
      }
      // An Ultra pick made while the last turn or its background work ran: the earliest slot is ahead of this prompt
      let rebuild = self.reserve_ultracode_rebuild(&mut c, true);
      Gate::Go { compact_first, rebuild }
    }
  }

  /// A reserved prompt (`Origin::reserved`) is not going out: the session stops holding the queue for it
  fn release_reservation(&self, c: &mut Core, origin: Origin) {
    if origin == Origin::Gate {
      c.hooks.gating = false;
    } else {
      c.switching = false;
    }
    self.touch(c);
  }

  /// The payload, staged now unless the queue or an edit already did; a fork's first prompt also builds its history
  async fn stage_turn(&self, text: &str, drafts: &[Draft], staged: Option<Staged>, origin: Origin) -> Staging {
    let edited_staged = staged.as_ref().is_some_and(|s| s.edited);
    let queued_id = staged.as_ref().and_then(|s| s.id.clone());
    let prepared = match staged {
      Some(s) => s.prepared,
      None => {
        let caps = self.caps(&self.core.lock());
        prepare_prompt(&self.id, text, drafts, &self.deps.blobs, &self.file_reader(Some(drafts)).await, Some(caps)).await
      }
    };
    // A fork or rebound peer receives the retained transcript once, on its next prompt.
    let (proc, caps, turns_copy, fork, cwd) = {
      let c = self.core.lock();
      let copy = if origin != Origin::Compact && c.lineage.history_pending { Some(c.state.turns.clone()) } else { None };
      (c.proc.clone(), self.caps(&c), copy, c.lineage.forked_from.clone(), self.cwd.clone())
    };
    let mut fork_history = None;
    let mut fork_error = None;
    if let (Some(turns), Some(proc)) = (turns_copy, proc.as_ref()) {
      let lead = self.prompts().text("fork/history", acpira_agent::prompt::BASE);
      let spec = Handoff { lead: &lead, fork: fork.as_ref(), cwd: &cwd, trim: true, readable: self.reads_itself() };
      match history_context(&self.id, &turns, proc, &self.deps.blobs, &self.file_reader(None).await, caps, spec).await {
        Ok(h) => fork_history = h,
        Err(e) => {
          self.log(&format!("fork context skipped: {e}"));
          fork_error = Some(e.to_string());
        }
      }
    }
    Staging { prepared, edited_staged, queued_id, fork_history, fork_error }
  }

  /// Staging is over: the user turn as it will be recorded, or None when a cancel or close came first
  fn accept(self: &Arc<Self>, text: &str, origin: Origin, plan_id: Option<&str>, staging: Staging) -> Option<Accepted> {
    let Staging { mut prepared, edited_staged, queued_id, fork_history, fork_error } = staging;
    let mut edited = edited_staged;
    let mut c = self.core.lock();
    c.phase.staging = false;
    if c.phase.staging_aborted || c.status != SessionStatus::Ready {
      drop(c);
      self.log("prompt dropped: cancelled or closed while staging");
      let mut c = self.core.lock();
      c.phase.running = false;
      self.touch(&mut c);
      drop(c);
      self.flush_queue();
      return None;
    }
    if origin != Origin::Compact && c.lineage.history_pending {
      c.lineage.history_pending = false;
      match &fork_history {
        Some(h) => {
          c.lineage.history_context = true;
          let mut blocks = h.blocks.clone();
          blocks.append(&mut prepared.blocks);
          prepared.blocks = blocks;
          edited = true;
          // An agent that can read_session fetches the left-out turns on demand behind their outline: nothing for
          // the user to act on, so only the log records it
          if h.omitted > 0 && h.recoverable {
            self.log(&format!("fork context trimmed: {} earlier turns behind an outline", h.omitted));
          } else if h.omitted > 0 {
            self.notify(&tp("host.forkContextTrimmed", &[("count", &h.omitted.to_string())]));
          }
        }
        None => self.notify(&match &fork_error {
          Some(e) => tp("host.forkContextFailed", &[("error", e)]),
          None => t("host.forkContextTooLarge"),
        }),
      }
    }
    for p in &prepared.problems {
      self.log(p);
      self.notify(p);
    }
    if !prepared.attachments.is_empty() {
      let skip = usize::from(!text.is_empty());
      let kinds: Vec<&str> = prepared.blocks.iter().skip(skip).map(|b| b.get("type").and_then(Value::as_str).unwrap_or("?")).collect();
      self.log(&format!("attachments: {}", kinds.join(" ")));
    }
    let before = capture_turn_settings(&c.state.controls);
    let command = named_command(&c.state.commands, text).map(|x| x.name.clone());
    let command_name = command_name(text).map(str::to_owned);
    let user_turn = match origin {
      Origin::Compact => UserTurn { text: text.to_owned(), auto: Some(true), ..Default::default() },
      Origin::Continue | Origin::Retry | Origin::Gate | Origin::Goal | Origin::GoalContinue | Origin::Subagents => UserTurn {
        id: Some(random_uuid()),
        text: text.to_owned(),
        settings: Some(before.clone()),
        auto: Some(true),
        auto_reason: Some(match origin {
          Origin::Retry => AutoReason::Retry,
          Origin::Gate => AutoReason::Gate,
          Origin::Goal | Origin::GoalContinue => AutoReason::Goal,
          Origin::Subagents => AutoReason::Subagents,
          _ => AutoReason::AccountSwitch,
        }),
        goal_continue: origin == Origin::GoalContinue,
        ..Default::default()
      },
      Origin::User => UserTurn {
        id: Some(queued_id.unwrap_or_else(random_uuid)),
        text: text.to_owned(),
        settings: Some(before.clone()),
        command,
        edited,
        plan_id: plan_id.map(str::to_owned),
        attachments: (!prepared.attachments.is_empty()).then(|| prepared.attachments.clone()),
        ..Default::default()
      },
    };
    Some(Accepted {
      user_turn,
      prepared,
      compacting: is_compact_command(text),
      command_name,
      before,
      forked: fork_history.is_some(),
      edited_staged,
    })
  }

  /// The over-budget /compact that goes out ahead of an accepted prompt, which waits below it as `pending_prompt`.
  /// false = the session did not come back Ready: the bubble is kept and the prompt goes no further
  async fn compact_before(self: &Arc<Self>, user_turn: &UserTurn) -> bool {
    {
      let mut c = self.core.lock();
      self.log_usage_threshold(&c, "auto /compact before prompt");
      c.pending_prompt = Some(Turn::User(user_turn.clone()));
      c.phase.running = false;
    }
    self.compact(true).await.ok();
    let mut c = self.core.lock();
    c.pending_prompt = None;
    // Keep the accepted bubble even if the peer disconnected during compaction
    if c.status != SessionStatus::Ready {
      c.state.turns.push(Turn::User(user_turn.clone()));
      self.touch(&mut c);
      return false;
    }
    c.phase.running = true;
    true
  }

  /// Record the user turn, title an untitled session after it and open the agent turn the answer streams into
  fn open_turn(&self, text: &str, origin: Origin, plan_id: Option<&str>, accepted: &mut Accepted) -> OpenTurn {
    let mut c = self.core.lock();
    c.lineage.agent_title_muted = c.lineage.history_context || c.lineage.forked_from.is_some() || accepted.forked || accepted.edited_staged || (c.unprompted && c.lineage.agent_title_muted);
    c.compaction.completion = Some(CompactionCompletion::new(accepted.compacting.then_some(self.agent.as_str())));
    c.turn_failure = None;
    c.state.turns.push(Turn::User(std::mem::take(&mut accepted.user_turn)));
    let untitled = c.state.title.as_deref().is_none_or(|x| x.is_empty() || x == t("session.untitled"));
    if origin == Origin::User && plan_id.is_none() && untitled {
      let summary = summarize_prompt(text, &accepted.prepared.attachments);
      c.state.title = Some(crate::util::tidy_title(&summary, TITLE_MAX));
    }
    let started_at = now_ms();
    let activity = activity_of(&c.state.turns);
    c.state.turns.push(Turn::Agent(AgentTurn {
      started_at: Some(started_at),
      activity,
      command: accepted.command_name.clone().map(|n| CommandReceipt { name: n, mode: None, options: None }),
      ..Default::default()
    }));
    let agent_idx = c.state.turns.len() - 1;
    if std::mem::take(&mut c.retry_pending) {
      c.retried_turn = Some(started_at);
    }
    self.touch(&mut c);
    self.schedule_usage_poll(&mut c);
    OpenTurn {
      proc: c.proc.clone(),
      acp_id: c.acp_session_id.clone(),
      proc_gen: c.proc_gen,
      usage_before: c.usage.revision,
      agent_idx,
      started_at,
    }
  }

  /// session/prompt answered: usage, a structured session failure, the compaction and usage waits, the empty-answer
  /// check. None = the session moved on meanwhile (replaced, closed, no longer Ready) and the turn is not settled here
  async fn prompt_answered(
    self: &Arc<Self>,
    r: Value,
    turn: &OpenTurn,
    accepted: &Accepted,
    auto: bool,
    droid_baseline: Option<droid_usage::Baseline>,
  ) -> Option<Settled> {
    let (agent_idx, started_at) = (turn.agent_idx, turn.started_at);
    if !turn.live(&self.core.lock()) {
      return None;
    }
    let reason = r.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_owned();
    self.log(&format!("prompt done: {reason}"));
    let mut stop = TurnStop::parse(&reason).unwrap_or(TurnStop::EndTurn);
    let mut exhausted = false;
    let usage = prompt_usage_of(&r, droid_baseline).await;
    let log = |line: &str| self.log(line);
    let failure = failure_of(r.get("_meta"), Some(&log));
    {
      let mut c = self.core.lock();
      if !turn.live(&c) || (self.vendor == Vendor::Droid && c.status != SessionStatus::Ready) {
        return None;
      }
      if let Some(u) = usage
        && let Some(turn) = agent_turn_mut(&mut c, agent_idx, started_at)
      {
        let ctx = turn.usage.as_ref().and_then(|x| x.context);
        let mut merged = turn.usage.clone().unwrap_or_default();
        merge_usage(&mut merged, u);
        if merged.context.is_none() {
          merged.context = ctx;
        }
        turn.usage = Some(merged);
      }
    }
    if let Some(f) = failure.as_ref().filter(|f| f.severity == Severity::Error) {
      let mut c = self.core.lock();
      apply_session_failure(&mut c.state, f);
      drop(c);
      self.log(&format!(
        "prompt failed: sessionFailure {} rev {} ({})",
        f.id,
        js_num(f.revision),
        serde_json::to_value(f.category).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
      ));
      let mut c = self.core.lock();
      stop = TurnStop::Cancelled;
      let turn_error = failure_turn_error(f);
      exhausted = self.reserve_switch(&mut c, &turn_error);
      self.settle(&mut c, TurnStop::Cancelled, Some(turn_error));
      if f.actions.contains(&FailureAction::Login) {
        c.status = SessionStatus::AuthRequired;
      }
      return Some(Settled { stop, exhausted });
    }
    if let Some(f) = &failure {
      apply_session_failure(&mut self.core.lock().state, f);
    }
    if stop == TurnStop::EndTurn {
      if self.vendor.corrects_window() {
        let c = self.core.lock();
        if let Some(size) = c.usage.reported_window {
          claude_window::confirm(c.account_id.as_deref(), &c.state.controls, size);
        }
      }
      let pending = self.core.lock().compaction.completion.as_mut().and_then(CompactionCompletion::wait);
      if let Some(rx) = pending {
        self.log("waiting for compaction completion");
        let _ = rx.await;
      }
      if self.status() != SessionStatus::Ready {
        self.flush_queue();
        return None;
      }
      {
        let mut c = self.core.lock();
        let after = c.compaction.completion.as_ref().and_then(|x| x.tokens_after);
        if (auto || accepted.compacting)
          && let (Some(after), Some(u)) = (after, c.state.usage.as_mut())
        {
          u.used = num(after);
        }
        let silent = accepted.compacting && self.mark_silent_compaction(&mut c, turn);
        self.mark_compacted_usage(&mut c, turn, auto || accepted.compacting, after.is_some(), silent);
      }
      if !auto && accepted.command_name.is_none() && self.wait_for_late_usage(turn.usage_before).await {
        stop = TurnStop::Cancelled;
      }
      if self.status() != SessionStatus::Ready {
        return None;
      }
    }
    self.refresh_context_usage().await;
    // The guard lives in this block: an empty completion reads the CLI's session log afterwards, across an await
    {
      let mut c = self.core.lock();
      if !turn.live(&c) {
        return None;
      }
      let controls = c.state.controls.clone();
      if stop == TurnStop::EndTurn
        && let Some(turn) = agent_turn_mut(&mut c, agent_idx, started_at)
        && let Some(cmd) = turn.command.as_mut()
      {
        let (mode, options) = command_changes(&accepted.before, &controls);
        cmd.mode = mode;
        cmd.options = options;
      }
      // Antigravity reports a failed turn as the reply's last text: it becomes the turn's error, an automatic /compact's
      // too (left as text, it read as a finished turn with a stray warning)
      let failed = if stop == TurnStop::EndTurn {
        agent_turn_mut(&mut c, agent_idx, started_at).and_then(|turn| take_reply_error(self.vendor, turn))
      } else {
        None
      };
      // On a cancel agy appends a note meant for its model (`antigravity::cancel_note`): the stop row already says it
      if stop == TurnStop::Cancelled
        && let Some(turn) = agent_turn_mut(&mut c, agent_idx, started_at)
      {
        take_cancel_note(self.vendor, turn);
      }
      if let Some(e) = failed {
        let error = self.reply_error(&mut c, e);
        self.log(&format!("prompt failed in the reply: {}", error.message));
        self.reserve_reauth(&mut c, &error, accepted.compacting);
        self.settle(&mut c, TurnStop::Cancelled, Some(error));
        return Some(Settled { stop: TurnStop::Cancelled, exhausted });
      }
      // Some CLIs acknowledge provider failures as empty end_turn responses: record the missing output
      let empty = stop == TurnStop::EndTurn
        && !auto
        && agent_turn_mut(&mut c, agent_idx, started_at).is_some_and(|turn| {
          turn.command.is_none() && turn.blocks.iter().all(|b| matches!(b, AgentBlock::Text(x) if x.markdown.trim().is_empty()))
        });
      if !empty {
        self.settle(&mut c, stop, None);
        return Some(Settled { stop, exhausted });
      }
    }
    // Kimi answers a failed turn with an empty end_turn: its own session log still holds the cause
    let error = match self.logged_failure(turn).await {
      Some(f) => {
        self.log(&format!("prompt failed (from the CLI's session log): {}", f.message));
        TurnError { message: f.message, kind: f.code, retryable: f.retryable, ..Default::default() }
      }
      None => {
        self.log("prompt empty: end_turn without output or error details");
        TurnError { message: t("host.emptyResponse"), kind: Some("empty_response".into()), retryable: Some(true), ..Default::default() }
      }
    };
    let mut c = self.core.lock();
    if !turn.live(&c) {
      return None;
    }
    stop = TurnStop::Cancelled;
    self.settle(&mut c, stop, Some(error));
    Some(Settled { stop, exhausted })
  }

  /// session/prompt failed: the error card, and a login or reconnect state when resending cannot help. None = disposal or a
  /// replacement process already settled the turn
  async fn prompt_failed(self: &Arc<Self>, e: RpcError, turn: &OpenTurn) -> Option<Settled> {
    // Disposal already settled and persisted the interrupted turn
    if !turn.live(&self.core.lock()) {
      return None;
    }
    self.log(&format!("prompt failed: {e}"));
    self.refresh_context_usage().await;
    let mut c = self.core.lock();
    if !turn.live(&c) {
      return None;
    }
    let code = e.code;
    let err = anyhow::Error::new(e);
    let failure = c.turn_failure.clone();
    let turn_error = match &failure {
      Some(f) => TurnError { code: Some(code), ..failure_turn_error(f) },
      None => turn_error_of(&err),
    };
    let exhausted = self.reserve_switch(&mut c, &turn_error);
    self.settle(&mut c, TurnStop::Cancelled, Some(turn_error));
    if is_auth(&err) || failure.as_ref().is_some_and(|f| f.actions.contains(&FailureAction::Login)) {
      c.status = SessionStatus::AuthRequired;
      // The sign-in notice replaces the error card: it keeps the provider's reason behind "Copy details"
      if c.error.is_none() {
        c.error = Some(error_text(&err));
      }
    } else if is_session_gone(&err) || !c.proc.as_ref().is_some_and(|p| p.alive()) {
      // Resending over this connection can only fail the same way: the error Notice's Retry reconnects
      c.status = SessionStatus::Error;
      c.error = Some(err.to_string());
    }
    Some(Settled { stop: TurnStop::Cancelled, exhausted })
  }

  /// The failure a CLI that swallows them (`Vendor::failure_log`) logged for this turn in its own session store
  async fn logged_failure(&self, turn: &OpenTurn) -> Option<logged_failure::Failure> {
    let log = self.vendor.failure_log()?;
    let session_id = turn.acp_id.clone()?;
    // The agent entry's own env first: a store moved for this entry is where the CLI wrote
    let env = |key: &str| self.def().env.and_then(|e| e.get(key).cloned()).or_else(|| std::env::var(key).ok());
    let since = turn.started_at;
    let read: Box<dyn FnOnce() -> Option<logged_failure::Failure> + Send> = match log {
      FailureLog::Kimi => {
        let home = kimi_failure::home_of(env(kimi_failure::HOME_ENV));
        Box::new(move || kimi_failure::read(&home, &session_id, since))
      }
      FailureLog::Pi => {
        let agent_dir = pi_usage::agent_dir_of(env(pi_usage::AGENT_DIR_ENV));
        let cwd = self.cwd.clone();
        Box::new(move || pi_failure::read(&agent_dir, &cwd, &session_id, since))
      }
    };
    tokio::task::spawn_blocking(read).await.ok().flatten()
  }

  /// The error card for a failure the agent sent as text. A shared MCP server it could not start is left out of this
  /// session, and the connection is rebuilt without it once the turn is settled, so Retry can go through
  fn reply_error(&self, c: &mut Core, e: ReplyError) -> TurnError {
    let (message, kind) = match e {
      ReplyError::McpFailed { server, reason } => {
        let reason = reason.unwrap_or_else(|| "?".into());
        let ours = c.mcp_sent.contains(&server);
        if ours && !c.mcp_skip.contains(&server) {
          c.mcp_skip.push(server.clone());
          c.reconnect_after_turn = true;
        }
        let key = if ours { "host.agentMcpSkipped" } else { "host.agentMcpFailed" };
        (tp(key, &[("name", &server), ("reason", &reason), ("agent", &self.def().name)]), "mcp_failed")
      }
      ReplyError::Region => (tp("host.regionUnsupported", &[("agent", &self.def().name)]), "region_unsupported"),
      ReplyError::AuthExpired(text) => (text, "auth_expired"),
      ReplyError::Other(text) => (text, "agent_error"),
    };
    TurnError { message, kind: Some(kind.into()), retryable: Some(true), ..Default::default() }
  }

  /// A detached run (a background command's exit woke the model, `late_reply.rs`) that ended on a failure Antigravity
  /// sends as text: cut off the last turn into its error, as for a prompt. An expired sign-in reserves the reconnect the
  /// caller then starts (`reauth_due`)
  pub(crate) fn detached_error(&self, c: &mut Core) -> Option<TurnError> {
    let Some(Turn::Agent(turn)) = c.state.turns.last_mut() else { return None };
    let e = take_reply_error(self.vendor, turn)?;
    let error = self.reply_error(c, e);
    self.log(&format!("detached run failed in the reply: {}", error.message));
    self.reserve_reauth(c, &error, false);
    Some(error)
  }

  /// After a settled turn: the compaction mark, then either park the queue (context overflow), switch accounts
  /// (quota exhausted), hand a finished turn to the workspace gate, or compact / flush as usual. A turn that does not
  /// reach the gate leaves its span to the next one that does (`hooks.rs`)
  fn finish_turn(self: &Arc<Self>, turn: &OpenTurn, auto: bool, compacting: bool, settled: Settled) {
    self.watch_late_replies();
    let context_error = {
      let mut c = self.core.lock();
      if auto || compacting {
        c.compaction.at = Some(c.state.usage.map(|u| u.used.0).unwrap_or(0.0));
      }
      c.compaction.auto_eligible = !auto && !compacting && settled.stop == TurnStop::EndTurn;
      self.touch(&mut c);
      agent_turn_mut(&mut c, turn.agent_idx, turn.started_at).is_some_and(|t| is_context_length_error(t.error.as_ref()))
    };
    // Queued messages stay parked until the context is compacted or the input changes
    if context_error {
      return;
    }
    if settled.exhausted {
      tokio::spawn(self.clone().switch_after_exhaustion(turn.agent_idx, turn.started_at));
      return;
    }
    if std::mem::take(&mut self.core.lock().reauth_due) {
      tokio::spawn(self.clone().reauth_after_expiry(turn.agent_idx, turn.started_at));
      return;
    }
    // The queue flushes once the new connection is ready
    if std::mem::take(&mut self.core.lock().reconnect_after_turn) {
      let me = self.clone();
      tokio::spawn(async move {
        if let Err(e) = me.reconnect().await {
          me.log(&format!("reconnect after the turn failed: {e}"));
        }
      });
      return;
    }
    // The gate flushes the queue (or sends its findings back) once it has decided
    if !auto && !compacting && settled.stop == TurnStop::EndTurn && self.hooks_after_turn() {
      return;
    }
    // A plugin may try a failed turn again; it lets the session go on otherwise
    if !auto && !compacting && self.plugin_retry(turn.agent_idx, turn.started_at, auto) {
      return;
    }
    self.after_prompt(auto, settled.stop);
  }

  pub(crate) fn settle(&self, c: &mut Core, stop: TurnStop, error: Option<TurnError>) {
    // Whatever settles the turn of a late reply still being written (`late_reply.rs`) ends its hold with it
    if std::mem::take(&mut c.late.open) {
      c.peer.detached = false;
    }
    if let Some(f) = c.usage.finish_refresh.take() {
      let _ = f.send(false);
    }
    c.usage.clear_timer();
    c.perms.epoch += 1;
    if let Some(comp) = c.compaction.completion.as_mut() {
      comp.close();
    }
    c.compaction.completion = None;
    // Steers held back missed this turn: the queue sends them as ordinary follow-ups
    c.queue.held_steers.clear();
    // The turn's steers are its own from here on: a later stop has none of them to give back
    c.queue.steered.clear();
    // A steer's own prompt the turn never took up: the entry goes out as the next prompt (`steer_by_prompt`)
    if let Some(s) = c.queue.prompt_steer.take() {
      s.finish(PromptSteerEnd::Missed);
    }
    // The parent prompt returned: children still reported running are disconnected, never failed. A stopped or failed
    // turn keeps the cut: the interrupt may have killed them with no word on the wire
    let native_outlive = error.is_none() && stop == TurnStop::EndTurn && self.vendor.children_outlive_prompt();
    // The agent's own sidekick (Devin Fusion) was cut with the turn; one whose turn ended is handed back by `settle`
    if error.is_some() || stop == TurnStop::Cancelled {
      c.tree.native_rounds_end(acpira_shared::subagents::SubagentState::Cancelled);
    }
    c.tree.settle("prompt-returned", native_outlive);
    self.drain_terminal(c);
    let failed = error.is_some();
    let ended = !failed && stop == TurnStop::EndTurn;
    self.emit_turn_ended(c, stop, error.as_ref());
    // A command Antigravity sent to the background, or a droid `Task` still running its child, keeps its row past the
    // turn (`late_reply.rs`)
    if ended {
      self.park_background_commands(c);
    }
    match error {
      Some(e) => fail_turn(&mut c.state, e),
      None => end_turn(&mut c.state, stop),
    }
    self.plugin_settled(c, true);
    c.goal_steered = None;
    // Claude's goal holds the turn until it is met, so a turn that ended on its own met it (`goal::met_by_turn_end`)
    if ended
      && self.vendor.goal_met_at_turn_end()
      && let Some(met) = goal::met_by_turn_end(c.state.goal.as_ref())
    {
      apply_goal(&mut c.state, Some(met), true);
    }
    // A met goal shows on the dock only until the turn that met it ends; a later prompt must not bring it back
    if c.state.goal.as_ref().is_some_and(|g| g.status == GoalStatus::Complete) {
      c.state.goal = None;
    }
    // An agent that leaves continuing its goal to the client is asked once nothing of the user's waits (`goal_wake`),
    // after a turn that ended on its own with the goal still active: a stop, a failure or a refusal ends the run there
    c.goal_due = !failed
      && matches!(stop, TurnStop::EndTurn | TurnStop::MaxTokens | TurnStop::MaxTurnRequests)
      && c.state.goal.as_ref().is_some_and(|g| g.status == GoalStatus::Active)
      && c.proc.as_ref().is_some_and(|p| goal::continue_prompt(&p.init).is_some());
    self.cancel_turn_cards(c);
    c.phase.running = false;
    // A steer's detached peer turn continues this one: it keeps running until the peer reports its thread idle
    if c.peer.detached && !failed {
      self.reopen_for_detached(c);
    }
  }

  /// The user's stop (and a send-now's): steers the agent will drop unanswered go back to the queue
  pub async fn cancel(self: &Arc<Self>) {
    self.cancel_turn(true).await
  }

  /// A stop for a handover to another engine: the queue stays behind, so the steers stay where the transcript has them
  pub async fn cancel_for_handover(self: &Arc<Self>) {
    self.cancel_turn(false).await
  }

  async fn cancel_turn(self: &Arc<Self>, requeue: bool) {
    // Claude Code's project tools still running on an SSH host stop with the turn
    self.stop_remote_calls();
    let mut c = self.core.lock();
    // Summoned children outlive the tool call that started them, not the user's stop: each gets its own cancel, whatever
    // state the root turn is in
    let summoned = c.tree.relay_running();
    if !summoned.is_empty() {
      self.relay_cancel(&mut c, &summoned);
      self.touch(&mut c);
    }
    // Nothing they finished is delivered after a stop either
    c.relays.void(None);
    let Some(proc) = c.proc.clone() else { return };
    if !c.phase.running {
      return;
    }
    if let Some(f) = c.usage.finish_refresh.take() {
      let _ = f.send(true);
    }
    c.perms.epoch += 1;
    drop(c);
    self.log("cancel");
    let mut c = self.core.lock();
    // Nothing is on the wire yet: just make sure the prompt being staged never goes out
    if c.phase.staging {
      c.phase.staging_aborted = true;
      return;
    }
    self.cancel_all_permissions(&mut c);
    self.cancel_all_questions(&mut c);
    // A turn parked behind a background compaction has no request left on the wire; releasing the latch lets it settle
    if let Some(comp) = c.compaction.completion.as_mut() {
      comp.close();
    }
    // Before the cancel goes out, so no answer to a steer can stream in between
    if requeue {
      self.requeue_unanswered_steers(&mut c);
    }
    if let Some(sid) = c.acp_session_id.clone() {
      proc.notify("session/cancel", json!({ "sessionId": sid }));
    }
    // After the cancel, so Codex takes its host command's end as the stop's and not as a failure to retry
    self.stop_exec_commands();
    // An autonomous cycle has no prompt response to settle it, nor has a late reply's turn: the stop does
    if c.peer.autonomous || c.late.open {
      // The stop is the user's: the watcher that followed the late reply retires, a replay it is reading is dropped,
      // and what the CLI still streams of the reply is not followed
      if c.late.open {
        c.late.watch += 1;
        c.late.stopped = true;
      }
      self.end_detached(&mut c, TurnStop::Cancelled);
      self.touch(&mut c);
    }
  }
}

/// The agent turn a prompt opened, if the transcript still has it where it was put
/// Cut a failure the vendor sends as text off the turn's last text block (the block goes when nothing else is left)
fn take_reply_error(vendor: Vendor, turn: &mut AgentTurn) -> Option<ReplyError> {
  let Some(AgentBlock::Text(x)) = turn.blocks.last_mut() else { return None };
  let (at, e) = vendor.reply_error(&x.markdown)?;
  let kept = x.markdown.get(..at)?.trim_end().to_owned();
  if kept.is_empty() {
    turn.blocks.pop();
  } else {
    x.markdown = kept;
  }
  Some(e)
}

/// Drop the note a vendor appends to the last text block when a turn is cancelled (the block goes when nothing is left)
fn take_cancel_note(vendor: Vendor, turn: &mut AgentTurn) {
  let Some(AgentBlock::Text(x)) = turn.blocks.last_mut() else { return };
  let Some(kept) = vendor.cancel_note(&x.markdown).and_then(|at| x.markdown.get(..at)) else { return };
  let kept = kept.trim_end().to_owned();
  if kept.is_empty() {
    turn.blocks.pop();
  } else {
    x.markdown = kept;
  }
}

impl AcpSession {
  /// A /compact that completed while the agent reported no context (Devin sends neither `usage_update` nor a token
  /// count with it): the snapshot is marked stale so the ring can drop at once, and the next report replaces it.
  /// Agents whose usage is polled get a fresh read right after this, so they are left alone. A silent compaction
  /// (`mark_silent_compaction`) is marked whatever came in during it: agy's usage then is the size the compaction
  /// request itself sent, from before it
  fn mark_compacted_usage(&self, c: &mut Core, turn: &OpenTurn, compact: bool, counted: bool, silent: bool) {
    if !compact || counted || (!silent && (c.usage.revision != turn.usage_before || self.polls_usage(c))) {
      return;
    }
    let done = agent_turn_mut(c, turn.agent_idx, turn.started_at)
      .is_some_and(|t| t.blocks.iter().any(|b| matches!(b, AgentBlock::Compaction(x) if x.status == CompactionStatus::Completed)));
    if let Some(u) = c.state.usage.as_mut().filter(|_| done) {
      u.compacted_from.get_or_insert(u.used);
    }
  }
}

impl AcpSession {
  /// A `/compact` that ended on its own with nothing on the wire (`Vendor::compacted_silently`): the turn gets the
  /// completed compaction row the CLI never sent. Errors and stops never reach here (only an `end_turn` does)
  fn mark_silent_compaction(&self, c: &mut Core, turn: &OpenTurn) -> bool {
    let vendor = self.vendor;
    let Some(t) = agent_turn_mut(c, turn.agent_idx, turn.started_at) else { return false };
    if t.error.is_some() || !vendor.compacted_silently(&t.blocks) {
      return false;
    }
    // Nothing but empty words: the row stands in for them
    t.blocks.clear();
    let id = format!("{SILENT_COMPACTION}{}", turn.started_at);
    t.blocks.push(AgentBlock::Compaction(CompactionBlock { id, status: CompactionStatus::Completed, error: None }));
    self.log("/compact ended with nothing on the wire: counted as a finished compaction");
    true
  }
}

/// Id prefix of the compaction row a silent `/compact` gets (`mark_silent_compaction`)
const SILENT_COMPACTION: &str = "silent-compaction-";

pub(crate) fn agent_turn_mut(c: &mut Core, idx: usize, started_at: i64) -> Option<&mut AgentTurn> {
  match c.state.turns.get_mut(idx) {
    Some(Turn::Agent(a)) if a.started_at == Some(started_at) => Some(a),
    _ => None,
  }
}

/// Fields `u` reports replace the ones `into` holds; the rest stay
pub(crate) fn merge_usage(into: &mut TurnUsage, u: TurnUsage) {
  macro_rules! take {
    ($($f:ident),*) => { $( if u.$f.is_some() { into.$f = u.$f; } )* };
  }
  take!(input, output, cached_read, cached_write, reasoning, total, model_calls, decode_ms, model, request_id, context);
}

/// First line of the text, or what was attached when there is no text
pub(crate) fn summarize_prompt(text: &str, attachments: &[Attachment]) -> String {
  let first = text.trim().split('\n').next().unwrap_or("").trim();
  if first.is_empty() { crate::acp::session::attachments::describe_drafts(attachments) } else { first.to_owned() }
}
