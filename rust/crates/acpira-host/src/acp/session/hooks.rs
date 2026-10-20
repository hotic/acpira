//! Workspace hooks in the session loop (`crate::hooks`): the first prompt carries the project's context files, every
//! user prompt asks `beforePrompt` first (its output rides along, a block keeps the prompt from being sent), an edit
//! announced by a permission request asks `beforeEdit` first, and a finished turn goes through the gate — the turn's
//! changed files are checked against `beforeEdit` again (edits made without asking), then `afterTurn` runs. Whatever
//! blocks goes back to the agent as an automatic follow-up (`Origin::Gate`), at most `rounds` times per user prompt;
//! prompts the user sends meanwhile queue behind the gate, and the turn keeps its cross-engine lease until it is done.
//!
//! The gate covers a span, not a single turn: a turn that ends without finishing (stopped, failed, out of quota, a
//! reconnect) leaves its snapshot and edit rows open, and the next turn that finishes is gated over both. A detached
//! peer turn (a steer's new turn, a Claude autonomous cycle) that finishes is gated over what it added after the last
//! gate
//!
//! A shared work tree changes under other people and agents too, so the work trees' own account only counts when a tool
//! of the span could have changed any file (a shell command, a subagent, an MCP or unknown tool): a span of reads takes
//! no files, a span of edits naming their files takes those files only

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::slash_commands::command_name;
use acpira_shared::transcript::*;

use crate::acp::session::prompt::Origin;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::{READ_SESSION_VERB, SEARCH_SESSIONS_VERB, SHOW_IMAGE_VERB, async_task_live};
use crate::acp::transport::rpc::BoxFuture;
use crate::hooks::run::{PromptVerdict, Verdict, run, run_prompt};
use crate::hooks::snapshot::Snapshot;
use crate::hooks::{CONTEXT_MAX_BYTES, Hook, HooksConfig};
use crate::i18n::{t, tp};
use crate::platform::file_url::path_to_file_url;
use crate::util::{now_ms, random_uuid};

#[derive(Default)]
pub(crate) struct HookState {
  /// The project's hooks as the running (or last) turn found them
  pub config: Option<Arc<HooksConfig>>,
  /// The watched work trees when the span the next gate covers started. Only the gate takes it: a turn that ends
  /// without one leaves it for the next turn's gate
  pub before: Option<Snapshot>,
  /// The watched work trees as the last gate found them: the baseline of a detached turn that continues past it
  pub settled: Option<Snapshot>,
  /// Agent turns started at or after this (ms) belong to the open span
  pub since: Option<i64>,
  /// (turn start, block count) of the edit rows the last gate already took: a detached turn continuing that agent turn
  /// is gated over the blocks after them only
  pub audited: Option<(i64, usize)>,
  /// Edit rows whose permission request already passed `beforeEdit`: the audit leaves their files out
  pub asked: BTreeSet<String>,
  /// Every file the session's turns changed, for `afterTurn`'s `changed_files`, with where the gate learnt it from
  pub changed: BTreeMap<PathBuf, Source>,
  /// Edits `beforeEdit` rejected during the running turn: what the agent is told when the turn ends
  pub denied: Vec<String>,
  /// Automatic gate follow-ups since the user last prompted
  pub round: u32,
  /// The gate is deciding about the turn that just ended: prompts queue until it is done
  pub gating: bool,
  /// A detached turn ended while the gate was deciding: the gate runs once more over it before letting go
  pub rerun: bool,
  /// A turn ended while summoned children still ran: its gate waits for them, so the span covers what they change
  /// meanwhile; the first turn to end without any (or the last child ending while the session is idle) runs it
  pub deferred: bool,
  /// The last hooks-file error shown, so a broken file is reported once rather than every turn
  pub reported: Option<String>,
}

/// What a turn's prompt carries besides the user's own blocks: the context files in front (a first prompt) and the
/// `beforePrompt` context behind
#[derive(Default)]
pub(crate) struct TurnContext {
  pub lead: Vec<Value>,
  pub tail: Vec<Value>,
}

/// How a gate knows a file is the span's: an edit tool call named it, or only the work trees' snapshots differ. Ordered
/// so that a tool call wins when both say so
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Source {
  Snapshot,
  Tool,
}

impl Source {
  fn as_str(self) -> &'static str {
    match self {
      Source::Snapshot => "snapshot",
      Source::Tool => "tool",
    }
  }
}

