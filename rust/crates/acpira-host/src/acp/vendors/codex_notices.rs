//! Codex startup / config notices and MCP startup reports (codex-acp 2.8.0–2.12.1 / codex 0.170.0+).
//!
//! Unlike `session/new` (which attaches its event tracker after `thread/start`), `session/resume` attaches
//! `startCodexTurnTracker` before `thread/resume` and flushes buffered notifications asynchronously when the
//! session installs. `thread/resume` emits global config warnings (`configWarning`, `deprecationNotice`) and a
//! model-mismatch warning (`This session was recorded with model … but is resuming with …`, because `thread/resume`
//! omits `model` and starts on `config.toml`'s default before Acpira's `adopt_controls` restores the session's model).
//! Spawning native Codex subagents mid-turn also re-loads `config.toml` and re-emits the same global `configWarning`
//! and `deprecationNotice` events (which carry no `threadId`, so `codex-acp` routes them onto the root session).
//! Left unfiltered, those notices land on settled turns or split live streaming text mid-sentence.
//!
//! After `session/new` / `resume` / `load`, `McpSessionStartup` also publishes skipped or failed MCP server startups
//! as `status: "failed"` `tool_call` updates (`toolCallId: mcp_startup.<server>.<uuid>`, `title: mcp__<server>__startup`).
//! Left in the normalizer, those pseudo tool calls fabricate an orphan `AgentTurn` (rendered as "Done" on an empty
//! session) or append onto the last settled turn of a resumed conversation.

use serde_json::Value;

/// A `codex-acp` MCP startup status report sent as a failed `tool_call` (`McpStartupReporter`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpStartupReport {
  /// Server skipped because Codex's own config already defines the same name (`McpStartupReporter.skipped`)
  Skipped { server: String, detail: String },
  /// Server failed to start or startup was cancelled (`McpStartupReporter.failures`)
  Failed { server: String, detail: String },
}

impl McpStartupReport {
  /// Concise single-line toast text for a failed startup (strips Codex's nested `MCP client for ... failed to start` wrappers)
  pub(crate) fn toast_text(&self) -> String {
    let (McpStartupReport::Failed { server, detail } | McpStartupReport::Skipped { server, detail }) = self;
    let first = detail.lines().next().unwrap_or(detail).trim();
    let mut cause = first;
    for marker in [
      "MCP startup failed: handshaking with MCP server failed: ",
      "MCP startup failed: ",
      "failed to start: ",
    ] {
      if let Some((_, tail)) = cause.rsplit_once(marker)
        && !tail.trim().is_empty()
      {
        cause = tail.trim();
        break;
      }
    }
    let clean = format!("MCP server `{server}` failed to start: {cause}");
    crate::util::clip(&clean, 240)
  }
}

/// Recognizes a `codex-acp` `mcp_startup.*` pseudo `tool_call` update so it is logged / toasted instead of
/// creating a conversation turn.
pub(crate) fn mcp_startup_report(u: &Value) -> Option<McpStartupReport> {
  let id = u.get("toolCallId").and_then(Value::as_str)?;
  let rest = id.strip_prefix("mcp_startup.")?;
  let server = u
    .get("title")
    .and_then(Value::as_str)
    .and_then(|t| t.strip_prefix("mcp__"))
    .and_then(|t| t.strip_suffix("__startup"))
    .filter(|s| !s.is_empty())
    .map(str::to_owned)
    .or_else(|| rest.rsplit_once('.').map(|(name, _)| name.to_owned()).filter(|s| !s.is_empty()))
    .unwrap_or_else(|| "MCP".to_owned());
  let raw_text = startup_content_text(u);
  let detail = raw_text
    .strip_prefix("[codex-acp forwarded startup error]")
    .or_else(|| raw_text.strip_prefix("[codex-acp]"))
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| raw_text.trim())
    .to_owned();
  let detail = if detail.is_empty() { format!("MCP server `{server}` failed to start") } else { detail };
  if detail.contains("was not started, because the Codex config already defines") {
    Some(McpStartupReport::Skipped { server, detail })
  } else {
    Some(McpStartupReport::Failed { server, detail })
  }
}

fn startup_content_text(u: &Value) -> String {
  let mut parts = Vec::new();
  if let Some(items) = u.get("content").and_then(Value::as_array) {
    for item in items {
      let text = item
        .pointer("/content/text")
        .and_then(Value::as_str)
        .or_else(|| item.get("text").and_then(Value::as_str));
      if let Some(t) = text.map(str::trim).filter(|t| !t.is_empty()) {
        parts.push(t);
      }
    }
  }
  parts.join("\n")
}

