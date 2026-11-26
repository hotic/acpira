//! Antigravity ends a failed turn with `end_turn` and the failure as the turn's last `agent_message_chunk`. `agy acp`
//! (1.5.9, verified live with an exhausted quota) sends it as its own chunk, one line `\n⚠ <text>\n`, e.g.
//! `⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 46m9s.`. The other
//! patterns are the earlier `agy_acp_server` package's (`server.py` `prompt`, 1.1.1 / 1.2.1 sources), unverified against
//! `agy acp`: a failed MCP server load, an exhausted quota, a browser-tool start failure, a lost harness connection, or
//! any other SDK error as `Agent execution error: <e>`. `reply_error` finds that tail so the session can show it as the
//! turn's error instead of a reply
//!
//! A `run_command` sent to the background outlives its turn, and the reply the CLI has the model write when it exits is
//! stored but never sent. `session/load` on the live process replays the conversation with it: the `replay_*` helpers
//! tell what such a replay holds that the transcript does not (`session/late_reply.rs`)
//!
//! Steering is a second `session/prompt` sent while one runs (`agy acp` from 1.5.0, its changelog; verified on 1.5.12 with
//! `scripts/probe/steering.ts --via-prompt`): the running prompt answers `end_turn` at once with its usage so far, and the new
//! one owns the rest of the turn, a running tool included, until it answers itself. The steer is taken up at the next
//! model call and never echoed. A cancel answers the owning prompt `cancelled`, yet the steer stays in the conversation

use std::ffi::OsStr;
use std::sync::LazyLock;

use acpira_shared::model_shapes::is_model_control;
use acpira_shared::transcript::{AgentBlock, ConfigControl};
use regex::Regex;
use serde_json::Value;

use crate::platform::command::Os;

/// Disables `agy`'s detached `--bg-updater` spawn (exact lowercase `"true"`, verified on `agy` 1.5.14): on Windows
/// `--bg-updater` detaches from the hidden console and then runs `agy --version` without `CREATE_NO_WINDOW`, flashing a
/// console window over the desktop
pub const DISABLE_AUTO_UPDATE_ENV: &str = "AGY_CLI_DISABLE_AUTO_UPDATE";

/// Whether a long-lived `agy acp` process should default `DISABLE_AUTO_UPDATE_ENV` to `"true"`: Windows only, and only
/// when the host environment did not set it, so an explicit setting still wins
pub fn suppress_acp_auto_update(os: Os, inherited: Option<&OsStr>) -> bool {
  os == Os::Windows && inherited.is_none()
}

/// A `session/prompt` sent while a turn runs steers it (`agy acp` 1.5.0 and later): `steering::supported` stays false, as
/// the CLI has no `_session/steering`. The older `agy_acp_server` package reported versions below that
pub fn steers_by_prompt(init: &Value) -> bool {
  init.pointer("/agentInfo/version").and_then(Value::as_str).and_then(super::steering::version_triple).is_some_and(|v| v >= (1, 5, 0))
}

/// The refusal of a prompt sent while the running one waits on a permission or question card (1.5.12: `Can't add this
/// prompt to session <id>: the running prompt is waiting for a permission or question answer; answer it or send
/// session/cancel, then send this prompt again`)
pub fn steer_bounced(message: &str) -> bool {
  message.contains("waiting for a permission or question answer")
}

/// What the failure text says
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyError {
  /// `mcp_errors.mcp_load_failed_message`: the executor could not start this server, so no turn can run while the
  /// session carries it
  McpFailed { server: String, reason: Option<String> },
  /// Google refused the request from this network's region (`User location is not supported for the API use.`)
  Region,
  /// `⚠ Authentication expired. Please log back in.` (agy 1.5.13): the process reads its token once, at start, and a
  /// failed refresh ends every later turn so. A ticket renewed in its token file since (a plugin's account) reaches only a
  /// new process
  AuthExpired(String),
  /// Already worded for a person (quota, browser tool, lost connection, other SDK errors)
  Other(String),
}

