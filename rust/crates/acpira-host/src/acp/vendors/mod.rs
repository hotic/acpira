//! Per-agent wire quirks. The session state machine asks a `Vendor` what an agent does instead of comparing agent ids;
//! the modules below hold the details of each quirk. Keyed by the built-in agent id, so a user-defined agent that
//! launches one of these CLIs under another id gets none of them

pub mod antigravity;
pub mod claude_auth;
pub mod claude_autonomous;
pub mod claude_retry;
pub mod claude_thinking;
pub mod claude_ultracode;
pub mod claude_wakeup;
pub mod claude_workflow;
pub mod claude_workflow_log;
pub mod claude_window;
pub(crate) mod droid_retry;
pub(crate) mod droid_usage;
pub mod goal;
pub mod grok;
pub mod hermes;
pub mod kimi_failure;
pub mod logged_failure;
pub mod pi_failure;
pub mod pi_usage;
pub mod steering;

use acpira_shared::transcript::ConfigControl;

/// The built-in agents whose adapters the session handles specially
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vendor {
  Antigravity,
  Claude,
  Codex,
  Droid,
  Grok,
  Hermes,
  Kimi,
  Pi,
  Other,
}

/// Where the context snapshot of an agent that never sends `usage_update` comes from
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UsagePoll {
  /// `_x.ai/session/info`
  Grok,
  /// The session file (`pi_usage`)
  Pi,
}

/// Where a CLI that swallows turn failures keeps them (`Vendor::failure_log`)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FailureLog {
  /// `wire.jsonl` under `KIMI_CODE_HOME` (`kimi_failure`)
  Kimi,
  /// pi's session file (`pi_failure`)
  Pi,
}

impl UsagePoll {
  /// The snapshot belongs to the model (Grok): a model switch drops it until the next read. Pi keeps the same messages
  /// and only the window moves
  pub fn per_model(self) -> bool {
    self == UsagePoll::Grok
  }
}

impl Vendor {
  pub fn of(agent: &str) -> Vendor {
    match agent {
      "antigravity" => Vendor::Antigravity,
      "claude" => Vendor::Claude,
      "codex" => Vendor::Codex,
      "droid" => Vendor::Droid,
      "grok" => Vendor::Grok,
      "hermes" => Vendor::Hermes,
      "kimi" => Vendor::Kimi,
      "pi" => Vendor::Pi,
      _ => Vendor::Other,
    }
  }

  /// Context usage polled while a prompt is on the wire, until the process sends a `usage_update` itself
  pub fn usage_poll(self) -> Option<UsagePoll> {
    match self {
      Vendor::Grok => Some(UsagePoll::Grok),
      Vendor::Pi => Some(UsagePoll::Pi),
      _ => None,
    }
  }

  /// The context snapshot arrives after the prompt response (Kimi): auto-compaction waits for it before deciding
  pub fn late_usage(self) -> bool {
    self == Vendor::Kimi
  }

  /// Tool titles carry the tool's name and a terminal result arrives as a Markdown list (`hermes::tidy_tool`); the
  /// reply ends with the reasoning and the reply again (`hermes::reasoning_recap`)
  pub fn hermes_output(self) -> bool {
    self == Vendor::Hermes
  }

  /// session/resume streams the history back like session/load (hermes-agent 0.22.5 `_session_response_fields`), so a
  /// resume over a local transcript is a replay too
  pub fn replays_on_resume(self) -> bool {
    self == Vendor::Hermes
  }

  /// The adapter's `usage_update.size` is corrected from the catalogue and confirmed after a clean turn (`claude_window`)
  pub fn corrects_window(self) -> bool {
    self == Vendor::Claude
  }

  /// The adapter streams turns of its own after `end_turn` (task-notification followups) and ends each with an
  /// origin-stamped `usage_update` (`claude_autonomous`)
  pub fn autonomous_cycles(self) -> bool {
    self == Vendor::Claude
  }

  /// A stop gives the steers the agent has not answered back to the queue, where the agent drops them on cancel
  /// (`steering::cancel_drops_unanswered` tells by the adapter version)
  pub fn requeues_steers_on_cancel(self) -> bool {
    self == Vendor::Claude
  }

  /// session/new, resume and load ask for summarized thinking (`claude_thinking`)
  pub fn summarized_thinking(self) -> bool {
    self == Vendor::Claude
  }

  /// session/new, resume and load ask for the raw workflow progress frames, and the CLI starts with dynamic workflows
  /// on by default (`claude_workflow`)
  pub fn workflows(self) -> bool {
    self == Vendor::Claude
  }

