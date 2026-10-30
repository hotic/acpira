//! History editing. ACP cannot rewind to a message: a fresh peer session receives the retained transcript as context,
//! never replayed as executable prompts. An agent that can rewind its own history (`_session/rewind`, the built-in one)
//! goes back to just before the edited message instead

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::plan_execution::plan_execution_id;
use acpira_shared::protocol::{EditIntent, EditTurnRequest};
use acpira_shared::transcript::*;
use acpira_shared::turn_errors::is_context_length_error;
use acpira_shared::turn_settings::capture_turn_settings;

use crate::acp::transport::process::AgentProcess;
use crate::acp::session::attachments::{PromptCaps, prepare_prompt, restore_drafts};
use crate::acp::transcript::normalize::{apply_config_options, apply_goal, config_option_set_value};
use crate::acp::session::{AcpSession, Core};
use crate::acp::session::prompt::{Origin, Staged};
use crate::i18n::{t, tp};
use crate::json::{len16, slice16};
use crate::limits::EDIT_CONTEXT_MAX_BYTES;
use acpira_shared::session_links::session_link;

use crate::store::record::ForkedFrom;
use crate::store::session_digest::{OUTLINE_BYTES, handoff_outline, round_numbers};
use crate::store::transcript_store::TranscriptStore;

