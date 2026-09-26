//! The turn lifecycle of a session: prompt, settle, cancel, the follow-up queue and update routing

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::plan_execution::plan_execution_prompt;
use acpira_shared::slash_commands::{command_changes, command_name, named_command};
use acpira_shared::transcript::*;
use acpira_shared::turn_errors::is_context_length_error;
use acpira_shared::turn_settings::capture_turn_settings;

use super::attachments::{PreparedPrompt, prepare_prompt, prompt_caps_of, restore_drafts};
use super::compaction::{CompactionCompletion, is_compact_command};
use super::normalize::{activity_of, apply_async_task, apply_session_failure, apply_update, end_turn, fail_turn, set_stop_requested};
use super::pi_usage;
use super::plans::{capture_plan, plan_documents, plan_documents_mut};
use super::rpc::{BoxFuture, RpcError};
use super::session::{AcpSession, Core, QueuedEntry, USAGE_POLL_INTERVAL, clear_usage_timer, num};
use super::session_edit::{FORK_HISTORY_LEAD, history_context};
use super::session_errors::{is_auth, is_session_gone, turn_error_of};
use super::session_failure::{failure_of, failure_turn_error};
use super::subagent_tree::{Route, RouteCtx};
use super::turn_usage::turn_usage_of;
use super::wire::{ExtensionUpdate, extension_of};
use crate::i18n::{t, tp};
use crate::limits::TITLE_MAX;
use crate::util::{clip, now_ms, random_uuid};

/// An already staged payload: the queue flush hands its entry over, an edited turn also marks the user turn
#[derive(Clone)]
pub struct Staged {
  pub prepared: PreparedPrompt,
  pub edited: bool,
}

const RUNNING_KINDS: [&str; 6] =
  ["user_message_chunk", "agent_message_chunk", "agent_thought_chunk", "tool_call", "tool_call_update", "plan"];

impl AcpSession {
  pub(crate) fn caps(&self, c: &Core) -> super::attachments::PromptCaps {
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
    Box::pin(async move { me.prompt_inner(text, drafts, auto, staged, plan_id).await })
  }

