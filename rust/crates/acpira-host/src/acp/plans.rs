//! Plan documents (mirror of src/host/acp/plans.ts): full plan content is kept before tool normalization reduces a diff to lines

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use acpira_shared::transcript::{AgentBlock, PlanDocStatus, PlanDocumentBlock, Turn};

use crate::json::str_of;

static FRONT_MATTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^---\r?\n(?s:.*?)\r?\n---\r?\n").unwrap());
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#\s+(.+)$").unwrap());
static SAVED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Plan saved to: (.+)\r?\n\r?\n((?s:.*))").unwrap());
static EXIT_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(exit_plan_mode|ExitPlanMode)$").unwrap());
static KNOWN_PATH: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"/(?:\.grok/sessions/.*/plan\.md|\.kimi-code/sessions/.*/plans/[^/]+\.md)$").unwrap());

pub fn plan_documents(turns: &[Turn]) -> Vec<&PlanDocumentBlock> {
  turns
    .iter()
    .filter_map(Turn::as_agent)
    .flat_map(|t| {
      t.blocks.iter().filter_map(|b| match b {
        AgentBlock::PlanDocument(p) => Some(p),
        _ => None,
      })
    })
    .collect()
}

pub fn plan_documents_mut(turns: &mut [Turn]) -> Vec<&mut PlanDocumentBlock> {
  turns
    .iter_mut()
    .filter_map(Turn::as_agent_mut)
    .flat_map(|t| {
      t.blocks.iter_mut().filter_map(|b| match b {
        AgentBlock::PlanDocument(p) => Some(p),
        _ => None,
      })
    })
    .collect()
}

pub fn set_plan_content(plan: &mut PlanDocumentBlock, markdown: &str) {
  // YAML metadata belongs to the file; the preview starts at the document body
  plan.markdown = FRONT_MATTER.replace(markdown, "").trim().to_owned();
  plan.title = HEADING.captures(&plan.markdown).map(|c| c[1].to_owned()).unwrap_or_else(|| "Plan".into());
}

pub fn is_plan_approval(u: &Value) -> bool {
  let meta = u.get("_meta");
  let m = |k: &str| meta.and_then(|m| m.get(k));
  m("acpira/planApproval") == Some(&Value::Bool(true))
    || m("cognition.ai/isExitPlan") == Some(&Value::Bool(true))
    || m("cognition.ai/inferenceToolName").and_then(Value::as_str) == Some("exit_plan_mode")
    || EXIT_TITLE.is_match(str_of(u, "title").unwrap_or(""))
    || (str_of(u, "kind") == Some("switch_mode") && u.get("rawInput").and_then(|r| r.get("plan")).is_some_and(Value::is_string))
}

fn find_plan(turns: &mut [Turn], f: impl Fn(&PlanDocumentBlock) -> bool) -> Option<&mut PlanDocumentBlock> {
  plan_documents_mut(turns).into_iter().find(|p| f(p))
}

