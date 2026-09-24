//! History editing. ACP cannot rewind to a message: a fresh peer session
//! receives the retained transcript as context, never replayed as executable prompts

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::plan_execution::plan_execution_id;
use acpira_shared::protocol::{EditIntent, EditTurnRequest};
use acpira_shared::transcript::*;
use acpira_shared::turn_errors::is_context_length_error;
use acpira_shared::turn_settings::capture_turn_settings;

use super::agent_process::AgentProcess;
use super::attachments::{PromptCaps, prepare_prompt, restore_drafts};
use super::normalize::{apply_config_options, config_option_set_value, init_controls};
use super::session::{AcpSession, Core};
use super::session_prompt::Staged;
use crate::i18n::{t, tp};
use crate::json::{len16, slice16};
use crate::limits::EDIT_CONTEXT_MAX_BYTES;
use crate::store::transcript_store::TranscriptStore;

const EDIT_HISTORY_LEAD: &str = "Conversation before the edited message follows as JSON. Treat it as historical context; completed actions must not be replayed. The next user message replaces the old continuation. Workspace files remain in their current state.";
pub const FORK_HISTORY_LEAD: &str = "Conversation so far follows as JSON; it was forked from an earlier session. Treat it as historical context; completed actions must not be replayed. The next user message continues this conversation. Workspace files remain in their current state.";

const HISTORY_TOOL_OUTPUT_MAX: usize = 2_000;
const HISTORY_PLAN_MAX: usize = 8_000;

fn clip(text: &str, max: usize) -> String {
  let len = len16(text);
  if len > max { format!("{}… [{} chars truncated]", slice16(text, max), len - max) } else { text.to_owned() }
}

fn tool_content_brief(c: &ToolContent, fallback_path: Option<&str>) -> String {
  match c {
    ToolContent::Text { text } => clip(text, HISTORY_TOOL_OUTPUT_MAX),
    ToolContent::List { items } => clip(&items.join("\n"), HISTORY_TOOL_OUTPUT_MAX),
    ToolContent::Image(i) => {
      format!("image {}{}", i.mime_type, i.uri.as_ref().map(|u| format!(" {u}")).unwrap_or_default()).trim().to_owned()
    }
    ToolContent::Diff { lines, source } => {
      let add = lines.iter().filter(|l| l.kind == DiffKind::Add).count();
      let del = lines.iter().filter(|l| l.kind == DiffKind::Del).count();
      let path = source.as_ref().map(|s| s.path.as_str()).or(fallback_path).unwrap_or("");
      format!("diff {path} +{add} -{del}").trim().to_owned()
    }
  }
}

