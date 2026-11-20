//! Permission cards: a `session/request_permission` decided by mode, plugin hooks or plan approval, else shown as a card until answered or withdrawn

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::oneshot;

use acpira_shared::transcript::*;

use super::{AcpSession, PendingPermission, Target, cancelled_permission, state_of_block};
use crate::acp::transport::cancel::Cancel;
use crate::acp::transcript::normalize::{NormalizeState, activity_of, apply_update, command_from_raw, permission_tool_update};
use crate::acp::transcript::plans::{capture_plan, plan_documents_mut, set_plan_content};
use crate::acp::transport::rpc::RpcError;
use crate::acp::session::Core;
use crate::acp::session::controls::in_full_access;
use crate::acp::session::errors::{auto_allow, permission_kind};
use crate::acp::session::hooks::edit_paths;
use crate::acp::session::plugin_hooks::PermVerdict;
use crate::i18n::{t, tp};
use crate::limits::PLAN_PREVIEW_MAX_BYTES;

impl AcpSession {
  // Permission gate

  /// A permission request from the agent: the plugins hear it asked and answered around everything that decides it
  pub(crate) async fn on_permission(self: &Arc<Self>, mut req: Value, cancel: Cancel) -> Result<Value, RpcError> {
    self.nativize_permission(&mut req);
    self.emit_permission_asked(&req);
    let answer = self.decide_permission(req.clone(), cancel).await;
    match &answer {
      Ok(v) => self.emit_permission_replied(&req, v),
      Err(e) => self.emit_permission_replied(&req, &json!({ "outcome": { "outcome": "error", "message": e.message } })),
    }
    answer
  }