  async fn prompt_inner(self: Arc<Self>, text: String, drafts: Vec<Draft>, auto: bool, staged: Option<Staged>, plan_id: Option<String>) {
    enum Gate {
      Queue,
      Drop,
      Go(bool),
    }
    let gate = {
      let mut c = self.core.lock();
      if c.status == SessionStatus::Starting {
        Gate::Queue
      } else if c.status != SessionStatus::Ready
        || (text.trim().is_empty() && drafts.is_empty() && staged.as_ref().is_none_or(|s| s.prepared.blocks.is_empty()))
      {
        Gate::Drop
      } else if c.phase.running || (!auto && c.pending_prompt.is_some()) {
        Gate::Queue
      } else {
        // Mid-turn /compact cannot be injected: the next user-facing request is the earliest slot, compact that first
        let compact_first = !auto && !is_compact_command(&text) && self.should_auto_compact(&c);
        c.phase.running = true;
        c.auto_compact_eligible = false;
        c.phase.staging = true;
        c.phase.staging_aborted = false;
        if auto {
          self.touch(&mut c);
        } else {
          self.bump(&mut c);
        }
        Gate::Go(compact_first)
      }
    };
    let compact_first = match gate {
      Gate::Queue => {
        self.enqueue(text, drafts, staged.map(|s| s.prepared)).await;
        return;
      }
      Gate::Drop => return,
      Gate::Go(first) => first,
    };
    let edited_staged = staged.as_ref().is_some_and(|s| s.edited);
    let mut prepared = match staged {
      Some(s) => s.prepared,
      None => {
        let caps = self.caps(&self.core.lock());
        prepare_prompt(&self.id, &text, &drafts, &self.deps.blobs, Some(caps)).await
      }
    };
    let mut edited = edited_staged;
    // A fork's copied transcript has never reached the peer: its first prompt carries it as retained context
    let (history_pending, proc, caps, turns_copy) = {
      let c = self.core.lock();
      (c.history_pending, c.proc.clone(), self.caps(&c), if !auto && c.history_pending { Some(c.state.turns.clone()) } else { None })
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
    let _ = history_pending;
    let (user_turn, compacting, name, before) = {
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
        return;
      }
      if c.history_pending {
        c.history_pending = false;
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
      let command = named_command(&c.state.commands, &text).map(|x| x.name.clone());
      let name = command_name(&text).map(str::to_owned);
      let user_turn = if auto {
        UserTurn { text: text.clone(), auto: Some(true), ..Default::default() }
      } else {
        UserTurn {
          id: Some(random_uuid()),
          text: text.clone(),
          settings: Some(before.clone()),
          command,
          edited,
          plan_id: plan_id.clone(),
          attachments: (!prepared.attachments.is_empty()).then(|| prepared.attachments.clone()),
          ..Default::default()
        }
      };
      (user_turn, is_compact_command(&text), name, before)
    };
    if compact_first {
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
        c.state.turns.push(Turn::User(user_turn));
        self.touch(&mut c);
        return;
      }
      c.phase.running = true;
    }
    let (proc, acp_id, prompt_gen, usage_before, agent_idx, started_at) = {
      let mut c = self.core.lock();
      c.agent_title_muted = c.forked_from.is_some() || fork_history.is_some() || edited_staged;
      c.completion = Some(CompactionCompletion::new(compacting.then_some(self.agent.as_str())));
      c.turn_failure = None;
      c.state.turns.push(Turn::User(user_turn));
      let untitled = c.state.title.as_deref().is_none_or(|x| x.is_empty() || x == t("session.untitled"));
      if !auto && plan_id.is_none() && untitled {
        let summary = summarize_prompt(&text, &prepared.attachments);
        c.state.title = Some(clip(&summary, TITLE_MAX));
      }
      let started_at = now_ms();
      let activity = activity_of(&c.state.turns);
      c.state.turns.push(Turn::Agent(AgentTurn {
        started_at: Some(started_at),
        activity,
        command: name.clone().map(|n| CommandReceipt { name: n, mode: None, options: None }),
        ..Default::default()
      }));
      let agent_idx = c.state.turns.len() - 1;
      self.touch(&mut c);
      self.schedule_usage_poll(&mut c);
      (c.proc.clone(), c.acp_session_id.clone(), c.proc_gen, c.usage_revision, agent_idx, started_at)
    };
    let live = |c: &Core| c.proc_gen == prompt_gen && c.status != SessionStatus::Closed;
    let mut stop;
    let blocks = std::mem::take(&mut prepared.blocks);
    let result = match &proc {
      Some(p) => p.request("session/prompt", json!({ "sessionId": acp_id, "prompt": blocks })).await,
      None => Err(RpcError::internal("no process")),
    };
    match result {
      Ok(r) => {
        if !live(&self.core.lock()) {
          return;
        }
        let reason = r.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_owned();
        self.log(&format!("prompt done: {reason}"));
        stop = TurnStop::parse(&reason).unwrap_or(TurnStop::EndTurn);
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
          self.settle(&mut c, TurnStop::Cancelled, Some(failure_turn_error(f)));
          if f.actions.contains(&FailureAction::Login) {
            c.status = SessionStatus::AuthRequired;
          }
        } else {
          if let Some(f) = &failure {
            apply_session_failure(&mut self.core.lock().state, f);
          }
          if stop == TurnStop::EndTurn {
            let pending = self.core.lock().completion.as_mut().and_then(CompactionCompletion::wait);
            if let Some(rx) = pending {
              self.log("waiting for compaction completion");
              let _ = rx.await;
            }
            if self.status() != SessionStatus::Ready {
              self.flush_queue();
              return;
            }
            {
              let mut c = self.core.lock();
              let after = c.completion.as_ref().and_then(|x| x.tokens_after);
              if (auto || compacting)
                && let (Some(after), Some(u)) = (after, c.state.usage.as_mut())
              {
                u.used = num(after);
              }
            }
            if !auto && name.is_none() && self.wait_for_kimi_usage(usage_before).await {
              stop = TurnStop::Cancelled;
            }
            if self.status() != SessionStatus::Ready {
              return;
            }
          }
          self.refresh_context_usage().await;
          let mut c = self.core.lock();
          if !live(&c) {
            return;
          }
          let controls = c.state.controls.clone();
          if stop == TurnStop::EndTurn
            && let Some(turn) = agent_turn_mut(&mut c, agent_idx, started_at)
            && let Some(cmd) = turn.command.as_mut()
          {
            let (mode, options) = command_changes(&before, &controls);
            cmd.mode = mode;
            cmd.options = options;
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
        }
      }
      Err(e) => {
        // Disposal already settled and persisted the interrupted turn
        if !live(&self.core.lock()) {
          return;
        }
        stop = TurnStop::Cancelled;
        self.log(&format!("prompt failed: {e}"));
        self.refresh_context_usage().await;
        let mut c = self.core.lock();
        if !live(&c) {
          return;
        }
        let code = e.code;
        let err = anyhow::Error::new(e);
        let failure = c.turn_failure.clone();
        let turn_error = match &failure {
          Some(f) => TurnError { code: Some(code), ..failure_turn_error(f) },
          None => turn_error_of(&err),
        };
        self.settle(&mut c, TurnStop::Cancelled, Some(turn_error));
        if is_auth(&err) || failure.as_ref().is_some_and(|f| f.actions.contains(&FailureAction::Login)) {
          c.status = SessionStatus::AuthRequired;
        } else if is_session_gone(&err) || !c.proc.as_ref().is_some_and(|p| p.alive()) {
          // Resending over this connection can only fail the same way: the error Notice's Retry reconnects
          c.status = SessionStatus::Error;
          c.error = Some(err.to_string());
        }
      }
    }
    let context_error = {
      let mut c = self.core.lock();
      if auto || compacting {
        c.compacted_at = Some(c.state.usage.map(|u| u.used.0).unwrap_or(0.0));
      }
      c.auto_compact_eligible = !auto && !compacting && stop == TurnStop::EndTurn;
      self.touch(&mut c);
      agent_turn_mut(&mut c, agent_idx, started_at).is_some_and(|t| is_context_length_error(t.error.as_ref()))
    };
    // Queued messages stay parked until the context is compacted or the input changes
    if context_error {
      return;
    }
    self.after_prompt(auto, stop);
  }

