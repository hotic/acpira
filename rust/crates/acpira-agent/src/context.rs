//! The context report (`_acpira/context`, `_acpira/contextItem`, `_acpira/contextRaw`): the last model request split by
//! what fills it, for the composer ring's card and the report tab (`acpira-shared/src/context.rs`). Each model call
//! leaves a snapshot of what it sent and got back (`Snapshot::of_call`); a compaction leaves an estimate of the history
//! it left (`Snapshot::estimate`), since that is what the ring shows next. The parts are the agent's own size estimates
//! scaled to the provider's count: the request's parts to the input, the reply's to the output, which also carries
//! reasoning the reply does not show. So the parts add up to the figure the ring shows.
//!
//! Turns are numbered only while nothing is compacted: a summary's carried messages include steered ones, so the turns
//! it took in are not known, and a guessed number would contradict the transcript.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use acpira_rpc::rpc::RpcError;
use serde_json::{Value, json};

use acpira_shared::context::{ContextCategory, ContextCategorySplit, ContextContent, ContextNode, ContextNodeKind, ContextRaw, ContextReport};
use acpira_shared::providers::ApiFormat;

use crate::acp::{Server, SessionState};
use crate::chunk::{strip_notes, tokens};
use crate::llm::family::Family;
use crate::llm::{self, Item, Part, Request};
use crate::prompt::{Section, SectionKind};

pub const METHOD_REPORT: &str = "_acpira/context";
pub const METHOD_ITEM: &str = "_acpira/contextItem";
pub const METHOD_RAW: &str = "_acpira/contextRaw";

/// Advertised at initialize as `_meta["acpira/context"]`
pub fn capability() -> Value {
  json!({ "version": 1 })
}

/// Size estimate of an image, as the compaction's (`chunk::size`)
const IMAGE_TOKENS: u64 = 1000;
/// A leaf's content is cut here (each piece of a call: its input and its output)
const MAX_CONTENT: usize = 256 * 1024;
/// A string in the raw request is cut here
const MAX_RAW_STRING: usize = 64 * 1024;
/// What a steered message's note closes with (`steer.rs`)
const STEER_CLOSE: &str = "\n</steer>";
/// A turn's or a call's title is cut here
const MAX_TITLE: usize = 160;

/// Where the request went: what the raw view rebuilds its body for (the API key is never kept)
#[derive(Debug, Clone)]
pub struct Wire {
  pub format: ApiFormat,
  pub url: String,
  pub family: Family,
}

/// An MCP tool as the report names it
#[derive(Debug, Clone)]
pub struct McpName {
  /// The server's key (the tool name's prefix) and its configured name
  pub server: String,
  pub label: String,
  /// The tool's own name
  pub tool: String,
}

/// What one model call sent and got back, or the history a compaction left
#[derive(Debug)]
pub struct Snapshot {
  rev: String,
  at: u64,
  request: Request,
  /// The reply as it went into the history; None for an estimate
  reply: Option<Item>,
  wire: Option<Wire>,
  /// The prompt's parts when `request.system` is the composed prompt; empty when a plugin rewrote it
  sections: Vec<Section>,
  mcp: HashMap<String, McpName>,
  /// The first item past the compacted part (a summary, folded chunks)
  body: usize,
  /// Nothing is compacted, so turns are numbered from 1
  numbered: bool,
  input: u64,
  output: u64,
  cached: u64,
  estimated: bool,
}

/// What a call ran with, beside the request
pub struct Setting<'a> {
  /// The composed prompt and its sections
  pub system: &'a str,
  pub sections: &'a [Section],
  pub mcp: &'a [crate::mcp::Tool],
}