  async fn decide_permission(self: &Arc<Self>, req: Value, cancel: Cancel) -> Result<Value, RpcError> {
    if cancel.is_cancelled() {
      return Ok(cancelled_permission());
    }
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let tool_call = req.get("toolCall").cloned().unwrap_or(Value::Null);
    let tool_call_id = tool_call.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    let auto_rechecked = req.pointer("/_meta/acpira~1autoApprove").and_then(Value::as_bool) == Some(true);
    // A summoned child's request is read in its own CLI's dialect
    let vendor = session_id.as_deref().and_then(|sid| self.relay_vendor(sid)).unwrap_or(self.vendor);
    if vendor.question_permission(&tool_call) {
      return Ok(self.on_permission_question(req, tool_call_id, cancel).await);
    }
    let judged = self.deps.plugin_runtime.as_ref().is_some_and(|rt| rt.has_hook("permission"));
    let (target, epoch, plan_file, last_agent, captured, edit, row) = {
      let mut c = self.core.lock();
      let Some(target) = Self::target_for(&mut c, session_id.as_deref()) else {
        drop(c);
        self.log(&format!("permission request for unknown session {}", session_id.as_deref().unwrap_or("undefined")));
        return Ok(cancelled_permission());
      };
      let epoch = c.perms.epoch;
      let Some(state) = Self::target_state(&mut c, &target) else { return Ok(cancelled_permission()) };
      let last_agent = matches!(state.turns.last(), Some(Turn::Agent(_))).then(|| state.turns.len() - 1);
      // The verb / command on the card come from the tool row; the request itself often carries only a title
      let existing = last_agent.and_then(|i| {
        state.turns[i].as_agent().and_then(|t| {
          t.blocks.iter().find_map(|b| match b {
            AgentBlock::ToolCall(tc) if tc.id == tool_call_id => Some(tc.clone()),
            _ => None,
          })
        })
      });
      apply_update(state, &permission_tool_update(existing.as_ref(), &tool_call));
      // The row as the request left it (a low-fidelity request may not say it edits; the row already knows)
      let edit = last_agent.and_then(|i| {
        state.turns[i].as_agent().and_then(|t| {
          t.blocks.iter().find_map(|b| match b {
            AgentBlock::ToolCall(tc) if tc.id == tool_call_id => Some((tc.kind, edit_paths(tc))),
            _ => None,
          })
        })
      });
      // The row as the plugins' `permission` hooks see it
      let row = last_agent.filter(|_| judged).and_then(|i| {
        state.turns[i].as_agent().and_then(|t| {
          t.blocks.iter().find_map(|b| match b {
            AgentBlock::ToolCall(tc) if tc.id == tool_call_id => Some(crate::acp::session::plugin_events::tool_view(tc)),
            _ => None,
          })
        })
      });
      let plan_id = capture_plan(&mut state.turns, &tool_call);
      let plan_file = plan_id.as_ref().and_then(|id| {
        plan_documents_mut(&mut state.turns)
          .into_iter()
          .find(|p| &p.id == id)
          .filter(|p| p.markdown.is_empty())
          .and_then(|p| p.path.clone())
          .map(|path| (id.clone(), path))
      });
      (target, epoch, plan_file, last_agent, plan_id, edit, row)
    };
    // A stand-in's gate reads the call as its row knows it: a sparse request (an id and a title) says neither its kind
    // nor its paths, and would pass as no write or command at all
    let classified = match (&edit, tool_call.as_object()) {
      (Some((kind, paths)), Some(o)) => {
        let mut o = o.clone();
        o.entry("kind").or_insert_with(|| json!(kind));
        if !paths.is_empty() {
          o.entry("locations").or_insert_with(|| json!(paths.iter().map(|p| json!({ "path": p })).collect::<Vec<_>>()));
        }
        Value::Object(o)
      }
      _ => tool_call.clone(),
    };
    // The workspace gate sees an announced edit before anyone approves it, yolo included
    if let Some((kind, paths)) = edit
      && self.hooks_check_edit(&tool_call_id, kind, paths, tool_call.get("rawInput").cloned()).await.is_some()
    {
      return Ok(rejected_permission(&req));
    }
    // The plugins' processes judge it before yolo, a plan's auto-approval or a card: a denial rejects it, an ask insists
    // on the card
    let insist = if judged {
      let auto = {
        let c = self.core.lock();
        c.perms.auto_approve || in_full_access(&c.state.controls) || Self::access_native_full(&c) || auto_rechecked
      };
      match self.plugin_permission(&req, row, auto).await {
        PermVerdict::Deny(plugin, reason) => {
          self.log(&format!("plugin {plugin}: permission denied: {reason}"));
          self.notify(&tp("host.plugin.denied", &[("name", &plugin), ("reason", &reason)]));
          return Ok(rejected_permission(&req));
        }
        PermVerdict::Ask => true,
        PermVerdict::Pass => false,
      }
    } else {
      false
    };
    // A resumed Devin session may send only the plan path: load that exact file before presenting approval
    if let Some((plan_id, path)) = plan_file
      && let Ok(meta) = tokio::fs::metadata(&path).await
      && meta.is_file()
      && meta.len() <= PLAN_PREVIEW_MAX_BYTES
      && let Ok(text) = tokio::fs::read_to_string(&path).await
    {
      let mut c = self.core.lock();
      if let Some(state) = Self::target_state(&mut c, &target)
        && let Some(p) = plan_documents_mut(&mut state.turns).into_iter().find(|p| p.id == plan_id)
      {
        set_plan_content(p, &text);
        if !p.markdown.is_empty() {
          p.status = PlanDocStatus::Ready;
        }
      }
    }
    // Read outside the session lock: the setting lives with the shell
    let plan_auto_on = self.deps.policy.plan_auto_approve(&self.agent);
    let remote_perm = self.remote_perm(&tool_call_id, &classified);
    if remote_perm == crate::acp::session::stand::RemotePerm::BlockedLocalTool {
      return Ok(rejected_permission(&req));
    }
    let rx = {
      let mut c = self.core.lock();
      // The epoch tells a request of an ended root turn (or replaced process); a summoned child outlives both, and its
      // node going terminal is checked below
      let summoned = matches!(&target, Target::Node(id) if c.tree.relay_thread_of(id).is_some());
      if cancel.is_cancelled() || (!summoned && epoch != c.perms.epoch) {
        return Ok(cancelled_permission());
      }
      // The owner may have gone terminal while the plan file was being read
      if Self::target_for(&mut c, session_id.as_deref()).is_none() {
        return Ok(cancelled_permission());
      }
      let options: Vec<Value> = req.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
      // The root's full access (synthesized yolo or a native bypass mode) and plan auto-approval cover the root and its
      // own native children; a summoned child answers to its own round mode alone, so a consult or work child under a
      // full-access root still gets cards
      let root = !summoned;
      let synth = c.perms.auto_approve;
      // The built-in agent rechecks waiting actions after a live mode/approval change. It has already applied its
      // mode denies and configuration guard; the host's hooks above still get the final say.
      let full_access = root
        && !insist
        && remote_perm != crate::acp::session::stand::RemotePerm::Unjudged
        && (synth || in_full_access(&c.state.controls) || Self::access_native_full(&c) || auto_rechecked);
      // A summoned child in a bypass round: answered without a card, preferring the one-time allow so nothing lasting
      // lands in that CLI's own settings; a request with no plain allow still gets a card
      let bypass = matches!(&target, Target::Node(id) if c.tree.relay_bypass(id));
      if bypass && !insist && let Some(id) = single_allow_once(&options).or_else(|| auto_allow(&options)) {
        return Ok(json!({ "outcome": { "outcome": "selected", "optionId": id } }));
      }
      // A summoned child in a consult round stays read-only: its request to leave that mode (Claude Code's ExitPlanMode,
      // a plan's approval) is declined without a card, since the round's access is the caller's to set, not a card
      // for the user mid-round
      let consult = matches!(&target, Target::Node(id) if c.tree.relay_consult(id));
      if consult && !insist && (captured.is_some() || tool_call.get("kind").and_then(Value::as_str) == Some("switch_mode")) {
        return Ok(rejected_permission(&req));
      }
      let Some(state) = Self::target_state(&mut c, &target) else { return Ok(cancelled_permission()) };
      let plan = captured
        .as_ref()
        .and_then(|id| plan_documents_mut(&mut state.turns).into_iter().find(|p| &p.id == id))
        .map(|p| (p.id.clone(), p.markdown.clone(), p.approval_tool_call_id.clone()));
      // Full access: approve directly without a card. Synthesized yolo (where the CLI stays in default mode) prefers
      // allow_always so the same tool doesn't keep coming back; a native full-access mode (where the CLI only asks on
      // bypass-immune checks) prefers allow_once so nothing lasting is written to the CLI's settings. An ambiguous set
      // (two options of the picked kind) still gets a card
      if full_access && options.is_empty() {
        return Err(RpcError::internal(t("host.noPermissionOptions")));
      }
      let picked = if !full_access {
        None
      } else if synth {
        auto_allow(&options)
      } else {
        single_allow_once(&options).or_else(|| auto_allow(&options))
      };
      if let Some(id) = picked {
        if let Some((pid, _, approval)) = &plan
          && approval.as_deref() == Some(tool_call_id.as_str())
          && let Some(p) = plan_documents_mut(&mut state.turns).into_iter().find(|p| &p.id == pid)
        {
          p.status = PlanDocStatus::Approved;
        }
        return Ok(json!({ "outcome": { "outcome": "selected", "optionId": id } }));
      }
      // Plan mode under acpira.planAutoApprove: Claude Code 2.1.284 (unlike 2.1.7) no longer skips prompts in plan mode under the SDK,
      // so read-only probes would each ask. Answer with the single allow_once (never allow_always, which would write a
      // lasting rule into the CLI's settings); the plan's own approval and a request the adapter marks default-to-no
      // still get a card
      let in_plan = root && c.state.controls.mode_id.as_deref() == Some("plan");
      if plan_auto_on
        && !insist
        && in_plan
        && captured.is_none()
        && remote_perm != crate::acp::session::stand::RemotePerm::HostEdit
        && tool_call.get("kind").and_then(Value::as_str) != Some("switch_mode")
        && default_to_no(&req) != Some(true)
        && let Some(id) = single_allow_once(&options)
      {
        return Ok(json!({ "outcome": { "outcome": "selected", "optionId": id } }));
      }
      // Remote tools on an SSH host (`session/stand.rs`) ask where native ones would not: read-only tools on CLIs without
      // `allowedTools` (`agy acp`), the CLI's own plan / artifact files here, and host writes in accept-edits mode
      let accept_edits = c.state.controls.mode_id.as_deref() == Some("acceptEdits") || c.access.wire.as_deref() == Some("acceptEdits");
      if root
        && !insist
        && (matches!(
          remote_perm,
          crate::acp::session::stand::RemotePerm::ReadOnly | crate::acp::session::stand::RemotePerm::AgentOwnFile
        ) || (accept_edits && remote_perm == crate::acp::session::stand::RemotePerm::HostEdit))
        && let Some(id) = single_allow_once(&options)
      {
        return Ok(json!({ "outcome": { "outcome": "selected", "optionId": id } }));
      }
      c.perms.seq += 1;
      let block_id = format!("perm-{}", c.perms.seq);
      let Some(state) = Self::target_state(&mut c, &target) else { return Ok(cancelled_permission()) };
      let tool = last_agent.and_then(|i| {
        state.turns.get(i).and_then(Turn::as_agent).and_then(|t| {
          t.blocks.iter().find_map(|b| match b {
            AgentBlock::ToolCall(tc) if tc.id == tool_call_id => Some(tc.clone()),
            _ => None,
          })
        })
      });
      let raw = tool_call.get("rawInput").and_then(Value::as_object);
      let meta = permission_meta(&req);
      let v1 = meta.is_some_and(|m| m.get("version").and_then(Value::as_f64) == Some(1.0));
      let meta_str = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
      let meta_title = if v1 { meta_str(meta.and_then(|m| m.get("title"))) } else { None };
      let meta_desc = if v1 { meta_str(meta.and_then(|m| m.get("description"))) } else { None };
      let default_to_no = default_to_no(&req);
      let title = meta_title.unwrap_or_else(|| match &tool {
        // A mode switch names itself ("Exit plan mode"), so the verb would only repeat it
        Some(tc) if tc.kind == ToolKind::SwitchMode && tc.target.as_deref().is_some_and(|x| !x.is_empty()) => {
          tp("host.needApprovalFor", &[("what", tc.target.as_deref().unwrap_or_default())])
        }
        Some(tc) => {
          let target =
            if tc.kind != ToolKind::Execute { tc.target.as_ref().map(|x| format!(" {x}")).unwrap_or_default() } else { String::new() };
          tp("host.needApprovalFor", &[("what", &format!("{}{target}", tc.verb))])
        }
        None => match tool_call.get("title").and_then(Value::as_str).filter(|x| !x.is_empty()) {
          Some(x) => tp("host.needApprovalFor", &[("what", x)]),
          None => t("host.needApproval"),
        },
      });
      let plan_id = plan
        .as_ref()
        .filter(|(_, md, approval)| !md.is_empty() && approval.as_deref() == Some(tool_call_id.as_str()))
        .map(|(id, _, _)| id.clone());
      let block = PermissionBlock {
        id: block_id.clone(),
        title,
        command: command_from_raw(raw).or_else(|| tool.as_ref().filter(|tc| tc.kind == ToolKind::Execute).and_then(|tc| tc.target.clone())),
        description: meta_desc,
        plan_id: plan_id.clone(),
        default_to_no,
        options: options
          .iter()
          .map(|o| PermissionOption {
            id: o.get("optionId").and_then(Value::as_str).unwrap_or("").to_owned(),
            label: o.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
            kind: permission_kind(o.get("kind").unwrap_or(&Value::Null)),
            detail: meta_str(permission_meta(o).and_then(|p| p.get("description"))),
          })
          .collect(),
      };
      if let Some(i) = last_agent
        && let Some(Turn::Agent(turn)) = state.turns.get_mut(i)
      {
        turn.blocks.push(AgentBlock::Permission(block));
        let activity = activity_of(&state.turns);
        if let Some(Turn::Agent(turn)) = state.turns.get_mut(i) {
          turn.activity = activity;
        }
        Self::bump_target(&mut c, &target);
      }
      let (tx, rx) = oneshot::channel();
      c.perms.pending.push(PendingPermission { tx, block_id: block_id.clone(), options, plan_id, node_id: target.node_id(), insist });
      self.touch(&mut c);
      (rx, block_id)
    };
    let (rx, block_id) = rx;
    tokio::select! {
      r = rx => Ok(r.unwrap_or_else(|_| cancelled_permission())),
      _ = cancel.cancelled() => {
        let mut c = self.core.lock();
        if let Some(i) = c.perms.pending.iter().position(|p| p.block_id == block_id) {
          c.perms.pending.remove(i);
          remove_perm_blocks(&mut c, Some(&block_id));
          self.touch(&mut c);
        }
        Ok(cancelled_permission())
      }
    }
  }

