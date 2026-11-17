use super::*;

/// A built-in prompt for the plain family
fn text(name: &str) -> String {
  Library::builtin().text(name, crate::prompt::BASE)
}

/// The summary prompt with the built-in texts
fn summary_prompt(head: &[Item], previous: Option<&str>, room: usize, words: u64) -> (String, bool) {
  super::summary_prompt(head, previous, room, words, &Library::builtin(), crate::prompt::BASE)
}
use crate::llm::ToolCall;

fn user(t: &str) -> Item {
  Item::User(vec![Part::Text(t.into())])
}

fn call(id: &str, name: &str) -> Item {
  Item::Assistant { text: String::new(), reasoning: String::new(), tool_calls: vec![ToolCall { id: id.into(), name: name.into(), arguments: "{}".into() }], native: None }
}

fn result(id: &str, name: &str, bytes: usize) -> Item {
  Item::ToolResult { call_id: id.into(), name: name.into(), content: "x".repeat(bytes), is_error: false, images: vec![] }
}

#[test]
fn a_prune_clears_old_results_past_the_protected_recent_ones() {
  // 100 KB a result = 25k tokens; the last two turns are skipped, then 40k tokens of results are protected
  let big = 100 * 1024;
  let items = vec![
    user("a"), call("1", "read"), result("1", "read", big),
    user("b"), call("2", "read"), result("2", "read", big),
    user("c"), call("3", "skill"), result("3", "skill", big), call("4", "read"), result("4", "read", big),
    user("d"), call("5", "read"), result("5", "read", big),
    user("e"), call("6", "read"), result("6", "read", big),
  ];
  // Walking back: e and d are the last two turns; c's read (25k) is protected, its skill result always; b's read
  // takes the total past 40k and is cleared, so is a's: 50k freed, over the minimum
  assert_eq!(prune_plan(&items), vec![5, 2]);
  // Below the minimum nothing is cleared
  let few = vec![user("a"), call("1", "read"), result("1", "read", big), user("b"), user("c"), call("2", "read"), result("2", "read", 200 * 1024)];
  assert_eq!(prune_plan(&few), Vec::<usize>::new());
  // An earlier prune ends the walk
  let mut pruned = items.clone();
  apply_prune(&mut pruned, &["1".to_owned()]);
  assert_eq!(prune_plan(&pruned), vec![5]);
  assert!(matches!(&pruned[2], Item::ToolResult { content, .. } if content == PRUNED));
}

#[test]
fn the_tail_keeps_the_last_two_turns_that_fit_and_splits_the_oldest() {
  let items = vec![user(&"a".repeat(400)), user(&"b".repeat(400)), call("1", "read"), result("1", "read", 4000), user(&"c".repeat(400))];
  // Turns b and c fit 2000 tokens together (100 + 1002 + 100): the tail starts at b
  assert_eq!(tail_start(&items, 0, 2000), Some(1));
  // With 1200, c fits and b's turn (1102) does not: it is split after its user message, where the rest (1002) fits
  assert_eq!(tail_start(&items, 0, 1200), Some(2));
  // With 500 nothing of b fits, since the split never lands in front of a tool result: the tail is c alone
  assert_eq!(tail_start(&items, 0, 500), Some(4));
  // With everything fitting the tail would be the whole conversation: none
  assert_eq!(tail_start(&items[1..], 0, 100_000), None);
  // Behind a summary item the same rule counts from the first real item
  let mut with_summary = vec![summary_item("old", &Carry::default())];
  with_summary.extend(items[1..].iter().cloned());
  assert_eq!(tail_start(&with_summary, 1, 100_000), None);
  assert_eq!(tail_start(&with_summary, 1, 1200), Some(2));
  assert_eq!(tail_start(&with_summary, 1, 500), Some(4));
  assert!(is_summary(&with_summary[0]) && !is_summary(&items[0]));
  assert_eq!(previous_summary(match &with_summary[0] { Item::User(p) => match &p[0] { Part::Text(t) => t, _ => "" }, _ => "" }), "old");
}

