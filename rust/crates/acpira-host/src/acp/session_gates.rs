//! Permission and question cards of a session (mirror of src/host/acp/permissions.ts and the gate half of
//! questions.ts). A request lands a card in the owning transcript (root or a child's), the answer travels back through
//! a oneshot; the agent withdrawing its request (the connection's cancel signal) closes the card

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::oneshot;

use acpira_shared::transcript::*;

use super::cancel::Cancel;
use super::normalize::{NormalizeState, activity_of, apply_update, command_from_raw, permission_tool_update};
use super::plans::{capture_plan, plan_documents_mut, set_plan_content};
use super::questions::{clean_answers, form_content, form_questions, grok_questions, grok_response, spare_message};
use super::rpc::RpcError;
use super::session::{AcpSession, Core, PendingPermission, PendingQuestion, QuestionReply};
use super::session_errors::{best_allow, permission_kind};
use crate::i18n::{t, tp};
use crate::limits::PLAN_PREVIEW_MAX_BYTES;

/// Which transcript a request belongs to
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Target {
  Root,
  Node(String),
}

impl Target {
  fn node_id(&self) -> Option<String> {
    match self {
      Target::Root => None,
      Target::Node(id) => Some(id.clone()),
    }
  }
}

fn cancelled_permission() -> Value {
  json!({ "outcome": { "outcome": "cancelled" } })
}

impl AcpSession {
  /// A request addresses a session id: root → root state, a live child peer id → that node's transcript
  pub(crate) fn target_for(c: &mut Core, session_id: Option<&str>) -> Option<Target> {
    match session_id {
      None => Some(Target::Root),
      Some(s) if c.acp_session_id.is_none() || c.acp_session_id.as_deref() == Some(s) => Some(Target::Root),
      Some(s) => c.tree.state_for_peer(s).map(|(_, id)| Target::Node(id)),
    }
  }