  pub fn resolve_permission(&self, block_id: &str, option_id: &str) {
    let mut c = self.core.lock();
    self.resolve_permission_locked(&mut c, block_id, option_id);
  }

  pub(crate) fn resolve_permission_locked(&self, c: &mut Core, block_id: &str, option_id: &str) {
    let Some(i) = c.perms.pending.iter().position(|p| p.block_id == block_id) else { return };
    let Some(option) = c.perms.pending[i].options.iter().find(|o| o.get("optionId").and_then(Value::as_str) == Some(option_id)).cloned() else {
      return;
    };
    let allow = option.get("kind").and_then(Value::as_str).is_some_and(|k| k.starts_with("allow"));
    if let Some(pid) = c.perms.pending[i].plan_id.clone()
      && let Some(t) = state_of_block(c, block_id)
      && let Some(state) = Self::target_state(c, &t)
      && let Some(p) = plan_documents_mut(&mut state.turns).into_iter().find(|p| p.id == pid)
    {
      p.status = if allow { PlanDocStatus::Approved } else { PlanDocStatus::Rejected };
    }
    let p = c.perms.pending.remove(i);
    remove_perm_blocks(c, Some(block_id));
    let _ = p.tx.send(json!({ "outcome": { "outcome": "selected", "optionId": option_id } }));
    self.touch(c);
  }

