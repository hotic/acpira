//! A call of an MCP server's tool (`crate::mcp`). The arguments go to the server as the model wrote them (the server
//! validates them against its own schema); the result's text goes through the output budget, its images go on the tool
//! card and, for a model that takes images, into the tool result (`turn::finish` takes them off the card's content)

use serde_json::{Value, json};

use super::{Action, Ctx, Output, budgeted, text_content};
use crate::budget::Keep;

/// Images per result that reach the model; more are named only
const MAX_IMAGES: usize = 8;
/// An image larger than this (base64) is named, not sent: Anthropic refuses images over 5 MB
pub(crate) const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// `images`: the turn's model takes images
pub fn prepare(tool: &crate::mcp::Tool, args: &Value, images: bool) -> Action {
  Action::Mcp { server: tool.server.clone(), tool: tool.tool.clone(), name: tool.name.clone(), args: args.clone(), read_only: tool.read_only, images }
}

pub async fn run(server: &str, tool: &str, args: Value, images: bool, ctx: &Ctx) -> Output {
  // Progress (`ask_agent` streams a summoned agent's steps) shows on the card until the result replaces it
  let progress: crate::mcp::Progress = {
    let send = ctx.progress.clone();
    std::sync::Arc::new(move |line: String| send(json!({ "content": [text_content(&line)] })))
  };
  let result = match ctx.mcp.call(server, tool, args, Some(progress)).await {
    Ok(r) => r,
    Err(e) => return Output::error(format!("The MCP server {server} failed to run {tool}: {e}")),
  };
  output(&crate::mcp::read_result(&result), images, ctx)
}

/// The result as the model's copy and the card's content
fn output(r: &crate::mcp::CallResult, images: bool, ctx: &Ctx) -> Output {
  let mut content = vec![];
  if !r.text.trim().is_empty() {
    content.push(text_content(&r.text));
  }
  let mut model = if r.text.trim().is_empty() { String::new() } else { budgeted(&r.text, Keep::Head, ctx) };
  let mut omitted = 0usize;
  for (i, (mime, data)) in r.images.iter().enumerate() {
    // The card shows every image; the model gets the first few that fit
    content.push(json!({ "type": "content", "content": { "type": "image", "data": data, "mimeType": mime } }));
    if !images || i >= MAX_IMAGES || data.len() > MAX_IMAGE_BYTES {
      omitted += 1;
      content.last_mut().unwrap()["_meta"] = json!({ "acpira/notForModel": true });
    }
  }
  if omitted > 0 {
    let why = if images { "too many or too large to send" } else { "this model does not accept images" };
    model.push_str(&format!("{}[{omitted} image(s) not shown to you: {why}. The user sees them on the tool card.]", if model.is_empty() { "" } else { "\n" }));
  }
  if model.is_empty() {
    model = if r.images.is_empty() { "(no output)".to_owned() } else { "(images only)".to_owned() };
  }
  Output { model, is_error: r.is_error, content, raw_output: None }
}

/// The images of a finished call's card content that go to the model (`output` marks the others)
pub fn model_images(content: &[Value]) -> Vec<crate::llm::Image> {
  content
    .iter()
    .filter(|c| c.pointer("/content/type").and_then(Value::as_str) == Some("image") && c.pointer("/_meta/acpira~1notForModel").is_none())
    .filter_map(|c| Some(crate::llm::Image { mime: c.pointer("/content/mimeType")?.as_str()?.to_owned(), data: c.pointer("/content/data")?.as_str()?.to_owned() }))
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ctx(dir: &std::path::Path) -> Ctx {
    Ctx {
      cwd: dir.to_owned(),
      outputs: dir.join("outputs"),
      call_id: "m1".into(),
      progress: std::sync::Arc::new(|_| {}),
      jobs: Default::default(),
      search: Default::default(),
      mcp: Default::default(),
      history: Default::default(),
    }
  }

  #[test]
  fn images_reach_the_model_only_when_it_takes_them() {
    let dir = tempfile::tempdir().unwrap();
    let r = crate::mcp::CallResult { text: "shot taken".into(), images: vec![("image/png".into(), "AAA".into())], is_error: false };
    let with = output(&r, true, &ctx(dir.path()));
    assert_eq!(with.model, "shot taken");
    assert_eq!(model_images(&with.content), vec![crate::llm::Image { mime: "image/png".into(), data: "AAA".into() }]);
    let without = output(&r, false, &ctx(dir.path()));
    assert!(without.model.contains("does not accept images"), "{}", without.model);
    assert!(model_images(&without.content).is_empty());
    // The card shows it either way
    assert_eq!(without.content.len(), 2);
    let empty = output(&crate::mcp::CallResult { text: String::new(), images: vec![], is_error: true }, true, &ctx(dir.path()));
    assert!(empty.is_error && empty.model == "(no output)");
  }
}
