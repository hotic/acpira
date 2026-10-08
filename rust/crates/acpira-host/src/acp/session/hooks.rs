//! Workspace hooks in the session loop (`crate::hooks`): the first prompt carries the project's context files, an edit
//! announced by a permission request asks `beforeEdit` first, and a finished turn goes through the gate — the turn's
//! changed files are checked against `beforeEdit` again (edits made without asking), then `afterTurn` runs. Whatever
//! blocks goes back to the agent as an automatic follow-up (`Origin::Gate`), at most `rounds` times per user prompt;
//! prompts the user sends meanwhile queue behind the gate

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::transcript::*;

use crate::acp::session::prompt::{Origin, agent_turn_mut};
use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::rpc::BoxFuture;
use crate::hooks::run::{Verdict, run};
use crate::hooks::snapshot::Snapshot;
use crate::hooks::{CONTEXT_MAX_BYTES, Hook, HooksConfig};
use crate::i18n::tp;
use crate::platform::file_url::path_to_file_url;
use crate::util::random_uuid;

#[derive(Default)]
pub(crate) struct HookState {
  /// The project's hooks as the running (or last) turn found them
  pub config: Option<Arc<HooksConfig>>,
  /// The watched work trees when the running turn started
  pub before: Option<Snapshot>,
  /// Every file the session's turns changed, for `afterTurn`'s `changed_files`
  pub changed: BTreeSet<PathBuf>,
  /// Edits `beforeEdit` rejected during the running turn: what the agent is told when the turn ends
  pub denied: Vec<String>,
  /// Automatic gate follow-ups since the user last prompted
  pub round: u32,
  /// The gate is deciding about the turn that just ended: prompts queue until it is done
  pub gating: bool,
  /// The last hooks-file error shown, so a broken file is reported once rather than every turn
  pub reported: Option<String>,
}

/// The Claude Code tool name a payload carries for an edit of this kind
fn tool_name(kind: ToolKind) -> &'static str {
  match kind {
    ToolKind::Delete => "Delete",
    ToolKind::Move => "Move",
    _ => "Edit",
  }
}

