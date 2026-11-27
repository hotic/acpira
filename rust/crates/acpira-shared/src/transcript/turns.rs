//! Turns: the user's prompt with its drafts and attachments, the agent's reply with usage, plugin cards and retries

use super::*;

/// Where a shared text came from. It rides on the `text` kind instead of a kind of its own: engines released before it
/// ignore the unknown field and read a plain text attachment, where an unknown kind would fail the whole record
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContext {
  /// Stable id of the sharing tool (`asgard.questCanvas`, `browser`)
  pub source_id: String,
  /// Its display name, written into the prompt header
  pub source_name: String,
  /// What the text was taken from (a document, a page URL)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub uri: Option<String>,
  /// The user's remark on it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Draft {
  #[serde(rename_all = "camelCase")]
  Image {
    mime_type: String,
    data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
  },
  Text {
    name: String,
    text: String,
    /// Set when the text was shared from another tool (an extension, a picked web page) rather than dropped as a file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<TextContext>,
  },
  File {
    uri: String,
    name: String,
    /// Base64 bytes of a file the webview only holds as a blob (an OS paste / drop): staged into the session's blob dir,
    /// whose path replaces `uri`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
  },
  /// A line range of an editor document; lines are 1-based and inclusive
  #[serde(rename_all = "camelCase")]
  Selection {
    uri: String,
    name: String,
    start_line: u32,
    end_line: u32,
    text: String,
  },
  /// Text quoted from the conversation with an optional remark
  Quote {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
  #[serde(rename_all = "camelCase")]
  Image {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
  },
  Text {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<TextContext>,
  },
  File {
    uri: String,
    name: String,
  },
  /// The selected text is in the blob store like dropped text
  #[serde(rename_all = "camelCase")]
  Selection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    uri: String,
    name: String,
    start_line: u32,
    end_line: u32,
  },
  Quote {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
  },
}

impl Attachment {
  /// What a chip / export line calls it; quotes have no name of their own
  pub fn label(&self) -> Option<String> {
    match self {
      Attachment::Image { name, .. } => name.clone(),
      Attachment::Text { name, .. } | Attachment::File { name, .. } => Some(name.clone()),
      Attachment::Selection { name, start_line, end_line, .. } => Some(format!("{name} {}", line_range_label(*start_line, *end_line))),
      Attachment::Quote { .. } => None,
    }
  }
}