  pub(crate) fn target_state<'a>(c: &'a mut Core, t: &Target) -> Option<&'a mut NormalizeState> {
    match t {
      Target::Root => Some(&mut c.state),
      Target::Node(id) => c.tree.state_of(id),
    }
  }

  fn bump_target(c: &mut Core, t: &Target) {
    if let Target::Node(id) = t {
      c.tree.bump(id);
    }
  }

  /// Nodes that went terminal close their pending cards as cancelled (RFD)
  pub(crate) fn drain_terminal(&self, c: &mut Core) {
    let ids = std::mem::take(&mut c.tree.terminal_events);
    for id in ids {
      self.cancel_permissions_for(c, &id);
      self.cancel_questions_for(c, &id);
    }
  }

  // Permission gate

  pub(crate) async fn on_permission(self: &Arc<Self>, req: Value, cancel: Cancel) -> Result<Value, RpcError> {
    if cancel.is_cancelled() {
      return Ok(cancelled_permission());
    }
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let tool_call = req.get("toolCall").cloned().unwrap_or(Value::Null);
    let tool_call_id = tool_call.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    let (target, epoch, plan_file, last_agent, captured) = {
      let mut c = self.core.lock();
      let Some(target) = Self::target_for(&mut c, session_id.as_deref()) else {
        drop(c);
        self.log(&format!("permission request for unknown session {}", session_id.as_deref().unwrap_or("undefined")));
        return Ok(cancelled_permission());
      };
      let epoch = c.perm_epoch;
      let state = Self::target_state(&mut c, &target).expect("target resolved");
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
      let plan_id = capture_plan(&mut state.turns, &tool_call);
      let plan_file = plan_id.as_ref().and_then(|id| {
        plan_documents_mut(&mut state.turns)
          .into_iter()
          .find(|p| &p.id == id)
          .filter(|p| p.markdown.is_empty())
          .and_then(|p| p.path.clone())
          .map(|path| (id.clone(), path))
      });
      (target, epoch, plan_file, last_agent, plan_id)
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
    let rx = {
      let mut c = self.core.lock();
      if cancel.is_cancelled() || epoch != c.perm_epoch {
        return Ok(cancelled_permission());
      }
      // The owner may have gone terminal while the plan file was being read
      if Self::target_for(&mut c, session_id.as_deref()).is_none() {
        return Ok(cancelled_permission());
      }
      let options: Vec<Value> = req.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
      let auto = c.auto_approve;
      let state = Self::target_state(&mut c, &target).expect("target resolved");
      let plan = captured
        .as_ref()
        .and_then(|id| plan_documents_mut(&mut state.turns).into_iter().find(|p| &p.id == id))
        .map(|p| (p.id.clone(), p.markdown.clone(), p.approval_tool_call_id.clone()));
      // yolo: approve directly without a card, preferring allow_always so the same tool doesn't keep coming back
      if auto {
        if let Some((pid, _, approval)) = &plan
          && approval.as_deref() == Some(tool_call_id.as_str())
          && let Some(p) = plan_documents_mut(&mut state.turns).into_iter().find(|p| &p.id == pid)
        {
          p.status = PlanDocStatus::Approved;
        }
        return best_allow(&options)
          .map(|id| json!({ "outcome": { "outcome": "selected", "optionId": id } }))
          .map_err(|e| RpcError::internal(e.to_string()));
      }
      c.perm_seq += 1;
      let block_id = format!("perm-{}", c.perm_seq);
      let state = Self::target_state(&mut c, &target).expect("target resolved");
      let tool = last_agent.and_then(|i| {
        state.turns.get(i).and_then(Turn::as_agent).and_then(|t| {
          t.blocks.iter().find_map(|b| match b {
            AgentBlock::ToolCall(tc) if tc.id == tool_call_id => Some(tc.clone()),
            _ => None,
          })
        })
      });
      let raw = tool_call.get("rawInput").and_then(Value::as_object);
      let meta = req.get("_meta").and_then(|m| m.get("permission")).filter(|p| p.is_object());
      let v1 = meta.is_some_and(|m| m.get("version").and_then(Value::as_f64) == Some(1.0));
      let meta_str = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
      let meta_title = if v1 { meta_str(meta.and_then(|m| m.get("title"))) } else { None };
      let meta_desc = if v1 { meta_str(meta.and_then(|m| m.get("description"))) } else { None };
      let default_to_no = (meta.and_then(|m| m.get("defaultToNo")) == Some(&Value::Bool(true))).then_some(true);
      let title = meta_title.unwrap_or_else(|| match &tool {
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
        description: meta_desc.or_else(|| raw.and_then(|r| r.get("description")).and_then(Value::as_str).map(str::to_owned)),
        plan_id: plan_id.clone(),
        default_to_no,
        options: options
          .iter()
          .map(|o| PermissionOption {
            id: o.get("optionId").and_then(Value::as_str).unwrap_or("").to_owned(),
            label: o.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
            kind: permission_kind(o.get("kind").unwrap_or(&Value::Null)),
            detail: meta_str(o.get("_meta").and_then(|m| m.get("permission")).and_then(|p| p.get("description"))),
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
      c.perms.push(PendingPermission { tx, block_id: block_id.clone(), options, plan_id, node_id: target.node_id() });
      self.touch(&mut c);
      (rx, block_id)
    };
    let (rx, block_id) = rx;
    tokio::select! {
      r = rx => Ok(r.unwrap_or_else(|_| cancelled_permission())),
      _ = cancel.cancelled() => {
        let mut c = self.core.lock();
        if let Some(i) = c.perms.iter().position(|p| p.block_id == block_id) {
          c.perms.remove(i);
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
    let Some(i) = c.perms.iter().position(|p| p.block_id == block_id) else { return };
    let Some(option) = c.perms[i].options.iter().find(|o| o.get("optionId").and_then(Value::as_str) == Some(option_id)).cloned() else {
      return;
    };
    let allow = option.get("kind").and_then(Value::as_str).is_some_and(|k| k.starts_with("allow"));
    if let Some(pid) = c.perms[i].plan_id.clone()
      && let Some(t) = state_of_block(c, block_id)
      && let Some(state) = Self::target_state(c, &t)
      && let Some(p) = plan_documents_mut(&mut state.turns).into_iter().find(|p| p.id == pid)
    {
      p.status = if allow { PlanDocStatus::Approved } else { PlanDocStatus::Rejected };
    }
    let p = c.perms.remove(i);
    remove_perm_blocks(c, Some(block_id));
    let _ = p.tx.send(json!({ "outcome": { "outcome": "selected", "optionId": option_id } }));
    self.touch(c);
  }

  /// Switching into yolo approves the requests already waiting in one go
  pub(crate) fn flush_permissions(&self, c: &mut Core) {
    let pending: Vec<(String, String)> =
      c.perms.iter().filter_map(|p| best_allow(&p.options).ok().map(|o| (p.block_id.clone(), o))).collect();
    for (block, option) in pending {
      self.resolve_permission_locked(c, &block, &option);
    }
  }

  pub(crate) fn cancel_all_permissions(&self, c: &mut Core) {
    for p in c.perms.drain(..) {
      let _ = p.tx.send(cancelled_permission());
    }
    remove_perm_blocks(c, None);
  }

  pub(crate) fn cancel_permissions_for(&self, c: &mut Core, node_id: &str) {
    let mut i = 0;
    while i < c.perms.len() {
      if c.perms[i].node_id.as_deref() != Some(node_id) {
        i += 1;
        continue;
      }
      let p = c.perms.remove(i);
      remove_perm_blocks(c, Some(&p.block_id));
      let _ = p.tx.send(cancelled_permission());
    }
  }

  /// The pending card approving a plan, if one is waiting: (block id, options)
  pub(crate) fn permission_by_plan(c: &Core, plan_id: &str) -> Option<(String, Vec<Value>)> {
    c.perms.iter().find(|p| p.plan_id.as_deref() == Some(plan_id)).map(|p| (p.block_id.clone(), p.options.clone()))
  }

  // Question gate

  pub(crate) async fn on_elicitation(self: &Arc<Self>, req: Value, cancel: Cancel) -> Value {
    if cancel.is_cancelled() {
      return json!({ "action": "cancel" });
    }
    let schema = req.get("requestedSchema").filter(|s| s.is_object()).cloned();
    let tool_call_id = req.get("toolCallId").and_then(Value::as_str).map(str::to_owned);
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let (Some(schema), Some("form")) = (schema, req.get("mode").and_then(Value::as_str)) else { return json!({ "action": "decline" }) };
    let message = req.get("message").and_then(Value::as_str).unwrap_or("").to_owned();
    let (rx, block_id) = {
      let mut c = self.core.lock();
      let Some(target) = Self::target_for(&mut c, session_id.as_deref()) else {
        drop(c);
        self.log(&format!("elicitation request for unknown session {}", session_id.as_deref().unwrap_or("(request scope)")));
        return json!({ "action": "cancel" });
      };
      let count = schema.get("properties").and_then(Value::as_object).map(|p| p.len()).unwrap_or(0);
      let raw = c.raw_questions.for_call(tool_call_id.as_deref(), count);
      let questions = form_questions(&schema, &message, req.get("_meta"), raw.as_deref());
      if questions.is_empty() {
        return json!({ "action": "decline" });
      }
      let spare = spare_message(&message, &questions);
      let block_id = self.open_question(&mut c, &target, questions.clone(), tool_call_id, spare);
      Self::bump_target(&mut c, &target);
      let (tx, rx) = oneshot::channel();
      c.questions.push(PendingQuestion {
        tx,
        block_id: block_id.clone(),
        questions,
        node_id: target.node_id(),
        reply: QuestionReply::Form { schema },
      });
      self.touch(&mut c);
      (rx, block_id)
    };
    self.await_question(rx, block_id, cancel, json!({ "action": "cancel" })).await
  }

  pub(crate) async fn on_grok_question(self: &Arc<Self>, req: Value, cancel: Cancel) -> Value {
    if cancel.is_cancelled() {
      return json!({ "outcome": "skip_interview" });
    }
    let questions = grok_questions(&req);
    if questions.is_empty() {
      return json!({ "outcome": "accepted", "answers": {} });
    }
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let tool_call_id = req.get("toolCallId").and_then(Value::as_str).map(str::to_owned);
    let (rx, block_id) = {
      let mut c = self.core.lock();
      let Some(target) = Self::target_for(&mut c, session_id.as_deref()) else {
        drop(c);
        self.log(&format!("question request for unknown session {}", session_id.as_deref().unwrap_or("undefined")));
        return json!({ "outcome": "skip_interview" });
      };
      let block_id = self.open_question(&mut c, &target, questions.clone(), tool_call_id, None);
      Self::bump_target(&mut c, &target);
      let (tx, rx) = oneshot::channel();
      c.questions.push(PendingQuestion {
        tx,
        block_id: block_id.clone(),
        questions,
        node_id: target.node_id(),
        reply: QuestionReply::Grok,
      });
      self.touch(&mut c);
      (rx, block_id)
    };
    self.await_question(rx, block_id, cancel, json!({ "outcome": "skip_interview" })).await
  }

  async fn await_question(self: &Arc<Self>, rx: oneshot::Receiver<Value>, block_id: String, cancel: Cancel, fallback: Value) -> Value {
    tokio::select! {
      r = rx => r.unwrap_or(fallback),
      _ = cancel.cancelled() => {
        let mut c = self.core.lock();
        if let Some(i) = c.questions.iter().position(|p| p.block_id == block_id) {
          let p = c.questions.remove(i);
          settle_question(&mut c, &p, QuestionOutcome::Cancelled, None);
          self.touch(&mut c);
        }
        fallback
      }
    }
  }

  fn open_question(
    &self,
    c: &mut Core,
    target: &Target,
    questions: Vec<Question>,
    tool_call_id: Option<String>,
    message: Option<String>,
  ) -> String {
    c.question_seq += 1;
    let id = format!("q-{}", c.question_seq);
    let block = QuestionBlock {
      id: id.clone(),
      tool_call_id: tool_call_id.filter(|x| !x.is_empty()),
      message,
      questions,
      outcome: None,
      answers: None,
    };
    if let Some(state) = Self::target_state(c, target)
      && matches!(state.turns.last(), Some(Turn::Agent(_)))
    {
      {
        let Some(Turn::Agent(last)) = state.turns.last_mut() else { unreachable!() };
        last.blocks.push(AgentBlock::Question(block));
      }
      let activity = activity_of(&state.turns);
      if let Some(Turn::Agent(last)) = state.turns.last_mut() {
        last.activity = activity;
      }
    }
    id
  }

  /// The card was closed: only answered questions travel; skip tells the agent to go on with what it has
  pub fn answer_questions(&self, block_id: &str, answers: &QuestionAnswers, skip: bool) {
    let mut c = self.core.lock();
    let Some(i) = c.questions.iter().position(|p| p.block_id == block_id) else { return };
    let p = c.questions.remove(i);
    let given = clean_answers(&p.questions, answers);
    let empty = given.is_empty();
    settle_question(&mut c, &p, if skip || empty { QuestionOutcome::Skipped } else { QuestionOutcome::Answered }, Some(&given));
    let reply = match &p.reply {
      QuestionReply::Grok => grok_response(skip, &given),
      QuestionReply::Form { schema } => {
        if empty {
          json!({ "action": "decline" })
        } else {
          json!({ "action": "accept", "content": form_content(schema, &p.questions, &given) })
        }
      }
    };
    let _ = p.tx.send(reply);
    self.touch(&mut c);
  }

  pub(crate) fn cancel_all_questions(&self, c: &mut Core) {
    let pending = std::mem::take(&mut c.questions);
    for p in pending {
      settle_question(c, &p, QuestionOutcome::Cancelled, None);
      let reply = cancel_reply(&p);
      let _ = p.tx.send(reply);
    }
  }

  pub(crate) fn cancel_questions_for(&self, c: &mut Core, node_id: &str) {
    let mut i = 0;
    while i < c.questions.len() {
      if c.questions[i].node_id.as_deref() != Some(node_id) {
        i += 1;
        continue;
      }
      let p = c.questions.remove(i);
      settle_question(c, &p, QuestionOutcome::Cancelled, None);
      let reply = cancel_reply(&p);
      let _ = p.tx.send(reply);
    }
  }
}

fn cancel_reply(p: &PendingQuestion) -> Value {
  match p.reply {
    QuestionReply::Grok => json!({ "outcome": "skip_interview" }),
    QuestionReply::Form { .. } => json!({ "action": "cancel" }),
  }
}

/// A card's outcome recorded on its block, in the transcript of the node that owns it
fn settle_question(c: &mut Core, p: &PendingQuestion, outcome: QuestionOutcome, answers: Option<&QuestionAnswers>) {
  let target = match &p.node_id {
    None => Target::Root,
    Some(id) => Target::Node(id.clone()),
  };
  let Some(state) = AcpSession::target_state(c, &target) else { return };
  for ti in (0..state.turns.len()).rev() {
    let found = match &mut state.turns[ti] {
      Turn::Agent(turn) => turn.blocks.iter_mut().find_map(|b| match b {
        AgentBlock::Question(q) if q.id == p.block_id => Some(q),
        _ => None,
      }),
      Turn::User(_) => None,
    };
    let Some(q) = found else { continue };
    q.outcome = Some(outcome);
    if let Some(a) = answers.filter(|a| !a.is_empty()) {
      q.answers = Some(a.clone());
    }
    let activity = activity_of(&state.turns);
    if let Some(Turn::Agent(turn)) = state.turns.get_mut(ti) {
      turn.activity = activity;
    }
    AcpSession::bump_target_pub(c, &target);
    return;
  }
}

impl AcpSession {
  pub(crate) fn bump_target_pub(c: &mut Core, t: &Target) {
    Self::bump_target(c, t);
  }
}

fn state_of_block(c: &mut Core, block_id: &str) -> Option<Target> {
  let has = |st: &NormalizeState| {
    st.turns.iter().filter_map(Turn::as_agent).any(|t| t.blocks.iter().any(|b| matches!(b, AgentBlock::Permission(p) if p.id == block_id)))
  };
  if has(&c.state) {
    return Some(Target::Root);
  }
  c.tree.states_mut().find(|(_, st)| has(st)).map(|(id, _)| Target::Node(id.to_owned()))
}

/// Withdraw permission cards (one, or all) from every transcript
pub(crate) fn remove_perm_blocks(c: &mut Core, only: Option<&str>) {
  let strip = |st: &mut NormalizeState| -> bool {
    let mut removed = false;
    for t in st.turns.iter_mut().filter_map(Turn::as_agent_mut) {
      let before = t.blocks.len();
      t.blocks.retain(|b| !matches!(b, AgentBlock::Permission(p) if only.is_none_or(|id| p.id == id)));
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