  /// A native subagent still running when its parent's prompt returns keeps streaming under its own session id and ends
  /// with the adapter's own state update: the next prompt's hand-off settles a held turn early, and a child launched in
  /// an autonomous cycle holds nothing (`agent-quirks-claude.md`). A droid `Task` runs past the turn that launched it
  /// and ends with its call's result (`load_child_sessions`)
  pub fn children_outlive_prompt(self) -> bool {
    matches!(self, Vendor::Claude | Vendor::Droid)
  }

  /// A delegation names its child session on the call (`_meta.subagent_session_info`), and the child streams only after
  /// a `session/load` of that id on the parent's process (`load_child_sessions`)
  pub fn loads_child_sessions(self) -> bool {
    self == Vendor::Droid
  }

  /// A `SendMessage` resume of a finished native subagent arrives as an announced run with no content of its own: the
  /// adapter drops it, so the run is read from the child's sidechain log (`SubagentTree` `resumed_task`)
  pub fn resumed_child_logs(self) -> bool {
    self == Vendor::Claude
  }

  /// The host offers ultracode as an "Ultra" effort level past Max, applied through the session settings
  /// (`claude_ultracode`); it needs dynamic workflows, so it is never offered without `workflows`
  pub fn ultracode(self) -> bool {
    self == Vendor::Claude && self.workflows()
  }

  /// `ask_question` arrives as `session/request_permission` on an `interaction_*` tool call whose options are the
  /// answers, all `allow_once` (antigravity-acp 1.2.1 source): it opens a question card instead of a permission card
  pub fn question_permission(self, tool_call: &serde_json::Value) -> bool {
    self == Vendor::Antigravity
      && tool_call.get("toolCallId").and_then(serde_json::Value::as_str).is_some_and(|id| id.starts_with("interaction_"))
      && tool_call.get("rawInput").is_none_or(|r| r.is_null() || r.as_object().is_some_and(|o| o.is_empty()))
  }

  /// A finished command row carries a timing receipt around its output (`antigravity::tidy_command`)
  pub fn command_receipt(self) -> bool {
    self == Vendor::Antigravity
  }

  /// A command sent to the background outlives its turn, and the reply that follows its exit is stored but never sent:
  /// the session reads it back from a replay (`session/late_reply.rs`)
  pub fn stores_late_replies(self) -> bool {
    self == Vendor::Antigravity
  }

  /// A delegation outlives the turn that launched it (droid's asynchronous `Task`), and the reply its end wakes the
  /// model for streams with no prompt running: its row carries a background task like Claude's background agents, and
  /// the reply runs as a detached turn (`session/late_reply.rs`)
  pub fn parks_delegations(self) -> bool {
    self == Vendor::Droid
  }

  /// Something a turn left running is followed past it (`session/late_reply.rs`)
  pub fn late_replies(self) -> bool {
    self.stores_late_replies() || self.parks_delegations()
  }

  /// A steer is a `session/prompt` of its own sent while the turn runs, which takes the turn over from the running one
  /// (`antigravity::steers_by_prompt`); the session follows the turn through every such prompt (`send_prompt`)
  pub fn steers_by_prompt(self, init: &serde_json::Value) -> bool {
    self == Vendor::Antigravity && antigravity::steers_by_prompt(init)
  }

  /// A failed turn arrives as the reply's last text with `end_turn` (`antigravity::reply_error`)
  pub fn reply_error(self, text: &str) -> Option<(usize, antigravity::ReplyError)> {
    if self == Vendor::Antigravity { antigravity::reply_error(text) } else { None }
  }

  /// A finished `/compact` sends nothing but an empty `end_turn` (`antigravity::compacted_silently`)
  pub fn compacted_silently(self, blocks: &[acpira_shared::transcript::AgentBlock]) -> bool {
    self == Vendor::Antigravity && antigravity::compacted_silently(blocks)
  }

  /// Where the note a cancelled turn's reply ends on starts (`antigravity::cancel_note`)
  pub fn cancel_note(self, text: &str) -> Option<usize> {
    if self == Vendor::Antigravity { antigravity::cancel_note(text) } else { None }
  }

  /// A failed turn arrives as an empty `end_turn`; the cause is only in the CLI's own session log (`logged_failure`)
  pub fn failure_log(self) -> Option<FailureLog> {
    match self {
      Vendor::Kimi => Some(FailureLog::Kimi),
      Vendor::Pi => Some(FailureLog::Pi),
      _ => None,
    }
  }

  /// `_session/goal` `clear` starts a turn of the agent's own (claude-agent-acp runs it as the `/goal clear` command);
  /// codex-acp only clears the goal (`goal::starts_turn`)
  pub fn goal_clear_starts_turn(self) -> bool {
    self != Vendor::Codex
  }

