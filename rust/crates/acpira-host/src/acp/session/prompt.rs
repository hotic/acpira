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
use crate::acp::session::edit::{FORK_HISTORY_LEAD, history_context};
use crate::acp::session::errors::{is_auth, is_session_gone, turn_error_of};
use crate::acp::session::failure::{failure_of, failure_turn_error};
use crate::acp::session::turn_usage::turn_usage_of;
use crate::acp::session::{AcpSession, Core, clear_usage_timer, num};
use crate::acp::transcript::normalize::{activity_of, apply_session_failure, end_turn, fail_turn};
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::acp::vendors::claude_window;
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

  pub(crate) async fn prompt_inner(self: Arc<Self>, text: String, drafts: Vec<Draft>, origin: Origin, staged: Option<Staged>, plan_id: Option<String>) {
    enum Gate {
      Queue,
      Drop,
      Go(bool),
    }
    let auto = origin == Origin::Compact;
    let gate = {
      let mut c = self.core.lock();
      if origin == Origin::Continue && (c.status != SessionStatus::Ready || c.phase.running) {
        // The switch reserved the session for this prompt; anything else taking it first means the continue is moot
        c.switching = false;
        Gate::Drop
      } else if c.status == SessionStatus::Starting {
        Gate::Queue
      } else if c.status != SessionStatus::Ready
        || (text.trim().is_empty() && drafts.is_empty() && staged.as_ref().is_none_or(|s| s.prepared.blocks.is_empty()))
      {
        Gate::Drop
      } else if origin != Origin::Continue && (c.switching || c.adopt_pending || c.phase.running || c.detached || (!auto && c.pending_prompt.is_some())) {
        Gate::Queue
      } else {
        c.switching = false;
        c.peer_idle = false;
        // Mid-turn /compact cannot be injected: the next user-facing request is the earliest slot, compact that first
        let compact_first = !auto && !is_compact_command(&text) && self.should_auto_compact(&c);
        c.phase.running = true;
        c.auto_compact_eligible = false;
        c.phase.staging = true;
        c.phase.staging_aborted = false;
        if origin == Origin::User {
          self.bump(&mut c);
        } else {
          self.touch(&mut c);
        }
        Gate::Go(compact_first)
      }
    };
    let compact_first = match gate {
      Gate::Queue => {
        self.enqueue(text, drafts, staged.map(|s| s.prepared)).await;
        return;
      }
      Gate::Drop => {
        if origin == Origin::Continue {
          self.flush_queue();
        }
        return;
      }
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
      let user_turn = match origin {
        Origin::Compact => UserTurn { text: text.clone(), auto: Some(true), ..Default::default() },
        Origin::Continue => UserTurn {
          id: Some(random_uuid()),
          text: text.clone(),
          settings: Some(before.clone()),
          auto: Some(true),
          auto_reason: Some(AutoReason::AccountSwitch),
          ..Default::default()
        },
        Origin::User => UserTurn {
          id: Some(random_uuid()),
          text: text.clone(),
          settings: Some(before.clone()),
          command,
          edited,
          plan_id: plan_id.clone(),
          attachments: (!prepared.attachments.is_empty()).then(|| prepared.attachments.clone()),
          ..Default::default()
        },
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
      if origin == Origin::User && plan_id.is_none() && untitled {
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
    // Set when the turn ran out of account quota and the session was reserved for an automatic switch
    let mut exhausted = false;
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
          let turn_error = failure_turn_error(f);
          exhausted = self.reserve_switch(&mut c, &turn_error);
          self.settle(&mut c, TurnStop::Cancelled, Some(turn_error));
          if f.actions.contains(&FailureAction::Login) {
            c.status = SessionStatus::AuthRequired;
          }
        } else {
          if let Some(f) = &failure {
            apply_session_failure(&mut self.core.lock().state, f);
          }
          if stop == TurnStop::EndTurn {
            if self.agent == "claude" {
              let c = self.core.lock();
              if let Some(size) = c.reported_window {
                claude_window::confirm(c.account_id.as_deref(), &c.state.controls, size);
              }
            }
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
        exhausted = self.reserve_switch(&mut c, &turn_error);
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
    if exhausted {
      tokio::spawn(self.clone().switch_after_exhaustion(agent_idx, started_at));
      return;
    }
    self.after_prompt(auto, stop);
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
    let failed = error.is_some();
    match error {
      Some(e) => fail_turn(&mut c.state, e),
      None => end_turn(&mut c.state, stop),
    }
    self.cancel_all_permissions(c);
    self.cancel_all_questions(c);
    c.phase.running = false;
    // A steer's detached peer turn continues this one: it keeps running until the peer reports its thread idle
    if c.detached && !failed {
      self.reopen_for_detached(c);
    }
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
}

/// The agent turn a prompt opened, if the transcript still has it where it was put
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