#[test]
fn a_steered_message_belongs_to_the_turn_it_was_sent_into() {
  let steer = crate::steer::item("Note.", &[json!([{ "type": "text", "text": "use printf" }])], true, &[]);
  let items = vec![user("a"), call("1", "bash"), result("1", "bash", 10), user("b"), call("2", "bash"), result("2", "bash", 10), steer.clone(), call("3", "bash"), result("3", "bash", 10)];
  // Two turns, not three: a tail holding both would be the whole conversation
  assert_eq!(tail_start(&items, 0, 100_000), None);
  // Summarized with the prompt it followed, it comes back verbatim behind the request
  assert_eq!(request_text(&items[3..]).as_deref(), Some("b\n\n[Added while the work was in progress]\nuse printf"));
  assert_eq!(serialize(&steer, false), "[User, while the assistant was working]: use printf");
}

#[test]
fn the_summary_prompt_holds_the_transcript_and_the_previous_summary() {
  let head = vec![user("fix it"), call("1", "bash"), result("1", "bash", 10), Item::ToolResult { call_id: "2".into(), name: "read".into(), content: "no".into(), is_error: true, images: vec![] }];
  let (p, _) = summary_prompt(&head, None, 1 << 20, 9000);
  assert!(p.starts_with("<conversation>\n[User]: fix it\n\n[Assistant tool call]: bash({})\n\n[Tool result]: xxxxxxxxxx\n\n[Tool error]: no\n</conversation>"), "{p}");
  assert!(p.ends_with(&format!("{} Keep the <summary> block within about 9000 words.", text("compact/instructions"))) && p.contains("9. Optional Next Step") && !p.contains(&text("compact/previous")));
  assert_eq!((summary_words(1000), summary_words(96_000), summary_words(1_000_000)), (1125, 9000, 9000));
  // An earlier summary comes first, in the order things happened, and the instructions say to fold it in
  let (p, _) = summary_prompt(&head, Some("earlier"), 1 << 20, 9000);
  assert!(p.starts_with("<previous-summary>\nearlier\n</previous-summary>\n\n<conversation>\n[User]: fix it") && p.contains(&text("compact/previous")), "{p}");
  // A pasted file keeps the message's start and end, so the instruction survives on either side of it
  let pasted = vec![user(&format!("Summarize the attachment.\n<attachment>{}</attachment>\nThen stop.", "hello ".repeat(100_000)))];
  let (p, _) = summary_prompt(&pasted, None, 1 << 20, 9000);
  assert!(p.contains("[User]: Summarize the attachment.") && p.contains("Then stop.") && p.contains("characters of this message left out"), "{}", p.len());
  assert!(p.find("</conversation>").unwrap() < 2 * USER_EDGE_CHARS + 1000);
  // Over the room the oldest part is left out at a line boundary
  let long: Vec<Item> = (0..50).map(|i| user(&format!("message {i} {}", "y".repeat(100)))).collect();
  let (p, cut) = summary_prompt(&long, None, 2000, 9000);
  assert!(cut && p.contains(&text("compact/omitted")) && !p.contains("message 0 ") && p.contains("message 49 "), "{p}");
  assert!(!summary_prompt(&long, None, 1 << 20, 9000).1);
}