/// `(12-19)`, or `(12)` for one line
pub fn line_range_label(start_line: u32, end_line: u32) -> String {
  if start_line == end_line { format!("({start_line})") } else { format!("({start_line}-{end_line})") }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode_id: Option<String>,
  #[serde(default)]
  pub config: StrMap,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTurn {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id: Option<String>,
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attachments: Option<Vec<Attachment>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub settings: Option<TurnSettings>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub edited: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub plan_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto: Option<bool>,
  /// Why an automatic turn was sent; absent on an automatic turn means the over-threshold /compact
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto_reason: Option<AutoReason>,
  /// With `AutoReason::Goal`: the goal's next turn, which the engine sent because the goal was still active when the turn
  /// before ended and the agent leaves continuing to the client (the built-in agent's `acpira/continue`); it reads as
  /// the reply before it carried on. A flag rather than an `AutoReason` value: `~/.acpira` is shared with engines released
  /// before it, which fail the whole record on an unknown value but skip an unknown field (and read a goal control)
  #[serde(default, skip_serializing_if = "is_false")]
  pub goal_continue: bool,
}

impl UserTurn {
  /// An active goal's next turn (`goal_continue`)
  pub fn is_goal_continue(&self) -> bool {
    self.auto == Some(true) && self.auto_reason == Some(AutoReason::Goal) && self.goal_continue
  }
}

/// What made Acpira send an automatic user turn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoReason {
  Compact,
  /// The previous turn ran out of account quota and the session moved to another account
  AccountSwitch,
  /// The previous turn failed after doing part of the work and the user pressed Retry: the agent carries on from its own
  /// context instead of receiving the message again
  Retry,
  /// The workspace gate (`.agents/hooks.json`) blocked the previous turn: its findings go back to the agent
  Gate,
  /// A goal control from the dock went out as its `/goal …` command; the goal row stands for it. Also the goal's next
  /// turn, with `UserTurn::goal_continue`
  Goal,
  /// Summoned agents the root sent off without waiting finished: their replies go to it in one message
  Subagents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStop {
  EndTurn,
  MaxTokens,
  MaxTurnRequests,
  Refusal,
  Cancelled,
  Error,
}

impl TurnStop {
  pub fn parse(s: &str) -> Option<TurnStop> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnError {
  pub message: String,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub code: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub kind: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retryable: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failure_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub actions: Option<Vec<FailureAction>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContextUse {
  pub used: Num,
  pub size: Num,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnUsage {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached_read: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached_write: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reasoning: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub total: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_calls: Option<Num>,
  /// Milliseconds the model calls spent streaming, first event to the end of each call, when the agent measures it
  /// itself (the built-in agent's `_meta.usage.decodeMs`); the output speed then needs no estimate
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub decode_ms: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub request_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context: Option<ContextUse>,
}

impl TurnUsage {
  /// `input` always counts the whole prompt, cache reads and writes included (OpenAI semantics, which Devin and the
  /// built-in agent report). claude-agent-acp, codex-acp and OpenCode report Anthropic semantics instead, where `input` is
  /// only the uncached remainder. The tally tells which one it is: a cache larger than `input` cannot be a part of it,
  /// and an exclusive tally adds up as `total = input + cache + output`. Folding makes `total = input + output`, so a
  /// second pass (a record saved after folding) leaves it alone
  pub fn fold_cache_into_input(&mut self) {
    let Some(input) = self.input else { return };
    let cache = self.cached_read.map_or(0.0, Num::get) + self.cached_write.map_or(0.0, Num::get);
    if cache <= 0.0 {
      return;
    }
    let output = self.output.map_or(0.0, Num::get);
    let reasoning = self.reasoning.map_or(0.0, Num::get);
    let sums_apart = self.total.is_some_and(|t| {
      let rest = t.get() - input.get() - cache - output;
      rest == 0.0 || rest == reasoning
    });
    if cache > input.get() || sums_apart {
      self.input = Some(Num(input.get() + cache));
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
  pub kind: ToolKind,
  pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOption {
  pub name: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReceipt {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub options: Option<Vec<CommandOption>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurn {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub observation: Option<Observation>,
  #[serde(default)]
  pub blocks: Vec<AgentBlock>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<TurnUsage>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub started_at: Option<i64>,
  /// When the first streamed block of a live prompt landed; the turn's clock counts from here, so the wait for the
  /// agent's first output is not shown as working time. `started_at` stays the turn's identity
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub responded_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub ended_at: Option<i64>,
  /// Milliseconds the live turn sat with only background work open and the model silent past a grace period
  /// (`normalize/idle.rs`): waiting on a background task, not generating. The output speed leaves it out
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub idle_ms: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub activity: Option<Activity>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stop: Option<TurnStop>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<TurnError>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<CommandReceipt>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retry: Option<TurnRetry>,
  /// Plugins an agent of this turn offered (`plugin_activate`), shown after the reply and clickable for good
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub plugin_cards: Vec<PluginCard>,
}

/// A plugin card (`docs/dev/plugins.md`, Activation): the frozen version an agent asked to enable, as the user sees it
/// in the transcript. Its state (offered, in effect, replaced) is read from the plugins' state, not kept here; only what
/// applying it from this card changed is, so the card can put it back
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCard {
  /// Unique within the session
  pub id: String,
  pub plugin: String,
  /// The frozen version it offers
  pub snapshot: String,
  pub name: String,
  #[serde(default)]
  pub description: String,
  /// `skin` (a theme), `ui` (interface pieces), `agent` (a specialised agent), `exec` (runs a program), `other`
  pub kind: String,
  /// What it runs, for `exec`: the first command, as the card names it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub runs: Option<String>,
  /// The slots its interface pieces take, for the thumbnail's map
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub slots: Vec<String>,
  /// The cron expressions of its schedules: it runs unattended, and may act on sessions while nobody watches
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub schedules: Vec<String>,
  /// What it contributes (`Contributes::kinds`: `tools`, `hooks`, `agent` …): where the card's mark flies once applied
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub adds: Vec<String>,
  /// The `STYLE_HOOKS` its stylesheet names, in the order they first appear: a style lands on those parts of the page
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub restyles: Vec<String>,
  /// The source folder, which the corner button reveals
  #[serde(default)]
  pub folder: String,
  #[serde(default, deserialize_with = "crate::num::lenient_i64")]
  pub at: i64,
  /// Set when it was applied from this card: what was in effect before
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub undo: Option<PluginCardUndo>,
  /// The tool call that offered it (the agent's `plugin_activate`): the page shows the card right after that row, where
  /// the agent asked, instead of under everything the turn streams later. Absent on older records: after the reply
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<String>,
}

/// What a plugin card's apply replaced: the plugin's own entry and the skin in effect
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCardUndo {
  /// The version enabled before, none when it was off
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snapshot: Option<String>,
  /// The skin before (the plugin id), empty for none; absent when the card is not a skin
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub skin: Option<String>,
}

/// A provider retry in progress. It lives in the turn's working label, never as a transcript row, and is cleared as
/// soon as the stream resumes or the turn ends
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRetry {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max: Option<u32>,
  /// The adapter's own wording (or the host's for prose retries), for the label's tooltip
  pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Turn {
  User(UserTurn),
  Agent(AgentTurn),
}

impl Turn {
  pub fn as_agent(&self) -> Option<&AgentTurn> {
    match self {
      Turn::Agent(a) => Some(a),
      Turn::User(_) => None,
    }
  }
  pub fn as_agent_mut(&mut self) -> Option<&mut AgentTurn> {
    match self {
      Turn::Agent(a) => Some(a),
      Turn::User(_) => None,
    }
  }
  pub fn as_user(&self) -> Option<&UserTurn> {
    match self {
      Turn::User(u) => Some(u),
      Turn::Agent(_) => None,
    }
  }
  pub fn is_agent(&self) -> bool {
    matches!(self, Turn::Agent(_))
  }
}
