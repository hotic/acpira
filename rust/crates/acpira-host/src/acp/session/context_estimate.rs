//! The context split of an agent that reports none (every CLI but the built-in agent): the transcript since the last
//! compaction, turn by turn, at a size estimated from its text and scaled into the figure the agent reported, so the
//! parts never add up past it. What is left of the figure is the agent's own (system prompt, tool definitions, its
//! instruction files, a compaction summary), which only the agent could split, so it stays one `rest` part. Marked
//! estimated: the transcript is what the user saw, not what the agent sent (it may have cut tool output, or dropped
//! earlier reasoning). An agent that reports its own category sums (Grok's session info, `grok_known`; Claude's own
//! log, `context_claude.rs`) gets those instead of one `agent` part worked out from the figure, with the turns scaled
//! into the conversation it reports.

use acpira_shared::context::{ContextCategory, ContextCategorySplit, ContextContent, ContextNode, ContextNodeKind, ContextReport};
use acpira_shared::transcript::{AgentBlock, CompactionStatus, ToolCallBlock, ToolContent, ToolStatus, Turn, Usage};
use serde_json::Value;

/// A report's `rev` prefix: a content request with it is answered from the transcript, not by the agent
pub const REV_PREFIX: &str = "tx:";
/// A leaf's content is cut here, as the agent's own report does
const CONTENT_MAX: usize = 64 * 1024;
/// A turn's title is its prompt's first line, cut here
const TITLE_MAX: usize = 80;
const REST_ID: &str = "rest";
const SUMMARY_ID: &str = "summary";

