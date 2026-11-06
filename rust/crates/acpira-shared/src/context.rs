//! A session's context window split by what fills it: the composer ring's card and the tool panel's context report tab. Mirror of
//! `packages/shared/context.ts`; the built-in agent builds it (`acpira-agent/src/context.rs`, `_acpira/context`), the
//! engine passes it through to the page

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// In the order a request carries them
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCategory {
  /// What an agent that reports no split keeps of its own (the engine's estimate, `rest`)
  Agent,
  System,
  Tools,
  Docs,
  Skills,
  Mcp,
  Chat,
}

/// What a node is (context.ts `ContextNodeKind` says what each carries)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextNodeKind {
  Base,
  Environment,
  Prompt,
  Doc,
  Tool,
  Skill,
  SkillTool,
  Server,
  McpTool,
  Summary,
  Turn,
  Message,
  Note,
  Thinking,
  Reply,
  Call,
  Rest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextNode {
  /// Stable while the conversation it describes stays as it is: the key a leaf's content is asked by
  pub id: String,
  pub kind: ContextNodeKind,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub index: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tool_kind: Option<String>,
  /// A group's tokens are its children's sum
  pub tokens: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub children: Option<Vec<ContextNode>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCategorySplit {
  pub id: ContextCategory,
  pub tokens: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached: Option<u64>,
  /// Only in a detailed report
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub nodes: Option<Vec<ContextNode>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextReport {
  /// The figure the ring shows (the last request and its reply) and the model's window
  pub used: u64,
  pub size: u64,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub estimated: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached: Option<u64>,
  /// When the last request went out, ms since the epoch
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub at: Option<u64>,
  /// The conversation this describes; a content request names it
  pub rev: String,
  pub categories: Vec<ContextCategorySplit>,
}

/// What a leaf sent
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContextContent {
  Text {
    text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
  },
  Def {
    description: String,
    schema: Value,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
  },
  Call {
    input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    images: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    error: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
  },
}

/// The last request as the agent sent it, in the provider's wire format (images and signatures elided)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRaw {
  pub format: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub url: Option<String>,
  pub body: Value,
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn reports_and_contents_keep_the_page_shape() {
    let r: ContextReport = serde_json::from_value(json!({
      "used": 10, "size": 100, "rev": "r", "categories": [
        { "id": "chat", "tokens": 10, "nodes": [{ "id": "t:0", "kind": "turn", "index": 1, "tokens": 10, "children": [
          { "id": "t:0:c:a", "kind": "call", "name": "read", "toolKind": "read", "tokens": 10, "cached": 4 },
        ] }] },
      ],
    }))
    .unwrap();
    let back = serde_json::to_value(&r).unwrap();
    assert_eq!(back["categories"][0]["nodes"][0]["children"][0]["toolKind"], "read");
    assert!(back.get("estimated").is_none());
    let c: ContextContent = serde_json::from_value(json!({ "kind": "call", "input": "{}", "output": "ok" })).unwrap();
    assert_eq!(serde_json::to_value(c).unwrap(), json!({ "kind": "call", "input": "{}", "output": "ok" }));
    let d = ContextContent::Def { description: "d".into(), schema: json!({}), truncated: false };
    assert_eq!(serde_json::to_value(d).unwrap(), json!({ "kind": "def", "description": "d", "schema": {} }));
  }
}
