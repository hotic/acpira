//! Grok's private requests (mirror of src/host/acp/grokPlan.ts and grokQuestions.ts), verified on the wire with Grok 1.0.18

use std::sync::Arc;

use serde_json::{Value, json};

use super::agent_process::ClientHandlers;
use super::cancel::Cancel;
use super::rpc::RpcError;
use crate::i18n::t;

pub const GROK_EXIT_PLAN: &str = "_x.ai/exit_plan_mode";
pub const GROK_ASK_QUESTION: &str = "_x.ai/ask_user_question";

fn invalid(what: &str) -> RpcError {
  RpcError::new(-32602, format!("Invalid params: {what}"))
}

/// { sessionId, toolCallId, planContent? } normalized
pub fn parse_exit_plan(v: &Value) -> Result<Value, RpcError> {
  let session_id = v.get("sessionId").and_then(Value::as_str);
  let tool_call_id = v.get("toolCallId").and_then(Value::as_str);
  let plan = v.get("planContent");
  let (Some(session_id), Some(tool_call_id)) = (session_id, tool_call_id) else { return Err(invalid("Invalid Grok plan approval")) };
  if plan.is_some_and(|p| !p.is_null() && !p.is_string()) {
    return Err(invalid("Invalid Grok plan approval"));
  }
  let mut out = json!({ "sessionId": session_id, "toolCallId": tool_call_id });
  if let Some(p) = plan.and_then(Value::as_str) {
    out["planContent"] = Value::from(p);
  }
  Ok(out)
}

/// Grok's plan approval goes through the permission gate as a two-option card; its answer is `{ outcome }`
pub async fn approve_plan(req: Value, cancel: Cancel, h: Arc<dyn ClientHandlers>) -> Result<Value, RpcError> {
  let mut raw_input = json!({});
  if let Some(p) = req.get("planContent") {
    raw_input["planContent"] = p.clone();
  }
  let permission = json!({
    "sessionId": req["sessionId"],
    "toolCall": { "toolCallId": req["toolCallId"], "title": "exit_plan_mode", "rawInput": raw_input, "_meta": { "acpira/planApproval": true } },
    "options": [
      { "optionId": "approved", "name": "Build", "kind": "allow_once" },
      { "optionId": "rejected", "name": t("plan.revise"), "kind": "reject_once" },
    ],
  });
  let r = h.on_permission(permission, cancel).await?;
  let outcome = &r["outcome"];
  let picked =
    if outcome["outcome"] == "selected" { outcome["optionId"].as_str().unwrap_or("rejected").to_owned() } else { "rejected".into() };
  Ok(json!({ "outcome": picked }))
}

/// { sessionId, toolCallId, questions: [{ question, options: [{ label, description? }], multiSelect? }], mode? } normalized
pub fn parse_question(v: &Value) -> Result<Value, RpcError> {
  let session_id = v.get("sessionId").and_then(Value::as_str);
  let tool_call_id = v.get("toolCallId").and_then(Value::as_str);
  let questions = v.get("questions").and_then(Value::as_array);
  let (Some(session_id), Some(tool_call_id), Some(questions)) = (session_id, tool_call_id, questions) else {
    return Err(invalid("Invalid Grok question request"));
  };
  let mut out_q = vec![];
  for q in questions {
    let Some(question) = q.get("question").and_then(Value::as_str) else { return Err(invalid("Invalid Grok question")) };
    let options: Vec<Value> = q
      .get("options")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(|o| {
        let label = o.get("label")?.as_str()?;
        let mut item = json!({ "label": label });
        if let Some(d) = o.get("description").and_then(Value::as_str).filter(|d| !d.is_empty()) {
          item["description"] = Value::from(d);
        }
        Some(item)
      })
      .collect();
    let mut item = json!({ "question": question, "options": options });
    if q.get("multiSelect").is_some_and(truthy) {
      item["multiSelect"] = Value::Bool(true);
    }
    out_q.push(item);
  }
  let mut out = json!({ "sessionId": session_id, "toolCallId": tool_call_id, "questions": out_q });
  if let Some(m) = v.get("mode").and_then(Value::as_str) {
    out["mode"] = Value::from(m);
  }
  Ok(out)
}

/// JS truthiness of a JSON value
pub fn truthy(v: &Value) -> bool {
  match v {
    Value::Null => false,
    Value::Bool(b) => *b,
    Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
    Value::String(s) => !s.is_empty(),
    _ => true,
  }
}