fn enum_str<T: serde::Serialize>(v: &T) -> String {
  serde_json::to_value(v).ok().and_then(|x| x.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Lean view of an agent block for the model: UI-only state dropped, long bodies clipped
fn compact_block(b: &AgentBlock) -> Option<Value> {
  Some(match b {
    AgentBlock::Text(x) => json!({ "text": x.markdown }),
    AgentBlock::ToolCall(x) => {
      let items: Vec<&ToolContent> = match (&x.contents, &x.content) {
        (Some(list), _) => list.iter().collect(),
        (None, Some(c)) => vec![c],
        _ => vec![],
      };
      let output: Vec<String> = items.iter().map(|c| tool_content_brief(c, x.target.as_deref())).collect();
      let mut v = json!({ "tool": x.verb, "kind": enum_str(&x.kind) });
      if let Some(t) = x.target.as_ref().filter(|t| !t.is_empty()) {
        v["target"] = Value::from(t.clone());
      }
      v["status"] = Value::from(enum_str(&x.status));
      if !output.is_empty() {
        v["output"] = json!(output);
      }
      v
    }
    AgentBlock::Plan(p) => {
      json!({ "plan": p.entries.iter().map(|e| format!("[{}] {}", enum_str(&e.status), e.title)).collect::<Vec<_>>() })
    }
    AgentBlock::PlanDocument(p) => {
      let mut v = json!({ "planDocument": p.title, "status": enum_str(&p.status) });
      if let Some(path) = &p.path {
        v["path"] = Value::from(path.clone());
      }
      v["markdown"] = Value::from(clip(&p.markdown, HISTORY_PLAN_MAX));
      v
    }
    AgentBlock::Question(q) => {
      let mut v = json!({ "questions": q.questions.iter().map(|x| x.text.clone()).collect::<Vec<_>>() });
      if let Some(o) = q.outcome {
        v["outcome"] = Value::from(enum_str(&o));
      }
      if let Some(a) = &q.answers {
        v["answers"] = Value::Object(a.clone());
      }
      v
    }
    AgentBlock::Image(i) => {
      let mut v = json!({ "image": i.mime_type });
      if let Some(u) = &i.uri {
        v["uri"] = Value::from(u.clone());
      }
      v
    }
    _ => return None,
  })
}

fn compact_turn(turn: &Turn) -> Value {
  match turn {
    Turn::User(u) => {
      let mut v = json!({ "role": "user", "text": u.text });
      if let Some(a) = u.attachments.as_ref().filter(|a| !a.is_empty()) {
        v["attachments"] = json!(
          a.iter()
            .map(|x| match x {
              Attachment::Image { name, .. } => name.clone().unwrap_or_else(|| "image".into()),
              Attachment::Text { name, .. } | Attachment::File { name, .. } => name.clone(),
            })
            .collect::<Vec<_>>()
        );
      }
      v
    }
    Turn::Agent(a) => {
      let mut v = json!({ "role": "agent", "blocks": a.blocks.iter().filter_map(compact_block).collect::<Vec<_>>() });
      if let Some(stop) = a.stop.filter(|s| *s != TurnStop::EndTurn) {
        v["stop"] = Value::from(enum_str(&stop));
      }
      if let Some(e) = &a.error {
        v["error"] = Value::from(e.message.clone());
      }
      v
    }
  }
}

/// Index of the first turn to keep so the history fits, aligned to a user turn
fn fit_start(items: &[String], turns: &[&Turn], budget: i64, trim: bool) -> Option<usize> {
  let mut total: i64 = 2 + items.iter().map(|s| s.len() as i64 + 1).sum::<i64>();
  if total <= budget {
    return Some(0);
  }
  if !trim {
    return None;
  }
  for (start, item) in items.iter().enumerate() {
    total -= item.len() as i64 + 1;
    let next = start + 1;
    if total <= budget && matches!(turns.get(next), Some(Turn::User(_))) {
      return Some(next);
    }
  }
  None
}

pub struct HistoryContext {
  pub blocks: Vec<Value>,
  pub omitted: usize,
}

/// The retained transcript as prompt context; `trim` keeps only the most recent turns of an oversized history (fork),
/// without it an oversized history yields None (edit falls back to the native session)
pub async fn history_context(
  session_id: &str,
  all: &[Turn],
  proc: &AgentProcess,
  blobs: &TranscriptStore,
  lead: &str,
  caps: PromptCaps,
  trim: bool,
) -> Result<Option<HistoryContext>> {
  let source: Vec<&Turn> = all.iter().filter(|t| !matches!(t, Turn::User(u) if u.auto == Some(true))).collect();
  if source.is_empty() {
    return Ok(Some(HistoryContext { blocks: vec![], omitted: 0 }));
  }
  let items: Vec<String> = source.iter().map(|t| compact_turn(t).to_string()).collect();
  let omit_note = |n: usize| format!("\n{n} earlier turns were omitted to fit the size limit.");
  let budget = EDIT_CONTEXT_MAX_BYTES as i64 - (lead.len() + omit_note(source.len()).len()) as i64 - 1;
  let Some(start) = fit_start(&items, &source, budget, trim) else { return Ok(None) };
  let history = format!("{lead}{}\n[{}]", if start > 0 { omit_note(start) } else { String::new() }, items[start..].join(","));
  let embedded = crate::json::truthy(proc.caps().get("promptCapabilities").and_then(|p| p.get("embeddedContext")));
  let mut context = vec![if embedded {
    json!({ "type": "resource", "resource": { "uri": format!("acpira://history/{session_id}"), "mimeType": "text/plain", "text": history } })
  } else {
    json!({ "type": "text", "text": history })
  }];
  for turn in &source[start..] {
    let Turn::User(u) = turn else { continue };
    let Some(att) = u.attachments.as_ref().filter(|a| !a.is_empty()) else { continue };
    let drafts = restore_drafts(session_id, att, blobs).await?;
    if drafts.len() != att.len() {
      return Err(anyhow!(t("history.missingAttachment")));
    }
    let old = prepare_prompt(session_id, "", &drafts, blobs, Some(caps)).await;
    if !old.problems.is_empty() {
      return Err(anyhow!(old.problems.join("\n")));
    }
    context.push(json!({ "type": "text", "text": format!("Attachments from earlier user message: {}", u.text) }));
    context.extend(old.blocks);
  }
  Ok(Some(HistoryContext { blocks: context, omitted: start }))
}

fn context_length_hint(c: &Core) -> String {
  t(if AcpSession::can_compact_of(c) { "alert.contextLength.text" } else { "alert.contextLength.unsupported" })
}

/// Resending an unchanged message after empty failures / cancellations is a retry on the native context
fn unchanged_failed_retry(c: &Core, edit: &EditTurnRequest) -> bool {
  let turns = &c.state.turns;
  let idx = edit.turn_index as usize;
  let Some(Turn::User(user)) = turns.get(idx) else { return false };
  let n = user.attachments.as_ref().map(Vec::len).unwrap_or(0);
  if user.edited
    || edit.text != user.text
    || !edit.attachments.is_empty()
    || edit.retained_attachments.len() != n
    || edit.retained_attachments.iter().enumerate().any(|(i, v)| *v != i as i64)
  {
    return false;
  }
  let suffix = &turns[idx..];
  if suffix.len() < 2 || !suffix.len().is_multiple_of(2) {
    return false;
  }
  suffix.iter().enumerate().all(|(i, turn)| {
    if i % 2 == 0 {
      matches!(turn, Turn::User(u) if u.auto != Some(true) && !u.edited && u.text == user.text && u.attachments.clone().unwrap_or_default() == user.attachments.clone().unwrap_or_default())
    } else {
      matches!(turn, Turn::Agent(a) if matches!(a.stop, Some(TurnStop::Error | TurnStop::Cancelled)) && a.blocks.is_empty())
    }
  })
}

impl AcpSession {
  fn check_edit_active(&self) -> Result<()> {
    let c = self.core.lock();
    if c.phase.staging_aborted || c.status != SessionStatus::Ready { Err(anyhow!(t("history.cancelled"))) } else { Ok(()) }
  }

  fn is_model_id(controls: &SessionControls, id: &str) -> bool {
    let c = controls.options.iter().find(|c| c.id == id);
    c.and_then(|c| c.category.as_deref()) == Some("model") || (c.is_none_or(|c| c.category.is_none()) && id == "model")
  }

  /// Apply the editor's selections on a native session (live or fresh); only the model is strict
  async fn apply_edit_settings(
    self: &Arc<Self>,
    proc: &AgentProcess,
    session_id: &str,
    controls: &mut SessionControls,
    settings: &TurnSettings,
  ) -> Result<()> {
    let live = self.core.lock().acp_session_id.as_deref() == Some(session_id);
    let mode_id = settings.mode_id.clone();
    if let Some(m) = &mode_id
      && !controls.modes.iter().any(|x| &x.id == m)
    {
      return Err(anyhow!(tp("history.optionUnavailable", &[("name", m)])));
    }
    let mut selections: Vec<(String, String)> = settings.config.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    selections.sort_by_key(|(id, _)| !Self::is_model_id(controls, id));
    let mut settled: HashSet<String> = HashSet::new();
    for (config_id, value) in &selections {
      let ctl = controls.options.iter().find(|c| &c.id == config_id).cloned();
      if !ctl.as_ref().is_some_and(|c| c.options.iter().any(|o| &o.id == value)) {
        if Self::is_model_id(controls, config_id) {
          return Err(anyhow!(tp("history.optionUnavailable", &[("name", config_id)])));
        }
        let keep = ctl.and_then(|c| c.value).unwrap_or_else(|| "no control".into());
        self.log(&format!("edit: {config_id}={value} is not offered after the model switch; keeping {keep}"));
        settled.insert(config_id.clone());
        continue;
      }
      let ctl = ctl.expect("checked");
      if ctl.value.as_ref() == Some(value) {
        continue;
      }
      self.check_edit_active()?;
      let mut params = json!({ "sessionId": session_id, "configId": config_id });
      for (k, v) in config_option_set_value(Some(&ctl), value) {
        params[k] = v;
      }
      let r = proc.request("session/set_config_option", params).await?;
      apply_config_options(controls, r.get("configOptions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]));
      if controls.options.iter().find(|c| &c.id == config_id).and_then(|c| c.value.as_ref()) != Some(value) {
        return Err(anyhow!(tp("history.optionUnavailable", &[("name", config_id)])));
      }
    }
    if let Some(mode) = &mode_id {
      if let Some(mode_config) = controls.mode_config_id.clone() {
        if controls.mode_id.as_ref() != Some(mode) {
          self.check_edit_active()?;
          let r =
            proc.request("session/set_config_option", json!({ "sessionId": session_id, "configId": mode_config, "value": mode })).await?;
          apply_config_options(controls, r.get("configOptions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]));
          if controls.mode_id.as_ref() != Some(mode) {
            return Err(anyhow!(tp("history.optionUnavailable", &[("name", mode)])));
          }
        }
      } else if controls.mode_id.as_ref() != Some(mode) {
        self.check_edit_active()?;
        let wire = if self.synthetic_modes().is_some() && mode == "yolo" { "default" } else { mode.as_str() };
        proc.request("session/set_mode", json!({ "sessionId": session_id, "modeId": wire })).await?;
      }
      controls.mode_id = Some(mode.clone());
      if live && self.synthetic_modes().is_some() {
        self.core.lock().auto_approve = mode == "yolo";
      }
    }
    for (id, value) in &selections {
      if !settled.contains(id) && controls.options.iter().find(|c| &c.id == id).and_then(|c| c.value.as_ref()) != Some(value) {
        return Err(anyhow!(tp("history.optionUnavailable", &[("name", id)])));
      }
    }
    self.check_edit_active()
  }

  /// Replay the notifications parked while editing that still belong to the given native session
  fn replay_edit_notifications(self: &Arc<Self>, notes: Vec<Value>, session: Option<&str>, usage_too: bool) {
    for n in notes {
      let kind = n.get("update").and_then(|u| u.get("sessionUpdate")).and_then(Value::as_str).unwrap_or("");
      let ok = kind == "available_commands_update" || (usage_too && kind == "usage_update");
      if ok && n.get("sessionId").and_then(Value::as_str) == session {
        self.on_update(n);
      }
    }
  }

  /// Commit locally only after attachments, session creation and all selections succeed
  pub async fn edit_turn(self: &Arc<Self>, edit: EditTurnRequest) -> Result<()> {
    let (user, proc) = {
      let mut c = self.core.lock();
      let Some(proc) = c.proc.clone() else { return Err(anyhow!(t("history.unavailable"))) };
      if c.phase.running || c.phase.editing || c.status != SessionStatus::Ready {
        return Err(anyhow!(t("history.unavailable")));
      }
      let idx = edit.turn_index as usize;
      let user = match c.state.turns.get(idx) {
        Some(Turn::User(u)) => u.clone(),
        _ => return Err(anyhow!(t("history.stale"))),
      };
      let previous = idx.checked_sub(1).and_then(|i| c.state.turns.get(i));
      if edit.session_id != self.id
        || edit.turn_count as usize != c.state.turns.len()
        || user.auto == Some(true)
        || plan_execution_id(&Turn::User(user.clone()), previous).is_some()
        || user.text != edit.original_text
        || user.id != edit.turn_id
      {
        return Err(anyhow!(t("history.stale")));
      }
      let kept = &edit.retained_attachments;
      let n = user.attachments.as_ref().map(Vec::len).unwrap_or(0) as i64;
      if kept.iter().collect::<HashSet<_>>().len() != kept.len() || kept.iter().any(|i| *i < 0 || *i >= n) {
        return Err(anyhow!(t("history.stale")));
      }
      if edit.text.trim().is_empty() && kept.is_empty() && edit.attachments.is_empty() {
        return Err(anyhow!(t("history.empty")));
      }
      c.phase.editing = true;
      c.phase.running = true;
      c.phase.staging = true;
      c.phase.edit_notifications.clear();
      c.phase.staging_aborted = false;
      self.bump(&mut c);
      (user, proc)
    };
    let mut accepted = false;
    let result: Result<()> = async {
      let idx = edit.turn_index as usize;
      let prefix: Vec<Turn> = self.core.lock().state.turns[..idx].to_vec();
      let attachments = user.attachments.clone().unwrap_or_default();
      let kept: Vec<Attachment> = edit.retained_attachments.iter().map(|i| attachments[*i as usize].clone()).collect();
      let restored = restore_drafts(&self.id, &kept, &self.deps.blobs).await?;
      if restored.len() != kept.len() {
        return Err(anyhow!(t("history.missingAttachment")));
      }
      let mut drafts = restored;
      drafts.extend(edit.attachments.iter().cloned());
      let caps = self.caps(&self.core.lock());
      let mut prepared = prepare_prompt(&self.id, &edit.text, &drafts, &self.deps.blobs, Some(caps)).await;
      if !prepared.problems.is_empty() {
        return Err(anyhow!(prepared.problems.join("\n")));
      }
      let retry = unchanged_failed_retry(&self.core.lock(), &edit);
      let mut continuing = edit.intent == Some(EditIntent::Continue);
      let mut rebuilt = None;
      if !continuing && !retry && !prefix.is_empty() {
        let history = history_context(&self.id, &prefix, &proc, &self.deps.blobs, EDIT_HISTORY_LEAD, caps, false).await?;
        let blocks = history.map(|h| {
          let mut b = h.blocks;
          b.extend(prepared.blocks.iter().cloned());
          b
        });
        continuing =
          blocks.as_ref().is_none_or(|b| serde_json::to_string(b).map(|s| s.len()).unwrap_or(usize::MAX) > EDIT_CONTEXT_MAX_BYTES);
        if !continuing {
          rebuilt = blocks;
        }
      }
      if continuing || retry {
        {
          let c = self.core.lock();
          if !continuing
            && let Some(Turn::Agent(last)) = c.state.turns.last()
            && is_context_length_error(last.error.as_ref())
          {
            return Err(anyhow!(context_length_hint(&c)));
          }
        }
        let (sid, mut controls) = {
          let c = self.core.lock();
          (c.acp_session_id.clone().unwrap_or_default(), c.state.controls.clone())
        };
        let applied = self.apply_edit_settings(&proc, &sid, &mut controls, &edit.settings).await;
        if applied.is_err() {
          // A later selection failed, yet the earlier ones already landed on the live peer: the view follows that agent truth
          self.core.lock().state.controls = controls.clone();
        }
        applied?;
        self.check_edit_active()?;
        let notes = {
          let mut c = self.core.lock();
          c.state.controls = controls;
          if !continuing {
            c.state.turns.truncate(idx);
            c.tree.truncate(idx);
          }
          c.phase.editing = false;
          c.phase.running = false;
          c.phase.staging = false;
          std::mem::take(&mut c.phase.edit_notifications)
        };
        self.replay_edit_notifications(notes, Some(sid.as_str()), true);
        accepted = true;
        self.log(if continuing {
          "Continuing in the native session without rebuilding history"
        } else {
          "Retrying unchanged failed or cancelled message in the native session"
        });
        let me = self.clone();
        let text = edit.text.clone();
        crate::util::run_prefix(me.prompt(text, drafts, false, Some(Staged { prepared, edited: false }), None));
        return Ok(());
      }
      if let Some(b) = rebuilt {
        prepared.blocks = b;
      }
      let fresh = proc.request("session/new", json!({ "cwd": self.cwd, "mcpServers": [] })).await?;
      let fresh_id = fresh.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
      let mut controls = SessionControls::default();
      init_controls(&mut controls, fresh.get("modes"), fresh.get("configOptions"));
      if controls.modes.is_empty()
        && let Some(syn) = self.synthetic_modes()
      {
        controls.modes = syn;
        controls.mode_id = Some("default".into());
      }
      self.apply_edit_settings(&proc, &fresh_id, &mut controls, &edit.settings).await?;
      let notes = {
        let mut c = self.core.lock();
        c.acp_session_id = Some(fresh_id.clone());
        c.state.controls = controls;
        c.state.turns.truncate(idx);
        c.tree.truncate(idx);
        c.state.usage = None;
        c.state.commands = vec![];
        c.compacted_at = None;
        c.auto_approve = self.synthetic_modes().is_some() && edit.settings.mode_id.as_deref() == Some("yolo");
        c.phase.editing = false;
        c.phase.running = false;
        c.phase.staging = false;
        std::mem::take(&mut c.phase.edit_notifications)
      };
      // Only the new session's command inventory replays, never old content or usage
      self.replay_edit_notifications(notes, Some(fresh_id.as_str()), false);
      accepted = true;
      let me = self.clone();
      let text = edit.text.clone();
      crate::util::run_prefix(me.prompt(text, drafts, false, Some(Staged { prepared, edited: true }), None));
      Ok(())
    }
    .await;
    if !accepted {
      let (notes, sid) = {
        let mut c = self.core.lock();
        c.phase.editing = false;
        c.phase.running = false;
        c.phase.staging = false;
        (std::mem::take(&mut c.phase.edit_notifications), c.acp_session_id.clone())
      };
      self.replay_edit_notifications(notes, sid.as_deref(), true);
      {
        let mut c = self.core.lock();
        self.touch(&mut c);
      }
      self.flush_queue();
    }
    self.core.lock().phase.edit_notifications.clear();
    result
  }

  /// An empty edited failure may not have reached the peer, so its context is rebuilt; once output exists the whole
  /// attempt stays and the same native session continues
  pub async fn retry_turn(self: &Arc<Self>) -> Result<()> {
    let (user, agent, len, turn_id_check, plan_id) = {
      let c = self.core.lock();
      if c.phase.running || c.status != SessionStatus::Ready {
        return Ok(());
      }
      let len = c.state.turns.len();
      let (Some(Turn::User(user)), Some(Turn::Agent(agent))) =
        (len.checked_sub(2).and_then(|i| c.state.turns.get(i)), c.state.turns.last())
      else {
        return Ok(());
      };
      if user.auto == Some(true) {
        return Ok(());
      }
      if is_context_length_error(agent.error.as_ref()) {
        return Err(anyhow!(context_length_hint(&c)));
      }
      if agent.stop.is_none_or(|s| matches!(s, TurnStop::EndTurn | TurnStop::Cancelled)) {
        return Ok(());
      }
      let plan_id = plan_execution_id(&Turn::User(user.clone()), len.checked_sub(3).and_then(|i| c.state.turns.get(i)));
      (user.clone(), agent.clone(), len, agent.started_at, plan_id)
    };
    let has_output = agent.blocks.iter().any(|b| !matches!(b, AgentBlock::Text(x) if x.markdown.trim().is_empty()));
    if user.edited && !has_output {
      let settings = user.settings.clone().unwrap_or_else(|| capture_turn_settings(&self.core.lock().state.controls));
      let n = user.attachments.as_ref().map(Vec::len).unwrap_or(0);
      return self
        .edit_turn(EditTurnRequest {
          session_id: self.id.clone(),
          turn_index: (len - 2) as u64,
          turn_count: len as u64,
          original_text: user.text.clone(),
          turn_id: user.id.clone(),
          text: user.text.clone(),
          attachments: vec![],
          retained_attachments: (0..n as i64).collect(),
          settings,
          intent: None,
        })
        .await;
    }
    let drafts = restore_drafts(&self.id, user.attachments.as_deref().unwrap_or(&[]), &self.deps.blobs).await?;
    {
      // Attachment reads yield; a second click or another send may have claimed the turn
      let mut c = self.core.lock();
      let same = matches!(c.state.turns.last(), Some(Turn::Agent(a)) if a.started_at == turn_id_check && *a == agent);
      if c.phase.running || c.status != SessionStatus::Ready || !same {
        return Ok(());
      }
      if !has_output {
        let keep = c.state.turns.len() - 2;
        c.state.turns.truncate(keep);
        c.tree.truncate(keep);
      }
    }
    self.prompt(user.text, drafts, false, None, plan_id).await;
    Ok(())
  }
}