/// How far the span's tool calls can have reached on disk; the span takes its widest row
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reach {
  /// Reads, searches, fetches and Acpira's own bookkeeping: nothing on disk is the span's
  #[default]
  None,
  /// Edit / delete rows that name their files: those files are the span's
  Edits,
  /// A shell command, a move, a subagent, an MCP or unknown tool, an edit naming no file, or a background task still
  /// running: any file that changed meanwhile may be the span's
  Any,
}

/// What a row of this tool call can have changed. Unknown tools count as able to change anything, so an agent whose
/// tools are not classified keeps the snapshot's account
pub(crate) fn row_reach(tc: &ToolCallBlock) -> Reach {
  // A subagent's own tool calls are not root rows, and an MCP tool (`ask_agent` included) can write anything
  if tc.subagent_id.is_some() || tc.mcp_server.is_some() {
    return Reach::Any;
  }
  match tc.kind {
    ToolKind::Read | ToolKind::Search | ToolKind::Fetch | ToolKind::SwitchMode => Reach::None,
    ToolKind::Edit | ToolKind::Delete if tc.status == ToolStatus::Failed => Reach::None,
    ToolKind::Edit | ToolKind::Delete if !edit_paths(tc).is_empty() => Reach::Edits,
    ToolKind::Think | ToolKind::Other
      if matches!(tc.verb_key.as_deref(), Some(k) if ["verb.todo", "verb.ask", SHOW_IMAGE_VERB, SEARCH_SESSIONS_VERB, READ_SESSION_VERB].contains(&k)) =>
    {
      Reach::None
    }
    _ => Reach::Any,
  }
}

/// One tool call of the span as `turn_tools` lists it
struct SpanTool {
  kind: ToolKind,
  status: ToolStatus,
  paths: Vec<String>,
}

/// The tool calls of the span a gate covers: its edit rows' files, split by whether `beforeEdit` already passed them in
/// a permission request, every row for the payload, and how far they reach
#[derive(Default)]
struct SpanTools {
  asked: Vec<String>,
  unasked: Vec<String>,
  rows: Vec<SpanTool>,
  reach: Reach,
}