#[test]
fn the_policy_comes_from_the_prompts_meta_and_the_window_caps_it() {
  let p = Policy::from_prompt(&json!({ "_meta": { "acpira/compaction": { "auto": false, "atTokens": 400000 } } }));
  assert_eq!(p, Policy { auto: false, at_tokens: Some(400_000) });
  assert_eq!(Policy::from_prompt(&json!({})), Policy::default());
  assert_eq!(p.threshold(1_050_000, Some(128_000)), 400_000);
  assert_eq!(p.threshold(128_000, Some(16_000)), 112_000);
  assert_eq!(Policy::default().threshold(128_000, None), 96_000);
  assert!(is_overflow(&LlmError::Http { status: 400, message: "This model's maximum context length is 131072 tokens".into(), retry_after: None }));
  assert!(is_overflow(&LlmError::Api("prompt is too long: 213462 tokens > 200000 maximum".into())));
  // Seen on the gateway, 2026-10-13
  for m in [
    "Your input exceeds the context window of this model. Please adjust your input and try again.",
    "Ollama upstream rejected the request: The prompt is too long: 1150041, model maximum context length: 1048576",
    "Ollama upstream rejected the request: invalid params, context window exceeds limit",
    "The input token count exceeds the maximum number of tokens allowed (1048576).",
  ] {
    assert!(is_overflow(&LlmError::Http { status: 400, message: m.into(), retry_after: None }), "{m}");
    assert!(is_overflow(&LlmError::Rejected(m.into())), "{m}");
  }
  let vague = LlmError::Rejected("There is an issue with this request, please try a different model.".into());
  assert!(!is_overflow(&vague) && is_refusal(&vague));
  assert!(!is_refusal(&LlmError::Api("upstream overloaded".into())));
  assert!(!is_overflow(&LlmError::Http { status: 500, message: "context".into(), retry_after: None }));
  assert!(!is_overflow(&LlmError::Http { status: 400, message: "invalid tool schema".into(), retry_after: None }));
}

#[test]
fn the_newest_tool_text_stays_whole_and_older_text_is_cut() {
  let big = |id: &str| Item::ToolResult { call_id: id.into(), name: "read".into(), content: format!("{id}{}", "z".repeat(90_000)), is_error: false, images: vec![] };
  let head = vec![user("go"), call("1", "read"), big("1"), call("2", "read"), big("2"), call("3", "read"), big("3")];
  // 200 KB of tool text fits the two newest results whole; the oldest is cut to its first 2000 characters
  let (p, _) = summary_prompt(&head, None, 1 << 22, 9000);
  assert_eq!(p.matches("left out of this excerpt").count(), 1, "{}", p.len());
  let cut = p.find("[Tool result]: 1").unwrap();
  assert!(p[cut..].starts_with(&format!("[Tool result]: 1{}\n[88001 more characters", "z".repeat(1999))));
  assert!(p.contains(&format!("[Tool result]: 3{}\n", "z".repeat(90_000))));
  // A smaller room lowers the budget to a third of it: every result is cut
  let (p, _) = summary_prompt(&head, None, 240_000, 9000);
  assert_eq!(p.matches("left out of this excerpt").count(), 3);
}

#[test]
fn the_reply_gives_its_summary_block_without_the_analysis() {
  assert_eq!(summary_text("<analysis>\nthinking\n</analysis>\n\n<summary>\n1. Primary Request\n</summary>"), "1. Primary Request");
  // Without tags the text is the summary; an analysis alone (cut off inside it) gives nothing
  assert_eq!(summary_text("## Objective\n- x"), "## Objective\n- x");
  assert_eq!(summary_text("<analysis>\nstill thinking"), "");
  assert_eq!(summary_text("<analysis>a</analysis>\nplain summary"), "plain summary");
  assert_eq!(summary_text("<summary>\nopen to the end"), "open to the end");
  // Tags quoted inside the summary or the analysis do not end them
  let quoted = "<analysis>\nthe parser looks for `</analysis>` and <summary>\n</analysis>\n<summary>\n1. Code:\n<details><summary>Logs</summary>\nx\n</details>\n7. Pending Tasks: y\n</summary>";
  assert_eq!(summary_text(quoted), "1. Code:\n<details><summary>Logs</summary>\nx\n</details>\n7. Pending Tasks: y");
  assert_eq!(summary_text("<summary>a <summary>b</summary> c</summary>"), "a <summary>b</summary> c");
}