  /// Switching into full access approves the requests already waiting in one go; ambiguous ones and summoned children's
  /// (which follow their own round mode) stay open
  pub(crate) fn flush_permissions(&self, c: &mut Core) {
    let summoned = |p: &PendingPermission| p.node_id.as_deref().is_some_and(|id| c.tree.relay_thread_of(id).is_some());
    let synth = c.perms.auto_approve;
    let pending: Vec<(String, String)> = c
      .perms
      .pending
      .iter()
      .filter(|p| !summoned(p) && !p.insist)
      .filter_map(|p| {
        let pick = if synth { auto_allow(&p.options) } else { single_allow_once(&p.options).or_else(|| auto_allow(&p.options)) };
        pick.map(|o| (p.block_id.clone(), o))
      })
      .collect();
    for (block, option) in pending {
      self.resolve_permission_locked(c, &block, &option);
    }
  }

  pub(crate) fn cancel_all_permissions(&self, c: &mut Core) {
    for p in c.perms.pending.drain(..) {
      let _ = p.tx.send(cancelled_permission());
    }
    remove_perm_blocks(c, None);
  }

  pub(crate) fn cancel_permissions_for(&self, c: &mut Core, node_id: &str) {
    let mut i = 0;
    while i < c.perms.pending.len() {
      if c.perms.pending[i].node_id.as_deref() != Some(node_id) {
        i += 1;
        continue;
      }
      let p = c.perms.pending.remove(i);
      remove_perm_blocks(c, Some(&p.block_id));
      let _ = p.tx.send(cancelled_permission());
    }
  }

