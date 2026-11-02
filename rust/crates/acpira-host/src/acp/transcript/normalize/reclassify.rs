//! What a plugin's `transcript` hook may do to a settled thought or reply text (docs/dev/plugins.md, Hooks): split it
//! and say which parts are thought and which are reply. The words stay the agent's own: an answer that adds, drops or
//! reorders any is refused, so the record keeps what the agent sent and only the boundary between the kinds moves

use serde_json::{Value, json};

use acpira_shared::transcript::*;

use super::NormalizeState;
use super::content::resolve_markdown_images;

/// Most parts one block may become
pub const PARTS_MAX: usize = 8;

/// A thought or a reply text, as the hook sees it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prose {
  pub thought: bool,
  pub text: String,
}

impl Prose {
  pub fn of(b: &AgentBlock) -> Option<Prose> {
    match b {
      AgentBlock::Thought(t) => Some(Prose { thought: true, text: t.text.clone() }),
      AgentBlock::Text(t) => Some(Prose { thought: false, text: t.markdown.clone() }),
      _ => None,
    }
  }

  pub fn to_json(&self) -> Value {
    json!({ "type": if self.thought { "thought" } else { "text" }, "text": self.text })
  }
}

pub fn parts_json(parts: &[Prose]) -> Value {
  Value::Array(parts.iter().map(Prose::to_json).collect())
}

/// An answer's `blocks`: one to PARTS_MAX of `{ type: "thought" | "text", text }`, none of them blank
pub fn parse_parts(v: &Value) -> Option<Vec<Prose>> {
  let list = v.as_array().filter(|l| !l.is_empty() && l.len() <= PARTS_MAX)?;
  list
    .iter()
    .map(|p| {
      let thought = match p.get("type")?.as_str()? {
        "thought" => true,
        "text" => false,
        _ => return None,
      };
      let text = p.get("text")?.as_str().filter(|t| !t.trim().is_empty())?;
      Some(Prose { thought, text: text.to_owned() })
    })
    .collect()
}

/// Whether `parts` hold the same words in the same order as `original`; only whitespace may differ (a split drops the
/// blank lines between paragraphs)
pub fn same_words(original: &[Prose], parts: &[Prose]) -> bool {
  let squeeze = |l: &[Prose]| l.iter().flat_map(|p| p.text.chars()).filter(|c| !c.is_whitespace()).collect::<String>();
  squeeze(original) == squeeze(parts)
}

/// Put `parts` in place of the block of turn `turn` that still reads as `original`. False when that block is gone or
/// changed meanwhile. The first thought part keeps the original's timing, the first text part its id and phase; a
/// text part keeps the original's images it still shows and reads those it newly shows
pub fn reclassify(s: &mut NormalizeState, turn: usize, original: &Prose, parts: &[Prose]) -> bool {
  let NormalizeState { turns, ctx, .. } = s;
  let Some(Turn::Agent(t)) = turns.get_mut(turn) else { return false };
  let Some(at) = t.blocks.iter().position(|b| Prose::of(b).as_ref() == Some(original)) else { return false };
  let old = t.blocks[at].clone();
  let (mut timing, mut identity, images, streaming) = match &old {
    AgentBlock::Thought(th) => (Some((th.started_at, th.duration_sec)), None, None, th.streaming),
    AgentBlock::Text(tx) => (None, Some((tx.id.clone(), tx.phase)), tx.images.clone(), tx.streaming),
    _ => return false,
  };
  let blocks: Vec<AgentBlock> = parts
    .iter()
    .map(|p| {
      if p.thought {
        let (started_at, duration_sec) = timing.take().unwrap_or((None, None));
        return AgentBlock::Thought(ThoughtBlock { text: p.text.clone(), started_at, duration_sec, streaming });
      }
      let (id, phase) = identity.take().unwrap_or((None, None));
      let kept: Vec<ImageRef> =
        images.iter().flatten().filter(|i| i.uri.as_deref().is_some_and(|u| p.text.contains(u))).cloned().collect();
      let mut block = TextBlock { id, phase, markdown: p.text.clone(), streaming, images: (!kept.is_empty()).then_some(kept) };
      resolve_markdown_images(&mut block, 0, ctx);
      AgentBlock::Text(block)
    })
    .collect();
  t.blocks.splice(at..=at, blocks);
  true
}

#[cfg(test)]
mod tests {
  use super::*;

  fn prose(thought: bool, text: &str) -> Prose {
    Prose { thought, text: text.into() }
  }

  #[test]
  fn parts_keep_the_words() {
    let original = [prose(true, "Plan the ask.\n\n我整理了三种方案。")];
    assert!(same_words(&original, &[prose(true, "Plan the ask."), prose(false, "我整理了三种方案。")]));
    assert!(!same_words(&original, &[prose(true, "Plan the ask."), prose(false, "我整理了四种方案。")]));
    assert!(!same_words(&original, &[prose(false, "我整理了三种方案。"), prose(true, "Plan the ask.")]));
  }

  #[test]
  fn answers_are_checked() {
    assert_eq!(parse_parts(&json!([{ "type": "text", "text": "a" }])), Some(vec![prose(false, "a")]));
    assert_eq!(parse_parts(&json!([])), None);
    assert_eq!(parse_parts(&json!([{ "type": "tool_call", "text": "a" }])), None);
    assert_eq!(parse_parts(&json!([{ "type": "text", "text": "  " }])), None);
    assert_eq!(parse_parts(&json!(vec![json!({ "type": "text", "text": "a" }); PARTS_MAX + 1])), None);
  }

  #[test]
  fn a_split_thought_keeps_its_timing_and_place() {
    let mut s = NormalizeState::default();
    let thought = ThoughtBlock { text: "Plan.\n\n结论。".into(), started_at: Some(5), duration_sec: Some(acpira_shared::num::Num(3.0)), streaming: Some(false) };
    s.turns.push(Turn::Agent(AgentTurn { blocks: vec![AgentBlock::Thought(thought.clone()), AgentBlock::Text(TextBlock { id: None, phase: None, markdown: "after".into(), streaming: None, images: None })], ..Default::default() }));
    let original = prose(true, "Plan.\n\n结论。");
    assert!(reclassify(&mut s, 0, &original, &[prose(true, "Plan."), prose(false, "结论。")]));
    let Turn::Agent(t) = &s.turns[0] else { panic!() };
    assert!(matches!(&t.blocks[0], AgentBlock::Thought(th) if th.text == "Plan." && th.started_at == Some(5)));
    assert!(matches!(&t.blocks[1], AgentBlock::Text(tx) if tx.markdown == "结论。" && tx.streaming == Some(false)));
    assert!(matches!(&t.blocks[2], AgentBlock::Text(tx) if tx.markdown == "after"));
    // The block is no longer there as it was: a late second answer changes nothing
    assert!(!reclassify(&mut s, 0, &original, &[prose(false, "Plan.\n\n结论。")]));
  }
}