/// What a gate run needs, taken out of the session under its lock
struct GateJob {
  cfg: Arc<HooksConfig>,
  before: Option<Snapshot>,
  tools: SpanTools,
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

/// The open span's tool calls not gated yet, and the mark that keeps the next gate from taking them again
fn span_tools(c: &mut Core) -> SpanTools {
  let mut out = SpanTools::default();
  let Some(since) = c.hooks.since else { return out };
  let mut last = None;
  for turn in c.state.turns.iter().filter_map(Turn::as_agent) {
    let at = turn.started_at.filter(|at| *at >= since);
    let skip = match (at, c.hooks.audited) {
      (Some(at), Some((t, n))) if t == at => n,
      (Some(_), _) => 0,
      (None, _) => turn.blocks.len(),
    };
    for (i, b) in turn.blocks.iter().enumerate() {
      let AgentBlock::ToolCall(tc) = b else { continue };
      if i < skip {
        // Before the span: only a background task still running can be changing files now
        if async_task_live(tc) || (tc.kind == ToolKind::Execute && tc.status.is_open()) {
          out.reach = Reach::Any;
        }
        continue;
      }
      out.reach = out.reach.max(row_reach(tc));
      out.rows.push(SpanTool { kind: tc.kind, status: tc.status, paths: edit_paths(tc) });
      if !is_edit(tc.kind) || tc.status == ToolStatus::Failed {
        continue;
      }
      let list = if c.hooks.asked.contains(&tc.id) { &mut out.asked } else { &mut out.unasked };
      list.extend(edit_paths(tc));
    }
    if let Some(at) = at {
      last = Some((at, turn.blocks.len()));
    }
  }
  if let Some((at, n)) = last {
    c.hooks.since = Some(at);
    c.hooks.audited = Some((at, n));
  }
  out
}

/// The files a tool call names: its locations and the paths of its diffs (for an edit, the files it touched)
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
    info: false,
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

/// `file_sources` of a payload: each file's path → "tool" | "snapshot"
fn file_sources<'a>(files: impl Iterator<Item = (&'a PathBuf, Source)>) -> Value {
  Value::Object(files.map(|(p, s)| (path_text(p), json!(s.as_str()))).collect())
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
        // Unknown keys are skipped, not fatal: said once per distinct list, like a broken file
        let warning = (!cfg.unknown.is_empty()).then(|| cfg.unknown.join(", "));
        let fresh = {
          let mut c = self.core.lock();
          let fresh = warning.is_some() && c.hooks.reported != warning;
          c.hooks.reported = warning.clone();
          fresh
        };
        if let Some(keys) = warning {
          self.log(&format!("hooks: unknown keys skipped: {keys}"));
          if fresh {
            self.notify(&tp("host.hooksUnknownKeys", &[("keys", &keys)]));
          }
        }
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

  /// A staged turn is about to be accepted: load the hooks, ask `beforePrompt` about a user's prompt, snapshot the
  /// watched trees, and return what the prompt carries besides its own blocks. Err(reason): `beforePrompt` blocked the
  /// prompt, and nothing of the hooks' state moved
  pub(crate) async fn hooks_before_turn(&self, origin: Origin, text: &str, attachments: &[Attachment]) -> Result<TurnContext, String> {
    if origin == Origin::Compact {
      return Ok(TurnContext::default());
    }
    let cfg = self.load_hooks();
    // The first prompt this peer sees: a fork's history still owed, or a native session no prompt reached yet (a new
    // session, an edit's rebuilt one), whatever the transcript already holds
    let first_of = |c: &Core| origin == Origin::User && (c.lineage.history_pending || c.unprompted);
    let mut tail = vec![];
    // Only what the user (or their goal) asks for; Acpira's own follow-ups and continues are not prompts of theirs
    if matches!(origin, Origin::User | Origin::Goal)
      && let Some(cfg) = cfg.as_ref()
      && let Some(hook) = cfg.before_prompt.as_ref()
    {
      let first = first_of(&self.core.lock());
      let payload = json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": self.id,
        "agent": self.agent,
        "cwd": path_text(&cfg.root),
        "prompt": text,
        "origin": if origin == Origin::Goal { "goal" } else { "user" },
        "attachments": attachments,
        "first": first,
      });
      match run_prompt(hook, &cfg.root, &payload, &self.hook_env()).await {
        PromptVerdict::Block(reason) => return Err(reason),
        // A slash command must stay the prompt's only text, or the agent no longer reads it as one
        PromptVerdict::Pass(Some(_)) if command_name(text).is_some() => self.log("hooks: beforePrompt context left out of a slash command"),
        PromptVerdict::Pass(Some(context)) => {
          self.log(&format!("hooks: beforePrompt context ({} bytes)", context.len()));
          tail.push(json!({ "type": "text", "text": format!("<workspace-hook-context hook=\"beforePrompt\">\n{context}\n</workspace-hook-context>") }));
        }
        PromptVerdict::Pass(None) => {}
        // No turn to hold a notice row yet: the warning is a toast
        PromptVerdict::Broken(e) => {
          self.log(&format!("hooks: `{}` broken: {e}", hook.run));
          self.notify(&format!("{}\n{e}", tp("host.hookBroken", &[("hook", &hook.run)])));
        }
      }
    }
    let (first, fresh) = {
      let mut c = self.core.lock();
      // A goal set or resumed from the dock is a request of the user's own, like a typed prompt
      if matches!(origin, Origin::User | Origin::Goal) {
        c.hooks.round = 0;
      }
      // A deferred gate still owes the agent the edits beforeEdit rejected before it
      if !c.hooks.deferred {
        c.hooks.denied.clear();
      }
      c.hooks.settled = None;
      // A turn that ended without its gate left the span open: this turn joins it, so its edits are still audited.
      // Only while the same trees are watched; a changed watch list starts over
      let carried = c.hooks.before.is_some() && matches!((&c.hooks.config, &cfg), (Some(a), Some(b)) if a.watch == b.watch);
      if !carried {
        c.hooks.before = None;
        c.hooks.since = Some(now_ms());
        c.hooks.asked.clear();
      }
      c.hooks.config = cfg.clone();
      (first_of(&c), !carried)
    };
    let Some(cfg) = cfg else { return Ok(TurnContext { lead: vec![], tail }) };
    if fresh && cfg.tracks_changes() {
      let snap = Snapshot::take(&cfg.watch).await;
      self.core.lock().hooks.before = Some(snap);
    }
    let lead = if first { self.context_blocks(&cfg).await } else { vec![] };
    Ok(TurnContext { lead, tail })
  }

  /// `beforePrompt` blocked a prompt the session had claimed: the claim is undone, the user sees why next to the
  /// prompt's text (the bubble never appears, so the text would be lost otherwise), and the queue moves on
  pub(crate) fn hooks_prompt_blocked(self: &Arc<Self>, text: &str, reason: &str) {
    self.log(&format!("hooks: prompt blocked: {reason}"));
    let title = t("host.hookPromptBlocked");
    let shown = {
      let mut c = self.core.lock();
      c.phase.staging = false;
      c.phase.running = false;
      let shown = c.state.turns.iter().any(|t| matches!(t, Turn::Agent(_)));
      if shown {
        let details = if text.trim().is_empty() { reason.to_owned() } else { format!("{reason}\n\n{text}") };
        push_notice(&mut c, notice(Severity::Warning, title.clone(), Some(details)));
      }
      self.touch(&mut c);
      shown
    };
    if !shown {
      self.notify(&format!("{title}\n{reason}"));
    }
    self.flush_queue();
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
  /// the agent is told when the turn ends. A pass is remembered for the row, so the turn's audit does not ask again
  pub(crate) async fn hooks_check_edit(&self, tool_call_id: &str, kind: ToolKind, paths: Vec<String>, raw_input: Option<Value>) -> Option<String> {
    if !is_edit(kind) || paths.is_empty() {
      return None;
    }
    let cfg = self.core.lock().hooks.config.clone().or_else(|| self.load_hooks())?;
    let hook = cfg.before_edit.as_ref()?;
    let files: Vec<String> = paths.iter().map(|p| path_text(&self.absolute(p))).collect();
    let verdict = self.run_before_edit(&cfg, hook, kind, &files, raw_input, "permission", json!({})).await;
    let reason = match verdict {
      Verdict::Block(r) => r,
      Verdict::Pass => {
        self.core.lock().hooks.asked.insert(tool_call_id.to_owned());
        return None;
      }
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

  /// `extra`: keys added to the payload (the audit's attribution)
  #[allow(clippy::too_many_arguments)]
  async fn run_before_edit(&self, cfg: &HooksConfig, hook: &Hook, kind: ToolKind, files: &[String], raw_input: Option<Value>, phase: &str, extra: Value) -> Verdict {
    let read = read_inputs(&self.core.lock().state.turns);
    // The tool's own input, with the first file under the key Claude Code's Edit uses, so scripts that read one path work
    let mut input = raw_input.filter(Value::is_object).unwrap_or_else(|| json!({}));
    if input.get("file_path").is_none() {
      input["file_path"] = json!(files.first());
    }
    let mut payload = json!({
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
    if let (Some(p), Value::Object(extra)) = (payload.as_object_mut(), extra) {
      p.extend(extra);
    }
    run(hook, &cfg.root, &payload, &self.hook_env()).await
  }

  fn hook_broken(&self, hook: &Hook, error: &str) {
    self.log(&format!("hooks: `{}` broken: {error}", hook.run));
    let mut c = self.core.lock();
    push_notice(&mut c, notice(Severity::Warning, tp("host.hookBroken", &[("hook", &hook.run)]), Some(error.to_owned())));
    self.touch(&mut c);
  }

  /// A turn finished: hand its span to the gate when the project has one. true = the gate took over and will flush the
  /// queue (or send its follow-up) itself
  pub(crate) fn hooks_after_turn(self: &Arc<Self>) -> bool {
    let job = {
      let mut c = self.core.lock();
      // Only a detached turn can end while a gate decides: that gate runs once more over it before it lets go
      if c.hooks.gating {
        c.hooks.rerun = true;
        return true;
      }
      // Summoned children still at work: the span stays open (`before`, `since` untouched) until they are done
      if c.hooks.config.is_some() && Self::relay_background(&c) {
        if !c.hooks.deferred {
          self.log("hooks: gate deferred until the summoned agents finish");
        }
        c.hooks.deferred = true;
        return false;
      }
      self.gate_job(&mut c)
    };
    match job {
      Some(job) => {
        tokio::spawn(self.clone().spawned_gate(job));
        true
      }
      None => false,
    }
  }

  /// A detached peer turn ended (`end_detached`). A finished one is gated like a prompt's turn, over what it added after
  /// the last gate; a stopped one leaves that for the next gate. Then the queue moves as after any turn
  pub(crate) fn hooks_after_detached(self: &Arc<Self>, stop: TurnStop) {
    if stop == TurnStop::EndTurn {
      if self.hooks_after_turn() {
        return;
      }
    } else {
      let mut c = self.core.lock();
      if c.hooks.before.is_none() {
        c.hooks.before = c.hooks.settled.take();
      }
    }
    self.after_prompt(false, stop);
  }

  /// Reserve the session for a gate run (`gating`) and take what it needs; None: the project gates nothing
  fn gate_job(&self, c: &mut Core) -> Option<GateJob> {
    c.hooks.deferred = false;
    let cfg = c.hooks.config.clone()?;
    if !cfg.tracks_changes() && c.hooks.denied.is_empty() {
      return None;
    }
    let tools = span_tools(c);
    // A detached turn past the last gate has no snapshot of its own: what that gate found is its baseline
    let before = c.hooks.before.take().or_else(|| c.hooks.settled.take());
    c.hooks.gating = true;
    self.touch(c);
    Some(GateJob { cfg, before, tools })
  }

  /// Boxed: a gate can start another run of itself (`Next::Again`), a cycle an `async fn` future cannot hold
  fn spawned_gate(self: Arc<Self>, job: GateJob) -> BoxFuture<()> {
    Box::pin(self.gate(job))
  }

  async fn gate(self: Arc<Self>, job: GateJob) {
    let GateJob { cfg, before, tools } = job;
    let after = if cfg.tracks_changes() { Some(Snapshot::take(&cfg.watch).await) } else { None };
    // What the span changed: the work trees' own account when one of its tools could have changed any file (a span of
    // reads or of edits naming their files leaves whatever else changed to whoever else shares the trees), plus the edit
    // tool calls (files outside the watched trees too)
    let mut turn: Vec<(PathBuf, Source)> = match (&before, &after) {
      (Some(b), Some(a)) if tools.reach == Reach::Any => a.changed_since(b).into_iter().map(|p| (p, Source::Snapshot)).collect(),
      _ => vec![],
    };
    self.core.lock().hooks.settled = after;
    let asked: Vec<PathBuf> = tools.asked.iter().map(|p| self.absolute(p)).collect();
    let unasked: Vec<PathBuf> = tools.unasked.iter().map(|p| self.absolute(p)).collect();
    for p in unasked.iter().chain(&asked) {
      match turn.iter_mut().find(|(t, _)| same_file(t, p)) {
        Some(entry) => entry.1 = Source::Tool,
        None => turn.push((p.clone(), Source::Tool)),
      }
    }
    // The audit skips a file only when every edit row naming it went through a permission request beforeEdit passed
    let audit: Vec<String> = turn
      .iter()
      .filter(|(t, _)| !asked.iter().any(|a| same_file(t, a)) || unasked.iter().any(|u| same_file(t, u)))
      .map(|(p, _)| path_text(p))
      .collect();
    let turn_tools: Vec<Value> = tools
      .rows
      .iter()
      .map(|r| json!({ "kind": r.kind, "status": r.status, "locations": r.paths.iter().map(|p| path_text(&self.absolute(p))).collect::<Vec<_>>() }))
      .collect();
    let audit_sources = file_sources(turn.iter().map(|(p, s)| (p, *s)).filter(|(p, _)| audit.contains(&path_text(p))));
    let (mut reasons, session_files, session_sources, round) = {
      let mut c = self.core.lock();
      for (p, s) in &turn {
        let entry = c.hooks.changed.entry(p.clone()).or_insert(*s);
        *entry = (*entry).max(*s);
      }
      let files: Vec<String> = c.hooks.changed.keys().map(|p| path_text(p)).collect();
      let srcs = file_sources(c.hooks.changed.iter().map(|(p, s)| (p, *s)));
      (std::mem::take(&mut c.hooks.denied), files, srcs, c.hooks.round)
    };
    let turn: Vec<String> = turn.iter().map(|(p, _)| path_text(p)).collect();
    self.log(&format!("hooks: gate after turn ({} changed, {} tool calls reaching {:?}, round {round})", turn.len(), turn_tools.len(), tools.reach));
    // Edits made without a permission request get the beforeEdit check now
    if let Some(hook) = cfg.before_edit.as_ref().filter(|_| !audit.is_empty()) {
      let extra = json!({ "turn_tools": turn_tools, "file_sources": audit_sources });
      match self.run_before_edit(&cfg, hook, ToolKind::Edit, &audit, None, "audit", extra).await {
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
        "turn_tools": turn_tools,
        "file_sources": session_sources,
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

  /// Send what blocked back to the agent, run the gate once more over a detached turn that ended meanwhile, or let the
  /// queue go
  async fn gate_decided(self: &Arc<Self>, cfg: &HooksConfig, reasons: Vec<String>) {
    enum Next {
      FollowUp(String),
      Again(GateJob),
      Flush,
    }
    let next = {
      let mut c = self.core.lock();
      let ready = c.status == SessionStatus::Ready && !c.phase.running;
      let rounds = cfg.rounds.to_string();
      let next = if std::mem::take(&mut c.hooks.rerun) {
        // What blocked here is decided together with the detached turn's own findings
        self.log("hooks: gate runs again over the detached turn");
        c.hooks.denied.extend(reasons);
        match self.gate_job(&mut c) {
          Some(job) => Next::Again(job),
          None => Next::Flush,
        }
      } else if reasons.is_empty() || !ready {
        if reasons.is_empty() {
          self.log("hooks: gate passed");
        } else if c.peer.detached {
          // A detached turn took the session meanwhile: its own gate tells the agent
          self.log("hooks: gate blocked, kept for the detached turn");
          c.hooks.denied.extend(reasons);
        } else {
          self.log("hooks: gate blocked, session no longer ready");
        }
        Next::Flush
      } else if c.hooks.round < cfg.rounds {
        c.hooks.round += 1;
        let round = c.hooks.round.to_string();
        let details = reasons.join("\n\n");
        push_notice(&mut c, notice(Severity::Warning, tp("host.gateFailed", &[("round", &round), ("rounds", &rounds)]), Some(details.clone())));
        Next::FollowUp(tp("host.gatePrompt", &[("reason", &details)]))
      } else {
        self.log("hooks: gate gave up");
        push_notice(&mut c, notice(Severity::Error, tp("host.gateGaveUp", &[("rounds", &rounds)]), Some(reasons.join("\n\n"))));
        Next::Flush
      };
      // The follow-up keeps the session reserved until its claim, another run until it decides; otherwise the gate lets
      // go here (and the turn's lease with it, on this change)
      if matches!(next, Next::Flush) {
        c.hooks.gating = false;
      }
      self.touch(&mut c);
      next
    };
    match next {
      Next::FollowUp(text) => {
        self.log("hooks: gate blocked, sending the findings back");
        self.gate_follow_up(text).await;
      }
      Next::Again(job) => {
        tokio::spawn(self.clone().spawned_gate(job));
      }
      Next::Flush => self.after_prompt(false, TurnStop::EndTurn),
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
      mcp_server: None,
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
  fn only_known_tools_keep_the_snapshot_out() {
    let reach = |b: AgentBlock, f: &dyn Fn(&mut ToolCallBlock)| {
      let AgentBlock::ToolCall(mut tc) = b else { unreachable!() };
      f(&mut tc);
      row_reach(&tc)
    };
    let none = |_: &mut ToolCallBlock| {};
    assert_eq!(reach(tool(ToolKind::Read, None, &["/w/a"]), &none), Reach::None);
    assert_eq!(reach(tool(ToolKind::Other, None, &[]), &|t| t.verb_key = Some("verb.todo".into())), Reach::None);
    assert_eq!(reach(tool(ToolKind::Edit, None, &["/w/a"]), &none), Reach::Edits);
    assert_eq!(reach(tool(ToolKind::Edit, None, &["/w/a"]), &|t| t.status = ToolStatus::Failed), Reach::None);
    // Anything that may write where it does not say
    assert_eq!(reach(tool(ToolKind::Edit, None, &[]), &none), Reach::Any);
    assert_eq!(reach(tool(ToolKind::Execute, None, &[]), &|t| t.status = ToolStatus::Failed), Reach::Any);
    assert_eq!(reach(tool(ToolKind::Move, None, &["/w/a"]), &none), Reach::Any);
    assert_eq!(reach(tool(ToolKind::Other, None, &[]), &none), Reach::Any);
    assert_eq!(reach(tool(ToolKind::Other, None, &[]), &|t| t.mcp_server = Some("Acpira".into())), Reach::Any);
    assert_eq!(reach(tool(ToolKind::Think, None, &[]), &|t| t.subagent_id = Some("s1".into())), Reach::Any);
  }

  #[test]
  fn an_edit_names_its_locations_and_diff_sources_once() {
    let AgentBlock::ToolCall(mut tc) = tool(ToolKind::Edit, None, &["/w/a.kt"]) else { unreachable!() };
    tc.content = Some(ToolContent::Diff { lines: vec![], source: Some(DiffSource { path: "/w/a.kt".into(), ..Default::default() }) });
    tc.contents = Some(vec![ToolContent::Diff { lines: vec![], source: Some(DiffSource { path: "/w/b.kt".into(), ..Default::default() }) }]);
    assert_eq!(edit_paths(&tc), ["/w/a.kt", "/w/b.kt"]);
  }
}