fn is_edit(kind: ToolKind) -> bool {
  matches!(kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
}

/// The files an edit tool call touched: its locations and the paths of its diffs
pub(crate) fn edit_paths(tc: &ToolCallBlock) -> Vec<String> {
  let mut out: Vec<String> = tc.locations.iter().flatten().map(|l| l.path.clone()).collect();
  for content in tc.content.iter().chain(tc.contents.iter().flatten()) {
    if let ToolContent::Diff { source: Some(s), .. } = content {
      out.push(s.path.clone());
    }
  }
  let mut seen = BTreeSet::new();
  out.retain(|p| !p.is_empty() && seen.insert(p.clone()));
  out
}

/// What the agent looked at, in the shape hook scripts search for document paths: every non-edit tool call's target
/// (a command line, a URL, a pattern) and the paths it read
pub(crate) fn read_inputs(turns: &[Turn]) -> Vec<String> {
  let mut out = vec![];
  for turn in turns.iter().filter_map(Turn::as_agent) {
    for b in &turn.blocks {
      let AgentBlock::ToolCall(tc) = b else { continue };
      if is_edit(tc.kind) {
        continue;
      }
      out.extend(tc.target.clone());
      out.extend(tc.locations.iter().flatten().map(|l| l.path.clone()));
      out.extend(tc.read_range.as_ref().map(|r| r.path.clone()));
    }
  }
  out.retain(|s| !s.is_empty());
  out
}

/// The same file spelled two ways (a symlinked directory, a relative path) counts once
fn same_file(a: &Path, b: &Path) -> bool {
  a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

fn notice(severity: Severity, title: String, details: Option<String>) -> AgentBlock {
  AgentBlock::Notice(NoticeBlock {
    id: format!("hooks:{}", random_uuid()),
    revision: acpira_shared::num::Num(1.0),
    category: FailureCategory::Request,
    severity,
    title,
    details,
    actions: vec![],
  })
}

/// Push a notice row onto the session's last agent turn
fn push_notice(c: &mut Core, block: AgentBlock) {
  if let Some(Turn::Agent(turn)) = c.state.turns.iter_mut().rev().find(|t| matches!(t, Turn::Agent(_))) {
    turn.blocks.push(block);
  }
}

fn path_text(p: &Path) -> String {
  p.to_string_lossy().into_owned()
}

impl AcpSession {
  fn hook_env(&self) -> Vec<(&'static str, String)> {
    vec![("ACPIRA_SESSION_ID", self.id.clone()), ("ACPIRA_AGENT", self.agent.clone())]
  }

  /// The project's hooks, read again (an edited file applies from the next turn); a broken file is reported once
  fn load_hooks(&self) -> Option<Arc<HooksConfig>> {
    match crate::hooks::load(&self.cwd) {
      None => None,
      Some(Ok(cfg)) => {
        self.core.lock().hooks.reported = None;
        Some(Arc::new(cfg))
      }
      Some(Err(e)) => {
        let fresh = {
          let mut c = self.core.lock();
          let fresh = c.hooks.reported.as_deref() != Some(e.as_str());
          c.hooks.reported = Some(e.clone());
          fresh
        };
        self.log(&format!("hooks: {e}"));
        if fresh {
          self.notify(&tp("host.hooksInvalid", &[("error", &e)]));
        }
        None
      }
    }
  }

  /// A turn is about to be staged: load the hooks, snapshot the watched trees, and return the context blocks a
  /// session's first user prompt carries in front of its own
  pub(crate) async fn hooks_before_turn(&self, origin: Origin) -> Vec<Value> {
    if origin == Origin::Compact {
      return vec![];
    }
    let cfg = self.load_hooks();
    let first = {
      let mut c = self.core.lock();
      if origin == Origin::User {
        c.hooks.round = 0;
      }
      c.hooks.denied.clear();
      c.hooks.before = None;
      c.hooks.config = cfg.clone();
      origin == Origin::User && !c.state.turns.iter().any(|t| matches!(t, Turn::User(u) if u.auto != Some(true)))
    };
    let Some(cfg) = cfg else { return vec![] };
    if cfg.tracks_changes() {
      let snap = Snapshot::take(&cfg.watch).await;
      self.core.lock().hooks.before = Some(snap);
    }
    if first { self.context_blocks(&cfg).await } else { vec![] }
  }

  async fn context_blocks(&self, cfg: &HooksConfig) -> Vec<Value> {
    let embedded = self.caps(&self.core.lock()).embedded_context;
    let mut blocks = vec![];
    for path in &cfg.context {
      let Ok(mut text) = tokio::fs::read_to_string(path).await else {
        self.log(&format!("hooks: context file {} unreadable", path.display()));
        self.notify(&tp("host.hookContextMissing", &[("path", &path_text(path))]));
        continue;
      };
      if text.len() > CONTEXT_MAX_BYTES {
        let mut end = CONTEXT_MAX_BYTES;
        while !text.is_char_boundary(end) {
          end -= 1;
        }
        text.truncate(end);
        text.push_str("\n…");
      }
      self.log(&format!("hooks: context {} ({} bytes)", path.display(), text.len()));
      blocks.push(if embedded {
        json!({ "type": "resource", "resource": { "uri": path_to_file_url(&path_text(path)), "mimeType": "text/markdown", "text": text } })
      } else {
        json!({ "type": "text", "text": format!("<workspace-instructions path=\"{}\">\n{text}\n</workspace-instructions>", path.display()) })
      });
    }
    blocks
  }

  /// `beforeEdit` for an edit a permission request announces. Some(reason): the edit is rejected, the turn shows why and
  /// the agent is told when the turn ends
  pub(crate) async fn hooks_check_edit(&self, kind: ToolKind, paths: Vec<String>, raw_input: Option<Value>) -> Option<String> {
    if !is_edit(kind) || paths.is_empty() {
      return None;
    }
    let cfg = self.core.lock().hooks.config.clone().or_else(|| self.load_hooks())?;
    let hook = cfg.before_edit.as_ref()?;
    let files: Vec<String> = paths.iter().map(|p| path_text(&self.absolute(p))).collect();
    let verdict = self.run_before_edit(&cfg, hook, kind, &files, raw_input, "permission").await;
    let reason = match verdict {
      Verdict::Block(r) => r,
      Verdict::Pass => return None,
      Verdict::Broken(e) => {
        self.hook_broken(hook, &e);
        return None;
      }
    };
    let file = files.first().cloned().unwrap_or_default();
    self.log(&format!("hooks: edit of {file} rejected"));
    let mut c = self.core.lock();
    c.hooks.denied.push(tp("host.gateDeniedEdit", &[("file", &file), ("reason", &reason)]));
    push_notice(&mut c, notice(Severity::Warning, tp("host.hookEditDenied", &[("file", &file)]), Some(reason.clone())));
    self.touch(&mut c);
    Some(reason)
  }

  fn absolute(&self, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() { path.to_path_buf() } else { Path::new(&self.cwd).join(path) }
  }

  async fn run_before_edit(&self, cfg: &HooksConfig, hook: &Hook, kind: ToolKind, files: &[String], raw_input: Option<Value>, phase: &str) -> Verdict {
    let read = read_inputs(&self.core.lock().state.turns);
    // The tool's own input, with the first file under the key Claude Code's Edit uses, so scripts that read one path work
    let mut input = raw_input.filter(Value::is_object).unwrap_or_else(|| json!({}));
    if input.get("file_path").is_none() {
      input["file_path"] = json!(files.first());
    }
    let payload = json!({
      "hook_event_name": "PreToolUse",
      "session_id": self.id,
      "agent": self.agent,
      "cwd": path_text(&cfg.root),
      "tool_name": tool_name(kind),
      "tool_input": input,
      "files": files,
      "read_files": read,
      "phase": phase,
    });
    run(hook, &cfg.root, &payload, &self.hook_env()).await
  }

  fn hook_broken(&self, hook: &Hook, error: &str) {
    self.log(&format!("hooks: `{}` broken: {error}", hook.run));
    let mut c = self.core.lock();
    push_notice(&mut c, notice(Severity::Warning, tp("host.hookBroken", &[("hook", &hook.run)]), Some(error.to_owned())));
    self.touch(&mut c);
  }

  /// A turn ended normally: hand it to the gate when the project has one. true = the gate took over and will flush the
  /// queue (or send its follow-up) itself
  pub(crate) fn hooks_after_turn(self: &Arc<Self>, agent_idx: usize, started_at: i64) -> bool {
    let (cfg, before, turn_files) = {
      let mut c = self.core.lock();
      let Some(cfg) = c.hooks.config.clone() else { return false };
      if !cfg.tracks_changes() && c.hooks.denied.is_empty() {
        return false;
      }
      let turn_files: Vec<String> = agent_turn_mut(&mut c, agent_idx, started_at)
        .map(|t| {
          t.blocks
            .iter()
            .filter_map(|b| match b {
              AgentBlock::ToolCall(tc) if is_edit(tc.kind) && tc.status != ToolStatus::Failed => Some(edit_paths(tc)),
              _ => None,
            })
            .flatten()
            .collect()
        })
        .unwrap_or_default();
      c.hooks.gating = true;
      self.touch(&mut c);
      (cfg, c.hooks.before.take(), turn_files)
    };
    tokio::spawn(self.clone().gate(cfg, before, turn_files));
    true
  }

  async fn gate(self: Arc<Self>, cfg: Arc<HooksConfig>, before: Option<Snapshot>, tool_files: Vec<String>) {
    // What the turn changed: the work trees' own account, plus the edit tool calls (files outside the watched trees)
    let mut turn: Vec<PathBuf> = match &before {
      Some(b) => Snapshot::take(&cfg.watch).await.changed_since(b),
      None => vec![],
    };
    for p in tool_files.iter().map(|p| self.absolute(p)) {
      if !turn.iter().any(|t| same_file(t, &p)) {
        turn.push(p);
      }
    }
    let turn: Vec<String> = turn.iter().map(|p| path_text(p)).collect();
    let (mut reasons, session_files, round) = {
      let mut c = self.core.lock();
      c.hooks.changed.extend(turn.iter().map(PathBuf::from));
      (std::mem::take(&mut c.hooks.denied), c.hooks.changed.iter().map(|p| path_text(p)).collect::<Vec<_>>(), c.hooks.round)
    };
    self.log(&format!("hooks: gate after turn ({} changed, round {round})", turn.len()));
    // Edits made without a permission request get the beforeEdit check now
    if let Some(hook) = cfg.before_edit.as_ref().filter(|_| !turn.is_empty()) {
      match self.run_before_edit(&cfg, hook, ToolKind::Edit, &turn, None, "audit").await {
        Verdict::Block(r) => reasons.push(r),
        Verdict::Pass => {}
        Verdict::Broken(e) => self.hook_broken(hook, &e),
      }
    }
    if let Some(hook) = cfg.after_turn.as_ref() {
      let payload = json!({
        "hook_event_name": "Stop",
        "session_id": self.id,
        "agent": self.agent,
        "cwd": path_text(&cfg.root),
        "stop_hook_active": round > 0,
        "round": round,
        "turn_files": turn,
        "changed_files": session_files,
        "read_files": read_inputs(&self.core.lock().state.turns),
      });
      match run(hook, &cfg.root, &payload, &self.hook_env()).await {
        Verdict::Block(r) => reasons.push(r),
        Verdict::Pass => {}
        Verdict::Broken(e) => self.hook_broken(hook, &e),
      }
    }
    self.gate_decided(&cfg, reasons).await;
  }

  /// Boxed: the follow-up's own end runs the gate again, a cycle an `async fn` future cannot hold
  fn gate_follow_up(self: &Arc<Self>, text: String) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move { me.prompt_inner(text, vec![], Origin::Gate, None, None).await })
  }

  /// Send what blocked back to the agent, or let the queue go
  async fn gate_decided(self: &Arc<Self>, cfg: &HooksConfig, reasons: Vec<String>) {
    let follow_up = {
      let mut c = self.core.lock();
      let ready = c.status == SessionStatus::Ready && !c.phase.running;
      let rounds = cfg.rounds.to_string();
      let follow_up = if reasons.is_empty() || !ready {
        self.log(if reasons.is_empty() { "hooks: gate passed" } else { "hooks: gate blocked, session no longer ready" });
        None
      } else if c.hooks.round < cfg.rounds {
        c.hooks.round += 1;
        let round = c.hooks.round.to_string();
        let details = reasons.join("\n\n");
        push_notice(&mut c, notice(Severity::Warning, tp("host.gateFailed", &[("round", &round), ("rounds", &rounds)]), Some(details.clone())));
        Some(tp("host.gatePrompt", &[("reason", &details)]))
      } else {
        self.log("hooks: gate gave up");
        push_notice(&mut c, notice(Severity::Error, tp("host.gateGaveUp", &[("rounds", &rounds)]), Some(reasons.join("\n\n"))));
        None
      };
      // The follow-up keeps the session reserved until its claim; otherwise the gate lets go here
      if follow_up.is_none() {
        c.hooks.gating = false;
      }
      self.touch(&mut c);
      follow_up
    };
    match follow_up {
      Some(text) => {
        self.log("hooks: gate blocked, sending the findings back");
        self.gate_follow_up(text).await;
      }
      None => self.after_prompt(false, TurnStop::EndTurn),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn tool(kind: ToolKind, target: Option<&str>, paths: &[&str]) -> AgentBlock {
    AgentBlock::ToolCall(ToolCallBlock {
      observation: None,
      id: random_uuid(),
      kind,
      verb: String::new(),
      verb_key: None,
      target: target.map(str::to_owned),
      target_mono: None,
      locations: Some(paths.iter().map(|p| Location { path: (*p).to_owned(), line: None }).collect()),
      read_range: None,
      status: ToolStatus::Completed,
      background: None,
      async_task: None,
      started_at: None,
      ended_at: None,
      meta: None,
      diff_stat: None,
      todo_entries: None,
      content: None,
      contents: None,
      subagent_id: None,
    })
  }

  #[test]
  fn reads_count_every_non_edit_tool_and_skip_edits() {
    let turns = vec![Turn::Agent(AgentTurn {
      blocks: vec![
        tool(ToolKind::Read, Some("README.md"), &["/w/docs/README.md"]),
        tool(ToolKind::Execute, Some("cat docs/systems/x/README.md"), &[]),
        tool(ToolKind::Edit, Some("a.kt"), &["/w/a.kt"]),
      ],
      ..Default::default()
    })];
    assert_eq!(read_inputs(&turns), ["README.md", "/w/docs/README.md", "cat docs/systems/x/README.md"]);
  }

  #[test]
  fn an_edit_names_its_locations_and_diff_sources_once() {
    let AgentBlock::ToolCall(mut tc) = tool(ToolKind::Edit, None, &["/w/a.kt"]) else { unreachable!() };
    tc.content = Some(ToolContent::Diff { lines: vec![], source: Some(DiffSource { path: "/w/a.kt".into(), ..Default::default() }) });
    tc.contents = Some(vec![ToolContent::Diff { lines: vec![], source: Some(DiffSource { path: "/w/b.kt".into(), ..Default::default() }) }]);
    assert_eq!(edit_paths(&tc), ["/w/a.kt", "/w/b.kt"]);
  }
}