static REVS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
  std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Snapshot {
  /// A call that went out and was counted: `request` as sent (after the plugins' `context` hook), `reply` the item it
  /// added to the history, `st` the history it went out with
  pub fn of_call(request: Request, reply: Item, wire: Wire, setting: Setting, usage: llm::Usage, estimated: bool, st: &SessionState) -> Snapshot {
    let sections = if request.system == setting.system { setting.sections.to_vec() } else { vec![] };
    let mut s = Snapshot::new(request, Some(reply), Some(wire), sections, setting.mcp, st);
    (s.input, s.output, s.cached, s.estimated) = (usage.input, usage.output, usage.cache_read.min(usage.input), estimated);
    s
  }

  /// The history right after a compaction, at the estimate the ring shows (`turn::setup::report_usage`)
  pub fn estimate(system: String, tools: Vec<llm::ToolSpec>, setting: Setting, used: u64, st: &SessionState) -> Snapshot {
    let request = Request {
      model: String::new(),
      system,
      items: st.items.clone(),
      tools,
      max_tokens: None,
      output_limit: None,
      sampling: Default::default(),
      thinking: Default::default(),
      effort: None,
      cache_key: None,
      fast: false,
    };
    let sections = if request.system == setting.system { setting.sections.to_vec() } else { vec![] };
    let mut s = Snapshot::new(request, None, None, sections, setting.mcp, st);
    (s.input, s.estimated) = (used, true);
    s
  }

  fn new(request: Request, reply: Option<Item>, wire: Option<Wire>, sections: Vec<Section>, mcp: &[crate::mcp::Tool], st: &SessionState) -> Snapshot {
    let at = now_ms();
    let n = REVS.fetch_add(1, Ordering::Relaxed);
    let body = (usize::from(st.summarized) + st.folded).min(request.items.len());
    Snapshot {
      rev: format!("{at:x}.{n:x}"),
      at,
      request,
      reply,
      wire,
      sections,
      mcp: mcp.iter().map(|t| (t.name.clone(), McpName { server: t.server.clone(), label: t.server_name.clone(), tool: t.tool.clone() })).collect(),
      body,
      numbered: body == 0,
      input: 0,
      output: 0,
      cached: 0,
      estimated: false,
    }
  }

  pub fn used(&self) -> u64 {
    self.input + self.output
  }

  /// The split; nodes only when `detail` (the report tab's tree)
  pub fn report(&self, size: u64, detail: bool) -> ContextReport {
    let mut cats = self.build();
    self.scale(&mut cats);
    self.mark_cached(&mut cats);
    let categories = cats
      .into_iter()
      .filter(|c| !c.nodes.is_empty())
      .map(|c| {
        let nodes: Vec<ContextNode> = c.nodes.into_iter().map(Node::into_node).collect();
        let tokens = nodes.iter().map(|n| n.tokens).sum();
        let cached: u64 = nodes.iter().map(|n| n.cached.unwrap_or(0)).sum();
        ContextCategorySplit { id: c.id, tokens, cached: (cached > 0).then_some(cached), nodes: detail.then_some(nodes) }
      })
      .collect();
    ContextReport {
      used: self.used(),
      size,
      estimated: self.estimated,
      cached: (self.cached > 0).then_some(self.cached),
      at: Some(self.at),
      rev: self.rev.clone(),
      categories,
    }
  }

  /// What a leaf sent; None for a group, an unknown id or another snapshot's
  pub fn content(&self, rev: &str, id: &str) -> Option<ContextContent> {
    if rev != self.rev {
      return None;
    }
    let cats = self.build();
    match cats.iter().flat_map(|c| c.nodes.iter()).find_map(|n| n.find(id)) {
      Some(leaf) => leaf.src.as_ref().map(|src| self.render(src)),
      // Reasoning the provider counted and kept hidden (`scale`)
      None if id.starts_with("t:") && id.ends_with(":think") && self.reply.is_some() => Some(text("")),
      None => None,
    }
  }

  /// The request in the provider's wire format, images and signatures elided; None for an estimate
  pub fn raw(&self) -> Option<ContextRaw> {
    let wire = self.wire.as_ref()?;
    let mut body = match wire.format {
      ApiFormat::OpenaiChat => llm::openai_chat::body(&self.request, &wire.family),
      ApiFormat::OpenaiResponses => llm::openai_responses::body(&self.request, &llm::openai_responses::origin(&wire.url, &self.request.model)),
      ApiFormat::Anthropic => llm::anthropic::body(&self.request, &wire.family, &llm::anthropic::origin(&wire.url, &self.request.model)),
    };
    elide(&mut body);
    // A query string may carry a key
    let url = wire.url.split(['?', '#']).next().unwrap_or_default().to_owned();
    Some(ContextRaw { format: wire.format.as_str().to_owned(), url: Some(url), body })
  }

  fn items(&self) -> impl Iterator<Item = &Item> {
    self.request.items.iter().chain(self.reply.iter())
  }

  fn item(&self, i: usize) -> Option<&Item> {
    self.request.items.get(i).or(if i == self.request.items.len() { self.reply.as_ref() } else { None })
  }

  /// Every part with its estimate and where it sits in the request, unscaled
  fn build(&self) -> Vec<Cat> {
    let mut system = Cat::new(ContextCategory::System);
    let mut tools = Cat::new(ContextCategory::Tools);
    let mut docs = Cat::new(ContextCategory::Docs);
    let mut skills = Cat::new(ContextCategory::Skills);
    let mut mcp = Cat::new(ContextCategory::Mcp);
    let mut chat = Cat::new(ContextCategory::Chat);

    // The system prompt, by its sections when they still cover it
    let text = &self.request.system;
    if covers(&self.sections, text) {
      let mut doc = 0;
      for s in &self.sections {
        let part = text.get(s.start..s.end).unwrap_or_default();
        let pos = Pos::new(Seg::System, s.start, 0);
        let src = Some(Src::Prompt(s.start, s.end));
        match s.kind {
          SectionKind::Base => system.nodes.push(Node::leaf("sys:base", ContextNodeKind::Base, tokens(part), pos, src)),
          SectionKind::Environment => system.nodes.push(Node::leaf("sys:env", ContextNodeKind::Environment, tokens(part), pos, src)),
          SectionKind::Doc => {
            let mut n = Node::leaf(&format!("doc:{doc}"), ContextNodeKind::Doc, tokens(part), pos, src);
            n.name = s.path.clone();
            docs.nodes.push(n);
            doc += 1;
          }
        }
      }
    } else if !text.is_empty() {
      system.nodes.push(Node::leaf("sys:prompt", ContextNodeKind::Prompt, tokens(text), Pos::new(Seg::System, 0, 0), Some(Src::Prompt(0, text.len()))));
    }

    // Tool definitions: the skill tool split into its own text and one line per skill, MCP tools under their server
    let mut servers: Vec<Node> = vec![];
    for (t, spec) in self.request.tools.iter().enumerate() {
      let schema = spec.parameters.to_string();
      if spec.name == crate::tools::SKILL
        && let Some((head, list)) = spec.description.split_once(SKILLS_HEAD)
      {
        let own = tokens(&spec.name) + tokens(head) + tokens(SKILLS_HEAD) + tokens(&schema);
        skills.nodes.push(Node::leaf("skilltool", ContextNodeKind::SkillTool, own, Pos::new(Seg::Tools, t, 0), Some(Src::SkillHead(t))));
        let mut at = head.len() + SKILLS_HEAD.len();
        for (l, line) in list.split('\n').enumerate() {
          let start = at;
          at += line.len() + 1;
          let Some(name) = skill_name(line) else { continue };
          let mut n = Node::leaf(&format!("skill:{name}"), ContextNodeKind::Skill, tokens(line), Pos::new(Seg::Tools, t, l + 1), Some(Src::SkillLine(t, start, start + line.len())));
          n.name = Some(name.to_owned());
          skills.nodes.push(n);
        }
        continue;
      }
      let own = tokens(&spec.name) + tokens(&spec.description) + tokens(&schema);
      let pos = Pos::new(Seg::Tools, t, 0);
      match self.mcp.get(&spec.name) {
        Some(m) => {
          let mut n = Node::leaf(&format!("mcp:{}", spec.name), ContextNodeKind::McpTool, own, pos, Some(Src::Def(t)));
          n.name = Some(m.tool.clone());
          let id = format!("srv:{}", m.server);
          match servers.iter_mut().find(|s| s.id == id) {
            Some(s) => s.children.push(n),
            None => {
              let mut s = Node::group(&id, ContextNodeKind::Server, pos);
              s.name = Some(m.label.clone());
              s.children.push(n);
              servers.push(s);
            }
          }
        }
        None => {
          let mut n = Node::leaf(&format!("tool:{}", spec.name), ContextNodeKind::Tool, own, pos, Some(Src::Def(t)));
          n.name = Some(spec.name.clone());
          tools.nodes.push(n);
        }
      }
    }
    mcp.nodes = servers;

    // The conversation: what a compaction left, then the turns
    let sent = self.request.items.len();
    if self.body > 0 {
      let est: u64 = self.request.items[..self.body].iter().map(item_tokens).sum();
      chat.nodes.push(Node::leaf("sum", ContextNodeKind::Summary, est, Pos::new(Seg::Chat, 0, 0), Some(Src::Summary)));
    }
    let names: Vec<&str> = self.request.tools.iter().map(|t| t.name.as_str()).collect();
    let mut turns: Vec<TurnB> = vec![];
    let mut prev: Option<&Item> = self.body.checked_sub(1).and_then(|i| self.request.items.get(i));
    for (i, item) in self.items().enumerate().skip(self.body) {
      let out = i >= sent;
      let starts = match item {
        // A note alone behind a round's tool results (the goal's budget) stays in the turn
        Item::User(_) if !item.is_steer() => !(crate::chunk::user_words(item).is_none() && matches!(prev, Some(Item::ToolResult { .. }))),
        _ => false,
      };
      if starts || turns.is_empty() {
        turns.push(TurnB { first: i, prompt: starts.then_some(i), ..Default::default() });
      }
      let Some(turn) = turns.last_mut() else { continue };
      match item {
        Item::User(parts) => {
          let steer = item.is_steer();
          for (p, part) in parts.iter().enumerate() {
            match part {
              Part::Text(t) => {
                // The notes the agent stacked in front: a prompt's, and a steered message's own note
                let note_len = if steer {
                  if p == 0 { t.find(STEER_CLOSE).map_or(t.len(), |k| k + STEER_CLOSE.len()) } else { 0 }
                } else if !starts && crate::chunk::user_words(item).is_none() {
                  t.len()
                } else {
                  t.len() - strip_notes(t).len()
                };
                if note_len > 0 {
                  turn.note.add(i, Frag::Text(p, 0, note_len), tokens(t.get(..note_len).unwrap_or_default()), out);
                }
                if note_len < t.len() {
                  turn.msg.add(i, Frag::Text(p, note_len, t.len()), tokens(t.get(note_len..).unwrap_or_default()), out);
                }
              }
              Part::Image { .. } => turn.msg.add(i, Frag::Image(p), IMAGE_TOKENS, out),
            }
          }
        }
        Item::Assistant { text, reasoning, tool_calls, .. } => {
          if !reasoning.is_empty() {
            turn.think.add(i, Frag::Reasoning, tokens(reasoning), out);
          }
          if !text.is_empty() {
            turn.reply.add(i, Frag::Reply, tokens(text), out);
          }
          for (c, call) in tool_calls.iter().enumerate() {
            let est = tokens(&call.name) + tokens(&call.arguments);
            turn.calls.push(CallB { item: Some(i), call: c, id: call.id.clone(), name: call.name.clone(), result: None, inp: if out { 0 } else { est }, out: if out { est } else { 0 } });
          }
        }
        Item::ToolResult { call_id, name, content, images, .. } => {
          let est = tokens(content) + IMAGE_TOKENS * images.len() as u64;
          match turn.calls.iter_mut().rev().find(|c| c.result.is_none() && &c.id == call_id) {
            Some(c) => {
              c.result = Some(i);
              c.inp += est;
            }
            // A result whose call went into a summary
            None => turn.calls.push(CallB { item: None, call: 0, id: call_id.clone(), name: name.clone(), result: Some(i), inp: est, out: 0 }),
          }
        }
      }
      prev = Some(item);
    }
    for (n, t) in turns.into_iter().enumerate() {
      chat.nodes.push(self.turn_node(n, t, &names));
    }
    vec![system, tools, docs, skills, mcp, chat]
  }

  fn turn_node(&self, n: usize, t: TurnB, names: &[&str]) -> Node {
    let id = format!("t:{n}");
    let mut g = Node::group(&id, ContextNodeKind::Turn, Pos::new(Seg::Chat, t.first, 0));
    g.index = self.numbered.then_some(n as u64 + 1);
    g.title = t.prompt.and_then(|i| self.item(i)).and_then(crate::chunk::user_words).map(|w| title_of(&w));
    for (key, kind, piece, sub) in [("note", ContextNodeKind::Note, t.note, 0), ("msg", ContextNodeKind::Message, t.msg, 1), ("think", ContextNodeKind::Thinking, t.think, 2), ("reply", ContextNodeKind::Reply, t.reply, 3)] {
      if let Some(first) = piece.frags.first().map(|f| f.0) {
        let mut leaf = Node::leaf(&format!("{id}:{key}"), kind, 0, Pos::new(Seg::Chat, first, sub), Some(Src::Piece(piece.frags)));
        (leaf.inp, leaf.out) = (piece.inp, piece.out);
        g.children.push(leaf);
      }
    }
    for (k, c) in t.calls.into_iter().enumerate() {
      let first = c.item.or(c.result).unwrap_or(t.first);
      let mut leaf = Node::leaf(&format!("{id}:c:{k}"), ContextNodeKind::Call, 0, Pos::new(Seg::Chat, first, 4 + c.call), Some(Src::Call { item: c.item, call: c.call, result: c.result }));
      (leaf.inp, leaf.out) = (c.inp, c.out);
      leaf.tool_kind = Some(crate::turn::kind_of(&c.name, names).to_owned());
      leaf.title = c.item.and_then(|i| match self.item(i) {
        Some(Item::Assistant { tool_calls, .. }) => tool_calls.get(c.call).and_then(|tc| call_title(&tc.arguments)),
        _ => None,
      });
      leaf.name = Some(c.name);
      g.children.push(leaf);
    }
    g.children.sort_by_key(|c| c.pos);
    g
  }

  /// Leaves to the provider's count: the request's to the input, the reply's to the output; output beyond what the
  /// reply shows is reasoning it kept hidden, which goes to the last turn's thinking. Rounded so the sum is exact
  fn scale(&self, cats: &mut [Cat]) {
    let (est_in, est_out) = {
      let mut leaves = vec![];
      for c in cats.iter_mut() {
        for n in c.nodes.iter_mut() {
          n.leaves(&mut leaves);
        }
      }
      (leaves.iter().map(|l| l.inp).sum::<u64>(), leaves.iter().map(|l| l.out).sum::<u64>())
    };
    let hidden = self.output.saturating_sub(est_out);
    if hidden > 0 && let Some(turn) = cats.last_mut().and_then(|c| c.nodes.iter_mut().rev().find(|n| n.kind == ContextNodeKind::Turn)) {
      match turn.children.iter_mut().find(|n| n.kind == ContextNodeKind::Thinking) {
        Some(t) => t.out += hidden,
        None => {
          let at = turn.children.last().map_or(turn.pos, |c| c.pos);
          let mut t = Node::leaf(&format!("{}:think", turn.id), ContextNodeKind::Thinking, 0, at, Some(Src::Piece(vec![])));
          t.out = hidden;
          turn.children.push(t);
        }
      }
    }
    let f_in = if est_in > 0 { self.input as f64 / est_in as f64 } else { 0.0 };
    let f_out = if est_out > self.output { self.output as f64 / est_out as f64 } else { 1.0 };
    let mut leaves = vec![];
    for c in cats.iter_mut() {
      for n in c.nodes.iter_mut() {
        n.leaves(&mut leaves);
      }
    }
    let exact: Vec<f64> = leaves.iter().map(|l| l.inp as f64 * f_in + l.out as f64 * f_out).collect();
    for (l, e) in leaves.iter_mut().zip(&exact) {
      l.tokens = e.floor() as u64;
    }
    // Largest remainders take the rounding's rest; what no estimate covers (an input with nothing to spread over) goes
    // to the largest part
    let mut rest = self.used().saturating_sub(leaves.iter().map(|l| l.tokens).sum());
    let mut order: Vec<usize> = (0..leaves.len()).collect();
    order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    for i in order {
      if rest == 0 {
        break;
      }
      leaves[i].tokens += 1;
      rest -= 1;
    }
    if rest > 0 && let Some(l) = leaves.iter_mut().max_by_key(|l| l.tokens) {
      l.tokens += rest;
    }
  }

  /// The provider's cache serves a request's start: walk the leaves in the order the format sends them
  fn mark_cached(&self, cats: &mut [Cat]) {
    let anthropic = self.wire.as_ref().is_some_and(|w| w.format == ApiFormat::Anthropic);
    let mut leaves = vec![];
    for c in cats.iter_mut() {
      for n in c.nodes.iter_mut() {
        n.leaves(&mut leaves);
      }
    }
    leaves.sort_by_key(|l| (l.pos.rank(anthropic), l.pos.at, l.pos.sub));
    let mut left = self.cached;
    for l in leaves {
      if left == 0 {
        break;
      }
      let c = l.tokens.min(left);
      l.cached = (c > 0).then_some(c);
      left -= c;
    }
  }

  fn render(&self, src: &Src) -> ContextContent {
    match src {
      Src::Prompt(a, b) => text(self.request.system.get(*a..*b).unwrap_or_default()),
      Src::Def(t) => self.request.tools.get(*t).map_or(text(""), |s| def(&s.description, &s.parameters)),
      Src::SkillHead(t) => self.request.tools.get(*t).map_or(text(""), |s| def(s.description.split(SKILLS_HEAD).next().unwrap_or_default(), &s.parameters)),
      Src::SkillLine(t, a, b) => text(self.request.tools.get(*t).and_then(|s| s.description.get(*a..*b)).unwrap_or_default()),
      Src::Summary => text(&self.request.items[..self.body].iter().map(item_text).collect::<Vec<_>>().join("\n\n")),
      Src::Piece(frags) => {
        let parts: Vec<String> = frags
          .iter()
          .filter_map(|(i, f)| {
            let item = self.item(*i)?;
            match (item, f) {
              (Item::User(parts), Frag::Text(p, a, b)) => match parts.get(*p) {
                Some(Part::Text(t)) => t.get(*a..*b).map(|s| s.trim().to_owned()),
                _ => None,
              },
              (Item::User(parts), Frag::Image(p)) => match parts.get(*p) {
                Some(Part::Image { mime, .. }) => Some(format!("[{mime}]")),
                _ => None,
              },
              (Item::Assistant { reasoning, .. }, Frag::Reasoning) => Some(reasoning.clone()),
              (Item::Assistant { text, .. }, Frag::Reply) => Some(text.clone()),
              _ => None,
            }
          })
          .filter(|s| !s.is_empty())
          .collect();
        text(&parts.join("\n\n"))
      }
      Src::Call { item, call, result } => {
        let input = item.and_then(|i| self.item(i)).and_then(|it| match it {
          Item::Assistant { tool_calls, .. } => tool_calls.get(*call).map(|c| c.arguments.clone()),
          _ => None,
        });
        let res = result.and_then(|i| self.item(i)).and_then(|it| match it {
          Item::ToolResult { content, is_error, images, .. } => Some((content.clone(), *is_error, images.len() as u64)),
          _ => None,
        });
        let input = input.unwrap_or_default();
        let truncated = input.len() > MAX_CONTENT || res.as_ref().is_some_and(|r| r.0.len() > MAX_CONTENT);
        ContextContent::Call {
          input: crate::budget::cut(&input, MAX_CONTENT).to_owned(),
          output: res.as_ref().map(|r| crate::budget::cut(&r.0, MAX_CONTENT).to_owned()),
          images: res.as_ref().map(|r| r.2).filter(|n| *n > 0),
          error: res.as_ref().is_some_and(|r| r.1),
          truncated,
        }
      }
    }
  }
}

