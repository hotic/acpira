//! ChatGPT event mirror reducer: a pure transactional reducer, the
//! caller owns the file lock and persists only on success

use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use acpira_shared::transcript::*;

use crate::acp::diff::diff_lines;
use crate::json::{len16, slice16};
use crate::util::{iso_of_ms, ms_of_iso};

pub const CHATGPT_ID: &str = "chatgpt";
pub const STALE_AFTER_MS: i64 = 45_000;
pub const OUTPUT_LIMIT: usize = 256_000;
const EVENT_LIMIT: usize = 2_000_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptRecord {
  pub version: u32,
  pub id: String,
  pub source_key: String,
  pub title: String,
  pub cwd: String,
  pub created_at: String,
  pub updated_at: String,
  pub last_event_at: String,
  pub revision: i64,
  pub turns: Vec<Turn>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub active_turn_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub active_event_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pinned: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub deleted_at: Option<i64>,
  pub receipts: Map<String, Value>,
}

pub fn chatgpt_session_id(key: &str) -> Result<String> {
  if key.trim().is_empty() || key.encode_utf16().count() > 512 {
    bail!("A nonempty source session key (at most 512 characters) is required");
  }
  let digest = Sha256::digest(key.as_bytes());
  Ok(format!("chatgpt-{}", digest.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>()))
}

static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^chatgpt-[a-f0-9]{32}$").unwrap());
static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w.-]{1,160}$").unwrap());

pub fn is_chatgpt_id(id: &str) -> bool {
  ID_RE.is_match(id)
}

fn text<'a>(e: &'a Map<String, Value>, field: &str) -> Result<&'a str> {
  e.get(field).and_then(Value::as_str).ok_or_else(|| anyhow!("Expected string: {field}"))
}

fn token<'a>(e: &'a Map<String, Value>, field: &str, label: &str) -> Result<&'a str> {
  match e.get(field).and_then(Value::as_str) {
    Some(v) if TOKEN_RE.is_match(v) && !["__proto__", "constructor", "prototype"].contains(&v) => Ok(v),
    _ => Err(anyhow!("Invalid {label}")),
  }
}

/// The validated event object, as received
pub fn parse_chatgpt_event(value: &Value) -> Result<Map<String, Value>> {
  let e = value.as_object().ok_or_else(|| anyhow!("Expected an event object"))?;
  if serde_json::to_string(e)?.len() > EVENT_LIMIT {
    bail!("Event exceeds 2 MB; split output into chunks");
  }
  token(e, "id", "event id")?;
  token(e, "turnId", "turn id")?;
  match e.get("type").and_then(Value::as_str) {
    Some("turn_start") => {
      text(e, "text")?;
      if e.contains_key("previousTurnId") {
        token(e, "previousTurnId", "previous turn id")?;
      }
    }
    Some("turn_resume" | "heartbeat") => {}
    Some("message") => {
      token(e, "messageId", "message id")?;
      text(e, "text")?;
      if !matches!(e.get("phase").and_then(Value::as_str), Some("commentary" | "final")) {
        bail!("Only visible commentary/final messages may be mirrored");
      }
    }
    Some("tool_start") => {
      token(e, "callId", "call id")?;
      let name = text(e, "name")?;
      let kind_ok = e.get("kind").and_then(Value::as_str).and_then(ToolKind::parse).is_some();
      if name.is_empty() || !kind_ok {
        bail!("Invalid tool name or kind");
      }
      if e.contains_key("target") {
        text(e, "target")?;
      }
    }
    Some("tool_output") => {
      token(e, "callId", "call id")?;
      text(e, "text")?;
    }
    Some("tool_end") => {
      token(e, "callId", "call id")?;
      if !matches!(e.get("status").and_then(Value::as_str), Some("completed" | "failed" | "cancelled")) {
        bail!("Invalid tool status");
      }
      if e.contains_key("detail") {
        text(e, "detail")?;
      }
      if let Some(d) = e.get("diff") {
        let d = d.as_object().ok_or_else(|| anyhow!("Expected an event object"))?;
        text(d, "path")?;
        text(d, "oldText")?;
        text(d, "newText")?;
      }
    }
    Some("turn_end") => {
      if e.get("stop").and_then(Value::as_str).and_then(TurnStop::parse).is_none() {
        bail!("Invalid turn stop");
      }
    }
    _ => bail!("Unsupported ChatGPT bridge event"),
  }
  Ok(e.clone())
}