/// Where the failure starts in the last text block, and what it is. None = an ordinary reply
pub fn reply_error(text: &str) -> Option<(usize, ReplyError)> {
  let at = TAIL.find_iter(text).last()?.start();
  let tail = text.get(at..)?.trim();
  if let Some(m) = MCP.captures(tail) {
    let reason = m.get(2).map(|r| r.as_str().trim().to_owned()).filter(|r| !r.is_empty());
    return Some((at, ReplyError::McpFailed { server: m[1].to_owned(), reason }));
  }
  if let Some(rest) = tail.strip_prefix(EXECUTION) {
    if rest.contains("User location is not supported") {
      return Some((at, ReplyError::Region));
    }
    // `Agent execution terminated due to error. ("<cause>")`: the cause alone reads better
    let cause = WRAPPED.captures(rest).map(|m| m[1].to_owned()).unwrap_or_else(|| rest.trim().to_owned());
    return Some((at, ReplyError::Other(cause)));
  }
  if let Some(line) = tail.strip_prefix(WARNING) {
    // A reply of the model's own that ends on a warning sign spans more than the one line the CLI sends
    if line.contains('\n') {
      return None;
    }
    let line = line.trim().to_owned();
    return Some((at, if line.starts_with("Authentication expired") { ReplyError::AuthExpired(line) } else { ReplyError::Other(line) }));
  }
  Some((at, ReplyError::Other(tail.to_owned())))
}

/// `agy acp` 1.5.12 answers `session/cancel` mid-stream by recording `The stream was interrupted. Please continue the
/// task you were working on.` for its model and sending it as a warning line, as it does a failure. The cancel is no
/// failure and the stop row already shows it: where that line starts, when the last text block ends on it
pub fn cancel_note(text: &str) -> Option<usize> {
  let body = text.trim_end();
  let at = body.rfind(CANCEL_NOTE)?;
  let (before, line) = body.split_at_checked(at)?;
  ((before.is_empty() || before.ends_with('\n')) && !line.contains('\n')).then_some(at)
}

/// `agy acp` 1.5.13 compacts on `/compact` with nothing on the wire: no `compaction_update`, no words, only `end_turn`
/// (its own transcript logs the checkpoint). A `/compact` turn with no compaction row and no reply or failure text is
/// that compaction, even if stray tool calls or thoughts from an overlapping detached run landed in it before `/compact`
/// ran; any failure line, reply, or explicit compaction row is left as the CLI sent it
pub fn compacted_silently(blocks: &[AgentBlock]) -> bool {
  !blocks.iter().any(|b| match b {
    AgentBlock::Compaction(_) => true,
    AgentBlock::Text(t) => !t.markdown.trim().is_empty(),
    _ => false,
  })
}

/// `agy acp` 1.5.9 finishes a `run_command` row with a receipt as its text (and `rawOutput.output`): `Created At: …`,
/// `Completed At: …`, `Command exited with code N.`, a blank line, `Command output:` and then the output. The row's
/// output card gets the output alone, with a failing exit code noted under it as for the other agents; any other text
/// stays as the CLI wrote it
pub fn tidy_command(u: &mut Value) {
  if u.get("kind").and_then(Value::as_str) != Some("execute") {
    return;
  }
  let Some(items) = u.get_mut("content").and_then(Value::as_array_mut) else { return };
  for item in items.iter_mut() {
    if let Some(out) = item.pointer("/content/text").and_then(Value::as_str).and_then(command_output) {
      item["content"]["text"] = Value::from(out);
    }
  }
}

fn command_output(text: &str) -> Option<String> {
  let m = RECEIPT.captures(text)?;
  let code: i64 = m[1].parse().ok()?;
  let out = m.get(2).map_or("", |o| o.as_str()).trim_end_matches(['\r', '\n']);
  // The same note the normalizer appends to other agents' receipts (`append_exit_code`)
  Some(match code {
    0 => out.to_owned(),
    _ if out.is_empty() => format!("exit code {code}"),
    _ => format!("{out}\nexit code {code}"),
  })
}

/// The task a background command's row carries, before the call's id
pub const COMMAND_TASK: &str = "agy-command:";

