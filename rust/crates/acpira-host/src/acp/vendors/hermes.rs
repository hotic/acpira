//! Hermes Agent's ACP output (hermes-agent 0.22.5 `acp_adapter/tools.py`, `gateway/run_turn_persistence.py`).
//!
//! - With `display.show_reasoning` on (its default), an editor session's reply ends with one more message chunk: the
//!   last reasoning as a `💭 **<label>**` fenced block followed by the whole reply again. The reasoning already streamed
//!   as thought chunks, so that chunk only repeats the turn.
//! - Tool titles are `<tool>: <preview>`; for the tools below the preview is the row's target. A terminal start carries
//!   `$ <command>` as content, and its result is the generic formatter's list (`terminal result`, `- **output:** …`,
//!   `- **exit_code:** N`, the output cut at 500 characters), which a terminal card would show with its asterisks.

use serde_json::Value;

/// Tools whose title preview is what the row names: the command, the path, the pattern
const TARGET_TOOLS: [&str; 5] = ["terminal", "read_file", "write_file", "patch", "search_files"];
const OUTPUT: &str = "- **output:** ";
const EXIT: &str = "- **exit_code:** ";

/// Whether a message chunk is the reasoning projection: the `💭` header, a fence, and then exactly the reply the turn
/// already streamed
pub fn reasoning_recap(text: &str, reply: &str) -> bool {
  let (text, reply) = (text.trim(), reply.trim());
  !reply.is_empty() && text.starts_with("💭 **") && text.contains("```") && text.len() > reply.len() && text.ends_with(reply)
}

/// A tool_call / tool_call_update made to read like the other agents' rows
pub fn tidy_tool(u: &mut Value) {
  let split = u.get("title").and_then(Value::as_str).and_then(|t| t.split_once(": ")).map(|(tool, rest)| (tool.to_owned(), rest.to_owned()));
  let command = match split {
    Some((tool, rest)) if TARGET_TOOLS.contains(&tool.as_str()) && !rest.trim().is_empty() => {
      u["title"] = Value::from(rest.as_str());
      (tool == "terminal").then_some(rest)
    }
    _ => None,
  };
  let Some(items) = u.get_mut("content").and_then(Value::as_array_mut) else { return };
  if let Some(cmd) = command {
    let echo = format!("$ {cmd}");
    items.retain(|i| i.pointer("/content/text").and_then(Value::as_str) != Some(echo.as_str()));
  }
  for item in items.iter_mut() {
    if let Some(out) = item.pointer("/content/text").and_then(Value::as_str).and_then(terminal_output) {
      item["content"]["text"] = Value::from(out);
    }
  }
}

/// The plain output of a formatted terminal result, with a failing exit code noted under it; None for anything else,
/// which stays as Hermes wrote it
fn terminal_output(text: &str) -> Option<String> {
  let body = text.strip_prefix("terminal result\n").or_else(|| text.strip_prefix("✅ terminal completed\n"))?;
  let (out, code) = match body.strip_prefix(EXIT) {
    Some(code) => ("", code),
    None => {
      let (out, code) = body.rsplit_once(&format!("\n{EXIT}"))?;
      (out.strip_prefix(OUTPUT)?, code)
    }
  };
  let code: i64 = code.trim().parse().ok()?;
  Some(if code == 0 { out.to_owned() } else { format!("{out}\n\nexit code {code}") })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn the_reasoning_projection_is_told_from_a_reply_that_only_looks_like_one() {
    let reply = "\n\nShell output: `ok`\n\nDominant color: black";
    let recap = "\n\n💭 **推理：**\n```\nWe have terminal output.\n```\n\nShell output: `ok`\n\nDominant color: black";
    assert!(reasoning_recap(recap, reply));
    // Nothing streamed yet, or a reply of its own that merely starts with the glyph
    assert!(!reasoning_recap(recap, ""));
    assert!(!reasoning_recap("💭 **Note**\n```\nx\n```\n\nsomething else", reply));
  }

  #[test]
  fn terminal_rows_get_the_command_as_target_and_the_plain_output() {
    let mut start = json!({ "sessionUpdate": "tool_call", "title": "terminal: echo hi", "kind": "execute",
      "content": [{ "type": "content", "content": { "type": "text", "text": "$ echo hi" } }] });
    tidy_tool(&mut start);
    assert_eq!((start["title"].as_str(), start["content"].as_array().map(Vec::len)), (Some("echo hi"), Some(0)));
    let mut done = json!({ "sessionUpdate": "tool_call_update", "content": [{ "type": "content", "content": { "type": "text",
      "text": "terminal result\n- **output:** line 1\nline 2\n- **exit_code:** 0" } }] });
    tidy_tool(&mut done);
    assert_eq!(done["content"][0]["content"]["text"], "line 1\nline 2");
    assert_eq!(terminal_output("terminal result\n- **output:** boom\n- **exit_code:** 2").as_deref(), Some("boom\n\nexit code 2"));
    assert_eq!(terminal_output("✅ terminal completed\n- **exit_code:** 0").as_deref(), Some(""));
    // Another key after the exit code: not this shape, left alone
    assert_eq!(terminal_output("terminal result\n- **output:** x\n- **exit_code:** 1\n- **error:** y"), None);
  }

  #[test]
  fn other_tools_keep_their_name() {
    let mut u = json!({ "title": "vision_analyze: What color?", "kind": "read" });
    tidy_tool(&mut u);
    assert_eq!(u["title"], "vision_analyze: What color?");
    let mut r = json!({ "title": "read_file: src/main.rs", "kind": "read" });
    tidy_tool(&mut r);
    assert_eq!(r["title"], "src/main.rs");
  }
}