  fn log_usage_threshold(&self, c: &Core, what: &str) {
    let used = c.state.usage.map(|u| js_num(u.used.0)).unwrap_or_else(|| "undefined".into());
    self.log(&format!("usage {used} ≥ threshold, {what}"));
  }

  pub(crate) fn settle(&self, c: &mut Core, stop: TurnStop, error: Option<TurnError>) {
    if let Some(f) = c.finish_usage_refresh.take() {
      let _ = f.send(false);
    }
    clear_usage_timer(c);
    c.perm_epoch += 1;
    if let Some(comp) = c.completion.as_mut() {
      comp.close();
    }
    c.completion = None;
    // The parent prompt returned: children still reported running are disconnected, never failed
    c.tree.settle("prompt-returned");
    self.drain_terminal(c);
    match error {
      Some(e) => fail_turn(&mut c.state, e),
      None => end_turn(&mut c.state, stop),
    }
    self.cancel_all_permissions(c);
    self.cancel_all_questions(c);
    c.phase.running = false;
  }

  pub async fn cancel(self: &Arc<Self>) {
    let mut c = self.core.lock();
    let Some(proc) = c.proc.clone() else { return };
    if !c.phase.running {
      return;
    }
    if let Some(f) = c.finish_usage_refresh.take() {
      let _ = f.send(true);
    }
    c.perm_epoch += 1;
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
    if let Some(comp) = c.completion.as_mut() {
      comp.close();
    }
    if let Some(sid) = c.acp_session_id.clone() {
      proc.notify("session/cancel", json!({ "sessionId": sid }));
    }
  }

  // Prompt queue

  async fn stage(&self, text: &str, drafts: &[Draft]) -> PreparedPrompt {
    let caps = self.caps(&self.core.lock());
    let prepared = prepare_prompt(&self.id, text, drafts, &self.deps.blobs, Some(caps)).await;
    for p in &prepared.problems {
      self.log(p);
      self.notify(p);
    }
    PreparedPrompt { problems: vec![], ..prepared }
  }

  /// Queue a prompt behind the running turn (or while starting); staged now so the row can show attachments
  pub(crate) async fn enqueue(self: &Arc<Self>, text: String, drafts: Vec<Draft>, staged: Option<PreparedPrompt>) {
    let prepared = match staged {
      Some(p) => p,
      None => self.stage(&text, &drafts).await,
    };
    let flush = {
      let mut c = self.core.lock();
      if prepared.blocks.is_empty() || !matches!(c.status, SessionStatus::Ready | SessionStatus::Starting) {
        return;
      }
      c.queue.push(QueuedEntry { id: random_uuid(), text, prepared });
      self.bump(&mut c);
      !(c.phase.running || c.pending_prompt.is_some())
    };
    if flush {
      self.flush_queue();
    }
  }