/// A child delegation launched by the root agent: either a native `/swarm` worker add or `invoke_subagent`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AntigravityDelegation {
  /// `swarm { action: "add", name, brief, model?, n?, tasks? }`
  SwarmAdd {
    /// Expected worker roster name(s) (`<name>` for a single worker, `<name>-1..k` when `n > 1` or `tasks` is set)
    names: Vec<String>,
    brief: Option<String>,
    model: Option<String>,
  },
  /// `invoke_subagent { TypeName, Role, Prompt, Model? }`
  InvokeSubagent {
    role: Option<String>,
    type_name: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
  },
}

/// Canonical 36-byte hyphenated UUID (`8-4-4-4-12` ASCII hex), as `agy` mints conversation ids
fn is_uuid(s: &str) -> bool {
  let b = s.as_bytes();
  if b.len() != 36 {
    return false;
  }
  for (i, &c) in b.iter().enumerate() {
    let hyphen = matches!(i, 8 | 13 | 18 | 23);
    if hyphen {
      if c != b'-' {
        return false;
      }
    } else if !c.is_ascii_hexdigit() {
      return false;
    }
  }
  true
}

/// `agy acp` 1.5.16 forwards child subagent / swarm worker tool calls on the root stream with `toolCallId` prefixed by
/// the child's conversation UUID (`<child_uuid>:<inner_call_id>`). Returns `(child_uuid, inner_call_id)` when `id`
/// belongs to a child conversation other than `root_session_id`
pub fn split_child_tool_call_id<'a>(id: &'a str, root_session_id: Option<&str>) -> Option<(&'a str, &'a str)> {
  let (child, inner) = id.split_once(':')?;
  if inner.is_empty() || !is_uuid(child) || root_session_id == Some(child) {
    return None;
  }
  Some((child, inner))
}

/// Whether a root `session/update` is a tool call belonging to an Antigravity child conversation
pub fn is_child_tool_call(u: &Value, root_session_id: Option<&str>) -> bool {
  matches!(kind_of(u), "tool_call" | "tool_call_update")
    && u.get("toolCallId").and_then(Value::as_str).and_then(|id| split_child_tool_call_id(id, root_session_id)).is_some()
}

fn non_empty_str(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn tool_name_of(u: &Value) -> Option<&str> {
  if let Some(name) = u.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
    return Some(name);
  }
  let title = u.get("title").and_then(Value::as_str)?.trim();
  Some(title.strip_prefix("Run ").unwrap_or(title))
}

/// Recognizes a root Antigravity delegation call (`swarm` with `action: "add"`, or `invoke_subagent`) from its tool
/// name / title and `rawInput`
pub fn delegation_of(u: &Value) -> Option<AntigravityDelegation> {
  let tool = tool_name_of(u)?;
  let raw = u.get("rawInput").and_then(Value::as_object)?;
  match tool {
    "swarm" => {
      if raw.get("action").and_then(Value::as_str) != Some("add") {
        return None;
      }
      let base = non_empty_str(raw.get("name"))?;
      let shard_count = raw
        .get("tasks")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .map(Vec::len)
        .or_else(|| raw.get("n").and_then(Value::as_u64).filter(|&n| n > 1).map(|n| n as usize));
      let names = match shard_count {
        Some(k) => (1..=k.min(32)).map(|i| format!("{base}-{i}")).collect(),
        None => vec![base],
      };
      let brief = non_empty_str(raw.get("brief"));
      let model = non_empty_str(raw.get("model"));
      Some(AntigravityDelegation::SwarmAdd { names, brief, model })
    }
    "invoke_subagent" => {
      let role = non_empty_str(raw.get("Role").or_else(|| raw.get("role")));
      let type_name = non_empty_str(raw.get("TypeName").or_else(|| raw.get("typeName")));
      let prompt = non_empty_str(raw.get("Prompt").or_else(|| raw.get("prompt")));
      let model = non_empty_str(raw.get("Model").or_else(|| raw.get("model"))).filter(|m| m != "inherit");
      if role.is_none() && type_name.is_none() && prompt.is_none() {
        return None;
      }
      Some(AntigravityDelegation::InvokeSubagent { role, type_name, prompt, model })
    }
    _ => None,
  }
}

/// Antigravity's orchestrator models (`gemini-pro-orchestrator[-<lead>]`, agy 1.5.16): a planning model and a faster
/// execution model take turns inside the one conversation (`subagent_tree/orchestrator.rs`)
pub fn is_orchestrator(model_id: &str) -> bool {
  model_id.contains("orchestrator")
}