/// A leaf of a turn and the transcript it stands for
enum Leaf<'a> {
  Message(&'a str),
  Thinking(Vec<&'a str>),
  Reply(Vec<&'a str>),
  Call(&'a ToolCallBlock),
}

struct Group<'a> {
  id: String,
  index: Option<u64>,
  title: Option<String>,
  /// In the order the turn produced them; one thinking and one reply leaf gather the turn's pieces
  leaves: Vec<(String, Leaf<'a>)>,
}

/// Rough tokens of a text: one per four ASCII bytes, one per other character (CJK runs about that)
pub fn approx_tokens(s: &str) -> u64 {
  let other = s.chars().filter(|c| !c.is_ascii()).count() as u64;
  let ascii = s.len() as u64 - s.chars().filter(|c| !c.is_ascii()).map(|c| c.len_utf8() as u64).sum::<u64>();
  ascii.div_ceil(4) + other
}

fn tool_parts(x: &ToolCallBlock) -> Vec<&ToolContent> {
  match &x.contents {
    Some(all) if !all.is_empty() => all.iter().collect(),
    _ => x.content.iter().collect(),
  }
}

fn content_tokens(c: &ToolContent) -> u64 {
  match c {
    ToolContent::Text { text } => approx_tokens(text),
    ToolContent::Diff { lines, .. } => lines.iter().map(|l| approx_tokens(&l.text) + 1).sum(),
    ToolContent::List { items } => items.iter().map(|i| approx_tokens(i) + 1).sum(),
    ToolContent::Image(_) => 0,
  }
}

fn content_text(c: &ToolContent) -> Option<String> {
  match c {
    ToolContent::Text { text } => Some(text.clone()),
    ToolContent::Diff { lines, .. } => Some(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")),
    ToolContent::List { items } => Some(items.join("\n")),
    ToolContent::Image(_) => None,
  }
}

impl Leaf<'_> {
  fn tokens(&self) -> u64 {
    match self {
      Leaf::Message(t) => approx_tokens(t),
      Leaf::Thinking(ts) | Leaf::Reply(ts) => ts.iter().map(|t| approx_tokens(t)).sum(),
      // The call's own words: its target (a path, a command), then what came back
      Leaf::Call(x) => approx_tokens(x.target.as_deref().unwrap_or(&x.verb)) + tool_parts(x).into_iter().map(content_tokens).sum::<u64>(),
    }
  }

  fn node(&self, id: String, tokens: u64) -> ContextNode {
    let (kind, name, title, tool_kind) = match self {
      Leaf::Message(_) => (ContextNodeKind::Message, None, None, None),
      Leaf::Thinking(_) => (ContextNodeKind::Thinking, None, None, None),
      Leaf::Reply(_) => (ContextNodeKind::Reply, None, None, None),
      Leaf::Call(x) => {
        let kind = serde_json::to_value(x.kind).ok().and_then(|v| v.as_str().map(str::to_owned));
        (ContextNodeKind::Call, Some(x.mcp_server.clone().unwrap_or_else(|| x.verb.clone())), x.target.clone(), kind)
      }
    };
    ContextNode { id, kind, name, title, index: None, tool_kind, tokens, cached: None, children: None }
  }

  fn content(&self) -> ContextContent {
    let text = |parts: &[&str]| cut(parts.join("\n\n"));
    match self {
      Leaf::Message(t) => text(&[t]),
      Leaf::Thinking(ts) | Leaf::Reply(ts) => text(ts),
      Leaf::Call(x) => {
        let out: Vec<String> = tool_parts(x).into_iter().filter_map(content_text).collect();
        let images = tool_parts(x).into_iter().filter(|c| matches!(c, ToolContent::Image(_))).count() as u64;
        let (output, truncated) = if out.is_empty() { (None, false) } else { clip(out.join("\n")) };
        ContextContent::Call {
          input: x.target.clone().unwrap_or_default(),
          output,
          images: (images > 0).then_some(images),
          error: x.status == ToolStatus::Failed,
          truncated,
        }
      }
    }
  }
}

fn clip(mut s: String) -> (Option<String>, bool) {
  if s.len() <= CONTENT_MAX {
    return (Some(s), false);
  }
  let mut at = CONTENT_MAX;
  while !s.is_char_boundary(at) {
    at -= 1;
  }
  s.truncate(at);
  (Some(s), true)
}

fn cut(s: String) -> ContextContent {
  let (text, truncated) = clip(s);
  ContextContent::Text { text: text.unwrap_or_default(), truncated }
}

/// Where the last finished compaction sits, (turn, block): everything up to it went into the agent's summary
fn last_compaction(turns: &[Turn]) -> Option<(usize, usize)> {
  turns.iter().enumerate().rev().find_map(|(t, turn)| {
    let a = turn.as_agent()?;
    a.blocks.iter().rposition(|b| matches!(b, AgentBlock::Compaction(c) if c.status == CompactionStatus::Completed)).map(|b| (t, b))
  })
}

fn title_of(text: &str) -> Option<String> {
  let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
  let mut out: String = line.chars().take(TITLE_MAX).collect();
  if line.chars().count() > TITLE_MAX {
    out.push('…');
  }
  Some(out)
}

/// The turns still in the window, each opened by its prompt. Turns are numbered only while nothing was compacted, as
/// the built-in agent's report does
fn groups(turns: &[Turn]) -> Vec<Group<'_>> {
  let cut = last_compaction(turns);
  let mut out: Vec<Group> = vec![];
  let mut index = 0u64;
  for (t, turn) in turns.iter().enumerate() {
    match turn {
      Turn::User(u) => {
        index += 1;
        if cut.is_some_and(|(ct, _)| t <= ct) {
          continue;
        }
        let id = format!("t:{t}");
        out.push(Group {
          leaves: vec![(format!("{id}:msg"), Leaf::Message(&u.text))],
          id,
          index: cut.is_none().then_some(index),
          title: title_of(&u.text),
        });
      }
      Turn::Agent(a) => {
        for (b, block) in a.blocks.iter().enumerate() {
          if cut.is_some_and(|(ct, cb)| t < ct || (t == ct && b <= cb)) {
            continue;
          }
          if out.is_empty() {
            out.push(Group { id: format!("t:{t}"), index: None, title: None, leaves: vec![] });
          }
          let Some(g) = out.last_mut() else { continue };
          match block {
            AgentBlock::Thought(x) => push_text(g, "think", x.text.as_str(), Leaf::Thinking),
            AgentBlock::Text(x) => push_text(g, "reply", x.markdown.as_str(), Leaf::Reply),
            AgentBlock::Steer(x) => g.leaves.push((format!("{}:s:{t}:{b}", g.id), Leaf::Message(&x.text))),
            AgentBlock::ToolCall(x) => g.leaves.push((format!("{}:c:{t}:{b}", g.id), Leaf::Call(x))),
            _ => {}
          }
        }
      }
    }
  }
  out
}

/// A turn's thinking (or reply) pieces gather in one leaf, placed where the first one came
fn push_text<'a>(g: &mut Group<'a>, key: &str, text: &'a str, make: fn(Vec<&'a str>) -> Leaf<'a>) {
  let id = format!("{}:{key}", g.id);
  match g.leaves.iter_mut().find(|(i, _)| *i == id) {
    Some((_, Leaf::Thinking(ts) | Leaf::Reply(ts))) => ts.push(text),
    _ => g.leaves.push((id, make(vec![text]))),
  }
}