/// What the skill tool's description lists the skills under (`tools/skill.rs`)
const SKILLS_HEAD: &str = "\n\nAvailable skills:\n";

/// A listed skill's name: `- name: description`, or `- name`
fn skill_name(line: &str) -> Option<&str> {
  let rest = line.strip_prefix("- ")?;
  let name = rest.split_once(": ").map_or(rest, |(n, _)| n).trim();
  (!name.is_empty()).then_some(name)
}

/// Sections in order, back to back, over the whole text
fn covers(sections: &[Section], text: &str) -> bool {
  let mut at = 0;
  for s in sections {
    if s.start != at || s.end < s.start || text.get(s.start..s.end).is_none() {
      return false;
    }
    at = s.end;
  }
  !sections.is_empty() && at == text.len()
}

fn item_tokens(item: &Item) -> u64 {
  match item {
    Item::User(parts) => parts
      .iter()
      .map(|p| match p {
        Part::Text(t) => tokens(t),
        Part::Image { .. } => IMAGE_TOKENS,
      })
      .sum(),
    Item::Assistant { text, reasoning, tool_calls, .. } => tokens(text) + tokens(reasoning) + tool_calls.iter().map(|c| tokens(&c.name) + tokens(&c.arguments)).sum::<u64>(),
    Item::ToolResult { content, images, .. } => tokens(content) + IMAGE_TOKENS * images.len() as u64,
  }
}