/// The execution model behind the Gemini orchestrators, as `transcript.jsonl` names its steps (agy 1.5.16). The menu
/// offers it nowhere; it stands in until the session's own transcript names the model that ran
pub const ORCHESTRATOR_WORKER: &str = "gemini-flash-worker-agy";

/// One side of an orchestrator's relay on the root stream: the planner's `exit_plan_mode` carrying the plan, or the
/// executor's `enter_plan_mode` carrying its report (both `tool_call`s of kind `other`, agy 1.5.16)
#[derive(Debug, PartialEq, Eq)]
pub enum PlanRelay {
  Handoff { plan: Option<String> },
  Handback { reason: Option<String> },
}

pub fn plan_relay_of(u: &Value) -> Option<PlanRelay> {
  if !matches!(kind_of(u), "tool_call" | "tool_call_update") {
    return None;
  }
  let raw = u.get("rawInput").or_else(|| u.get("args"));
  let arg = |key: &str| non_empty_str(raw.and_then(|r| r.get(key)));
  match tool_name_of(u)? {
    "exit_plan_mode" => Some(PlanRelay::Handoff { plan: arg("plan") }),
    "enter_plan_mode" => Some(PlanRelay::Handback { reason: arg("reason") }),
    _ => None,
  }
}

/// A plan's one-line title: its first line of prose, since a plan opens with a generic heading ("Plan", "实施计划")
pub fn plan_title(plan: &str) -> Option<String> {
  static NUMBERED: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^\d+[.)]\s+"));
  let lines = plan.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("```"));
  let prose = lines.clone().find(|l| !l.starts_with('#')).or_else(|| lines.clone().next())?;
  let line = prose.trim_start_matches(['#', '>', '-', '*', ' ']);
  let line = NUMBERED.replace(line, "").replace("**", "").replace('`', "");
  let line = line.trim();
  (!line.is_empty()).then(|| {
    let clipped: String = line.chars().take(PLAN_TITLE_MAX).collect();
    if clipped.len() < line.len() { format!("{clipped}…") } else { clipped }
  })
}

const PLAN_TITLE_MAX: usize = 80;

/// What a relay round calls the execution model a transcript step names: the model select's own label for it (a level
/// suffix aside), else the id spelled out (`gemini-flash-worker-agy` → `Gemini Flash Worker`)
pub fn worker_name(id: &str, controls: &[ConfigControl]) -> String {
  static LEVEL: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"-(low|medium|high|xhigh)$"));
  let options = controls.iter().filter(|c| is_model_control(c)).flat_map(|c| &c.options);
  let family = LEVEL.replace(id, "");
  if let Some(o) = options.clone().find(|o| o.id == id).or_else(|| options.clone().find(|o| o.id == family)) {
    return o.name.clone();
  }
  let bare = family.strip_suffix("-agy").unwrap_or(&family);
  bare
    .split('-')
    .filter(|w| !w.is_empty())
    .map(|w| {
      let mut c = w.chars();
      c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
    })
    .collect::<Vec<_>>()
    .join(" ")
}

/// The agent's own updates, never the prompts: what a replay may add to the transcript, and what a late reply streams
pub const REPLAY_AGENT_KINDS: [&str; 5] = ["agent_message_chunk", "agent_thought_chunk", "tool_call", "tool_call_update", "plan"];

fn kind_of(u: &Value) -> &str {
  u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("")
}

