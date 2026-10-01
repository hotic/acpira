//! Per-agent wire quirks. The session state machine asks a `Vendor` what an agent does instead of comparing agent ids;
//! the modules below hold the details of each quirk. Keyed by the built-in agent id, so a user-defined agent that
//! launches one of these CLIs under another id gets none of them

pub mod claude_thinking;
pub mod claude_window;
pub mod grok;
pub mod pi_usage;
pub mod steering;

/// The built-in agents whose adapters the session handles specially
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vendor {
  Antigravity,
  Claude,
  Grok,
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
      "grok" => Vendor::Grok,
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

  /// The adapter's `usage_update.size` is corrected from the catalogue and confirmed after a clean turn (`claude_window`)
  pub fn corrects_window(self) -> bool {
    self == Vendor::Claude
  }

  /// session/new, resume and load ask for summarized thinking (`claude_thinking`)
  pub fn summarized_thinking(self) -> bool {
    self == Vendor::Claude
  }

  /// `ask_question` arrives as `session/request_permission` on an `interaction_*` tool call whose options are the
  /// answers, all `allow_once` (antigravity-acp 1.2.1 source): it opens a question card instead of a permission card
  pub fn question_permission(self, tool_call: &serde_json::Value) -> bool {
    self == Vendor::Antigravity
      && tool_call.get("toolCallId").and_then(serde_json::Value::as_str).is_some_and(|id| id.starts_with("interaction_"))
      && tool_call.get("rawInput").is_none_or(|r| r.is_null() || r.as_object().is_some_and(|o| o.is_empty()))
  }

  /// Leaving plan mode shows up only in the approval tool's output, without a current_mode_update (Kimi 0.41.0)
  pub fn plan_exit_in_tool_output(self) -> bool {
    self == Vendor::Kimi
  }
}