  /// Send the first queued prompt, if any; nobody awaits it
  pub(crate) fn flush_queue(self: &Arc<Self>) -> bool {
    let next = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || c.phase.running || c.pending_prompt.is_some() || c.queue.is_empty() {
        return false;
      }
      c.sending_id = None;
      c.queue.remove(0)
    };
    let me = self.clone();
    crate::util::run_prefix(me.prompt(next.text, vec![], false, Some(Staged { prepared: next.prepared, edited: false }), None));
    true
  }

  pub fn dequeue(&self, id: &str) {
    let mut c = self.core.lock();
    if c.sending_id.as_deref() == Some(id) {
      return;
    }
    let before = c.queue.len();
    c.queue.retain(|q| q.id != id);
    if c.queue.len() != before {
      self.touch(&mut c);
    }
  }

  /// Reserve the selected entry before cancelling the active turn
  pub async fn send_queued(self: &Arc<Self>, id: &str) -> Result<()> {
    let running = {
      let mut c = self.core.lock();
      if c.sending_id.is_some() || c.status != SessionStatus::Ready {
        return Ok(());
      }
      let Some(i) = c.queue.iter().position(|q| q.id == id) else { return Ok(()) };
      let entry = c.queue.remove(i);
      c.queue.insert(0, entry);
      c.sending_id = Some(id.to_owned());
      self.touch(&mut c);
      c.phase.running
    };
    // cancel is only a notification; prompt completion owns the next flush, so prompts never overlap
    if running {
      self.cancel().await
    } else {
      self.flush_queue();
    }
    Ok(())
  }

  /// Replace a queued prompt in place: kept attachments come back from their blobs, new drafts are staged alongside
  pub async fn edit_queued(self: &Arc<Self>, id: &str, text: String, retained: Vec<i64>, drafts: Vec<Draft>) -> Result<()> {
    let kept = {
      let c = self.core.lock();
      if c.sending_id.as_deref() == Some(id) {
        return Ok(());
      }
      let entry = c.queue.iter().find(|q| q.id == id).ok_or_else(|| anyhow!(t("queue.gone")))?;
      retained.iter().filter_map(|i| usize::try_from(*i).ok().and_then(|i| entry.prepared.attachments.get(i)).cloned()).collect::<Vec<_>>()
    };
    let mut all = restore_drafts(&self.id, &kept, &self.deps.blobs).await?;
    all.extend(drafts);
    let prepared = self.stage(&text, &all).await;
    let mut c = self.core.lock();
    if c.sending_id.as_deref() == Some(id) {
      return Err(anyhow!(t("queue.gone")));
    }
    let Some(i) = c.queue.iter().position(|q| q.id == id) else { return Err(anyhow!(t("queue.gone"))) };
    let has_content = prepared.blocks.iter().any(|b| {
      b.get("type").and_then(Value::as_str) != Some("text") || b.get("text").and_then(Value::as_str).is_some_and(|x| !x.trim().is_empty())
    });
    if !has_content {
      c.queue.remove(i);
      self.touch(&mut c);
      return Ok(());
    }
    c.queue[i].text = text;
    c.queue[i].prepared = prepared;
    self.touch(&mut c);
    Ok(())
  }

  // Compaction

  /// ACP has no dedicated compaction request: send the agent's own /compact
  pub fn compact(self: &Arc<Self>, auto: bool) -> BoxFuture<Result<()>> {
    let me = self.clone();
    Box::pin(async move {
      let can = Self::can_compact_of(&me.core.lock());
      if !can {
        return if auto { Ok(()) } else { Err(anyhow!(t("host.noCompact"))) };
      }
      me.prompt("/compact".into(), vec![], auto, None, None).await;
      Ok(())
    })
  }

  pub(crate) fn should_auto_compact(&self, c: &Core) -> bool {
    let Some(policy) = self.deps.compaction.as_ref().map(|f| f()) else { return false };
    let used = c.state.usage.map(|u| u.used.0).unwrap_or(0.0);
    if !policy.auto || used == 0.0 || !Self::can_compact_of(c) || c.status != SessionStatus::Ready || used < policy.at_tokens {
      return false;
    }
    c.compacted_at.is_none_or(|at| used >= at + policy.at_tokens / 10.0)
  }

  /// Compact before flushing so a queued follow-up is not the request that runs over budget
  pub(crate) fn after_prompt(self: &Arc<Self>, auto: bool, stop: TurnStop) {
    let compact = {
      let c = self.core.lock();
      if c.pending_prompt.is_some() {
        return;
      }
      let yes = !auto && stop == TurnStop::EndTurn && self.should_auto_compact(&c);
      if yes {
        self.log_usage_threshold(&c, "auto /compact");
      }
      yes
    };
    if compact {
      let me = self.clone();
      tokio::spawn(async move {
        if let Err(e) = me.compact(true).await {
          me.log(&format!("auto /compact failed: {e}"));
          me.flush_queue();
        }
      });
      return;
    }
    self.flush_queue();
  }

  // Usage snapshots

  /// Agents that never send `usage_update` and have their context snapshot polled instead: Grok over
  /// `_x.ai/session/info`, Pi from its session file (`pi_usage`)
  fn polls_usage(&self, c: &Core) -> bool {
    !c.usage_notifications && (self.agent == "pi" || (self.agent == "grok" && !c.grok_usage_unavailable))
  }

  /// Refresh before settling a turn so auto-compaction sees the current window; Grok only fills context.used after
  /// a model round and Pi's file grows per reply, so both are polled while a prompt is on the wire
  pub(crate) async fn refresh_context_usage(self: &Arc<Self>) {
    clear_usage_timer(&mut self.core.lock());
    let _serial = self.usage_lock.lock().await;
    self.core.lock().usage_inflight = true;
    match self.agent.as_str() {
      "grok" => self.read_grok_usage().await,
      "pi" => self.read_pi_usage().await,
      _ => {}
    }
    self.core.lock().usage_inflight = false;
  }

  async fn read_pi_usage(self: &Arc<Self>) {
    let (sid, revision, stamp) = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || !self.polls_usage(&c) {
        return;
      }
      let Some(sid) = c.acp_session_id.clone() else { return };
      c.usage_revision += 1;
      (sid, c.usage_revision, c.pi_stamp.clone())
    };
    let (cwd, id) = (self.cwd.clone(), sid.clone());
    let Ok(read) = tokio::task::spawn_blocking(move || pi_usage::read(&cwd, &id, stamp.as_ref())).await else { return };
    let pi_usage::Snapshot::Fresh(stamp, usage) = read else { return };
    let mut c = self.core.lock();
    if c.acp_session_id.as_deref() != Some(sid.as_str()) || c.status != SessionStatus::Ready || c.usage_revision != revision {
      return;
    }
    c.pi_stamp = Some(stamp);
    self.apply_context_usage(&mut c, usage);
  }

  async fn read_grok_usage(self: &Arc<Self>) {
    let (proc, sid, revision) = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || !self.polls_usage(&c) {
        return;
      }
      let (Some(proc), Some(sid)) = (c.proc.clone(), c.acp_session_id.clone()) else { return };
      c.usage_revision += 1;
      (proc, sid, c.usage_revision)
    };
    let r = tokio::time::timeout(Duration::from_secs(5), proc.request("_x.ai/session/info", json!({ "sessionId": sid }))).await;
    let usage = match r {
      Ok(Ok(v)) => grok_context_usage(&v, &sid),
      Ok(Err(e)) => {
        if e.code == -32601 {
          self.core.lock().grok_usage_unavailable = true;
        }
        self.log(&format!("context unavailable: {e}"));
        return;
      }
      Err(_) => {
        self.log("context unavailable: Grok context request timed out");
        return;
      }
    };
    let mut c = self.core.lock();
    if !c.proc.as_ref().is_some_and(|p| Arc::ptr_eq(p, &proc))
      || c.acp_session_id.as_deref() != Some(sid.as_str())
      || c.status != SessionStatus::Ready
      || c.usage_revision != revision
    {
      return;
    }
    self.apply_context_usage(&mut c, usage);
  }

  /// A polled snapshot becomes the session's usage and the context mark of the turn it follows
  fn apply_context_usage(&self, c: &mut Core, usage: Option<Usage>) {
    let prev = c.state.usage;
    c.state.usage = usage;
    if let (Some(u), Some(Turn::Agent(last))) = (usage, c.state.turns.last_mut()) {
      last.usage.get_or_insert_with(Default::default).context = Some(ContextUse { used: u.used, size: u.size });
    }
    if prev.map(|p| (p.used, p.size, p.cost)) != usage.map(|u| (u.used, u.size, u.cost)) {
      self.touch(c);
    }
  }

  pub(crate) fn schedule_usage_poll(&self, c: &mut Core) {
    if !self.polls_usage(c) || !c.phase.running || c.usage_timer.is_some() || c.usage_inflight {
      return;
    }
    let weak = self.me.clone();
    let handle = tokio::spawn(async move {
      tokio::time::sleep(USAGE_POLL_INTERVAL).await;
      let Some(me) = weak.upgrade() else { return };
      me.core.lock().usage_timer = None;
      me.refresh_context_usage().await;
      let mut c = me.core.lock();
      me.schedule_usage_poll(&mut c);
    });
    c.usage_timer = Some(handle.abort_handle());
  }

  /// Kimi emits its context snapshot asynchronously after end_turn: park the queue until it lands (bounded).
  /// Resolves to whether the wait was cancelled
  async fn wait_for_kimi_usage(self: &Arc<Self>, revision: u64) -> bool {
    let rx = {
      let mut c = self.core.lock();
      let auto = self.deps.compaction.as_ref().is_some_and(|f| f().auto);
      if self.agent != "kimi" || !auto || c.usage_revision != revision {
        return false;
      }
      let (tx, rx) = tokio::sync::oneshot::channel();
      c.finish_usage_refresh = Some(tx);
      rx
    };
    match tokio::time::timeout(Duration::from_secs(5), rx).await {
      Ok(Ok(cancelled)) => cancelled,
      Ok(Err(_)) => false,
      Err(_) => {
        self.log("context refresh unavailable after prompt");
        self.core.lock().finish_usage_refresh = None;
        false
      }
    }
  }

  // Update routing

  pub(crate) fn on_update(self: &Arc<Self>, n: Value) {
    let session_id = n.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
    let mut u = n.get("update").cloned().unwrap_or(Value::Null);
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("").to_owned();
    let mut c = self.core.lock();
    if c.phase.editing {
      if matches!(kind.as_str(), "available_commands_update" | "usage_update") {
        c.phase.edit_notifications.push(n);
      }
      return;
    }
    // Extension updates: subagent lifecycle and AIR async tasks
    let log = |line: &str| self.log(line);
    if let Some(ext) = extension_of(&u, &log) {
      match ext {
        ExtensionUpdate::Ignored(k) => self.log(&format!("{k} ignored")),
        ExtensionUpdate::AsyncTask(e) => self.async_task_update(&mut c, &session_id, &e),
        ExtensionUpdate::Lifecycle(l) => {
          let root = c.acp_session_id.clone();
          let turn_index = current_turn_index(&c);
          let Core { tree, state, .. } = &mut *c;
          tree.lifecycle(&session_id, root.as_deref(), l, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
          self.drain_terminal(&mut c);
        }
      }
      self.touch(&mut c);
      return;
    }
    if c.acp_session_id.as_deref().is_some_and(|a| a != session_id) {
      if c.tree.has_peer_session(&session_id) {
        // Replay content is dropped for a node restored from the record
        if !c.replaying || !c.tree.peer_restored(&session_id) {
          let turn_index = current_turn_index(&c);
          let node = c.tree.node_for_peer(&session_id);
          let Core { tree, state, .. } = &mut *c;
          if let Some(node) = node {
            tree.apply_child(&node, &u, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
          }
          self.drain_terminal(&mut c);
        }
        self.touch(&mut c);
      } else {
        c.tree.buffer_orphan(&session_id, u);
      }
      return;
    }
    // Conversation content cannot belong to a session that does not exist yet (pi-acp's banner during session/new)
    if c.acp_session_id.is_none()
      && c.status == SessionStatus::Starting
      && ["agent_message_chunk", "agent_thought_chunk", "tool_call", "tool_call_update", "plan"].contains(&kind.as_str())
    {
      drop(c);
      self.log(&format!("startup {kind} ignored (no session yet)"));
      return;
    }
    if kind == "current_mode_update" && self.def().ignore_modes {
      return;
    }
    // yolo is host-side state: a mode pushed by the CLI must not drag the UI back
    if c.auto_approve && kind == "current_mode_update" {
      u["currentModeId"] = Value::from("yolo");
    }
    if c.replaying && RUNNING_KINDS.contains(&kind.as_str()) {
      return;
    }
    if c.replaying && kind == "session_info_update" && failure_of(u.get("_meta"), None).is_some() {
      return;
    }
    if c.phase.running
      && kind == "session_info_update"
      && let Some(f) = failure_of(u.get("_meta"), None).filter(|f| f.severity == Severity::Error)
    {
      c.turn_failure = Some(f);
    }
    if !c.replaying
      && let Some(comp) = c.completion.as_mut()
    {
      comp.update(&u);
    }
    // A user_message_chunk echoed mid-turn is the one just sent
    if c.phase.running && kind == "user_message_chunk" {
      return;
    }
    if kind == "session_info_update"
      && c.agent_title_muted
      && let Some(title) = u.get("title").and_then(Value::as_str).filter(|x| !x.is_empty())
    {
      let head: String = title.chars().take(60).collect();
      self.log(&format!("agent title ignored: {head}"));
      u["title"] = Value::Null;
    }
    if kind == "agent_message_chunk"
      && let Some(banner) = c.startup_banner.clone()
      && u.get("content").and_then(|x| x.get("type")).and_then(Value::as_str) == Some("text")
      && u["content"].get("text").and_then(Value::as_str) == Some(banner.as_str())
    {
      self.log("startup banner ignored");
      c.startup_banner = None;
      return;
    }
    let turn_index = current_turn_index(&c);
    let routed = {
      let Core { tree, state, .. } = &mut *c;
      tree.route_root(&u, &mut RouteCtx { turn_index, root_turns: &mut state.turns })
    };
    self.drain_terminal(&mut c);
    if matches!(routed, Route::Consumed) {
      self.touch(&mut c);
      return;
    }
    if !apply_update(&mut c.state, &u) {
      return;
    }
    if kind == "usage_update" {
      c.usage_notifications = true;
      c.usage_revision += 1;
      if let Some(f) = c.finish_usage_refresh.take() {
        let _ = f.send(false);
      }
      clear_usage_timer(&mut c);
    } else if c.phase.running {
      self.schedule_usage_poll(&mut c);
    }
    if matches!(kind.as_str(), "tool_call" | "tool_call_update") {
      {
        let Core { tree, state, .. } = &mut *c;
        tree.annotate_root(&u, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      }
      self.drain_terminal(&mut c);
      c.raw_questions.remember(&u);
      let plan = capture_plan(&mut c.state.turns, &u);
      // Kimi 0.41.0 confirms the plan exit in tool output but omits current_mode_update
      let tool_call_id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("");
      if self.agent == "kimi"
        && let Some(pid) = plan
        && plan_documents(&c.state.turns).iter().any(|p| p.id == pid && p.approval_tool_call_id.as_deref() == Some(tool_call_id))
        && u.get("status").and_then(Value::as_str) == Some("completed")
        && u.get("rawOutput").and_then(Value::as_str).is_some_and(|o| o.starts_with("Exited plan mode. Plan mode deactivated."))
      {
        c.state.controls.mode_id = Some("default".into());
      }
    }
    if c.phase.running {
      let activity = activity_of(&c.state.turns);
      if let Some(Turn::Agent(last)) = c.state.turns.last_mut() {
        last.activity = activity;
      }
    }
    self.touch(&mut c);
    // Kimi reports usage after the prompt response: re-evaluate only the live, successfully completed user turn
    let reevaluate =
      !c.replaying && !c.phase.running && c.auto_compact_eligible && matches!(kind.as_str(), "usage_update" | "available_commands_update");
    drop(c);
    if reevaluate {
      self.after_prompt(false, TurnStop::EndTurn);
    }
  }

  /// An AIR async task lands on the transcript of the session whose stream carried it
  fn async_task_update(&self, c: &mut Core, peer: &str, e: &super::wire::AsyncTaskEvent) {
    if c.replaying {
      return;
    }
    let root = c.acp_session_id.as_deref() == Some(peer);
    c.task_peer.insert(e.async_task_id.clone(), peer.to_owned());
    if root {
      apply_async_task(&mut c.state, e);
      return;
    }
    match c.tree.task_state(peer) {
      Some((state, node)) => {
        if apply_async_task(state, e) {
          c.tree.bump(&node);
        }
      }
      None => {
        c.task_peer.remove(&e.async_task_id);
        self.log(&format!("async task {} on unknown session {peer} dropped", e.async_task_id));
      }
    }
  }

  /// Ask the adapter to stop one background task on the session that owns it; stopRequested is only optimistic
  pub async fn stop_async_task(self: &Arc<Self>, task_id: &str) -> Result<()> {
    let (proc, peer) = {
      let mut c = self.core.lock();
      let peer = c.task_peer.get(task_id).cloned();
      let root = peer.is_some() && peer == c.acp_session_id;
      let info = match &peer {
        Some(p) if root => {
          let _ = p;
          c.state.tasks.tasks.get(task_id).cloned()
        }
        Some(p) => c.tree.task_state(p).and_then(|(st, _)| st.tasks.tasks.get(task_id).cloned()),
        None => None,
      };
      let (Some(info), Some(peer)) = (info, peer) else {
        drop(c);
        self.log(&format!("stopAsyncTask {task_id}: unknown task"));
        return Ok(());
      };
      if info.stop_requested || !matches!(info.state, AsyncTaskState::Running | AsyncTaskState::Paused) {
        return Ok(());
      }
      if !info.can_stop {
        drop(c);
        self.log(&format!("stopAsyncTask {task_id}: the adapter says it cannot be stopped"));
        return Ok(());
      }
      let Some(proc) = c.proc.clone().filter(|p| p.alive()) else { return Ok(()) };
      self.mark_stop(&mut c, &peer, task_id, true);
      self.touch(&mut c);
      (proc, peer)
    };
    let r = proc.request("_session/async_task/stop", json!({ "sessionId": peer, "asyncTaskId": task_id })).await;
    let refused = matches!(&r, Ok(v) if v.get("stopped") == Some(&Value::Bool(false)));
    if r.is_err() || refused {
      let mut c = self.core.lock();
      self.mark_stop(&mut c, &peer, task_id, false);
      self.touch(&mut c);
    }
    match r {
      Err(e) => Err(anyhow::Error::new(e)),
      Ok(_) if refused => Err(anyhow!(t("asyncTask.stopRefused"))),
      Ok(_) => Ok(()),
    }
  }

  fn mark_stop(&self, c: &mut Core, peer: &str, task_id: &str, on: bool) {
    if c.acp_session_id.as_deref() == Some(peer) {
      set_stop_requested(&mut c.state, task_id, on);
    } else if let Some((st, node)) = c.tree.task_state(peer) {
      set_stop_requested(st, task_id, on);
      c.tree.bump(&node);
    }
  }

  /// Ask the agent to cancel one delegated child; its pending cards (and its descendants') close first
  pub async fn cancel_subagent(self: &Arc<Self>, id: &str) {
    let (proc, peer) = {
      let mut c = self.core.lock();
      let Some(peer) = c.tree.cancel(id) else { return };
      let mut ids = vec![id.to_owned()];
      ids.extend(c.tree.descendants(id));
      for node in ids {
        self.cancel_permissions_for(&mut c, &node);
        self.cancel_questions_for(&mut c, &node);
      }
      self.touch(&mut c);
      (c.proc.clone(), peer)
    };
    if let Some(p) = proc {
      p.notify("session/cancel", json!({ "sessionId": peer }));
    }
  }

  /// Apply the selected execution model before releasing approval or dispatching an implementation turn
  pub async fn build_plan(self: &Arc<Self>, plan_id: &str, model: Option<(String, String)>, option_id: Option<String>) -> Result<()> {
    let (plan, permission, option) = {
      let c = self.core.lock();
      if c.building_plan || c.status != SessionStatus::Ready {
        return Ok(());
      }
      let Some(plan) = plan_documents(&c.state.turns).into_iter().find(|p| p.id == plan_id).cloned() else { return Ok(()) };
      if plan.markdown.is_empty() || plan.status == PlanDocStatus::Executing {
        return Ok(());
      }
      let permission = Self::permission_by_plan(&c, plan_id);
      if c.phase.running && permission.is_none() {
        return Ok(());
      }
      // An expired approval click must never become a fresh implementation prompt
      if option_id.is_some() && permission.is_none() {
        return Ok(());
      }
      let kind = |o: &Value| o.get("kind").and_then(Value::as_str).unwrap_or("").to_owned();
      let option = permission.as_ref().and_then(|(_, opts)| match &option_id {
        Some(id) => opts
          .iter()
          .find(|o| {
            o.get("optionId").and_then(Value::as_str) == Some(id.as_str())
              && (kind(o).starts_with("allow") || kind(o).starts_with("reject"))
          })
          .cloned(),
        None => opts.iter().find(|o| kind(o) == "allow_once").cloned(),
      });
      if permission.is_some() && option.is_none() {
        return Err(anyhow!(t("host.planOptionsStale")));
      }
      (plan, permission, option)
    };
    self.core.lock().building_plan = true;
    let result: Result<()> = async {
      let option_id = option.as_ref().and_then(|o| o.get("optionId").and_then(Value::as_str)).map(str::to_owned);
      let reject = option.as_ref().and_then(|o| o.get("kind").and_then(Value::as_str)).is_some_and(|k| k.starts_with("reject"));
      if let (Some((block, _)), true) = (&permission, reject) {
        let mut c = self.core.lock();
        if c.perms.iter().any(|p| &p.block_id == block) {
          self.resolve_permission_locked(&mut c, block, option_id.as_deref().unwrap_or(""));
        }
        return Ok(());
      }
      if let Some((config_id, value)) = &model {
        let current = {
          let c = self.core.lock();

          c.state.controls.options.iter().find(|x| &x.id == config_id && x.category.as_deref() == Some("model")).cloned()
        };
        let Some(ctl) = current.filter(|x| x.options.iter().any(|o| &o.id == value)) else {
          return Err(anyhow!(t("host.executorUnavailable")));
        };
        if ctl.value.as_ref() != Some(value) {
          self.set_config(config_id.clone(), value.clone()).await?;
        }
      }
      if self.status() != SessionStatus::Ready {
        return Ok(());
      }
      if let Some((block, _)) = &permission {
        let mut c = self.core.lock();
        if c.perms.iter().any(|p| &p.block_id == block) {
          self.resolve_permission_locked(&mut c, block, option_id.as_deref().unwrap_or(""));
        }
        return Ok(());
      }
      let (running, mode, in_plan) = {
        let c = self.core.lock();
        let mode = c
          .state
          .controls
          .modes
          .iter()
          .find(|m| ["default", "accept-edits", "agent", "code"].contains(&m.id.as_str()))
          .map(|m| m.id.clone());
        (c.phase.running, mode, c.state.controls.mode_id.as_deref() == Some("plan"))
      };
      if running {
        return Ok(());
      }
      if in_plan {
        let mode = mode.ok_or_else(|| anyhow!(t("host.noExecutableMode")))?;
        self.set_mode(mode).await?;
      }
      {
        let mut c = self.core.lock();
        if c.status != SessionStatus::Ready || c.phase.running {
          return Ok(());
        }
        if let Some(p) = plan_documents_mut(&mut c.state.turns).into_iter().find(|p| p.id == plan_id) {
          p.status = PlanDocStatus::Executing;
        }
      }
      // Model-facing instruction: fixed English regardless of UI language
      self.prompt(plan_execution_prompt(&plan.markdown), vec![], false, None, Some(plan.id.clone())).await;
      Ok(())
    }
    .await;
    let mut c = self.core.lock();
    c.building_plan = false;
    self.touch(&mut c);
    result
  }
}

/// The root agent turn a new subagent anchors to: the live one, or the index the next update is about to open
pub(crate) fn current_turn_index(c: &Core) -> usize {
  match c.state.turns.last() {
    Some(Turn::Agent(_)) => c.state.turns.len() - 1,
    _ => c.state.turns.len(),
  }
}

/// The agent turn a prompt opened, if the transcript still has it where it was put
fn agent_turn_mut(c: &mut Core, idx: usize, started_at: i64) -> Option<&mut AgentTurn> {
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
  if first.is_empty() { super::attachments::describe_drafts(attachments) } else { first.to_owned() }
}

/// The session-info context snapshot of a Grok `_x.ai/session/info` answer
pub fn grok_context_usage(v: &Value, session_id: &str) -> Option<Usage> {
  let result = v.get("result")?;
  if result.get("sessionId").and_then(Value::as_str) != Some(session_id) {
    return None;
  }
  let ctx = result.get("context")?;
  let used = ctx.get("used")?.as_f64()?;
  let total = ctx.get("total")?.as_f64()?;
  let safe = |n: f64| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0;
  if !safe(used) || used < 0.0 || !safe(total) || total <= 0.0 {
    return None;
  }
  Some(Usage { used: num(used), size: num(total), cost: None })
}

pub(crate) fn js_num(n: f64) -> String {
  if n.fract() == 0.0 && n.abs() < 1e21 { format!("{}", n as i64) } else { format!("{n}") }
}
