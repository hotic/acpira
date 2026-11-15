//! Antigravity ends a failed turn with `end_turn` and the failure as the turn's last `agent_message_chunk`. `agy acp`
//! (1.5.9, verified live with an exhausted quota) sends it as its own chunk, one line `\n⚠ <text>\n`, e.g.
//! `⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 46m9s.`. The other
//! patterns are the earlier `agy_acp_server` package's (`server.py` `prompt`, 1.1.1 / 1.2.1 sources), unverified against
//! `agy acp`: a failed MCP server load, an exhausted quota, a browser-tool start failure, a lost harness connection, or
//! any other SDK error as `Agent execution error: <e>`. `reply_error` finds that tail so the session can show it as the
//! turn's error instead of a reply

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

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

static RECEIPT: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r"(?s)\A(?:Created At: [^\n]*\n)?(?:Completed At: [^\n]*\n)?Command exited with code (-?\d+)\.\n\nCommand output:\n?(.*)\z")
});

/// How `agy acp` marks the line it reports a failed turn with
const WARNING: &str = "\u{26a0} ";

const EXECUTION: &str = "Agent execution error: ";

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
}