/// Changes whenever the transcript or the figure does, so content asked for an older report gets nothing
fn rev_of(turns: &[Turn], used: u64) -> String {
  let last = turns.last().and_then(Turn::as_agent).map_or(0, |a| a.blocks.len());
  format!("{REV_PREFIX}{}:{last}:{used}", turns.len())
}

const CATEGORY_ORDER: [ContextCategory; 7] =
  [ContextCategory::Agent, ContextCategory::System, ContextCategory::Tools, ContextCategory::Docs, ContextCategory::Skills, ContextCategory::Mcp, ContextCategory::Chat];

fn cat_key(c: ContextCategory) -> &'static str {
  match c {
    ContextCategory::Agent => "agent",
    ContextCategory::System => "system",
    ContextCategory::Tools => "tools",
    ContextCategory::Docs => "docs",
    ContextCategory::Skills => "skills",
    ContextCategory::Mcp => "mcp",
    ContextCategory::Chat => "chat",
  }
}

fn num(n: f64) -> u64 {
  if n.is_finite() && n > 0.0 { n.round() as u64 } else { 0 }
}

/// Category sums an agent reports of its own context, the conversation among them
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
  pub parts: Vec<(ContextCategory, u64)>,
  pub chat: u64,
  /// The text a compaction left in place of the turns before it, the first part of the conversation
  pub summary: Option<String>,
  /// Of the last request, what the provider served from its prompt cache
  pub cached: Option<u64>,
}

/// Grok's split (`_x.ai/session/info` `context`, grok 1.0.62): the system prompt, the tool definitions and the messages,
/// of which `usageCategories` names what Grok put in front (skills, workflows, AGENTS.md); the rest of the messages is
/// the conversation. A label it adds later stays with the agent's own part
pub fn grok_known(v: &Value) -> Option<Known> {
  let ctx = v.pointer("/result/context")?;
  let n = |k: &str| ctx.get(k).and_then(Value::as_u64).unwrap_or(0);
  let mut parts: Vec<(ContextCategory, u64)> = vec![];
  let mut add = |cat: ContextCategory, tokens: u64| match parts.iter_mut().find(|(c, _)| *c == cat) {
    Some((_, t)) => *t += tokens,
    None if tokens > 0 => parts.push((cat, tokens)),
    None => {}
  };
  add(ContextCategory::System, n("systemPromptTokens"));
  add(ContextCategory::Tools, n("toolDefinitionsTokens"));
  let mut injected = 0;
  for c in ctx.get("usageCategories").and_then(Value::as_array).into_iter().flatten() {
    let tokens = c.get("tokens").and_then(Value::as_u64).unwrap_or(0);
    let cat = match c.get("label").and_then(Value::as_str).unwrap_or_default() {
      "Skills" | "Workflows" => ContextCategory::Skills,
      "AGENTS.md" => ContextCategory::Docs,
      "MCP" => ContextCategory::Mcp,
      _ => ContextCategory::Agent,
    };
    add(cat, tokens);
    injected += tokens;
  }
  let chat = n("messageTokens").saturating_sub(injected);
  if parts.is_empty() && chat == 0 {
    return None;
  }
  Some(Known { parts, chat, summary: None, cached: None })
}