  /// The pending card approving a plan, if one is waiting: (block id, options)
  pub(crate) fn permission_by_plan(c: &Core, plan_id: &str) -> Option<(String, Vec<Value>)> {
    c.perms.pending.iter().find(|p| p.plan_id.as_deref() == Some(plan_id)).map(|p| (p.block_id.clone(), p.options.clone()))
  }
}

/// The request's own reject (once before always); without one the request is cancelled
fn rejected_permission(req: &Value) -> Value {
  let options = req.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
  let pick = |kind: &str| options.iter().find(|o| o.get("kind").and_then(Value::as_str) == Some(kind)).and_then(|o| o.get("optionId")).cloned();
  match pick("reject_once").or_else(|| pick("reject_always")) {
    Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
    None => cancelled_permission(),
  }
}

/// `_meta.permission` (or `_meta.jetbrains.air.permission` on claude-agent-acp / codex-acp for AIR clients)
fn permission_meta(v: &Value) -> Option<&Value> {
  let m = v.get("_meta")?;
  m.pointer("/jetbrains/air/permission").or_else(|| m.get("permission")).filter(|p| p.is_object())
}

/// `_meta.permission.defaultToNo` (claude-agent-acp / codex-acp): the adapter's own safety asks lean to No
pub(super) fn default_to_no(req: &Value) -> Option<bool> {
  let meta = permission_meta(req);
  (meta.and_then(|m| m.get("defaultToNo")) == Some(&Value::Bool(true))).then_some(true)
}

/// The one `allow_once` option; none or several (answers rather than a yes) leave the request to a person
pub(super) fn single_allow_once(options: &[Value]) -> Option<String> {
  let mut once = options.iter().filter(|o| o.get("kind").and_then(Value::as_str) == Some("allow_once"));
  let id = once.next()?.get("optionId").and_then(Value::as_str)?.to_owned();
  once.next().is_none().then_some(id)
}

/// Withdraw permission cards (one, or all) from every transcript
pub(crate) fn remove_perm_blocks(c: &mut Core, only: Option<&str>) {
  strip_perm_blocks(c, |id| only.is_none_or(|o| id == o));
}

/// Withdraw every permission card but the ones still waiting (`keep`)
pub(super) fn remove_perm_blocks_except(c: &mut Core, keep: &[String]) {
  strip_perm_blocks(c, |id| !keep.iter().any(|k| k == id));
}

fn strip_perm_blocks(c: &mut Core, gone: impl Fn(&str) -> bool) {
  let strip = |st: &mut NormalizeState| -> bool {
    let mut removed = false;
    for t in st.turns.iter_mut().filter_map(Turn::as_agent_mut) {
      let before = t.blocks.len();
      t.blocks.retain(|b| !matches!(b, AgentBlock::Permission(p) if gone(&p.id)));
      removed |= t.blocks.len() != before;
    }
    removed
  };
  strip(&mut c.state);
  let mut bumped = vec![];
  for (id, st) in c.tree.states_mut() {
    if strip(st) {
      bumped.push(id.to_owned());
    }
  }
  for id in bumped {
    c.tree.bump(&id);
  }
}