#[test]
fn the_users_messages_are_carried_newest_first_within_the_budget() {
  let steer = crate::steer::item("Note.", &[json!([{ "type": "text", "text": "use printf" }])], true, &[]);
  let items = vec![user("<mode>\nPlan mode is off.\n</mode>\n\nfix the build"), call("1", "bash"), result("1", "bash", 10), steer, user("now the tests")];
  let (m, omitted) = carried_messages(&["first ask".to_owned()], 0, &items, 1000);
  // The agent's notes are left out, a steered message counts, the earlier carried ones come first
  assert_eq!((m, omitted), (vec!["first ask".to_owned(), "fix the build".to_owned(), "use printf".to_owned(), "now the tests".to_owned()], 0));
  // Over the budget the oldest go, and the count of left-out ones adds up across compactions
  let (m, omitted) = carried_messages(&["first ask".to_owned()], 2, &items, 8);
  assert_eq!((m, omitted), (vec!["use printf".to_owned(), "now the tests".to_owned()], 4));
  // A long message keeps its start and end
  let (m, _) = carried_messages(&[], 0, &[user(&format!("start {} end", "p".repeat(20_000)))], 1 << 20);
  assert!(m[0].starts_with("start ") && m[0].ends_with(" end") && m[0].len() < 2 * CARRIED_EDGE_CHARS + 100);
}

#[test]
fn the_files_touched_last_are_restored_from_disk() {
  let dir = tempfile::tempdir().unwrap();
  let cwd = dir.path();
  std::fs::write(cwd.join("a.rs"), "fn a() {}\n").unwrap();
  std::fs::write(cwd.join("b.rs"), "fn b() {}\n").unwrap();
  std::fs::write(cwd.join("big.txt"), "line\n".repeat(10_000)).unwrap();
  std::fs::write(cwd.join("bin.dat"), [0u8, 1, 2]).unwrap();
  let touch = |id: &str, name: &str, args: Value| Item::Assistant {
    text: String::new(),
    reasoning: String::new(),
    tool_calls: vec![ToolCall { id: id.into(), name: name.into(), arguments: args.to_string() }],
    native: None,
  };
  let done = |id: &str, is_error: bool| Item::ToolResult { call_id: id.into(), name: "read".into(), content: "ok".into(), is_error, images: vec![] };
  std::fs::write(cwd.join("secret.env"), "TOKEN=1\n").unwrap();
  let head = vec![
    user("go"),
    touch("1", "read", json!({ "path": "a.rs", "paths": ["big.txt"] })),
    done("1", false),
    touch("2", "edit", json!({ "path": "b.rs", "old_string": "x", "new_string": "y" })),
    done("2", false),
    touch("3", "read", json!({ "path": "bin.dat" })),
    done("3", false),
    touch("4", "read", json!({ "path": "gone.rs" })),
    done("4", true),
    touch("5", "bash", json!({ "command": "cat c.rs" })),
    done("5", false),
    // Refused on its permission card: never restored
    touch("7", "read", json!({ "path": "secret.env" })),
    done("7", true),
  ];
  // Newest first; binary and missing files are skipped; a file the tail touches is not restored
  let files = restore_files(&head, &[touch("6", "write", json!({ "path": "a.rs", "content": "" }))], &[], cwd, 25_000, &|p| std::fs::read(p).ok());
  assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["b.rs", "big.txt"]);
  // The files restored before follow the ones touched since, once each
  let again = restore_files(&head[..5], &[], &["b.rs".to_owned(), "c.rs".to_owned(), "gone.rs".to_owned()], cwd, 25_000, &|p| std::fs::read(p).ok());
  std::fs::write(cwd.join("c.rs"), "fn c() {}\n").unwrap();
  assert_eq!(again.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["b.rs", "big.txt", "a.rs"]);
  let again = restore_files(&head[..5], &[], &["c.rs".to_owned()], cwd, 25_000, &|p| std::fs::read(p).ok());
  assert_eq!(again.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["b.rs", "big.txt", "a.rs", "c.rs"]);
  assert_eq!(files[0].text, "fn b() {}\n");
  // A long file is cut at a line, within its 5k tokens, and says where to read on
  assert!(files[1].text.len() <= 20_000 + 100 && files[1].text.ends_with("[6000 more lines not shown; read the file from line 4001 for the rest]"), "{}", &files[1].text[19_900..]);
  // Too small a budget restores nothing
  assert!(restore_files(&head, &[], &[], cwd, 400, &|p| std::fs::read(p).ok()).is_empty());
  // A file read again and again ranks above one read once since
  let ranked = vec![
    touch("8", "read", json!({ "path": "a.rs" })),
    done("8", false),
    touch("9", "read", json!({ "path": "a.rs", "paths": ["a.rs:1-1"] })),
    done("9", false),
    touch("10", "read", json!({ "path": "c.rs" })),
    done("10", false),
  ];
  let files = restore_files(&ranked, &[], &[], cwd, 25_000, &|p| std::fs::read(p).ok());
  assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.rs", "c.rs"]);
}