fn active_turn<'a>(r: &'a mut ChatGptRecord, id: &str) -> Result<&'a mut AgentTurn> {
  let active = r.active_turn_id.as_deref() == Some(id);
  match r.turns.last_mut() {
    Some(Turn::Agent(t)) if active && t.stop.is_none() => Ok(t),
    _ => Err(anyhow!("Turn is not active; open it with turn_start first")),
  }
}

fn tool<'a>(turn: &'a mut AgentTurn, id: &str) -> Result<&'a mut ToolCallBlock> {
  turn
    .blocks
    .iter_mut()
    .find_map(|b| match b {
      AgentBlock::ToolCall(tc) if tc.id == id => Some(tc),
      _ => None,
    })
    .ok_or_else(|| anyhow!("Unknown tool call: {id}"))
}

/// Some(next) when the event changed the record; None for an idempotent retry
pub fn apply_chatgpt_event(record: &ChatGptRecord, value: &Value, now: i64) -> Result<Option<ChatGptRecord>> {
  let e = parse_chatgpt_event(value)?;
  if record.deleted_at.is_some() {
    bail!("This mirror was deleted; refusing to recreate it");
  }
  let digest: String = Sha256::digest(serde_json::to_string(&e)?.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
  let id = e["id"].as_str().unwrap().to_owned();
  if let Some(prev) = record.receipts.get(&id) {
    if prev.as_str() != Some(digest.as_str()) {
      bail!("Event ID was reused with different content");
    }
    return Ok(None);
  }
  if record.receipts.len() >= 100_000 {
    bail!("Mirror event limit reached; start a new mirror");
  }
  let mut r = record.clone();
  let at = iso_of_ms(now);
  let turn_id = e["turnId"].as_str().unwrap().to_owned();
  let s = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
  match e["type"].as_str().unwrap() {
    "turn_start" => {
      let text = s("text").unwrap();
      if let Some(existing) = r.turns.iter().find_map(|t| t.as_user().filter(|u| u.id.as_deref() == Some(turn_id.as_str()))) {
        if existing.text != text {
          bail!("Turn ID reused with different text");
        }
        return Ok(None);
      }
      let previous = s("previousTurnId");
      if r.active_turn_id.is_some() && previous != r.active_turn_id {
        bail!("Finish the active turn before starting another, or explicitly name --previous-turn");
      }
      if previous.is_some() && previous != r.active_turn_id {
        bail!("Previous turn changed; refresh before continuing");
      }
      r.turns.push(Turn::User(UserTurn { id: Some(turn_id.clone()), text, ..Default::default() }));
      r.turns.push(Turn::Agent(AgentTurn { started_at: Some(now), ..Default::default() }));
      r.active_turn_id = Some(turn_id.clone());
      r.active_event_at = Some(at.clone());
      r.updated_at = at.clone();
    }
    "turn_resume" => {
      let index = r.turns.iter().position(|t| t.as_user().is_some_and(|u| u.id.as_deref() == Some(turn_id.as_str())));
      let ok = index.is_some_and(|i| i + 2 == r.turns.len() && matches!(r.turns.get(i + 1), Some(Turn::Agent(a)) if a.stop.is_none()));
      if !ok {
        bail!("Only the latest unfinished turn can resume");
      }
      if r.active_turn_id.as_ref().is_some_and(|a| *a != turn_id) {
        bail!("Another turn is active");
      }
      r.active_turn_id = Some(turn_id.clone());
      r.active_event_at = Some(at.clone());
    }
    kind => {
      // A command already running can report its result after the next user turn: route by turn identity
      let late = matches!(kind, "tool_output" | "tool_end" | "turn_end" | "heartbeat");
      let index =
        if late { r.turns.iter().position(|t| t.as_user().is_some_and(|u| u.id.as_deref() == Some(turn_id.as_str()))) } else { None };
      let late_turn = index.filter(|i| matches!(r.turns.get(i + 1), Some(Turn::Agent(_)))).map(|i| i + 1);
      let active_id = r.active_turn_id.clone();
      let turn: &mut AgentTurn = match late_turn {
        Some(i) => r.turns[i].as_agent_mut().unwrap(),
        None => active_turn(&mut r, &turn_id)?,
      };
      match kind {
        "message" => {
          let message_id = s("messageId").unwrap();
          let phase = if s("phase").as_deref() == Some("final") { TextPhase::Final } else { TextPhase::Commentary };
          let text = s("text").unwrap();
          match turn.blocks.iter_mut().find_map(|b| match b {
            AgentBlock::Text(t) if t.id.as_deref() == Some(message_id.as_str()) => Some(t),
            _ => None,
          }) {
            Some(existing) => {
              existing.markdown = text;
              existing.phase = Some(phase);
            }
            None => {
              turn.blocks.push(AgentBlock::Text(TextBlock { id: Some(message_id), phase: Some(phase), markdown: text, streaming: None }))
            }
          }
        }
        "tool_start" => {
          let call_id = s("callId").unwrap();
          if turn.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.id == call_id)) {
            bail!("Tool call ID already exists");
          }
          let input = match e.get("input") {
            None => String::new(),
            Some(v) => format!("Input:\n{}\n\nOutput:\n", crate::json::pretty(v)),
          };
          turn.blocks.push(AgentBlock::ToolCall(ToolCallBlock {
            kind: e.get("kind").and_then(Value::as_str).and_then(ToolKind::parse).unwrap(),
            verb: s("name").unwrap(),
            target: s("target"),
            target_mono: Some(true),
            status: ToolStatus::InProgress,
            started_at: Some(now),
            content: Some(ToolContent::Text { text: input }),
            ..crate::acp::normalize::empty_tool(call_id)
          }));
        }
        "tool_output" => {
          let b = tool(turn, &s("callId").unwrap())?;
          if b.status != ToolStatus::InProgress {
            bail!("Tool is already settled");
          }
          let previous = match &b.content {
            Some(ToolContent::Text { text }) => text.clone(),
            _ => String::new(),
          };
          if len16(&previous) < OUTPUT_LIMIT {
            let joined = previous + &s("text").unwrap();
            let text = if len16(&joined) > OUTPUT_LIMIT {
              format!("{}\n[Output truncated at 256,000 characters]", slice16(&joined, OUTPUT_LIMIT))
            } else {
              joined
            };
            b.content = Some(ToolContent::Text { text });
          }
        }
        "tool_end" => {
          let b = tool(turn, &s("callId").unwrap())?;
          if b.status != ToolStatus::InProgress {
            bail!("Tool is already settled");
          }
          let status = ToolStatus::parse(&s("status").unwrap()).unwrap();
          b.status = status;
          b.ended_at = Some(now);
          b.meta = s("detail");
          if let Some(d) = e.get("diff") {
            if status != ToolStatus::Completed {
              bail!("A failed tool cannot claim an applied diff");
            }
            let (path, old, new) = (
              d["path"].as_str().unwrap().to_owned(),
              d["oldText"].as_str().unwrap().to_owned(),
              d["newText"].as_str().unwrap().to_owned(),
            );
            b.content = Some(ToolContent::Diff {
              lines: diff_lines(&old, &new),
              source: Some(DiffSource { path: path.clone(), old_text: old, new_text: new }),
            });
            b.locations = Some(vec![Location { path, line: None }]);
          }
        }
        "turn_end" => {
          let stop = TurnStop::parse(&s("stop").unwrap()).unwrap();
          if let Some(existing) = turn.stop {
            if existing != stop {
              bail!("Conflicting turn completion receipt");
            }
            return Ok(None);
          }
          if stop == TurnStop::EndTurn
            && turn.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.status == ToolStatus::InProgress))
          {
            bail!("Cannot finish while tools have no completion receipt");
          }
          turn.stop = Some(stop);
          turn.ended_at = Some(now);
          turn.activity = None;
          if active_id.as_deref() == Some(turn_id.as_str()) {
            r.active_turn_id = None;
            r.active_event_at = None;
          }
        }
        "heartbeat" => {
          // A still-running older command must neither fail nor renew the newest turn
          if active_id.as_deref() != Some(turn_id.as_str()) {
            return Ok(None);
          }
        }
        _ => unreachable!("validated"),
      }
    }
  }
  if r.active_turn_id.as_deref() == Some(turn_id.as_str()) {
    r.active_event_at = Some(at.clone());
  }
  r.receipts.insert(id, Value::from(digest));
  r.revision += 1;
  r.last_event_at = at;
  Ok(Some(r))
}