/// An item's text as the summary's content shows it
fn item_text(item: &Item) -> String {
  match item {
    Item::User(parts) => parts
      .iter()
      .map(|p| match p {
        Part::Text(t) => t.clone(),
        Part::Image { mime, .. } => format!("[{mime}]"),
      })
      .collect::<Vec<_>>()
      .join("\n\n"),
    Item::Assistant { text, .. } => text.clone(),
    Item::ToolResult { content, .. } => content.clone(),
  }
}

/// The first line of a text, cut for a title
fn title_of(text: &str) -> String {
  let line = text.trim().lines().next().unwrap_or_default().trim();
  let cut = crate::budget::cut(line, MAX_TITLE);
  if cut.len() < line.len() { format!("{cut}…") } else { cut.to_owned() }
}

/// What a call worked on, from its arguments: a path, a command, a pattern, ...
fn call_title(arguments: &str) -> Option<String> {
  let args: Value = serde_json::from_str(arguments).ok()?;
  for key in ["path", "file_path", "paths", "command", "pattern", "query", "url", "name", "agent", "description", "prompt"] {
    match args.get(key) {
      Some(Value::String(s)) if !s.trim().is_empty() => return Some(title_of(s)),
      Some(Value::Array(a)) => {
        let first = a.iter().find_map(|v| v.as_str().or_else(|| v.get("path").and_then(Value::as_str)))?;
        return Some(if a.len() > 1 { format!("{} +{}", title_of(first), a.len() - 1) } else { title_of(first) });
      }
      _ => {}
    }
  }
  None
}

