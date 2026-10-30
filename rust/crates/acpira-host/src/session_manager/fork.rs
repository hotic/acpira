//! Forks: a new session from a source's turns up to an agent turn, on the same agent or another, with the blobs the
//! copied turns point at

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use acpira_shared::model_shapes::is_model_control;
use acpira_shared::subagents::{StateSource, SubagentState};
use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use super::{SessionManager, Viewer};
use crate::acp::session::AcpSession;
use crate::i18n::{t, tp};
use crate::limits::RENAME_MAX;
use crate::store::record::{ForkedFrom, SessionRecord};
use crate::store::transcript_store::is_session_id;
use crate::util::{clip, ms_of_iso, now_iso, random_uuid};

/// A fork's agent was handed its copied turns as a clipped (possibly trimmed) history, never as its own conversation:
/// reading its own session is how it gets them in full. Any other session's own agent has its conversation in context
pub(super) fn holds_unseen_turns(record: &SessionRecord) -> bool {
  record.forked_from.as_ref().is_some_and(|f| f.turn_index >= 0)
}

/// Same agent / account / cwd and still empty: keep the process instead of spawning another
/// The model the forked reply came from, as the source showed it: the prompt's recorded model resolved against the
/// source's model list, else the id the turn's usage reported, else the source's current model
fn fork_model_label(source: &SessionRecord, idx: usize) -> Option<String> {
  let control = source.controls.options.iter().find(|c| is_model_control(c));
  let name_of = |id: &str| control.and_then(|c| c.options.iter().find(|o| o.id == id)).map(|o| o.name.clone()).unwrap_or_else(|| id.to_owned());
  let prompt = source.turns[..idx].iter().rev().find_map(|t| match t {
    Turn::User(u) => Some(u),
    _ => None,
  });
  let chosen = control.and_then(|c| prompt.and_then(|u| u.settings.as_ref()).and_then(|s| s.config.get(&c.id)));
  let reported = source.turns[idx].as_agent().and_then(|a| a.usage.as_ref()).and_then(|u| u.model.as_deref());
  chosen.map(|id| name_of(id)).or_else(|| reported.map(name_of)).or_else(|| control.and_then(|c| c.value.as_deref()).map(name_of))
}

/// Every blob reference in a transcript: prompt and steer attachments, agent image blocks, images a reply's markdown
/// embeds, and image results of tool calls
fn blob_slots<'a>(turns: &'a mut [Turn], out: &mut Vec<&'a mut Option<String>>) {
  fn image(c: &mut ToolContent) -> Option<&mut Option<String>> {
    match c {
      ToolContent::Image(r) => Some(&mut r.blob),
      _ => None,
    }
  }
  fn attachments<'a>(list: &'a mut Option<Vec<Attachment>>, out: &mut Vec<&'a mut Option<String>>) {
    for a in list.iter_mut().flatten() {
      match a {
        Attachment::Image { blob, .. } | Attachment::Text { blob, .. } | Attachment::Selection { blob, .. } => out.push(blob),
        Attachment::File { .. } | Attachment::Quote { .. } => {}
      }
    }
  }
  for turn in turns {
    match turn {
      Turn::User(u) => attachments(&mut u.attachments, out),
      Turn::Agent(a) => {
        for b in &mut a.blocks {
          match b {
            AgentBlock::Steer(s) => attachments(&mut s.attachments, out),
            AgentBlock::Image(i) => out.push(&mut i.blob),
            AgentBlock::Text(t) => out.extend(t.images.iter_mut().flatten().map(|r| &mut r.blob)),
            AgentBlock::ToolCall(c) => {
              out.extend(c.content.as_mut().and_then(image));
              out.extend(c.contents.iter_mut().flatten().filter_map(image));
            }
            _ => {}
          }
        }
      }
    }
  }
}