/// The estimated split of the figure the agent reported; none without one
pub fn report(turns: &[Turn], usage: Option<&Usage>, detail: bool) -> Option<ContextReport> {
  report_with(turns, usage, detail, None)
}

/// `report`, with the agent's own category sums when it reports them (`Known`)
pub fn report_with(turns: &[Turn], usage: Option<&Usage>, detail: bool, known: Option<&Known>) -> Option<ContextReport> {
  let usage = usage?;
  let (used, size) = (num(usage.used.0), num(usage.size.0));
  if used == 0 || size == 0 {
    return None;
  }
  // The agent's sums scaled into the figure (they describe its last request, the figure may include the reply)
  let known_total = known.map_or(0, |k| k.parts.iter().map(|p| p.1).sum::<u64>() + k.chat);
  let fit = |n: u64| ((n as f64) * (used as f64) / (known_total as f64)).floor() as u64;
  let parts: Vec<(ContextCategory, u64)> = match known {
    Some(k) if known_total > 0 => k.parts.iter().map(|(c, n)| (*c, fit(*n))).filter(|p| p.1 > 0).collect(),
    _ => vec![],
  };
  let chat_target = known.filter(|_| known_total > 0).map(|_| used.saturating_sub(parts.iter().map(|p| p.1).sum()));
  let groups = groups(turns);
  let raw: Vec<Vec<u64>> = groups.iter().map(|g| g.leaves.iter().map(|(_, l)| l.tokens()).collect()).collect();
  let summary = known.and_then(|k| k.summary.as_deref()).filter(|s| !s.is_empty());
  let summary_raw = summary.map_or(0, approx_tokens);
  let total: u64 = raw.iter().flatten().sum::<u64>() + summary_raw;
  // The transcript can hold more than the agent sent (output it cut, reasoning it dropped): scaled into the figure, or
  // into the conversation the agent reports, up or down
  let scale = |n: u64| match chat_target {
    Some(t) if total > 0 => ((n as f64) * (t as f64) / (total as f64)).floor() as u64,
    _ if total > used => ((n as f64) * (used as f64) / (total as f64)).floor() as u64,
    _ => n,
  };
  let mut chat_nodes = vec![];
  let mut chat = 0u64;
  if summary.is_some() {
    let tokens = scale(summary_raw);
    chat += tokens;
    chat_nodes.push(ContextNode { id: SUMMARY_ID.to_owned(), kind: ContextNodeKind::Summary, name: None, title: None, index: None, tool_kind: None, tokens, cached: None, children: None });
  }
  for (g, sizes) in groups.iter().zip(&raw) {
    let children: Vec<ContextNode> = g.leaves.iter().zip(sizes).map(|((id, leaf), n)| leaf.node(id.clone(), scale(*n))).collect();
    let tokens: u64 = children.iter().map(|c| c.tokens).sum();
    chat += tokens;
    chat_nodes.push(ContextNode {
      id: g.id.clone(),
      kind: ContextNodeKind::Turn,
      name: None,
      title: g.title.clone(),
      index: g.index,
      tool_kind: None,
      tokens,
      cached: None,
      children: Some(children),
    });
  }
  let mut categories = vec![];
  let unsplit = |cat: ContextCategory, tokens: u64| {
    let id = if cat == ContextCategory::Agent { REST_ID.to_owned() } else { format!("{REST_ID}:{}", cat_key(cat)) };
    let node = ContextNode { id, kind: ContextNodeKind::Rest, name: None, title: None, index: None, tool_kind: None, tokens, cached: None, children: None };
    ContextCategorySplit { id: cat, tokens, cached: None, nodes: detail.then(|| vec![node]) }
  };
  for (cat, tokens) in &parts {
    categories.push(unsplit(*cat, *tokens));
  }
  // What the parts leave of the figure: the agent's own, or with its sums, rounding and a conversation it reported
  // that the transcript has none of
  let rest = used.saturating_sub(chat + parts.iter().map(|p| p.1).sum::<u64>());
  if rest > 0 {
    let cat = if chat_target.is_some() && parts.iter().any(|p| p.0 == ContextCategory::System) { ContextCategory::System } else { ContextCategory::Agent };
    match categories.iter_mut().find(|c| c.id == cat) {
      Some(c) => {
        c.tokens += rest;
        if let Some(n) = c.nodes.as_mut().and_then(|ns| ns.first_mut()) {
          n.tokens += rest;
        }
      }
      None => categories.insert(0, unsplit(cat, rest)),
    }
  }
  categories.sort_by_key(|c| CATEGORY_ORDER.iter().position(|o| *o == c.id));
  if chat > 0 {
    categories.push(ContextCategorySplit { id: ContextCategory::Chat, tokens: chat, cached: None, nodes: detail.then_some(chat_nodes) });
  }
  let at = turns.iter().rev().filter_map(Turn::as_agent).find_map(|a| a.ended_at.or(a.started_at)).and_then(|t| u64::try_from(t).ok());
  let cached = known.and_then(|k| k.cached).map(|c| c.min(used));
  Some(ContextReport { used, size, estimated: true, cached, at, rev: rev_of(turns, used), categories })
}