pub fn chatgpt_view(r: &ChatGptRecord, now: i64) -> SessionView {
  let lease = r.active_event_at.as_deref().unwrap_or(&r.last_event_at);
  let stale = r.active_turn_id.is_some() && now - ms_of_iso(lease).unwrap_or(0) > STALE_AFTER_MS;
  let turns = r
    .turns
    .iter()
    .enumerate()
    .map(|(i, turn)| {
      let Turn::Agent(a) = turn else { return turn.clone() };
      let observed = i > 0 && matches!(&r.turns[i - 1], Turn::User(u) if u.id.is_some() && u.id == r.active_turn_id) && !stale;
      let mut a = a.clone();
      a.observation = (a.stop.is_none() && !observed).then_some(Observation::Unknown);
      for b in &mut a.blocks {
        if let AgentBlock::ToolCall(tc) = b
          && tc.status == ToolStatus::InProgress
          && !observed
        {
          tc.observation = Some(Observation::Unknown);
        }
      }
      Turn::Agent(a)
    })
    .collect();
  let state = if r.turns.is_empty() {
    ExternalState::Unbound
  } else if stale {
    ExternalState::Stale
  } else if r.active_turn_id.is_some() {
    ExternalState::Receiving
  } else {
    ExternalState::Idle
  };
  SessionView {
    id: r.id.clone(),
    external: Some(ExternalSessionInfo {
      source: "chatgpt".into(),
      source_key: r.source_key.clone(),
      connection_prompt: None,
      state,
      active_turn_id: r.active_turn_id.clone(),
      last_event_at: r.last_event_at.clone(),
    }),
    agent: CHATGPT_ID.into(),
    account_id: None,
    title: r.title.clone(),
    cwd: r.cwd.clone(),
    status: SessionStatus::Readonly,
    error: None,
    auth_methods: None,
    turns,
    running: r.active_turn_id.is_some() && !stale,
    rev: Some(r.revision * 2 + i64::from(stale)),
    controls: SessionControls::default(),
    model_shapes: None,
    usage: None,
    commands: vec![],
    queued: None,
    subagents: None,
    created_at: r.created_at.clone(),
    updated_at: r.updated_at.clone(),
  }
}

pub fn chatgpt_summary(r: &ChatGptRecord, now: i64) -> SessionSummary {
  let v = chatgpt_view(r, now);
  let stale = v.external.as_ref().is_some_and(|x| x.state == ExternalState::Stale);
  SessionSummary {
    id: r.id.clone(),
    external: Some(true),
    title: r.title.clone(),
    agent: CHATGPT_ID.into(),
    account_id: None,
    acp_session_id: None,
    cwd: r.cwd.clone(),
    updated_at: r.updated_at.clone(),
    pinned: r.pinned,
    state: if v.running {
      Some(SummaryState::Working)
    } else if stale {
      Some(SummaryState::Waiting)
    } else {
      None
    },
  }
}