impl SessionManager {
  /// Fork from an agent turn: a fresh session whose transcript is the source's turns up to that reply. `agent` moves the
  /// fork onto another agent, which starts the way a new session of it does (default account, remembered settings)
  pub async fn fork_session(self: &Arc<Self>, v: &Arc<Viewer>, source_id: &str, turn_index: i64, agent: Option<String>) -> Result<()> {
    if !is_session_id(source_id) {
      return Ok(());
    }
    if self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(source_id)) {
      return Err(anyhow!(t("chatgpt.externalOnly")));
    }
    let live = self.live(source_id);
    let source = match &live {
      Some(l) => l.to_record(),
      None => match self.deps.store.load_detailed(source_id).await {
        Ok(record) => record,
        Err(error) => return Err(anyhow!(self.record_load_message(source_id, &error))),
      },
    };
    let idx = usize::try_from(turn_index)
      .ok()
      .filter(|i| matches!(source.turns.get(*i), Some(Turn::Agent(_))))
      .ok_or_else(|| anyhow!(t("host.forkStale")))?;
    if live.as_ref().is_some_and(|l| l.is_running()) && idx == source.turns.len() - 1 {
      return Err(anyhow!(t("host.forkRunning")));
    }
    // Another agent only when it is one this engine can launch; naming the source's own agent is a plain fork
    let target = agent.filter(|a| a != &source.agent);
    if let Some(id) = &target
      && !self.agents().iter().any(|a| &a.id == id && a.external != Some(true))
    {
      return Err(anyhow!(tp("host.forkAgentUnknown", &[("agent", id)])));
    }
    let model = target.as_ref().and_then(|_| fork_model_label(&source, idx));
    let mut turns: Vec<Turn> = source.turns[..=idx].to_vec();
    // Live-only state does not travel
    for turn in turns.iter_mut().filter_map(Turn::as_agent_mut) {
      turn.activity = None;
      for b in &mut turn.blocks {
        match b {
          AgentBlock::Text(x) if x.streaming == Some(true) => x.streaming = Some(false),
          AgentBlock::Thought(x) if x.streaming == Some(true) => x.streaming = Some(false),
          _ => {}
        }
      }
    }
    let now = now_iso();
    let subagents: Vec<_> = source
      .subagents
      .clone()
      .unwrap_or_default()
      .into_iter()
      .filter(|n| n.core.turn_index as usize <= idx)
      .map(|mut n| {
        if n.core.state == SubagentState::Running {
          n.core.state = SubagentState::Disconnected;
          n.core.state_source = StateSource::Local;
          n.core.ended_at = ms_of_iso(&now);
        }
        n.core.cancel_requested = None;
        n
      })
      .collect();
    // A fork onto another agent takes nothing of the source's agent setup: the target's default account, and its
    // controls come from its own start (seeded below like a new session's), not the source's model list
    let (agent_id, account_id, controls) = match &target {
      Some(id) => (id.clone(), self.default_account(id), SessionControls::default()),
      None => (source.agent.clone(), source.account_id.clone(), source.controls.clone()),
    };
    let mut record = SessionRecord {
      id: random_uuid(),
      agent: agent_id,
      account_id,
      acp_session_id: None,
      cwd: source.cwd.clone(),
      title: clip(&tp("session.forkTitle", &[("title", &source.title)]), RENAME_MAX),
      created_at: now.clone(),
      updated_at: now,
      turns,
      controls,
      usage: None,
      commands: vec![],
      pinned: None,
      archived: None,
      // A fork stays in its source's category
      category: source.category.clone(),
      renamed: false,
      history_pending: true,
      forked_from: Some(ForkedFrom { session_id: source.id.clone(), turn_index, agent: target.as_ref().map(|_| source.agent.clone()), model }),
      import_pending: false,
      imported_from: None,
      subagents: (!subagents.is_empty()).then_some(subagents),
      goal: None,
      copy_of: None,
    };
    // Every blob the copied turns point at (prompt / steer attachments and the images the agent produced, subagent
    // transcripts included) is re-saved under the fork's own blob dir; content-hash names keep the same file name
    let fork_id = record.id.clone();
    let mut slots = Vec::new();
    blob_slots(&mut record.turns, &mut slots);
    for node in record.subagents.iter_mut().flatten() {
      blob_slots(&mut node.turns, &mut slots);
    }
    // One copy per distinct blob; a name that could not be copied is dropped everywhere it appears
    let mut copied: HashMap<String, Option<String>> = HashMap::new();
    for blob in slots {
      let Some(name) = blob.clone() else { continue };
      if !copied.contains_key(&name) {
        let ext = std::path::Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let result = match self.deps.store.read_blob(&source.id, &name).await {
          Ok(bytes) => self.deps.store.save_blob(&fork_id, &ext, &bytes).await.map(|(n, _)| n),
          Err(e) => Err(e),
        };
        let next = result.map_err(|e| self.log(&format!("fork: blob {name} not copied ({e})"))).ok();
        copied.insert(name.clone(), next);
      }
      *blob = copied[&name].clone();
    }
    self.deps.store.flush(Arc::new(record.clone())).await?;
    self.drop_empty_current(v).await;
    let s = AcpSession::new(record, self.session_deps());
    s.emit("session.created", || serde_json::json!({ "how": "fork", "forkedFrom": source_id }));
    // The target's controls the way new_session_for seeds them: its known controls with the remembered settings laid
    // over before the spawn (held, so a prompt sent while it spawns waits for them), adopted once it is up
    let last = target.as_ref().and_then(|id| self.last_settings(id));
    if let Some(id) = &target {
      s.preview_controls(&self.known_start_controls(id).await, last.as_ref());
      if let Some(last) = &last {
        s.hold_settings(last);
      }
    }
    self.state.lock().live.insert(s.id.clone(), s.clone());
    self.set_active(v, Some(s.id.clone()));
    self.process_change(&s.id);
    s.start().await;
    match &target {
      Some(id) => {
        self.state.lock().probed.remove(id);
        if let Some(last) = last {
          let picked = s.adopt_controls(last).await;
          self.remember_picked(&s, &picked);
        }
      }
      None if s.status() == SessionStatus::Ready => {
        s.adopt_controls(capture_turn_settings(&source.controls)).await;
      }
      None => {}
    }
    Ok(())
  }
}