#[test]
fn the_last_to_do_list_is_carried_unless_the_tail_has_a_later_one() {
  let set = |id: &str, name: &str, args: Value| Item::Assistant {
    text: String::new(),
    reasoning: String::new(),
    tool_calls: vec![ToolCall { id: id.into(), name: name.into(), arguments: args.to_string() }],
    native: None,
  };
  let list = |content: &str| json!([{ "content": content, "status": "in_progress" }]);
  let head = vec![
    user("go"),
    set("1", "todo", json!({ "todos": list("old") })),
    result("1", "todo", 10),
    set("2", "todo", json!({ "todos": list("new") })),
    // A prune cleared the result; the call still says what the list became
    Item::ToolResult { call_id: "2".into(), name: "todo".into(), content: PRUNED.into(), is_error: false, images: vec![] },
  ];
  assert_eq!(last_todo(&head, &[], Some("carried")).as_deref(), Some("To-do list updated (0 pending, 1 in progress, 0 completed):\n[in_progress] new\n"));
  assert_eq!(last_todo(&head, &[set("3", "todo", json!({ "todos": list("tail") }))], Some("carried")), None);
  // A list another call carried counts the same; one an MCP tool or a refused todo call names does not
  let rode = [set("4", "edit", json!({ "path": "a.rs", "todos": list("rode") })), set("5", "notes_add", json!({ "todos": list("mcp") })), set("6", "todo", json!({ "todos": [{ "content": "x", "status": "later" }] }))];
  let head: Vec<Item> = head.into_iter().chain(rode).collect();
  assert!(last_todo(&head, &[], None).is_some_and(|t| t.ends_with("[in_progress] rode\n")), "{:?}", last_todo(&head, &[], None));
  assert_eq!(last_todo(&head, &[set("7", "bash", json!({ "command": "ls", "todos": list("t") }))], None), None);
  assert_eq!(last_todo(&[user("go")], &[], None), None);
  // Nothing newer: the list carried before goes on
  assert_eq!(last_todo(&[user("go")], &[], Some("carried")).as_deref(), Some("carried"));
}

#[test]
fn the_summary_item_carries_everything_and_reads_back_from_its_event() {
  let carry = Carry {
    messages: vec!["fix the build".into(), "不对，用 pnpm".into()],
    omitted: 3,
    todo: Some("[in_progress] build".into()),
    files: vec![RestoredFile { path: "src/a.rs".into(), text: "fn a() {}\n".into() }],
    request: Some("fix the build".into()),
  };
  let Item::User(parts) = summary_item("1. Primary Request and Intent:\n   build", &carry) else { panic!() };
  let Part::Text(text) = &parts[0] else { panic!() };
  assert!(text.starts_with("<conversation-summary>\n1. Primary Request and Intent:\n   build\n</conversation-summary>\n\nThis session is being continued"));
  assert!(text.contains("oldest first (3 earlier ones are left out; the summary covers them):\n<user-messages>\n<message>\nfix the build\n</message>\n<message>\n不对，用 pnpm\n</message>\n</user-messages>"), "{text}");
  assert!(text.contains("<todo>\n[in_progress] build\n</todo>") && text.contains("<file path=\"src/a.rs\">\nfn a() {}\n</file>") && text.contains("<request>\nfix the build\n</request>"));
  assert!(text.ends_with("Pick up the last task as if the break never happened."));
  assert_eq!(previous_summary(text), "1. Primary Request and Intent:\n   build");
  let mut ev = json!({ "type": "compaction", "summary": "s" });
  carry.log_into(&mut ev);
  assert_eq!(Carry::from_event(&ev), carry);
  // An event from before these fields carries its request only
  assert_eq!(Carry::from_event(&json!({ "request": "r" })), Carry { request: Some("r".into()), ..Default::default() });
}