fn text(s: &str) -> ContextContent {
  ContextContent::Text { text: crate::budget::cut(s, MAX_CONTENT).to_owned(), truncated: s.len() > MAX_CONTENT }
}

fn def(description: &str, schema: &Value) -> ContextContent {
  ContextContent::Def { description: crate::budget::cut(description, MAX_CONTENT).to_owned(), schema: schema.clone(), truncated: description.len() > MAX_CONTENT }
}

/// Image data, signed and encrypted reasoning are elided; very long strings are cut
fn elide(v: &mut Value) {
  match v {
    Value::Object(map) => {
      for (k, x) in map.iter_mut() {
        if let Value::String(s) = x
          && s.len() > 64
          && (matches!(k.as_str(), "data" | "signature" | "encrypted_content") || s.starts_with("data:"))
        {
          *s = format!("[{} bytes elided]", s.len());
        } else {
          elide(x);
        }
      }
    }
    Value::Array(a) => a.iter_mut().for_each(elide),
    Value::String(s) if s.starts_with("data:") && s.len() > 64 => *s = format!("[{} bytes elided]", s.len()),
    Value::String(s) if s.len() > MAX_RAW_STRING => {
      let n = s.len();
      *s = format!("{}\n[cut at {} KB of {n} bytes]", crate::budget::cut(s, MAX_RAW_STRING), MAX_RAW_STRING / 1024);
    }
    _ => {}
  }
}