fn squash(text: &str) -> String {
  text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How much of a replay the transcript already holds: up to the reply its last turn ended on or the last call that
/// turn made, whichever comes later. A replay read as the turn ends may already hold what follows a command that
/// exited at once, so its length alone is the answer only when neither is found. A replay gives each message whole
/// where the wire sent chunks, hence the whitespace-blind comparison
pub fn replay_known(replay: &[Value], last_reply: Option<&str>, has_call: impl Fn(&str) -> bool) -> usize {
  let reply = last_reply.map(squash).filter(|r| !r.is_empty());
  let held = |u: &Value| match kind_of(u) {
    "agent_message_chunk" => reply.as_deref().is_some_and(|r| u.pointer("/content/text").and_then(Value::as_str).is_some_and(|t| squash(t) == r)),
    "tool_call" | "tool_call_update" => u.get("toolCallId").and_then(Value::as_str).is_some_and(&has_call),
    _ => false,
  };
  replay.iter().rposition(held).map_or(replay.len(), |at| at + 1)
}

/// The row of this call in a replay once its command is over, with the receipt the wire never carried
pub fn replay_closed<'a>(replay: &'a [Value], id: &str) -> Option<&'a Value> {
  replay.iter().rev().find(|u| {
    matches!(kind_of(u), "tool_call" | "tool_call_update")
      && u.get("toolCallId").and_then(Value::as_str) == Some(id)
      && matches!(u.get("status").and_then(Value::as_str), Some("completed" | "failed" | "cancelled"))
  })
}

/// What the agent produced past the known part of a replay
pub fn replay_tail(replay: &[Value], known: usize) -> Vec<Value> {
  replay.iter().skip(known).filter(|u| REPLAY_AGENT_KINDS.contains(&kind_of(u))).cloned().collect()
}

static RECEIPT: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r"(?s)\A(?:Created At: [^\n]*\n)?(?:Completed At: [^\n]*\n)?Command exited with code (-?\d+)\.\n\nCommand output:\n?(.*)\z")
});

/// How `agy acp` marks the line it reports a failed turn with
const WARNING: &str = "\u{26a0} ";

const EXECUTION: &str = "Agent execution error: ";

const CANCEL_NOTE: &str = "\u{26a0} The stream was interrupted.";