/// What a leaf of the report `rev` names stands for; none once the transcript moved on, or for a group
pub fn content(turns: &[Turn], usage: Option<&Usage>, rev: &str, id: &str, known: Option<&Known>) -> Option<ContextContent> {
  let used = num(usage?.used.0);
  if rev != rev_of(turns, used) {
    return None;
  }
  if id == SUMMARY_ID {
    return known.and_then(|k| k.summary.clone()).map(cut);
  }
  groups(turns).iter().flat_map(|g| g.leaves.iter()).find(|(i, _)| i == id).map(|(_, l)| l.content())
}

#[cfg(test)]
mod tests {
  #![allow(clippy::unwrap_used, reason = "test code")]
  use super::*;
  use acpira_shared::num::Num;
  use serde_json::json;

  fn turns(v: serde_json::Value) -> Vec<Turn> {
    serde_json::from_value(v).unwrap()
  }

  fn usage(used: f64) -> Usage {
    Usage { used: Num(used), size: Num(200_000.0), cost: None, compacted_from: None }
  }

  fn tool(id: &str, kind: &str, target: &str, out: &str) -> serde_json::Value {
    json!({ "type": "tool_call", "id": id, "kind": kind, "verb": "Read", "target": target, "status": "completed", "content": { "type": "text", "text": out } })
  }