/// Where a leaf sits in the request: the cache serves its start
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Pos {
  seg: Seg,
  at: usize,
  sub: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Seg {
  System,
  Tools,
  Chat,
}

impl Pos {
  fn new(seg: Seg, at: usize, sub: usize) -> Pos {
    Pos { seg, at, sub }
  }

  /// Anthropic sends the tools before the system prompt; the OpenAI formats are taken as the prompt first
  fn rank(self, anthropic: bool) -> u8 {
    match (self.seg, anthropic) {
      (Seg::Tools, true) | (Seg::System, false) => 0,
      (Seg::System, true) | (Seg::Tools, false) => 1,
      (Seg::Chat, _) => 2,
    }
  }
}

/// A piece of an item a leaf holds
#[derive(Debug, Clone)]
enum Frag {
  /// A user text part's byte range
  Text(usize, usize, usize),
  Image(usize),
  Reasoning,
  Reply,
}

/// Where a leaf's content is
#[derive(Debug, Clone)]
enum Src {
  Prompt(usize, usize),
  Def(usize),
  SkillHead(usize),
  SkillLine(usize, usize, usize),
  Summary,
  Piece(Vec<(usize, Frag)>),
  Call { item: Option<usize>, call: usize, result: Option<usize> },
}

#[derive(Debug, Default)]
struct PieceB {
  frags: Vec<(usize, Frag)>,
  inp: u64,
  out: u64,
}

impl PieceB {
  fn add(&mut self, item: usize, frag: Frag, est: u64, out: bool) {
    self.frags.push((item, frag));
    if out { self.out += est } else { self.inp += est }
  }
}

#[derive(Debug)]
struct CallB {
  item: Option<usize>,
  call: usize,
  id: String,
  name: String,
  result: Option<usize>,
  inp: u64,
  out: u64,
}

#[derive(Debug, Default)]
struct TurnB {
  first: usize,
  prompt: Option<usize>,
  note: PieceB,
  msg: PieceB,
  think: PieceB,
  reply: PieceB,
  calls: Vec<CallB>,
}

struct Cat {
  id: ContextCategory,
  nodes: Vec<Node>,
}

impl Cat {
  fn new(id: ContextCategory) -> Cat {
    Cat { id, nodes: vec![] }
  }
}

/// A node while the report is built: its estimates (request side, reply side), then its scaled tokens
#[derive(Debug)]
struct Node {
  id: String,
  kind: ContextNodeKind,
  name: Option<String>,
  title: Option<String>,
  index: Option<u64>,
  tool_kind: Option<String>,
  pos: Pos,
  inp: u64,
  out: u64,
  tokens: u64,
  cached: Option<u64>,
  src: Option<Src>,
  children: Vec<Node>,
  group: bool,
}

impl Node {
  fn leaf(id: &str, kind: ContextNodeKind, est: u64, pos: Pos, src: Option<Src>) -> Node {
    Node { id: id.to_owned(), kind, name: None, title: None, index: None, tool_kind: None, pos, inp: est, out: 0, tokens: 0, cached: None, src, children: vec![], group: false }
  }

  fn group(id: &str, kind: ContextNodeKind, pos: Pos) -> Node {
    Node { group: true, ..Node::leaf(id, kind, 0, pos, None) }
  }

  fn leaves<'a>(&'a mut self, out: &mut Vec<&'a mut Node>) {
    if self.group {
      for c in self.children.iter_mut() {
        c.leaves(out);
      }
    } else {
      out.push(self);
    }
  }

  fn find(&self, id: &str) -> Option<&Node> {
    if self.id == id {
      return (!self.group).then_some(self);
    }
    self.children.iter().find_map(|c| c.find(id))
  }

  fn into_node(self) -> ContextNode {
    let children: Option<Vec<ContextNode>> = self.group.then(|| self.children.into_iter().map(Node::into_node).collect());
    let (tokens, cached) = match &children {
      Some(c) => (c.iter().map(|n| n.tokens).sum(), c.iter().map(|n| n.cached.unwrap_or(0)).sum::<u64>()),
      None => (self.tokens, self.cached.unwrap_or(0)),
    };
    ContextNode {
      id: self.id,
      kind: self.kind,
      name: self.name,
      title: self.title,
      index: self.index,
      tool_kind: self.tool_kind,
      tokens,
      cached: (cached > 0).then_some(cached),
      children,
    }
  }
}

fn session_of(server: &Server, params: &Value) -> Result<std::sync::Arc<crate::acp::Session>, RpcError> {
  let id = params.get("sessionId").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "sessionId is required"))?;
  server.session(id)
}

/// `_acpira/context`: `{ report }`, none before the session's first counted call
pub fn report_of(server: &Server, params: &Value) -> Result<Value, RpcError> {
  let session = session_of(server, params)?;
  let detail = params.get("detail").and_then(Value::as_bool) == Some(true);
  let (snap, size) = {
    let st = session.state.lock();
    (st.context.clone(), st.reported.map(|r| r.1))
  };
  let Some(snap) = snap else { return Ok(json!({})) };
  let report = snap.report(size.unwrap_or(0), detail);
  Ok(json!({ "report": report }))
}

/// `_acpira/contextItem`: `{ content }` of a leaf of the report named by `rev`
pub fn item_of(server: &Server, params: &Value) -> Result<Value, RpcError> {
  let session = session_of(server, params)?;
  let (rev, id) = (params.get("rev").and_then(Value::as_str).unwrap_or_default(), params.get("id").and_then(Value::as_str).unwrap_or_default());
  let snap = session.state.lock().context.clone();
  Ok(match snap.and_then(|s| s.content(rev, id)) {
    Some(c) => json!({ "content": c }),
    None => json!({}),
  })
}