/// The start of every failure text `prompt` sends, each at the start of a line
static TAIL: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(
    r"(?m)^(?:\x{26a0} |Agent execution error: |The MCP server '[A-Za-z0-9_-]+' failed to initialize|Usage Limit Reached\n\nYou have reached your current quota|Browser automation could not start \(|Agent connection was lost and could not be re-established: )",
  )
});
static MCP: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r"(?s)^The MCP server '([A-Za-z0-9_-]+)' failed to initialize(?:: (.+?))?\. Fix the MCP server, or remove it from your configuration")
});
static WRAPPED: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r#"(?s)^Agent execution terminated due to error\. \("(.*)"\)\s*$"#));

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn orchestrator_hand_offs_titles_and_worker_names() {
    use serde_json::json;
    // agy 1.5.16 names the tool beside the ACP fields and titles the row `Run <tool>`
    let exit = json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "name": "exit_plan_mode", "title": "Run exit_plan_mode", "kind": "other", "rawInput": { "plan": "### 实施计划\n\n1. **创建脚本文件**：用 `write_file` 写 fib.py" } });
    assert_eq!(plan_relay_of(&exit), Some(PlanRelay::Handoff { plan: Some("### 实施计划\n\n1. **创建脚本文件**：用 `write_file` 写 fib.py".into()) }));
    let enter = json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c2", "title": "Run enter_plan_mode", "args": { "reason": "tests fail twice" } });
    assert_eq!(plan_relay_of(&enter), Some(PlanRelay::Handback { reason: Some("tests fail twice".into()) }));
    assert_eq!(plan_relay_of(&json!({ "sessionUpdate": "tool_call", "toolCallId": "c3", "name": "run_command", "title": "ls" })), None);
    assert_eq!(plan_relay_of(&json!({ "sessionUpdate": "agent_message_chunk", "name": "exit_plan_mode" })), None);
    // The title skips the plan's generic heading
    assert_eq!(plan_title("### 实施计划\n\n1. **创建脚本文件**：用 `write_file` 写 fib.py").as_deref(), Some("创建脚本文件：用 write_file 写 fib.py"));
    assert_eq!(plan_title("# Fix the parser").as_deref(), Some("Fix the parser"));
    assert_eq!(plan_title("```\n```"), None);
    assert!(is_orchestrator("gemini-pro-orchestrator") && is_orchestrator("gemini-pro-orchestrator-neon") && !is_orchestrator("gemini-4-neon"));
    // The worker's name: the model select's label, a level suffix aside, else the id spelled out
    let option = |id: &str, name: &str| acpira_shared::transcript::SessionOption { id: id.into(), name: name.into(), ..Default::default() };
    let model = ConfigControl {
      id: "model".into(),
      category: Some("model".into()),
      options: vec![option("gemini-4-hydrogen", "Gemini 4 Hydrogen"), option("claude-sonnet-5-5", "Claude Sonnet 5.5")],
      ..Default::default()
    };
    let controls = [model];
    assert_eq!(worker_name("claude-sonnet-5-5", &controls), "Claude Sonnet 5.5");
    assert_eq!(worker_name("gemini-4-hydrogen-high", &controls), "Gemini 4 Hydrogen");
    assert_eq!(worker_name(ORCHESTRATOR_WORKER, &controls), "Gemini Flash Worker");
  }

  #[test]
  fn agy_acp_steers_by_prompt_from_1_5_0() {
    let at = |v: &str| steers_by_prompt(&serde_json::json!({ "agentInfo": { "name": "Antigravity ACP Server", "version": v } }));
    assert!(at("1.5.0") && at("1.5.12") && at("2.0.0"));
    assert!(!at("1.2.1") && !at("1.4.12"));
    assert!(!steers_by_prompt(&serde_json::json!({})));
  }

  #[test]
  fn acp_auto_update_is_suppressed_only_on_windows_when_unset() {
    assert!(suppress_acp_auto_update(Os::Windows, None));
    assert!(!suppress_acp_auto_update(Os::Windows, Some(OsStr::new("false"))));
    assert!(!suppress_acp_auto_update(Os::Windows, Some(OsStr::new("true"))));
    assert!(!suppress_acp_auto_update(Os::Posix, None));
  }

  #[test]
  fn only_a_compact_turn_without_reply_or_compaction_row_compacted_silently() {
    use acpira_shared::transcript::{CompactionBlock, CompactionStatus, TextBlock};
    let text = |m: &str| AgentBlock::Text(TextBlock { id: None, phase: None, markdown: m.into(), streaming: None, images: None, ended_at: None, resumed_at: None });
    let edit: AgentBlock = serde_json::from_value(serde_json::json!({ "type": "tool_call", "id": "c1", "kind": "edit", "verb": "Edit", "status": "completed" })).unwrap();
    assert!(compacted_silently(&[]));
    assert!(compacted_silently(&[text(""), text("\n")]));
    // Stray work from an overlapping detached run does not hide the silent compaction
    assert!(compacted_silently(std::slice::from_ref(&edit)));
    assert!(compacted_silently(&[edit.clone(), text("\n")]));
    // A failure line or a reply is the CLI's own word on it
    assert!(!compacted_silently(&[text("\n⚠ Individual quota reached.\n")]));
    assert!(!compacted_silently(&[edit, text("\n⚠ Individual quota reached.\n")]));
    // A compaction the CLI reported itself needs no stand-in
    let row = AgentBlock::Compaction(CompactionBlock { id: "c1".into(), status: CompactionStatus::Completed, error: None });
    assert!(!compacted_silently(&[row]));
  }

  #[test]
  fn an_mcp_load_failure_names_the_server_and_its_reason() {
    let text = "The MCP server 'hilfa' failed to initialize: calling \"initialize\": rejected by transport: dial tcp [::1]:4262: connect: connection refused. Fix the MCP server, or remove it from your configuration and start a new session, to continue.";
    let (at, e) = reply_error(text).unwrap();
    assert_eq!(at, 0);
    assert_eq!(
      e,
      ReplyError::McpFailed {
        server: "hilfa".into(),
        reason: Some("calling \"initialize\": rejected by transport: dial tcp [::1]:4262: connect: connection refused".into())
      }
    );
    let bare = "The MCP server 'x_1' failed to initialize. Fix the MCP server, or remove it from your configuration and start a new session, to continue.";
    assert_eq!(reply_error(bare).unwrap().1, ReplyError::McpFailed { server: "x_1".into(), reason: None });
  }

  #[test]
  fn an_expired_sign_in_is_told_apart_from_other_warnings() {
    let text = "Running the tests.\n⚠ Authentication expired. Please log back in.\n";
    let (at, e) = reply_error(text).unwrap();
    assert_eq!((text.get(..at), e), (Some("Running the tests.\n"), ReplyError::AuthExpired("Authentication expired. Please log back in.".into())));
    assert_eq!(reply_error("\n⚠ Individual quota reached.\n").unwrap().1, ReplyError::Other("Individual quota reached.".into()));
  }

  #[test]
  fn an_execution_error_keeps_only_its_cause() {
    let region = "Agent execution error: Agent execution terminated due to error. (\"request failed (code 400): User location is not supported for the API use.\")";
    assert_eq!(reply_error(region).unwrap().1, ReplyError::Region);
    let other = "Agent execution error: Agent execution terminated due to error. (\"request failed (code 503): overloaded\")";
    assert_eq!(reply_error(other).unwrap().1, ReplyError::Other("request failed (code 503): overloaded".into()));
    assert_eq!(reply_error("Agent execution error: boom").unwrap().1, ReplyError::Other("boom".into()));
  }

  #[test]
  fn the_failure_is_found_after_streamed_output_and_plain_replies_are_left_alone() {
    let text = "Looking at the file now.\n\nUsage Limit Reached\n\nYou have reached your current quota for this period.";
    let (at, e) = reply_error(text).unwrap();
    assert_eq!(text.get(..at), Some("Looking at the file now.\n\n"));
    assert_eq!(e, ReplyError::Other("Usage Limit Reached\n\nYou have reached your current quota for this period.".into()));
    assert!(reply_error("The error said: Agent execution error: inline, mid-sentence").is_none());
    assert!(reply_error("All done.").is_none());
  }

  #[test]
  fn a_command_row_keeps_only_the_output_of_its_receipt() {
    let receipt = "Created At: 2026-11-15T15:35:15+09:00\nCompleted At: 2026-11-15T15:35:15+09:00\nCommand exited with code 0.\n\nCommand output:\nacpira-shell-ok\r\n";
    let mut u = serde_json::json!({ "kind": "execute", "content": [{ "type": "content", "content": { "type": "text", "text": receipt } }] });
    tidy_command(&mut u);
    assert_eq!(u["content"][0]["content"]["text"], "acpira-shell-ok");
    assert_eq!(command_output("Command exited with code 2.\n\nCommand output:\nno such file\n").as_deref(), Some("no such file\nexit code 2"));
    assert_eq!(command_output("Command exited with code 1.\n\nCommand output:\n").as_deref(), Some("exit code 1"));
    assert_eq!(command_output("Command is still running in the background."), None);
    let mut read = serde_json::json!({ "kind": "read", "content": [{ "type": "content", "content": { "type": "text", "text": receipt } }] });
    tidy_command(&mut read);
    assert_eq!(read["content"][0]["content"]["text"], receipt);
  }

  #[test]
  fn a_replay_is_split_at_what_the_transcript_holds() {
    use serde_json::json;
    let text = |kind: &str, t: &str| json!({ "sessionUpdate": kind, "content": { "type": "text", "text": t } });
    let call = |status: &str| json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "kind": "execute", "status": status });
    let started = "Started it; I will\nreport back.";
    // As the turn ends: the prompt, a thought, the command still running, the reply
    let mut replay = vec![text("user_message_chunk", "run it"), text("agent_thought_chunk", "hm"), call("in_progress"), text("agent_message_chunk", started)];
    let ours = |id: &str| id == "c1";
    assert_eq!(replay_known(&replay, Some("Started it; I will report back."), ours), 4);
    assert!(replay_closed(&replay, "c1").is_none());
    assert!(replay_tail(&replay, 4).is_empty());
    // The command exited and the model wrote on
    replay[2] = call("completed");
    replay.extend([text("agent_thought_chunk", "done"), text("user_message_chunk", "typed elsewhere"), text("agent_message_chunk", "Output: ok")]);
    assert_eq!(replay_closed(&replay, "c1").unwrap()["status"], "completed");
    assert!(replay_closed(&replay, "c2").is_none());
    let tail = replay_tail(&replay, 4);
    assert_eq!(tail.iter().map(|u| u["sessionUpdate"].as_str().unwrap()).collect::<Vec<_>>(), ["agent_thought_chunk", "agent_message_chunk"]);
    // A first read that already holds the late reply still ends at the turn's own reply
    assert_eq!(replay_known(&replay, Some(started), ours), 4);
    // A turn that ended on its call, with no reply to find, ends at the call
    assert_eq!(replay_known(&replay[..3], None, ours), 3);
    assert_eq!(replay_known(&replay, Some("something else"), |_| false), replay.len());
  }

  #[test]
  fn the_cli_reports_a_failure_as_one_warning_line() {
    let text = "\n\u{26a0} Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 46m9s.\n";
    let (at, e) = reply_error(text).unwrap();
    assert_eq!(at, 1);
    assert_eq!(e, ReplyError::Other("Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 46m9s.".into()));
    let cut = "Half the answer\n\u{26a0} Agent connection lost";
    let (at, _) = reply_error(cut).unwrap();
    assert_eq!(cut.get(..at), Some("Half the answer\n"));
    assert!(reply_error("Done.\n\n\u{26a0} Note: the tests\nstill need a run.").is_none());
  }

  #[test]
  fn the_note_a_cancel_leaves_is_found_on_its_own_last_line() {
    let note = "\n\u{26a0} The stream was interrupted. Please continue the task you were working on.\n";
    assert_eq!(cancel_note(note), Some(1));
    let cut = format!("Half the answer{note}");
    assert_eq!(cut.get(..cancel_note(&cut).unwrap()), Some("Half the answer\n"));
    assert!(cancel_note("Quoting it: \u{26a0} The stream was interrupted. Please continue").is_none());
    assert!(cancel_note(&format!("{note}And then more text")).is_none());
  }

  #[test]
  fn child_tool_call_ids_split_only_on_foreign_uuids() {
    let child = "40000000-0000-4000-8000-000000000004";
    let root = "10000000-0000-4000-8000-000000000001";
    assert_eq!(split_child_tool_call_id(&format!("{child}:call_123"), Some(root)), Some((child, "call_123")));
    assert_eq!(split_child_tool_call_id(&format!("{root}:call_123"), Some(root)), None);
    assert_eq!(split_child_tool_call_id("call_123", Some(root)), None);
    assert_eq!(split_child_tool_call_id(&format!("{child}:"), Some(root)), None);
    assert_eq!(split_child_tool_call_id("not-a-uuid:call_123", Some(root)), None);
    assert_eq!(split_child_tool_call_id("sk::call_123", Some(root)), None);
  }

  #[test]
  fn delegation_recognizes_swarm_add_and_invoke_subagent() {
    use serde_json::json;
    let add = json!({
      "sessionUpdate": "tool_call",
      "title": "Run swarm",
      "rawInput": { "action": "add", "name": "core", "brief": "Add settings validation", "model": "gemini-4-neon-xhigh" }
    });
    assert_eq!(
      delegation_of(&add),
      Some(AntigravityDelegation::SwarmAdd {
        names: vec!["core".into()],
        brief: Some("Add settings validation".into()),
        model: Some("gemini-4-neon-xhigh".into()),
      })
    );
    let sharded = json!({
      "sessionUpdate": "tool_call",
      "title": "swarm",
      "rawInput": { "action": "add", "name": "audit", "tasks": ["a", "b", "c"], "brief": "Audit shard" }
    });
    assert_eq!(
      delegation_of(&sharded),
      Some(AntigravityDelegation::SwarmAdd {
        names: vec!["audit-1".into(), "audit-2".into(), "audit-3".into()],
        brief: Some("Audit shard".into()),
        model: None,
      })
    );
    let title_only = json!({ "sessionUpdate": "tool_call", "title": "Run swarm", "rawInput": { "action": "title", "title": "Example project" } });
    assert_eq!(delegation_of(&title_only), None);
    let sub = json!({
      "sessionUpdate": "tool_call",
      "title": "Run invoke_subagent",
      "rawInput": { "TypeName": "research", "Role": "Codebase Researcher", "Prompt": "Find callers", "Model": "inherit" }
    });
    assert_eq!(
      delegation_of(&sub),
      Some(AntigravityDelegation::InvokeSubagent {
        role: Some("Codebase Researcher".into()),
        type_name: Some("research".into()),
        prompt: Some("Find callers".into()),
        model: None,
      })
    );
  }
}
