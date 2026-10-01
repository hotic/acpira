//! antigravity-acp ends a failed turn with `end_turn` and the failure as the turn's last `agent_message_chunk`
//! (`server.py` `prompt`, verified in the 1.1.1 and 1.2.1 sources): a failed MCP server load, an exhausted quota, a
//! browser-tool start failure, a lost harness connection, or any other SDK error as `Agent execution error: <e>`.
//! `reply_error` finds that tail so the session can show it as the turn's error instead of a reply

use std::sync::LazyLock;

use regex::Regex;

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
  let tail = text[at..].trim();
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
  Some((at, ReplyError::Other(tail.to_owned())))
}

const EXECUTION: &str = "Agent execution error: ";

/// The start of every failure text `prompt` sends, each at the start of a line
static TAIL: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(
    r"(?m)^(?:Agent execution error: |The MCP server '[A-Za-z0-9_-]+' failed to initialize|Usage Limit Reached\n\nYou have reached your current quota|Browser automation could not start \(|Agent connection was lost and could not be re-established: )",
  )
  .unwrap()
});
static MCP: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?s)^The MCP server '([A-Za-z0-9_-]+)' failed to initialize(?:: (.+?))?\. Fix the MCP server, or remove it from your configuration").unwrap()
});
static WRAPPED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)^Agent execution terminated due to error\. \("(.*)"\)\s*$"#).unwrap());

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
    assert_eq!(&text[..at], "Looking at the file now.\n\n");
    assert_eq!(e, ReplyError::Other("Usage Limit Reached\n\nYou have reached your current quota for this period.".into()));
    assert!(reply_error("The error said: Agent execution error: inline, mid-sentence").is_none());
    assert!(reply_error("All done.").is_none());
  }
}
