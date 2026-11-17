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


#[test]
fn calls_name_their_rows() {
  // The model gave "a" to calls in two rounds, and the last reply's call has no row yet (it is being run)
  let st = SessionState {
    items: vec![
      user("go"),
      assistant("", "", &[("a", "read", "{}")]),
      result("a", "x"),
      assistant("", "", &[("a", "read", "{}"), ("b", "read", "{}")]),
      result("a", "y"),
      result("b", "z"),
      assistant("", "", &[("c", "read", "{}")]),
    ],
    ..Default::default()
  };
  let snap = Snapshot::estimate("sys".into(), vec![], Setting { system: "sys", sections: &[], mcp: &[] }, 1_000, &st);
  // An older "a" went into a compaction before these: the visible ones take the latest rows
  let rows = [("a", "call-1"), ("a", "call-4"), ("a", "call-5"), ("b", "call-6")].map(|(i, r)| (i.to_owned(), r.to_owned()));
  fn calls(n: &ContextNode, out: &mut Vec<Option<String>>) {
    if n.kind == ContextNodeKind::Call {
      out.push(n.call.clone());
    }
    n.children.iter().flatten().for_each(|c| calls(c, out));
  }
  let mut got = vec![];
  for c in snap.report(1_000, true, &rows).categories {
    c.nodes.iter().flatten().for_each(|n| calls(n, &mut got));
  }
  assert_eq!(got, vec![Some("call-4".into()), Some("call-5".into()), Some("call-6".into()), None]);
  // Without detail there are no nodes to name
  assert!(snap.report(1_000, false, &rows).categories.iter().all(|c| c.nodes.is_none()));
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
  let r = snap.report(200_000, true, &[]);
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
  assert!(snap.report(200_000, false, &[]).categories.iter().all(|c| c.nodes.is_none()));
}

#[test]
fn leaves_open_what_was_sent() {
  let st = SessionState::default();
  let snap = snapshot(ApiFormat::Anthropic, llm::Usage { input: 5_000, output: 10, ..Default::default() }, &st);
  let rev = snap.report(1, false, &[]).rev;
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
  let r = snap.report(10_000, true, &[]);
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
  let r = snap.report(1_000, true, &[]);
  let chat = r.categories.iter().find(|c| c.id == ContextCategory::Chat).unwrap().nodes.clone().unwrap();
  assert_eq!(chat.len(), 1);
  let kids: Vec<&str> = chat[0].children.as_ref().unwrap().iter().map(|n| n.id.as_str()).collect();
  assert_eq!(kids, ["t:0:msg", "t:0:c:0", "t:0:note"]);
}
