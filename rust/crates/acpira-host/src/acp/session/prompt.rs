//! The turn itself: prompt, settle and cancel. The follow-up queue, usage snapshots, update routing, compaction policy,
//! the automatic account switch and plan execution are further `impl AcpSession` blocks in sibling modules

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::slash_commands::{command_changes, command_name, named_command};
use acpira_shared::transcript::*;
use acpira_shared::turn_errors::is_context_length_error;
use acpira_shared::turn_settings::capture_turn_settings;

use crate::acp::session::attachments::{PreparedPrompt, prepare_prompt, prompt_caps_of};
use crate::acp::session::compaction::{CompactionCompletion, is_compact_command};
use crate::acp::session::edit::{FORK_HISTORY_LEAD, HistoryContext, history_context};
use crate::acp::session::errors::{is_auth, is_session_gone, turn_error_of};
use crate::acp::session::failure::{failure_of, failure_turn_error};
use crate::acp::session::turn_usage::turn_usage_of;
use crate::acp::session::{AcpSession, Core, num};
use crate::acp::transcript::normalize::{activity_of, apply_session_failure, end_turn, fail_turn};
use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::acp::vendors::antigravity::ReplyError;
use crate::acp::vendors::{Vendor, claude_window};
use crate::i18n::{t, tp};
use crate::limits::TITLE_MAX;
use crate::util::{clip, js_num, now_ms, random_uuid};

/// An already staged payload: the queue flush hands its entry over, an edited turn also marks the user turn
#[derive(Clone)]
pub struct Staged {
  pub prepared: PreparedPrompt,
  pub edited: bool,
}

/// Who sent a prompt: the user (or the queue on the user's behalf), the over-threshold /compact, or the continue that
/// follows an automatic account switch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
  User,
  Compact,
  Continue,
}

/// Where `claim` sends a prompt: onto the wire (after an over-budget /compact first), behind the running turn, or nowhere
enum Gate {
  Queue,
  Drop,
  Go { compact_first: bool },
}