/// Whether a Codex `notice` update should be dropped instead of appended to the transcript.
///
/// Error notices are always kept. Before any turn has started on the current process, every non-error notice is a
/// startup / resume notification rather than conversation content. Global config / deprecation and resume-only
/// warnings are also recognized by text because `codex-acp` flushes them both around `thread/resume` and mid-turn
/// whenever Codex loads `config.toml` for a newly spawned subagent.
pub(crate) fn drop_on_update(u: &Value, turn_seen: bool) -> bool {
  let severity = u.get("severity").and_then(Value::as_str);
  if severity == Some("error") {
    return false;
  }
  if !turn_seen {
    return true;
  }
  severity == Some("warning") && u.get("title").and_then(Value::as_str).is_some_and(is_config_or_resume_warning)
}

/// Global config / deprecation / resume warnings Codex emits when loading config or reopening a thread
fn is_config_or_resume_warning(title: &str) -> bool {
  let t = title.trim();
  (t.starts_with("This session was recorded with model ") && t.contains(" but is resuming with "))
    || (t.starts_with("Codex is ignoring ") && t.contains("unrecognized configuration"))
    || t.contains("` is deprecated")
    || matches!(t, "Configuration warning" | "Deprecated configuration")
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;

  fn notice(severity: &str, title: &str) -> Value {
    json!({ "sessionUpdate": "notice", "severity": severity, "title": title })
  }

  #[test]
  fn pre_turn_warnings_and_info_are_dropped_while_errors_stay() {
    assert!(drop_on_update(&notice("warning", "Configuration warning"), false));
    assert!(drop_on_update(&notice("info", "Something at startup"), false));
    assert!(!drop_on_update(&notice("error", "Fatal session error"), false));
  }

  #[test]
  fn resume_config_and_model_mismatch_warnings_are_dropped_even_after_a_queued_turn_opens() {
    let deprecated = notice(
      "warning",
      "`[features].multi_agent` is deprecated. It still controls the V1 fallback. `[agents].enabled = false` disables subagent tools.",
    );
    let ignored = notice(
      "warning",
      "Codex is ignoring 2 unrecognized configuration settings. Check for typos or deprecated settings.\n  user (/Users/x/.codex/config.toml): `features.remote_connections` is ignored.",
    );
    let model = notice(
      "warning",
      "This session was recorded with model `gpt-6.2-astra` but is resuming with `gpt-6.1-sol`. Consider switching back to `gpt-6.2-astra` as it may affect Codex performance.",
    );
    assert!(drop_on_update(&deprecated, true));
    assert!(drop_on_update(&ignored, true));
    assert!(drop_on_update(&model, true));
    assert!(drop_on_update(&notice("warning", "Configuration warning"), true));
    assert!(drop_on_update(&notice("warning", "Deprecated configuration"), true));
  }

  #[test]
  fn live_turn_notices_stay_once_a_turn_has_started() {
    assert!(!drop_on_update(&notice("warning", "Heads up: Long threads and multiple compactions can reduce accuracy."), true));
    assert!(!drop_on_update(&notice("info", "Model rerouted"), true));
    assert!(!drop_on_update(&notice("warning", "Answer not sent"), true));
    assert!(!drop_on_update(&notice("warning", "PreToolUse hook blocked the command"), true));
  }

  #[test]
  fn codex_mcp_startup_failures_and_skips_are_classified_and_normal_mcp_calls_stay() {
    let failed = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": "mcp_startup.blockbench.95173602-3176-4849-b3fb-d0add41fb1cc",
      "kind": "other",
      "title": "mcp__blockbench__startup",
      "status": "failed",
      "content": [{
        "type": "content",
        "content": {
          "type": "text",
          "text": "[codex-acp forwarded startup error] MCP server `blockbench` failed to start: boom"
        }
      }]
    });
    assert_eq!(
      mcp_startup_report(&failed),
      Some(McpStartupReport::Failed {
        server: "blockbench".into(),
        detail: "MCP server `blockbench` failed to start: boom".into(),
      })
    );
    let nested = McpStartupReport::Failed {
      server: "broken-mock".into(),
      detail: "MCP server `broken-mock` failed to start: MCP client for `broken-mock` failed to start: MCP startup failed: handshaking with MCP server failed: binary missing".into(),
    };
    assert_eq!(nested.toast_text(), "MCP server `broken-mock` failed to start: binary missing");

    let skipped = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": "mcp_startup.acpira.11111111-2222-3333-4444-555555555555",
      "kind": "other",
      "title": "mcp__acpira__startup",
      "status": "failed",
      "content": [{
        "type": "content",
        "content": {
          "type": "text",
          "text": "[codex-acp] MCP server `acpira` was not started, because the Codex config already defines an MCP server with this name."
        }
      }]
    });
    assert_eq!(
      mcp_startup_report(&skipped),
      Some(McpStartupReport::Skipped {
        server: "acpira".into(),
        detail: "MCP server `acpira` was not started, because the Codex config already defines an MCP server with this name.".into(),
      })
    );

    let normal = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": "call_abc",
      "kind": "other",
      "title": "mcp.blockbench.list_models",
      "status": "failed"
    });
    assert_eq!(mcp_startup_report(&normal), None);
  }
}