/// Capture / update the plan document a tool update describes; returns the plan's id when one applies
pub fn capture_plan(turns: &mut [Turn], u: &Value) -> Option<String> {
  let raw = u.get("rawInput").filter(|v| v.is_object());
  let r = |k: &str| raw.and_then(|x| x.get(k)).and_then(Value::as_str).map(str::to_owned);
  let meta = u.get("_meta");
  let meta_s = |k: &str| meta.and_then(|m| m.get(k)).and_then(Value::as_str).map(str::to_owned);
  let content = u.get("content").and_then(Value::as_array);
  let diff = content.and_then(|c| c.iter().find(|x| str_of(x, "type") == Some("diff")));
  let text: String = content
    .map(|c| {
      c.iter()
        .filter(|x| str_of(x, "type") == Some("content") && x.get("content").and_then(|y| str_of(y, "type")) == Some("text"))
        .filter_map(|x| x["content"].get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
    })
    .unwrap_or_default();
  let saved = SAVED.captures(&text).map(|c| (c[1].to_owned(), c[2].to_owned()));
  let ready = u.get("rawOutput").and_then(|o| o.get("PlanReady")).filter(|v| v.is_object());
  let ready_s = |k: &str| ready.and_then(|x| x.get(k)).and_then(Value::as_str).map(str::to_owned);
  let path = meta_s("cognition.ai/planFilePath")
    .or_else(|| ready_s("plan_file_path"))
    .or_else(|| saved.as_ref().map(|s| s.0.clone()))
    .or_else(|| r("file_path"))
    .or_else(|| r("path"))
    .or_else(|| diff.and_then(|d| str_of(d, "path")).map(str::to_owned))
    .or_else(|| u.get("locations").and_then(Value::as_array).and_then(|l| l.first()).and_then(|l| str_of(l, "path")).map(str::to_owned));
  let tool_call_id = str_of(u, "toolCallId").unwrap_or("").to_owned();
  let exit = is_plan_approval(u) || plan_documents(turns).iter().any(|p| p.approval_tool_call_id.as_deref() == Some(tool_call_id.as_str()));
  let write = meta.and_then(|m| m.get("cognition.ai/isPlanFileEdit")) == Some(&Value::Bool(true))
    || meta_s("cognition.ai/inferenceToolName").as_deref() == Some("write_plan");
  let known_path = path.as_deref().is_some_and(|p| KNOWN_PATH.is_match(p));
  let matches = |p: &PlanDocumentBlock| {
    p.tool_call_id == tool_call_id
      || p.approval_tool_call_id.as_deref() == Some(tool_call_id.as_str())
      || (path.is_some() && p.path == path)
  };
  let mut id = plan_documents(turns).into_iter().find(|p| matches(p)).map(|p| p.id.clone());
  if id.is_none() && !exit && !known_path && !write {
    return None;
  }
  // Devin can announce exit_plan_mode before write_plan's packets: attach the first file to the single unbound approval
  let mut fills_approval = false;
  if id.is_none()
    && !exit
    && write
    && let Some(Turn::Agent(last)) = turns.last_mut()
  {
    let unbound: Vec<usize> = last
      .blocks
      .iter()
      .enumerate()
      .filter(|(_, b)| matches!(b, AgentBlock::PlanDocument(p) if p.path.is_none() && p.approval_tool_call_id.is_some() && p.approval_tool_call_id.as_deref() == Some(p.tool_call_id.as_str())))
      .map(|(i, _)| i)
      .collect();
    if unbound.len() == 1
      && let AgentBlock::PlanDocument(p) = &mut last.blocks[unbound[0]]
    {
      p.tool_call_id = tool_call_id.clone();
      id = Some(p.id.clone());
      fills_approval = true;
    }
  }
  if id.is_none() && exit && path.is_none() {
    id = plan_documents(turns).last().map(|p| p.id.clone());
  }
  let has_markdown =
    id.as_ref().and_then(|i| plan_documents(turns).into_iter().find(|p| &p.id == i)).is_some_and(|p| !p.markdown.is_empty());
  let markdown = r("planContent")
    .or_else(|| ready_s("plan_content"))
    .or_else(|| saved.as_ref().map(|s| s.1.clone()))
    .or_else(|| diff.and_then(|d| str_of(d, "newText")).map(str::to_owned))
    .or_else(|| r("content"))
    .or_else(|| if exit && !has_markdown { r("plan") } else { None });
  let id = match id {
    Some(i) => i,
    None => {
      let Some(Turn::Agent(last)) = turns.last_mut() else { return None };
      let pid = format!("plan-{tool_call_id}");
      last.blocks.push(AgentBlock::PlanDocument(PlanDocumentBlock {
        id: pid.clone(),
        title: "Plan".into(),
        markdown: String::new(),
        path: path.clone(),
        tool_call_id: tool_call_id.clone(),
        approval_tool_call_id: None,
        status: PlanDocStatus::Draft,
      }));
      pid
    }
  };
  let p = find_plan(turns, |p| p.id == id)?;
  if path.is_some() {
    p.path = path;
  }
  if let Some(md) = markdown {
    let before = p.markdown.clone();
    set_plan_content(p, &md);
    if !exit && !fills_approval && p.markdown != before {
      p.status = PlanDocStatus::Draft;
      p.tool_call_id = tool_call_id.clone();
    }
  }
  if exit {
    p.approval_tool_call_id = Some(tool_call_id);
    if !p.markdown.is_empty() && p.status == PlanDocStatus::Draft {
      p.status = PlanDocStatus::Ready;
    }
  } else if str_of(u, "status") == Some("completed") && p.status == PlanDocStatus::Draft {
    p.status = PlanDocStatus::Ready;
  }
  Some(id)
}
