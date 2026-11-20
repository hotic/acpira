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

use std::sync::LazyLock;

use acpira_shared::transcript::AgentBlock;
use regex::Regex;
use serde_json::Value;

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
    return (!line.contains('\n')).then(|| (at, ReplyError::Other(line.trim().to_owned())));
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
/// (its own transcript logs the checkpoint). A `/compact` turn that holds nothing but empty words is that compaction;
/// one with anything else (a failure line, a reply) is left as the CLI sent it
pub fn compacted_silently(blocks: &[AgentBlock]) -> bool {
  blocks.iter().all(|b| matches!(b, AgentBlock::Text(t) if t.markdown.trim().is_empty()))
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
  fn agy_acp_steers_by_prompt_from_1_5_0() {
    let at = |v: &str| steers_by_prompt(&serde_json::json!({ "agentInfo": { "name": "Antigravity ACP Server", "version": v } }));
    assert!(at("1.5.0") && at("1.5.12") && at("2.0.0"));
    assert!(!at("1.2.1") && !at("1.4.12"));
    assert!(!steers_by_prompt(&serde_json::json!({})));
  }

  #[test]
  fn only_a_compact_turn_with_nothing_but_empty_words_compacted_silently() {
    use acpira_shared::transcript::{CompactionBlock, CompactionStatus, TextBlock};
    let text = |m: &str| AgentBlock::Text(TextBlock { id: None, phase: None, markdown: m.into(), streaming: None, images: None });
    assert!(compacted_silently(&[]));
    assert!(compacted_silently(&[text(""), text("\n")]));
    // A failure line or a reply is the CLI's own word on it
    assert!(!compacted_silently(&[text("\n⚠ Individual quota reached.\n")]));
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
}