  #[test]
  fn the_transcript_fills_the_figure_and_the_agent_keeps_the_rest() {
    let t = turns(json!([
      { "role": "user", "text": "fix the build" },
      { "role": "agent", "blocks": [
        { "type": "thought", "text": "look at it" },
        tool("1", "read", "src/main.rs", &"x".repeat(4000)),
        { "type": "text", "markdown": "done" },
      ] },
    ]));
    let r = report(&t, Some(&usage(10_000.0)), true).unwrap();
    assert!(r.estimated);
    assert_eq!(r.categories.iter().map(|c| c.tokens).sum::<u64>(), 10_000);
    let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).unwrap();
    let turn = &chat.nodes.as_ref().unwrap()[0];
    assert_eq!((turn.index, turn.title.as_deref()), (Some(1), Some("fix the build")));
    let kinds: Vec<_> = turn.children.as_ref().unwrap().iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [ContextNodeKind::Message, ContextNodeKind::Thinking, ContextNodeKind::Call, ContextNodeKind::Reply]);
    let call = &turn.children.as_ref().unwrap()[2];
    assert_eq!((call.tool_kind.as_deref(), call.title.as_deref()), (Some("read"), Some("src/main.rs")));
    assert!(call.tokens >= 1000);
    // The leaf's content comes from the transcript, for this report only
    match content(&t, Some(&usage(10_000.0)), &r.rev, &call.id, None).unwrap() {
      ContextContent::Call { input, output, .. } => assert_eq!((input.as_str(), output.unwrap().len()), ("src/main.rs", 4000)),
      other => panic!("{other:?}"),
    }
    assert!(content(&t, Some(&usage(12_000.0)), &r.rev, &call.id, None).is_none());
  }

  #[test]
  fn a_compaction_leaves_only_what_came_after_it_unnumbered() {
    let t = turns(json!([
      { "role": "user", "text": "old work" },
      { "role": "agent", "blocks": [{ "type": "text", "markdown": "y".repeat(40_000) }, { "type": "compaction", "id": "c", "status": "completed" }] },
      { "role": "user", "text": "go on" },
      { "role": "agent", "blocks": [{ "type": "text", "markdown": "ok" }] },
    ]));
    let r = report(&t, Some(&usage(5_000.0)), true).unwrap();
    let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).unwrap();
    let nodes = chat.nodes.as_ref().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!((nodes[0].index, nodes[0].title.as_deref()), (None, Some("go on")));
    let rest = r.categories.iter().find(|c| c.id == ContextCategory::Agent).unwrap();
    assert!(rest.tokens > 4_900, "the summary and the agent's own parts");
  }

  #[test]
  fn more_transcript_than_figure_is_scaled_into_it() {
    let t = turns(json!([
      { "role": "user", "text": "read it" },
      { "role": "agent", "blocks": [tool("1", "read", "big.txt", &"z".repeat(400_000))] },
    ]));
    let r = report(&t, Some(&usage(20_000.0)), false).unwrap();
    assert!(r.categories.iter().map(|c| c.tokens).sum::<u64>() <= 20_000);
    assert!(r.categories.iter().all(|c| c.nodes.is_none()), "a card asks for sums only");
  }

  #[test]
  fn grok_reports_its_own_categories_and_the_turns_fill_its_conversation() {
    let info = json!({ "result": { "context": {
      "used": 20_000, "systemPromptTokens": 1000, "toolDefinitionsTokens": 8000, "messageTokens": 11_000,
      "usageCategories": [{ "label": "Skills", "tokens": 700 }, { "label": "Workflows", "tokens": 100 }, { "label": "AGENTS.md", "tokens": 200 }]
    } } });
    let known = grok_known(&info).unwrap();
    assert_eq!(known.chat, 10_000);
    let t = turns(json!([{ "role": "user", "text": "hi" }, { "role": "agent", "blocks": [{ "type": "text", "markdown": "hello" }] }]));
    let r = report_with(&t, Some(&usage(20_000.0)), true, Some(&known)).unwrap();
    let by: Vec<(ContextCategory, u64)> = r.categories.iter().map(|c| (c.id, c.tokens)).collect();
    assert_eq!(r.categories.iter().map(|c| c.tokens).sum::<u64>(), 20_000, "{by:?}");
    assert_eq!(by.iter().map(|b| b.0).collect::<Vec<_>>(), [ContextCategory::System, ContextCategory::Tools, ContextCategory::Docs, ContextCategory::Skills, ContextCategory::Chat]);
    assert_eq!(by[1].1, 8000);
    // Two short messages stand for Grok's whole conversation
    assert!(by[4].1 >= 9_990, "{by:?}");
  }

  #[test]
  fn cjk_counts_a_token_per_character() {
    assert_eq!(approx_tokens("abcd"), 1);
    assert_eq!(approx_tokens("修复构建"), 4);
  }
}