/// The staged payload, plus a fork's history context (or why it could not be built)
struct Staging {
  prepared: PreparedPrompt,
  edited_staged: bool,
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
      Gate::Queue => return self.enqueue(text, drafts, staged.map(|s| s.prepared)).await,
      Gate::Drop => {
        if origin == Origin::Continue {
          self.flush_queue();
        }
        return;
      }
      Gate::Go { compact_first } => compact_first,
    };
    let staging = self.stage_turn(&text, &drafts, staged, auto).await;
    let Some(mut accepted) = self.accept(&text, origin, plan_id.as_deref(), staging) else { return };
    if compact_first && !self.compact_before(&accepted.user_turn).await {
      return;
    }
    let turn = self.open_turn(&text, origin, plan_id.as_deref(), &mut accepted);
    let blocks = std::mem::take(&mut accepted.prepared.blocks);
    let result = match &turn.proc {
      Some(p) => p.request("session/prompt", json!({ "sessionId": turn.acp_id, "prompt": blocks })).await,
      None => Err(RpcError::internal("no process")),
    };
    let settled = match result {
      Ok(r) => self.prompt_answered(r, &turn, &accepted, auto).await,
      Err(e) => self.prompt_failed(e, &turn).await,
    };
    if let Some(settled) = settled {
      self.finish_turn(&turn, auto, accepted.compacting, settled);
    }
  }

  /// Where a prompt goes. running is claimed here, before staging, so a second send meanwhile queues
  fn claim(&self, text: &str, drafts: &[Draft], origin: Origin, staged: Option<&Staged>) -> Gate {
    let auto = origin == Origin::Compact;
    let mut c = self.core.lock();
    if origin == Origin::Continue && (c.status != SessionStatus::Ready || c.phase.running) {
      // The switch reserved the session for this prompt; anything else taking it first means the continue is moot
      c.switching = false;
      Gate::Drop
    } else if c.status == SessionStatus::Starting {
      Gate::Queue
    } else if c.status != SessionStatus::Ready
      || (text.trim().is_empty() && drafts.is_empty() && staged.is_none_or(|s| s.prepared.blocks.is_empty()))
    {
      Gate::Drop
    } else if origin != Origin::Continue && (c.switching || c.picks.adopt_pending || c.phase.running || c.peer.detached || (!auto && c.pending_prompt.is_some())) {
      Gate::Queue
    } else {
      c.switching = false;
      c.peer.idle = false;
      // Mid-turn /compact cannot be injected: the next user-facing request is the earliest slot, compact that first
      let compact_first = !auto && !is_compact_command(text) && self.should_auto_compact(&c);
      c.phase.running = true;
      c.compaction.auto_eligible = false;
      c.phase.staging = true;
      c.phase.staging_aborted = false;
      if origin == Origin::User {
        self.bump(&mut c);
      } else {
        self.touch(&mut c);
      }
      Gate::Go { compact_first }
    }
  }

  /// The payload, staged now unless the queue or an edit already did; a fork's first prompt also builds its history
  async fn stage_turn(&self, text: &str, drafts: &[Draft], staged: Option<Staged>, auto: bool) -> Staging {
    let edited_staged = staged.as_ref().is_some_and(|s| s.edited);
    let prepared = match staged {
      Some(s) => s.prepared,
      None => {
        let caps = self.caps(&self.core.lock());
        prepare_prompt(&self.id, text, drafts, &self.deps.blobs, Some(caps)).await
      }
    };
    // A fork's copied transcript has never reached the peer: its first prompt carries it as retained context
    let (proc, caps, turns_copy) = {
      let c = self.core.lock();
      (c.proc.clone(), self.caps(&c), if !auto && c.lineage.history_pending { Some(c.state.turns.clone()) } else { None })
    };
    let mut fork_history = None;
    let mut fork_error = None;
    if let (Some(turns), Some(proc)) = (turns_copy, proc.as_ref()) {
      match history_context(&self.id, &turns, proc, &self.deps.blobs, FORK_HISTORY_LEAD, caps, true).await {
        Ok(h) => fork_history = h,
        Err(e) => {
          self.log(&format!("fork context skipped: {e}"));
          fork_error = Some(e.to_string());
        }
      }
    }
    Staging { prepared, edited_staged, fork_history, fork_error }
  }

  /// Staging is over: the user turn as it will be recorded, or None when a cancel or close came first
  fn accept(self: &Arc<Self>, text: &str, origin: Origin, plan_id: Option<&str>, staging: Staging) -> Option<Accepted> {
    let Staging { mut prepared, edited_staged, fork_history, fork_error } = staging;
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
    if c.lineage.history_pending {
      c.lineage.history_pending = false;
      match &fork_history {
        Some(h) => {
          let mut blocks = h.blocks.clone();
          blocks.append(&mut prepared.blocks);
          prepared.blocks = blocks;
          edited = true;
          if h.omitted > 0 {
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
      Origin::Continue => UserTurn {
        id: Some(random_uuid()),
        text: text.to_owned(),
        settings: Some(before.clone()),
        auto: Some(true),
        auto_reason: Some(AutoReason::AccountSwitch),
        ..Default::default()
      },
      Origin::User => UserTurn {
        id: Some(random_uuid()),
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
    c.lineage.agent_title_muted = c.lineage.forked_from.is_some() || accepted.forked || accepted.edited_staged;
    c.compaction.completion = Some(CompactionCompletion::new(accepted.compacting.then_some(self.agent.as_str())));
    c.turn_failure = None;
    c.state.turns.push(Turn::User(std::mem::take(&mut accepted.user_turn)));
    let untitled = c.state.title.as_deref().is_none_or(|x| x.is_empty() || x == t("session.untitled"));
    if origin == Origin::User && plan_id.is_none() && untitled {
      let summary = summarize_prompt(text, &accepted.prepared.attachments);
      c.state.title = Some(clip(&summary, TITLE_MAX));
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
  async fn prompt_answered(self: &Arc<Self>, r: Value, turn: &OpenTurn, accepted: &Accepted, auto: bool) -> Option<Settled> {
    let (agent_idx, started_at) = (turn.agent_idx, turn.started_at);
    if !turn.live(&self.core.lock()) {
      return None;
    }
    let reason = r.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_owned();
    self.log(&format!("prompt done: {reason}"));
    let mut stop = TurnStop::parse(&reason).unwrap_or(TurnStop::EndTurn);
    let mut exhausted = false;
    let usage = turn_usage_of(&r);
    let log = |line: &str| self.log(line);
    let failure = failure_of(r.get("_meta"), Some(&log));
    {
      let mut c = self.core.lock();
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
      }
      if !auto && accepted.command_name.is_none() && self.wait_for_late_usage(turn.usage_before).await {
        stop = TurnStop::Cancelled;
      }
      if self.status() != SessionStatus::Ready {
        return None;
      }
    }
    self.refresh_context_usage().await;
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
    // Antigravity reports a failed turn as the reply's last text: it becomes the turn's error
    let failed = if stop == TurnStop::EndTurn && !auto {
      agent_turn_mut(&mut c, agent_idx, started_at).and_then(|turn| take_reply_error(self.vendor, turn))
    } else {
      None
    };
    if let Some(e) = failed {
      let error = self.reply_error(&mut c, e);
      self.log(&format!("prompt failed in the reply: {}", error.message));
      self.settle(&mut c, TurnStop::Cancelled, Some(error));
      return Some(Settled { stop: TurnStop::Cancelled, exhausted });
    }
    // Some CLIs acknowledge provider failures as empty end_turn responses: record the missing output
    let empty = agent_turn_mut(&mut c, agent_idx, started_at).is_some_and(|turn| {
      turn.command.is_none() && turn.blocks.iter().all(|b| matches!(b, AgentBlock::Text(x) if x.markdown.trim().is_empty()))
    });
    if stop == TurnStop::EndTurn && !auto && empty {
      drop(c);
      self.log("prompt empty: end_turn without output or error details");
      let mut c = self.core.lock();
      stop = TurnStop::Cancelled;
      self.settle(
        &mut c,
        stop,
        Some(TurnError {
          message: t("host.emptyResponse"),
          kind: Some("empty_response".into()),
          retryable: Some(true),
          ..Default::default()
        }),
      );
    } else {
      self.settle(&mut c, stop, None);
    }
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
    } else if is_session_gone(&err) || !c.proc.as_ref().is_some_and(|p| p.alive()) {
      // Resending over this connection can only fail the same way: the error Notice's Retry reconnects
      c.status = SessionStatus::Error;
      c.error = Some(err.to_string());
    }
    Some(Settled { stop: TurnStop::Cancelled, exhausted })
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
      ReplyError::Other(text) => (text, "agent_error"),
    };
    TurnError { message, kind: Some(kind.into()), retryable: Some(true), ..Default::default() }
  }

  /// After a settled turn: the compaction mark, then either park the queue (context overflow), switch accounts
  /// (quota exhausted) or compact / flush as usual
  fn finish_turn(self: &Arc<Self>, turn: &OpenTurn, auto: bool, compacting: bool, settled: Settled) {
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
    self.after_prompt(auto, settled.stop);
  }

  pub(crate) fn settle(&self, c: &mut Core, stop: TurnStop, error: Option<TurnError>) {
    if let Some(f) = c.usage.finish_refresh.take() {
      let _ = f.send(false);
    }
    c.usage.clear_timer();
    c.perms.epoch += 1;
    if let Some(comp) = c.compaction.completion.as_mut() {
      comp.close();
    }
    c.compaction.completion = None;
    // The parent prompt returned: children still reported running are disconnected, never failed
    c.tree.settle("prompt-returned");
    self.drain_terminal(c);
    let failed = error.is_some();
    match error {
      Some(e) => fail_turn(&mut c.state, e),
      None => end_turn(&mut c.state, stop),
    }
    self.cancel_all_permissions(c);
    self.cancel_all_questions(c);
    c.phase.running = false;
    // A steer's detached peer turn continues this one: it keeps running until the peer reports its thread idle
    if c.peer.detached && !failed {
      self.reopen_for_detached(c);
    }
  }

  pub async fn cancel(self: &Arc<Self>) {
    let mut c = self.core.lock();
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
    if let Some(sid) = c.acp_session_id.clone() {
      proc.notify("session/cancel", json!({ "sessionId": sid }));
    }
  }
}

/// The agent turn a prompt opened, if the transcript still has it where it was put
/// Cut a failure the vendor sends as text off the turn's last text block (the block goes when nothing else is left)
fn take_reply_error(vendor: Vendor, turn: &mut AgentTurn) -> Option<ReplyError> {
  let Some(AgentBlock::Text(x)) = turn.blocks.last_mut() else { return None };
  let (at, e) = vendor.reply_error(&x.markdown)?;
  let kept = x.markdown[..at].trim_end().to_owned();
  if kept.is_empty() {
    turn.blocks.pop();
  } else {
    x.markdown = kept;
  }
  Some(e)
}

pub(crate) fn agent_turn_mut(c: &mut Core, idx: usize, started_at: i64) -> Option<&mut AgentTurn> {
  match c.state.turns.get_mut(idx) {
    Some(Turn::Agent(a)) if a.started_at == Some(started_at) => Some(a),
    _ => None,
  }
}

fn merge_usage(into: &mut TurnUsage, u: TurnUsage) {
  macro_rules! take {
    ($($f:ident),*) => { $( if u.$f.is_some() { into.$f = u.$f; } )* };
  }
  take!(input, output, cached_read, cached_write, reasoning, total, model_calls, model, request_id, context);
}

/// First line of the text, or what was attached when there is no text
pub(crate) fn summarize_prompt(text: &str, attachments: &[Attachment]) -> String {
  let first = text.trim().split('\n').next().unwrap_or("").trim();
  if first.is_empty() { crate::acp::session::attachments::describe_drafts(attachments) } else { first.to_owned() }
}