  /// The goal is a Stop hook that keeps the turn going until the condition holds, so a turn that ends on its own ended
  /// with the goal met (`goal::met_by_turn_end`)
  pub fn goal_met_at_turn_end(self) -> bool {
    self == Vendor::Claude
  }

  /// A `/goal` command is answered with its local output as reply text (`goal::is_command_echo`)
  pub fn echoes_goal_command(self) -> bool {
    self == Vendor::Claude
  }

  /// Leaving plan mode shows up only in the approval tool's output, without a current_mode_update (Kimi 0.41.0)
  pub fn plan_exit_in_tool_output(self) -> bool {
    self == Vendor::Kimi
  }

  /// Tidy Claude's session controls at every boundary (probes, startup previews, live sessions, reopened records):
  /// - Drop `ultracode`: claude-agent-acp 0.97.0+ publishes its own boolean, while the host's Ultra effort level stands
  ///   for it (`session/ultracode.rs`).
  /// - Drop leaked built-in subagents (`web-fetch`) from Claude's `agent` persona select, and drop the select itself
  ///   when only `default` remains: claude-agent-acp 0.108.1's `BUILTIN_AGENT_NAMES` misses Claude Code 2.1.299's
  ///   built-in `web-fetch` subagent. Keep an entry while it is the session's current value so an already-selected
  ///   session can still switch back to `default`.
  pub fn tidy_controls(self, options: &mut Vec<ConfigControl>) {
    if self != Vendor::Claude {
      return;
    }
    options.retain_mut(|ctl| {
      if self.ultracode() && ctl.id == claude_ultracode::SETTINGS_KEY {
        return false;
      }
      if ctl.id != "agent" {
        return true;
      }
      ctl.options.retain(|o| o.id != "web-fetch" || ctl.value.as_deref() == Some(o.id.as_str()));
      ctl.options.iter().any(|o| o.id != "default")
    });
  }
}

#[cfg(test)]
mod tests {
  use acpira_shared::transcript::SessionOption;

  use super::*;

  fn agent_control(value: Option<&str>, ids: &[&str]) -> ConfigControl {
    ConfigControl {
      id: "agent".into(),
      name: "Agent".into(),
      category: None,
      kind: None,
      options: ids.iter().map(|&id| SessionOption { id: id.into(), name: id.into(), ..Default::default() }).collect(),
      value: value.map(str::to_owned),
    }
  }

  fn model_control() -> ConfigControl {
    ConfigControl {
      id: "model".into(),
      name: "Model".into(),
      category: Some("model".into()),
      kind: None,
      options: vec![SessionOption { id: "opus".into(), name: "Opus".into(), ..Default::default() }],
      value: Some("opus".into()),
    }
  }

  #[test]
  fn claude_drops_agent_control_when_only_leaked_web_fetch_was_offered() {
    let mut options = vec![model_control(), agent_control(Some("default"), &["default", "web-fetch"])];
    Vendor::Claude.tidy_controls(&mut options);
    assert_eq!(options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model"]);
  }

  #[test]
  fn claude_keeps_custom_agents_without_web_fetch() {
    let mut options = vec![agent_control(Some("default"), &["default", "web-fetch", "reviewer"])];
    Vendor::Claude.tidy_controls(&mut options);
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["default", "reviewer"]);
  }

  #[test]
  fn claude_keeps_web_fetch_while_it_is_the_selected_value() {
    let mut options = vec![agent_control(Some("web-fetch"), &["default", "web-fetch"])];
    Vendor::Claude.tidy_controls(&mut options);
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["default", "web-fetch"]);
  }

  #[test]
  fn other_vendors_leave_agent_control_alone() {
    let mut options = vec![agent_control(Some("default"), &["default", "web-fetch"])];
    Vendor::Codex.tidy_controls(&mut options);
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["default", "web-fetch"]);
  }

  #[test]
  fn claude_drops_ultracode_boolean_control_while_other_vendors_keep_it() {
    let ultracode = ConfigControl {
      id: claude_ultracode::SETTINGS_KEY.into(),
      name: "Ultracode".into(),
      category: Some("model_config".into()),
      kind: Some(acpira_shared::transcript::ControlType::Boolean),
      options: vec![
        SessionOption { id: "false".into(), name: "Off".into(), ..Default::default() },
        SessionOption { id: "true".into(), name: "On".into(), ..Default::default() },
      ],
      value: Some("false".into()),
    };
    let mut claude_opts = vec![model_control(), ultracode.clone()];
    Vendor::Claude.tidy_controls(&mut claude_opts);
    assert_eq!(claude_opts.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model"]);

    let mut other_opts = vec![model_control(), ultracode];
    Vendor::Codex.tidy_controls(&mut other_opts);
    assert_eq!(other_opts.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model", "ultracode"]);
  }
}