/// `_acpira/contextRaw`: `{ raw }`, the last request as it went out
pub fn raw_of(server: &Server, params: &Value) -> Result<Value, RpcError> {
  let session = session_of(server, params)?;
  let snap = session.state.lock().context.clone();
  Ok(match snap.and_then(|s| s.raw()) {
    Some(r) => json!({ "raw": r }),
    None => json!({}),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::llm::{Image, STEER_OPEN, ToolCall, ToolSpec};

  fn spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec { name: name.into(), description: description.into(), parameters: json!({ "type": "object" }) }
  }

  fn request(system: &str, items: Vec<Item>, tools: Vec<ToolSpec>) -> Request {
    Request {
      model: "m".into(),
      system: system.into(),
      items,
      tools,
      max_tokens: None,
      output_limit: None,
      sampling: Default::default(),
      thinking: Default::default(),
      effort: None,
      cache_key: None,
      fast: false,
    }
  }

  fn user(t: &str) -> Item {
    Item::User(vec![Part::Text(t.into())])
  }

  fn assistant(text: &str, reasoning: &str, calls: &[(&str, &str, &str)]) -> Item {
    Item::Assistant {
      text: text.into(),
      reasoning: reasoning.into(),
      tool_calls: calls.iter().map(|(id, name, args)| ToolCall { id: (*id).into(), name: (*name).into(), arguments: (*args).into() }).collect(),
      native: None,
    }
  }

  fn result(id: &str, content: &str) -> Item {
    Item::ToolResult { call_id: id.into(), name: "read".into(), content: content.into(), is_error: false, images: vec![Image { mime: "image/png".into(), data: "A".repeat(200) }] }
  }


  fn wire(format: ApiFormat) -> Wire {
    Wire { format, url: "https://api.example/v1/messages?key=secret".into(), family: crate::llm::family::GENERIC }
  }

  fn sections(system: &str, base: usize, doc: usize) -> Vec<Section> {
    vec![
      Section { kind: SectionKind::Base, path: None, start: 0, end: base },
      Section { kind: SectionKind::Doc, path: Some("/repo/AGENTS.md".into()), start: base, end: base + doc },
      Section { kind: SectionKind::Environment, path: None, start: base + doc, end: system.len() },
    ]
  }

  fn snapshot(format: ApiFormat, usage: llm::Usage, st: &SessionState) -> Snapshot {
    let system = format!("{}{}{}", "base ".repeat(40), "\n<instructions path=\"/repo/AGENTS.md\">\n## Rules\nbe terse\n</instructions>\n", "\n\n## Environment\n- here\n");
    let secs = sections(&system, 200, "\n<instructions path=\"/repo/AGENTS.md\">\n## Rules\nbe terse\n</instructions>\n".len());
    let tools = vec![
      spec("read", &"reads files ".repeat(20)),
      spec("skill", "Load a skill.\n\nAvailable skills:\n- review: review a change\n- lab-shot: screenshots"),
      spec("ctx7_docs", "look up docs"),
    ];
    let mcp = vec![crate::mcp::Tool {
      name: "ctx7_docs".into(),
      server: "ctx7".into(),
      server_name: "context7".into(),
      tool: "docs".into(),
      title: None,
      description: "look up docs".into(),
      schema: json!({}),
      read_only: true,
    }];
    let items = vec![
      user("<mode>\nAgent mode\n</mode>\nfix the build"),
      assistant("", "let me look", &[("c1", "read", "{\"path\":\"src/main.rs\"}")]),
      result("c1", &"fn main() {}\n".repeat(50)),
      Item::User(vec![Part::Text(format!("{STEER_OPEN}note{STEER_CLOSE}\n\nuse pnpm"))]),
    ];
    let reply = assistant("done", "", &[]);
    let req = request(&system, items, tools);
    Snapshot::of_call(req, reply, wire(format), Setting { system: &system, sections: &secs, mcp: &mcp }, usage, false, st)
  }

  fn leaves(n: &ContextNode, out: &mut Vec<ContextNode>) {
    match &n.children {
      Some(c) => c.iter().for_each(|k| leaves(k, out)),
      None => out.push(n.clone()),
    }
  }

  fn all(r: &ContextReport) -> Vec<ContextNode> {
    let mut out = vec![];
    for c in &r.categories {
      c.nodes.iter().flatten().for_each(|n| leaves(n, &mut out));
    }
    out
  }

  #[test]
  fn parts_add_up_to_the_providers_count_and_split_by_what_was_sent() {
    let st = SessionState::default();
    let usage = llm::Usage { input: 9_000, output: 700, cache_read: 1_000, ..Default::default() };
    let snap = snapshot(ApiFormat::Anthropic, usage, &st);
    let r = snap.report(200_000, true);
    assert_eq!(r.used, 9_700);
    assert_eq!(r.categories.iter().map(|c| c.tokens).sum::<u64>(), 9_700);
    assert_eq!(all(&r).iter().map(|n| n.tokens).sum::<u64>(), 9_700);
    let ids: Vec<String> = all(&r).into_iter().map(|n| n.id).collect();
    for id in ["sys:base", "doc:0", "sys:env", "tool:read", "skilltool", "skill:review", "skill:lab-shot", "mcp:ctx7_docs", "t:0:note", "t:0:msg", "t:0:think", "t:0:c:0", "t:0:reply"] {
      assert!(ids.contains(&id.to_owned()), "{id} missing from {ids:?}");
    }
    // The reply shows 1 token of the 700 the call wrote: the rest is hidden reasoning, in the turn's thinking
    let think = all(&r).into_iter().find(|n| n.id == "t:0:think").unwrap();
    assert!(think.tokens >= 690, "{think:?}");
    let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).unwrap();
    let turn = &chat.nodes.as_ref().unwrap()[0];
    assert_eq!((turn.index, turn.title.as_deref()), (Some(1), Some("fix the build")));
    let call = all(&r).into_iter().find(|n| n.id == "t:0:c:0").unwrap();
    assert_eq!((call.tool_kind.as_deref(), call.title.as_deref()), (Some("read"), Some("src/main.rs")));
    // Anthropic's cache covers the tools first
    let tool = all(&r).into_iter().find(|n| n.id == "tool:read").unwrap();
    assert_eq!(tool.cached, Some(tool.tokens));
    assert_eq!(r.cached, Some(1_000));
    let mcp = r.categories.iter().find(|c| c.id == ContextCategory::Mcp).unwrap();
    assert_eq!(mcp.nodes.as_ref().unwrap()[0].name.as_deref(), Some("context7"));
    // The card's summary carries no nodes
    assert!(snap.report(200_000, false).categories.iter().all(|c| c.nodes.is_none()));
  }

  #[test]
  fn leaves_open_what_was_sent() {
    let st = SessionState::default();
    let snap = snapshot(ApiFormat::Anthropic, llm::Usage { input: 5_000, output: 10, ..Default::default() }, &st);
    let rev = snap.report(1, false).rev;
    let Some(ContextContent::Text { text, .. }) = snap.content(&rev, "doc:0") else { panic!() };
    // A heading inside AGENTS.md stays in its section
    assert!(text.contains("## Rules") && text.contains("be terse"), "{text}");
    let Some(ContextContent::Text { text, .. }) = snap.content(&rev, "t:0:msg") else { panic!() };
    assert_eq!(text, "fix the build\n\nuse pnpm");
    let Some(ContextContent::Text { text, .. }) = snap.content(&rev, "t:0:note") else { panic!() };
    assert!(text.starts_with("<mode>") && text.contains("note"), "{text}");
    let Some(ContextContent::Call { input, output, images, .. }) = snap.content(&rev, "t:0:c:0") else { panic!() };
    assert_eq!((input.as_str(), images), ("{\"path\":\"src/main.rs\"}", Some(1)));
    assert!(output.unwrap().starts_with("fn main()"));
    let Some(ContextContent::Def { description, .. }) = snap.content(&rev, "skilltool") else { panic!() };
    assert_eq!(description, "Load a skill.");
    assert_eq!(snap.content(&rev, "skill:review"), Some(ContextContent::Text { text: "- review: review a change".into(), truncated: false }));
    // A group has no content, another report's leaves none either
    assert_eq!(snap.content(&rev, "t:0"), None);
    assert_eq!(snap.content("old", "doc:0"), None);
  }

  #[test]
  fn raw_requests_carry_no_images_signatures_or_keys() {
    let st = SessionState::default();
    let mut snap = snapshot(ApiFormat::Anthropic, llm::Usage { input: 5_000, output: 10, ..Default::default() }, &st);
    snap.request.items.push(Item::Assistant {
      text: String::new(),
      reasoning: String::new(),
      tool_calls: vec![],
      native: Some(llm::Native { origin: llm::anthropic::origin(&snap.wire.as_ref().unwrap().url, "m"), blocks: vec![json!({ "type": "thinking", "thinking": "t", "signature": "S".repeat(300) })] }),
    });
    let raw = snap.raw().unwrap();
    let body = raw.body.to_string();
    assert!(!body.contains("AAAAAAAA") && !body.contains("SSSSSSSS"), "{body}");
    assert_eq!(raw.url.as_deref(), Some("https://api.example/v1/messages"));
    assert_eq!(raw.format, "anthropic");
    for format in [ApiFormat::OpenaiChat, ApiFormat::OpenaiResponses] {
      let snap = snapshot(format, llm::Usage { input: 5_000, ..Default::default() }, &st);
      assert!(!snap.raw().unwrap().body.to_string().contains("AAAAAAAA"));
    }
  }

  #[test]
  fn a_compacted_history_is_one_summary_with_unnumbered_turns() {
    let st = SessionState { summarized: true, ..Default::default() };
    let items = vec![user("<summary>earlier work</summary>"), assistant("", "", &[("c9", "bash", "{\"command\":\"ls\"}")]), result("c9", "a b"), user("next")];
    let st = SessionState { items, ..st };
    let snap = Snapshot::estimate("sys".into(), vec![], Setting { system: "other", sections: &[], mcp: &[] }, 3_000, &st);
    let r = snap.report(10_000, true);
    assert!(r.estimated);
    assert_eq!(r.used, 3_000);
    let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).unwrap().nodes.clone().unwrap();
    assert_eq!(chat.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["sum", "t:0", "t:1"]);
    // The turn the summary took the prompt of has none; nothing is numbered
    assert_eq!((chat[1].title.as_deref(), chat[1].index), (None, None));
    assert_eq!((chat[2].title.as_deref(), chat[2].index), (Some("next"), None));
    // A rewritten prompt is one part
    let sys = r.categories.iter().find(|c| c.id == ContextCategory::System).unwrap().nodes.clone().unwrap();
    assert_eq!(sys.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["sys:prompt"]);
    assert!(snap.raw().is_none());
    assert_eq!(all(&r).iter().map(|n| n.tokens).sum::<u64>(), 3_000);
  }

  #[test]
  fn a_budget_note_behind_tool_results_stays_in_its_turn() {
    let items = vec![user("go"), assistant("", "", &[("c1", "read", "{}")]), result("c1", "x"), user("<goal>\nbudget spent\n</goal>")];
    let st = SessionState { items, ..Default::default() };
    let snap = Snapshot::estimate(String::new(), vec![], Setting { system: "", sections: &[], mcp: &[] }, 100, &st);
    let r = snap.report(1_000, true);
    let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).unwrap().nodes.clone().unwrap();
    assert_eq!(chat.len(), 1);
    let kids: Vec<&str> = chat[0].children.as_ref().unwrap().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(kids, ["t:0:msg", "t:0:c:0", "t:0:note"]);
  }
}