const EDIT_HISTORY_LEAD: &str = "Conversation before the edited message follows as JSON. Treat it as historical context; completed actions must not be replayed. The next user message replaces the old continuation. Workspace files remain in their current state.";
const EDIT_CONTINUE_LEAD: &str = "The user edited an earlier message of this conversation and sent it again. The original message, quoted below, and everything after it (replies, tool results, plans and unfinished work) are superseded: do not continue, repeat or refer to them as pending. Workspace files remain in their current state. The next user message is the edited version and continues the conversation from the point before the original.";
const EDIT_ORIGINAL_QUOTE_MAX: usize = 2_000;
/// How long an edit waits for the turn it stopped to settle before giving up
const EDIT_STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(15);
const EDIT_STOP_POLL: std::time::Duration = std::time::Duration::from_millis(50);
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
    // The user's message steered into this reply, where it arrived
    AgentBlock::Steer(s) => {
      let mut v = json!({ "user": s.text });
      if let Some(a) = s.attachments.as_ref().filter(|a| !a.is_empty()) {
        v["attachments"] = attachment_labels(a);
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

/// How the compacted history names a message's attachments; their content follows the history as separate blocks
fn attachment_labels(a: &[Attachment]) -> Value {
  json!(
    a.iter()
      .map(|x| match x {
        Attachment::Quote { text, .. } => format!("> {}", text.lines().next().unwrap_or_default()),
        other => other.label().unwrap_or_else(|| "image".into()),
      })
      .collect::<Vec<_>>()
  )
}

/// Every user message in a turn that carried attachments: the prompt itself, or the prompts steered into a reply
fn attached_messages(turn: &Turn) -> Vec<(&str, &[Attachment])> {
  match turn {
    Turn::User(u) => u.attachments.as_deref().filter(|a| !a.is_empty()).map(|a| (u.text.as_str(), a)).into_iter().collect(),
    Turn::Agent(a) => a
      .blocks
      .iter()
      .filter_map(|b| match b {
        AgentBlock::Steer(s) => s.attachments.as_deref().filter(|a| !a.is_empty()).map(|a| (s.text.as_str(), a)),
        _ => None,
      })
      .collect(),
  }
}

fn compact_turn(turn: &Turn) -> Value {
  match turn {
    Turn::User(u) => {
      let mut v = json!({ "role": "user", "text": u.text });
      // Kept among automatic messages because it carries the results summoned agents delivered; marked so the reader
      // does not take it for the user's own words
      if is_delivery(u) {
        v["from"] = Value::from("acpira: summoned agents' replies");
      }
      if let Some(a) = u.attachments.as_ref().filter(|a| !a.is_empty()) {
        v["attachments"] = attachment_labels(a);
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

/// Index of the first item to keep so the history fits, aligned to the start of a turn as `read_session` numbers them
fn fit_start(items: &[String], rounds: &[u32], budget: i64, trim: bool) -> Option<usize> {
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
    if total <= budget && rounds.get(next).is_some_and(|n| *n != rounds[start]) {
      return Some(next);
    }
  }
  None
}

pub struct HistoryContext {
  pub blocks: Vec<Value>,
  /// Turns left out, numbered as `read_session` counts them (the kept history starts at turn `omitted + 1`)
  pub omitted: u32,
}

/// How a history goes out: the lead it opens with, the fork it may have come from (another agent's turns get their
/// own note), the session's folder for the outline's paths, and whether an oversized history is trimmed (a fork's
/// first prompt) or refused (an edit, which then continues natively)
pub struct Handoff<'a> {
  pub lead: &'a str,
  pub fork: Option<&'a ForkedFrom>,
  pub cwd: &'a str,
  pub trim: bool,
  /// The agent can call Acpira's read_session on this session: the history says how to read what it clips or leaves out
  pub readable: bool,
}

/// How an agent that can read its own session gets back what the handoff clipped or left out
fn read_pointer(session_id: &str) -> String {
  let link = session_link(session_id);
  format!(
    "This conversation is the Acpira session {link}. read_session {{session: \"{link}\", from, to, detail: \"full\"}} returns any of its turns in full, tool output included; search_sessions {{session: \"{link}\", query}} finds text in it."
  )
}

/// The lead with the notes a fork's history needs. Turns another agent ran come with that agent's tool names and an
/// older view of the files. A history that ends on the failed reply it was forked from says how far that reply got: no
/// answer at all leaves the last message to take over, a partial one left work to check first. Any other history keeps
/// the lead as it is
fn handoff_lead(lead: &str, fork: Option<&ForkedFrom>, all: &[Turn], rounds: &[u32]) -> String {
  let Some(fork) = fork else { return lead.to_owned() };
  let mut out = lead.to_owned();
  if let Some(agent) = fork.agent.as_deref()
    && let Some(copied) = usize::try_from(fork.turn_index).ok().and_then(|i| rounds.get(i.min(rounds.len().saturating_sub(1))))
  {
    let who = format!("{agent}{}", fork.model.as_deref().map(|m| format!(", model {m}")).unwrap_or_default());
    let span = if rounds.last() == Some(copied) { "All of it".to_owned() } else { format!("Turns 1-{copied} of it") };
    out.push_str(&format!(
      " {span} ran on another agent ({who}), not on you: the tool names in it are that agent's and may not exist here, so use your own tools. Files may have changed since; re-read a file before editing it."
    ));
  }
  let tail = usize::try_from(fork.turn_index).ok().filter(|i| i + 1 == all.len()).and_then(|i| all[i].as_agent());
  if let Some(a) = tail.filter(|a| a.stop == Some(TurnStop::Error) || a.error.is_some()) {
    let answered = a.blocks.iter().any(|b| match b {
      AgentBlock::Text(x) => !x.markdown.trim().is_empty(),
      AgentBlock::Notice(_) | AgentBlock::Goal(_) => false,
      _ => true,
    });
    out.push_str(if answered {
      " Its last reply failed partway through: check the actions it lists, and the workspace, before carrying that request on."
    } else {
      " Its last message got no reply, the agent failed before answering: that request is still open."
    });
  }
  out
}

/// The retained transcript as prompt context. An oversized history is either trimmed to its most recent turns behind an
/// outline of the rest (`trim`) or yields None. An earlier message's attachments that are gone or that this agent cannot
/// take are named instead of sent: one stale image does not void the whole history
pub async fn history_context(
  session_id: &str,
  all: &[Turn],
  proc: &AgentProcess,
  blobs: &TranscriptStore,
  files: &crate::acp::session::attachments::FileReader,
  caps: PromptCaps,
  spec: Handoff<'_>,
) -> Result<Option<HistoryContext>> {
  let numbers = round_numbers(all);
  let (source, rounds): (Vec<&Turn>, Vec<u32>) =
    all.iter().zip(numbers.iter().copied()).filter(|(t, _)| !matches!(t, Turn::User(u) if u.auto == Some(true) && !is_delivery(u))).unzip();
  if source.is_empty() {
    return Ok(Some(HistoryContext { blocks: vec![], omitted: 0 }));
  }
  let lead = handoff_lead(spec.lead, spec.fork, all, &numbers);
  let items: Vec<String> = source.iter().map(|t| compact_turn(t).to_string()).collect();
  let omit_note = |n: u32| format!("\n{n} earlier turns were omitted to fit the size limit.");
  let last = rounds.last().copied().unwrap_or(0);
  let pointer = if spec.readable { read_pointer(session_id).len() + 1 } else { 0 };
  let budget = EDIT_CONTEXT_MAX_BYTES as i64 - (lead.len() + omit_note(last).len() + pointer) as i64 - 1;
  let (start, outline) = match fit_start(&items, &rounds, budget, false) {
    Some(start) => (start, None),
    None if !spec.trim => return Ok(None),
    // Trimmed: the outline of what was left out takes its share of the budget first
    None => {
      let room = budget - OUTLINE_BYTES as i64 - 2;
      let Some(start) = fit_start(&items, &rounds, room, true) else { return Ok(None) };
      (start, Some(handoff_outline(all, spec.cwd, rounds[start] - 1, None)))
    }
  };
  let omitted = if start > 0 { rounds[start] - 1 } else { 0 };
  let mut history = lead;
  if omitted > 0 {
    history.push_str(&omit_note(omitted));
  }
  if spec.readable {
    history.push('\n');
    history.push_str(&read_pointer(session_id));
  }
  if let Some(o) = &outline {
    history.push_str("\n\n");
    history.push_str(o.trim_end());
    history.push_str("\n\nThe most recent turns follow as JSON.");
  }
  history.push_str(&format!("\n[{}]", items[start..].join(",")));
  let mut context = vec![context_block(proc, &format!("acpira://history/{session_id}"), history)];
  for (text, att) in source[start..].iter().flat_map(|t| attached_messages(t)) {
    let drafts = restore_drafts(session_id, att, blobs).await.ok().filter(|d| d.len() == att.len());
    let old = match drafts {
      Some(d) => Some(prepare_prompt(session_id, "", &d, blobs, files, Some(caps)).await).filter(|p| p.problems.is_empty()),
      None => None,
    };
    match old {
      Some(old) => {
        context.push(json!({ "type": "text", "text": format!("Attachments from earlier user message: {text}") }));
        context.extend(old.blocks);
      }
      None => {
        let names: Vec<String> = attachment_labels(att).as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        context.push(json!({ "type": "text", "text": format!("Attachments from earlier user message: {text} (not included: {})", names.join(", ")) }));
      }
    }
  }
  Ok(Some(HistoryContext { blocks: context, omitted }))
}

/// Acpira's own extension: the agent cuts its history back to just before a turn named in that turn's `session/prompt`
/// (`_meta["acpira/turn"]`); `dryRun` only asks whether it can
const REWIND_METHOD: &str = "_session/rewind";

/// Advertised at initialize (`_meta["acpira/rewind"].supported`); the built-in agent does
pub(crate) fn rewinds(init: &Value) -> bool {
  init.pointer("/_meta/acpira~1rewind/supported").and_then(Value::as_bool) == Some(true)
}

/// Host-written context for the model: an embedded resource when the agent takes one, plain text otherwise
fn context_block(proc: &AgentProcess, uri: &str, text: String) -> Value {
  let embedded = crate::json::truthy(proc.caps().get("promptCapabilities").and_then(|p| p.get("embeddedContext")));
  if embedded {
    json!({ "type": "resource", "resource": { "uri": uri, "mimeType": "text/plain", "text": text } })
  } else {
    json!({ "type": "text", "text": text })
  }
}

/// The note an edit continuing in the native session sends ahead of the edited message: the peer still holds the original
/// and everything after it, while the transcript has replaced them
fn superseded_note(original: &str) -> String {
  format!("{EDIT_CONTINUE_LEAD}\n\nOriginal message:\n{}", clip(original, EDIT_ORIGINAL_QUOTE_MAX))
}

/// Bytes a rebuilt prompt spends on serialized history and text; image pixels are left out, since they are sent the same way
/// in an ordinary prompt and a single screenshot would otherwise push every edit onto the native-continue path
fn rebuilt_text_bytes(blocks: &[Value]) -> usize {
  blocks
    .iter()
    .filter(|b| b.get("type").and_then(Value::as_str) != Some("image"))
    .map(|b| serde_json::to_string(b).map(|s| s.len()).unwrap_or(usize::MAX))
    .fold(0, usize::saturating_add)
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
  /// Whether this session's agent can read the session back through Acpira's MCP server: the last session request
  /// carried it, the agent has not refused it since, and the agent launches handed-over servers at all (Pi takes the
  /// list and starts none of it)
  pub(crate) fn reads_itself(&self) -> bool {
    let sent = self.core.lock().mcp_sent.iter().any(|n| n == crate::host_mcp::SERVER_NAME);
    sent
      && self.deps.host_mcp.as_ref().is_some_and(|h| !h.refused(&self.agent))
      && crate::agent_ext::agent_ext(&self.agent).is_none_or(|e| e.shared.mcp)
  }

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
    // A session without modes has none to restore; turns recorded while pi-acp's mirrored modes leaked still carry one
    let mode_id = settings.mode_id.clone().filter(|_| !controls.modes.is_empty());
    if let Some(m) = &mode_id
      && !controls.modes.iter().any(|x| &x.id == m)
    {
      return Err(anyhow!(tp("history.optionUnavailable", &[("name", m)])));
    }
    // Claude's Ultra is no effort value of the agent: its wire target goes out (or nothing, when the select has none) and
    // the session request carries ultracode
    let mut selections: Vec<(String, String)> = settings
      .config
      .iter()
      .filter_map(|(k, v)| {
        if !self.vendor.ultracode() || v != crate::acp::vendors::claude_ultracode::LEVEL_ID {
          return Some((k.clone(), v.clone()));
        }
        crate::acp::vendors::claude_ultracode::wire_value(&controls.options, k, v).map(|wire| (k.clone(), wire))
      })
      .collect();
    selections.sort_by_key(|(id, _)| !Self::is_model_id(controls, id));
    let mut settled: HashSet<String> = HashSet::new();
    for (config_id, value) in &selections {
      let ctl = match controls.options.iter().find(|c| &c.id == config_id).cloned() {
        Some(ctl) if ctl.options.iter().any(|o| &o.id == value) => ctl,
        ctl => {
          if Self::is_model_id(controls, config_id) {
            return Err(anyhow!(tp("history.optionUnavailable", &[("name", config_id)])));
          }
          let keep = ctl.and_then(|c| c.value).unwrap_or_else(|| "no control".into());
          self.log(&format!("edit: {config_id}={value} is not offered after the model switch; keeping {keep}"));
          settled.insert(config_id.clone());
          continue;
        }
      };
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
        self.core.lock().perms.auto_approve = mode == "yolo";
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

  /// An edit may come while the agent still answers (any agent: it is a stop plus the ordinary edit): the running turn is
  /// stopped and settles as cancelled first, then the edit replaces it like any earlier turn
  pub async fn edit_turn(self: &Arc<Self>, edit: EditTurnRequest) -> Result<()> {
    self.stop_for_edit().await?;
    let result = self.edit_turn_now(edit).await;
    // The edit never took the session: the queue flush the stop held back goes now
    if std::mem::take(&mut self.core.lock().phase.edit_waiting) {
      self.flush_queue();
    }
    result
  }

  async fn stop_for_edit(self: &Arc<Self>) -> Result<()> {
    {
      let mut c = self.core.lock();
      if !c.phase.running {
        return Ok(());
      }
      if c.phase.editing || c.phase.edit_waiting || c.status != SessionStatus::Ready {
        return Err(anyhow!(t("history.unavailable")));
      }
      c.phase.edit_waiting = true;
    }
    self.log("edit: stopping the running turn first");
    self.cancel().await;
    // `session/cancel` is a notification; the turn is over once the prompt response settles it
    let deadline = tokio::time::Instant::now() + EDIT_STOP_WAIT;
    loop {
      if !self.core.lock().phase.running {
        return Ok(());
      }
      if tokio::time::Instant::now() >= deadline {
        self.core.lock().phase.edit_waiting = false;
        self.flush_queue();
        return Err(anyhow!(t("history.unavailable")));
      }
      tokio::time::sleep(EDIT_STOP_POLL).await;
    }
  }

  /// Commit locally only after attachments, session creation and all selections succeed
  async fn edit_turn_now(self: &Arc<Self>, edit: EditTurnRequest) -> Result<()> {
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
      if let Err(reason) = self.lease_turn(&mut c) {
        return Err(anyhow!(reason));
      }
      c.phase.editing = true;
      c.phase.running = true;
      c.phase.staging = true;
      // The edit owns the session from here; its own end (sent prompt or rollback) flushes the queue
      c.phase.edit_waiting = false;
      c.phase.edit_notifications.clear();
      c.phase.staging_aborted = false;
      self.bump(&mut c);
      (user, proc)
    };
    let mut accepted = false;
    let mut ultracode_before = None;
    let result: Result<()> = async {
      let idx = edit.turn_index as usize;
      let prefix: Vec<Turn> = self.core.lock().state.turns.get(..idx).ok_or_else(|| anyhow!(t("history.stale")))?.to_vec();
      let attachments = user.attachments.clone().unwrap_or_default();
      let kept: Vec<Attachment> = edit.retained_attachments.iter().map(|i| attachments[*i as usize].clone()).collect();
      let restored = restore_drafts(&self.id, &kept, &self.deps.blobs).await?;
      if restored.len() != kept.len() {
        return Err(anyhow!(t("history.missingAttachment")));
      }
      let mut drafts = restored;
      drafts.extend(edit.attachments.iter().cloned());
      let caps = self.caps(&self.core.lock());
      let files = self.file_reader(Some(&drafts)).await;
      let mut prepared = prepare_prompt(&self.id, &edit.text, &drafts, &self.deps.blobs, &files, Some(caps)).await;
      if !prepared.problems.is_empty() {
        return Err(anyhow!(prepared.problems.join("\n")));
      }
      // A fork whose history has not gone out yet: its native session holds nothing to rebuild or supersede, so the
      // edit moves the fork point and the prompt hands the shortened prefix over as the fork's first message.
      // `stale`: the history is owed again after a first prompt that ended without output (`reclaim_fork_history`), yet
      // that prompt went out and the peer may hold it, so a fresh native session takes the handoff instead
      let (pending, stale) = {
        let c = self.core.lock();
        (c.lineage.history_pending && c.unprompted, c.lineage.history_pending && !c.unprompted)
      };
      let retry = !stale && unchanged_failed_retry(&self.core.lock(), &edit);
      let mut continuing = !stale && edit.intent == Some(EditIntent::Continue);
      // An agent that rewinds its own history (the built-in one) goes back to just before the edited message the way the
      // transcript does, so the edit is a plain prompt with nothing to rebuild or supersede. A turn it holds no mark of
      // (sent before marks existed, or on an earlier native session) takes the paths below
      let rewind_to = match user.id.clone() {
        Some(turn_id) if !continuing && !pending && !stale && rewinds(&proc.init) => {
          let sid = self.core.lock().acp_session_id.clone().unwrap_or_default();
          match proc.request(REWIND_METHOD, json!({ "sessionId": sid, "turnId": turn_id, "dryRun": true })).await {
            Ok(_) => Some(turn_id),
            Err(e) => {
              self.log(&format!("edit: no native rewind to this turn ({}), falling back", e.message));
              None
            }
          }
        }
        _ => None,
      };
      let mut rebuilt = None;
      if rewind_to.is_none() && !continuing && !retry && !pending && !stale && !prefix.is_empty() {
        let (fork, cwd) = {
          let c = self.core.lock();
          (c.lineage.forked_from.clone(), self.cwd.clone())
        };
        let readable = self.reads_itself();
        let spec = Handoff { lead: EDIT_HISTORY_LEAD, fork: fork.as_ref(), cwd: &cwd, trim: false, readable: readable && fork.is_some() };
        let files = self.file_reader(None).await;
        let history = history_context(&self.id, &prefix, &proc, &self.deps.blobs, &files, caps, spec).await?;
        let blocks = history.map(|h| {
          let mut b = h.blocks;
          b.extend(prepared.blocks.iter().cloned());
          b
        });
        continuing = blocks.as_ref().is_none_or(|b| rebuilt_text_bytes(b) > EDIT_CONTEXT_MAX_BYTES);
        if !continuing {
          rebuilt = blocks;
        }
      }
      if rewind_to.is_some() || continuing || retry || pending {
        {
          let c = self.core.lock();
          if retry
            && !continuing
            && !pending
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
        let mut rewound_goal = None;
        // The settings went first so a refused one leaves the native history whole; the rewind is the last step that can
        // fail, and from its success on the transcript has to follow
        if let Some(turn_id) = &rewind_to {
          match proc.request(REWIND_METHOD, json!({ "sessionId": sid, "turnId": turn_id })).await {
            Ok(r) => rewound_goal = crate::acp::vendors::goal::snapshot_of(&r),
            Err(e) => {
              self.core.lock().state.controls = controls;
              return Err(anyhow!(e.message));
            }
          }
        }
        // Both replace the edited turn and what followed it on screen; a continuing edit tells the peer, which still holds
        // the original, that it is superseded, so the transcript and the model agree on where the conversation resumes
        if continuing && !pending {
          prepared.blocks.insert(0, context_block(&proc, &format!("acpira://edit/{}", self.id), superseded_note(&user.text)));
        }
        let notes = {
          let mut c = self.core.lock();
          c.state.controls = controls;
          if rewind_to.is_some() {
            // The context shrank with the history; the next call reports it again
            c.state.usage = None;
            // The goal went back with it (the agent's answer: the updates are held while editing), without a milestone
            // row, since the rows of the cut turns go too
            if let Some(goal) = rewound_goal.take() {
              apply_goal(&mut c.state, goal, false);
            }
            // Back to before the first message, the agent holds none: the edited one is its first again and carries
            // what a first prompt does (the workspace hooks' context files)
            if idx == 0 {
              c.unprompted = true;
            }
          }
          c.state.turns.truncate(idx);
          c.lineage.truncated(idx);
          self.relay_truncate(&mut c, idx);
          // The edited turn's ultracode: the prompt below rebuilds the native session with it first when it differs
          // from what that session was built with (`claim`)
          self.seed_ultracode(&mut c, Some(&edit.settings));
          c.phase.editing = false;
          c.phase.running = false;
          c.phase.staging = false;
          std::mem::take(&mut c.phase.edit_notifications)
        };
        self.replay_edit_notifications(notes, Some(sid.as_str()), true);
        accepted = true;
        self.log(if rewind_to.is_some() {
          "Rewound the native session to before the edited message"
        } else if pending {
          "Edit before the fork's history went out: the prompt hands over the shortened prefix"
        } else if continuing {
          "Continuing in the native session without rebuilding history"
        } else {
          "Retrying unchanged failed or cancelled message in the native session"
        });
        let me = self.clone();
        let text = edit.text.clone();
        let edited = (continuing && !pending) || (pending && !retry) || (rewind_to.is_some() && !retry);
        crate::util::run_prefix(me.prompt(text, drafts, false, Some(Staged { prepared, edited, id: None }), None));
        return Ok(());
      }
      if let Some(b) = rebuilt {
        prepared.blocks = b;
      }
      // The fresh session is built with the edited turn's ultracode; the live one keeps its own if the edit fails
      {
        let mut c = self.core.lock();
        ultracode_before = Some((c.ultracode.on, c.ultracode.sent));
        self.seed_ultracode(&mut c, Some(&edit.settings));
      }
      let req = self.session_request(&proc, None).await;
      let fresh = proc.request("session/new", req).await?;
      let fresh_id = fresh.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
      let mut controls = SessionControls::default();
      if !self.protocol_controls(&mut controls, fresh.get("modes"), fresh.get("configOptions"))
        && controls.modes.is_empty()
        && let Some(syn) = self.synthetic_modes()
      {
        controls.modes = syn;
        controls.mode_id = Some("default".into());
      }
      self.apply_edit_settings(&proc, &fresh_id, &mut controls, &edit.settings).await?;
      let notes = {
        let mut c = self.core.lock();
        c.acp_session_id = Some(fresh_id.clone());
        c.unprompted = true;
        c.state.controls = controls;
        c.state.turns.truncate(idx);
        c.lineage.truncated(idx);
        self.relay_truncate(&mut c, idx);
        c.state.usage = None;
        c.state.commands = vec![];
        c.compaction.at = None;
        c.perms.auto_approve = self.synthetic_modes().is_some() && edit.settings.mode_id.as_deref() == Some("yolo");
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
      crate::util::run_prefix(me.prompt(text, drafts, false, Some(Staged { prepared, edited: true, id: None }), None));
      Ok(())
    }
    .await;
    if !accepted {
      let (notes, sid) = {
        let mut c = self.core.lock();
        if let Some((on, sent)) = ultracode_before {
          c.ultracode.on = on;
          c.ultracode.sent = sent;
        }
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

  /// Retry the last turn that stopped on an error. Once the attempt produced output the peer already holds the message and
  /// the work done so far, so the attempt stays and a hidden continue prompt carries the task on (sending the message
  /// again would have the agent start over on top of its own partial work). An empty attempt may never have reached the
  /// peer: it is dropped and the message sent again, an empty edited failure rebuilding its context. A continue that fails
  /// is retried with another continue. Retrying a turn a Retry itself opened restarts the connection first (`reconnect`:
  /// a live process that keeps failing on its old session, Grok's -32603, needs a fresh one), so a single Retry covers both
  pub async fn retry_turn(self: &Arc<Self>) -> Result<()> {
    let (user, agent, len, turn_id_check, plan_id, again) = {
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
      // Only the continues (Retry's own, an account switch's, the gate's) and a delivery of summoned agents' replies are
      // retried among automatic turns; /compact is not
      if user.auto == Some(true) && !is_continue(user) && !is_delivery(user) {
        return Ok(());
      }
      if is_context_length_error(agent.error.as_ref()) {
        return Err(anyhow!(context_length_hint(&c)));
      }
      if agent.stop.is_none_or(|s| matches!(s, TurnStop::EndTurn | TurnStop::Cancelled)) {
        return Ok(());
      }
      let plan_id = plan_execution_id(&Turn::User(user.clone()), len.checked_sub(3).and_then(|i| c.state.turns.get(i)));
      let again = agent.started_at.is_some() && c.retried_turn == agent.started_at;
      (user.clone(), agent.clone(), len, agent.started_at, plan_id, again)
    };
    let has_output = agent.blocks.iter().any(|b| !matches!(b, AgentBlock::Text(x) if x.markdown.trim().is_empty()));
    let continuing = has_output || is_continue(&user);
    if again {
      self.log("retry: the retried turn failed again, reconnecting first");
      self.reconnect().await?;
      if self.status() != SessionStatus::Ready {
        return Ok(());
      }
    }
    if continuing {
      {
        let mut c = self.core.lock();
        let same = matches!(c.state.turns.last(), Some(Turn::Agent(a)) if a.started_at == turn_id_check && *a == agent);
        if c.phase.running || c.status != SessionStatus::Ready || !same {
          return Ok(());
        }
        // A continue that failed before doing anything leaves no trace; the turn it continues keeps its own error row
        if !has_output {
          let keep = c.state.turns.len() - 2;
          c.state.turns.truncate(keep);
          self.relay_truncate(&mut c, keep);
        }
        c.retry_pending = true;
      }
      // A goal's next turn that failed before doing anything goes out again as itself, so the agent runs it as a goal turn
      let (text, origin) = if user.is_goal_continue() && !has_output { (user.text.clone(), Origin::GoalContinue) } else { (t("host.retryContinuePrompt"), Origin::Retry) };
      self.clone().prompt_inner(text, vec![], origin, None, None).await;
      self.core.lock().retry_pending = false;
      return Ok(());
    }
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
    // A delivery of summoned agents' replies that failed before any output goes out again as the same automatic message:
    // its rounds were marked delivered when it was claimed, so nothing else would bring their results back
    if is_delivery(&user) {
      {
        let mut c = self.core.lock();
        let same = matches!(c.state.turns.last(), Some(Turn::Agent(a)) if a.started_at == turn_id_check && *a == agent);
        // The claim takes a delivery only into a free session with an empty queue: checked first, so a refused resend
        // never drops the failed turn
        if c.phase.running || c.status != SessionStatus::Ready || !same || !c.queue.entries.is_empty() || c.pending_prompt.is_some() {
          return Ok(());
        }
        let keep = c.state.turns.len() - 2;
        c.state.turns.truncate(keep);
        self.relay_truncate(&mut c, keep);
        c.retry_pending = true;
      }
      self.clone().prompt_inner(user.text, vec![], Origin::Subagents, None, None).await;
      self.core.lock().retry_pending = false;
      return Ok(());
    }
    let drafts = restore_drafts(&self.id, user.attachments.as_deref().unwrap_or(&[]), &self.deps.blobs).await?;
    {
      // Attachment reads yield; a second click or another send may have claimed the turn
      let mut c = self.core.lock();
      let same = matches!(c.state.turns.last(), Some(Turn::Agent(a)) if a.started_at == turn_id_check && *a == agent);
      if c.phase.running || c.status != SessionStatus::Ready || !same {
        return Ok(());
      }
      let keep = c.state.turns.len() - 2;
      c.state.turns.truncate(keep);
      self.relay_truncate(&mut c, keep);
      c.retry_pending = true;
    }
    self.prompt(user.text, drafts, false, None, plan_id).await;
    self.core.lock().retry_pending = false;
    Ok(())
  }
}

/// The automatic message delivering summoned agents' replies to the root
fn is_delivery(user: &UserTurn) -> bool {
  user.auto == Some(true) && user.auto_reason == Some(AutoReason::Subagents)
}

/// A hidden turn that asks the agent to carry on from its own context: Retry's, the one after an account switch, the
/// workspace gate's findings, or an active goal's next turn
fn is_continue(user: &UserTurn) -> bool {
  (user.auto == Some(true) && matches!(user.auto_reason, Some(AutoReason::Retry | AutoReason::AccountSwitch | AutoReason::Gate))) || user.is_goal_continue()
}
