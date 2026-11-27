//! Tool identity: web search, MCP calls, the host's own tools, skills, reused call ids, wakeups and mixed reads

use super::*;

#[test]
fn claude_web_search_reads_as_a_web_search_with_its_query() {
  let mut s = state();
  // claude-agent-acp 0.83 WebSearchReporter: the stream start carries no input, the refinement the query
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "ws", "kind": "fetch", "title": "Web search",
    "status": "pending", "rawInput": {} }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.search", "kind": "fetch" }));
  expect_absent(block(&s, 0, 0), "target");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "ws", "kind": "fetch",
    "title": "Search \"telegram storage\"", "rawInput": { "query": "telegram storage" } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.search", "kind": "fetch", "target": "telegram storage", "targetMono": false }));
  // WebFetch stays a fetch of its URL
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "wf", "kind": "fetch", "title": "Fetch https://a.dev",
    "rawInput": { "url": "https://a.dev", "prompt": "summarize" } }));
  expect_match(block(&s, 0, 1), json!({ "kind": "fetch", "target": "https://a.dev", "targetMono": true }));
  expect_absent(block(&s, 0, 1), "verbKey");
}

#[test]
fn codex_web_search_names_its_query_and_an_opened_page_its_url() {
  let mut s = state();
  // codex-acp 1.13 createWebSearchStartUpdate / createWebSearchCompleteUpdate
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "s1", "kind": "search", "title": "Web search",
    "status": "in_progress", "rawInput": { "type": "webSearch", "id": "s1", "query": "" } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.search", "kind": "fetch" }));
  expect_absent(block(&s, 0, 0), "target");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "s1", "title": "Web search: acp spec", "status": "completed",
    "rawInput": { "type": "webSearch", "id": "s1", "query": "acp", "action": { "type": "search", "query": "acp spec" } } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.search", "kind": "fetch", "target": "acp spec", "status": "completed" }));
  // The action may only arrive with the completion
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "p1", "kind": "search", "title": "Web search",
    "status": "in_progress", "rawInput": { "type": "webSearch", "id": "p1", "query": "" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "p1", "title": "Open page: https://a.dev/x", "status": "completed",
    "rawInput": { "type": "webSearch", "id": "p1", "query": "", "action": { "type": "openPage", "url": "https://a.dev/x" } } }));
  expect_match(block(&s, 0, 1), json!({ "kind": "fetch", "target": "https://a.dev/x", "targetMono": true }));
  expect_absent(block(&s, 0, 1), "verbKey");
}

#[test]
fn mcp_calls_name_their_server_and_take_the_tools_title_whatever_the_adapter_naming() {
  let mut s = state();
  // The built-in agent: `_meta["acpira/mcp"]` with the configured server name and the tool's own title
  let meta = json!({ "acpira/mcp": { "server": "blockbench", "tool": "screenshot", "title": "Take Screenshot" } });
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "a1", "title": "blockbench_screenshot", "kind": "other", "status": "pending", "_meta": meta }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "a1", "title": "blockbench_screenshot", "rawInput": { "path": "/tmp/x" }, "_meta": meta }));
  expect_match(block(&s, 0, 0), json!({ "mcpServer": "blockbench", "kind": "other", "target": "Take Screenshot", "targetMono": false }));
  // A later update without the note keeps the row as it is
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "a1", "status": "completed", "rawInput": { "command": "ls" } }));
  expect_match(block(&s, 0, 0), json!({ "mcpServer": "blockbench", "target": "Take Screenshot", "status": "completed" }));
  // claude-agent-acp: `mcp__<server>__<tool>`; without a title the tool's name stands in, its separators as spaces
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "name": "mcp__chrome-devtools__take_screenshot", "title": "mcp__chrome-devtools__take_screenshot",
    "kind": "other", "status": "pending", "rawInput": { "format": "png" } }));
  expect_match(block(&s, 0, 1), json!({ "mcpServer": "chrome-devtools", "kind": "other", "target": "take screenshot" }));
  // codex-acp: `mcp.<server>.<tool>`, filed as `execute`, in every update
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "x1", "title": "mcp.hilfa.query_npc_lore", "kind": "execute", "status": "in_progress",
    "rawInput": { "server": "hilfa", "tool": "query_npc_lore", "arguments": {} } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "x1", "kind": "execute", "status": "completed" }));
  expect_match(block(&s, 0, 2), json!({ "mcpServer": "hilfa", "kind": "other", "target": "query npc lore", "status": "completed" }));
  // The engine's own tools keep their rows: show_image is an image row, ask_agent stands behind its agent's node
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "s1", "title": "mcp__acpira__show_image", "status": "pending", "rawInput": { "path": "/tmp/a.png" } }));
  expect_match(block(&s, 0, 3), json!({ "verbKey": "verb.showImage" }));
  expect_absent(block(&s, 0, 3), "mcpServer");
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "k1", "title": "mcp__acpira__ask_agent", "status": "pending", "rawInput": { "agent": "codex" } }));
  expect_absent(block(&s, 0, 4), "mcpServer");
  // An ordinary tool that merely mentions mcp is no MCP call
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "o1", "title": "Read mcp.json", "kind": "read", "status": "pending" }));
  expect_absent(block(&s, 0, 5), "mcpServer");
  // A title opening with its server's name drops it: the verb already reads "Call acpira"
  let meta = json!({ "acpira/mcp": { "server": "acpira", "tool": "plugin_guide", "title": "Acpira plugin guide" } });
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "g1", "title": "acpira_plugin_guide", "kind": "other", "status": "pending", "_meta": meta }));
  expect_match(block(&s, 0, 6), json!({ "mcpServer": "acpira", "target": "plugin guide" }));
  // agy acp 1.5.9: `mcp_<server>_<tool>` in `name`, the title `Run <name>`, in every update; the result never arrives
  let agy = |update: &str, status: &str| {
    json!({ "sessionUpdate": update, "status": status, "toolCallId": "y1", "name": "mcp_acpira-probe_acpira_probe_ping",
      "title": "Run mcp_acpira-probe_acpira_probe_ping", "kind": "other", "rawInput": { "toolSummary": "Probe ping check" } })
  };
  apply(&mut s, agy("tool_call", "in_progress"));
  apply(&mut s, agy("tool_call_update", "completed"));
  expect_match(block(&s, 0, 7), json!({ "mcpServer": "acpira-probe", "kind": "other", "target": "ping", "status": "completed" }));
  // The same pair on the engine's own tools: show_image stays an image row
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "y2", "name": "mcp_acpira_show_image", "title": "Run mcp_acpira_show_image", "kind": "other",
    "status": "in_progress", "rawInput": { "path": "/tmp/a.png", "toolSummary": "Show the shot" } }));
  expect_match(block(&s, 0, 8), json!({ "verbKey": "verb.showImage" }));
  expect_absent(block(&s, 0, 8), "mcpServer");
  // A command that merely starts with mcp_ is no MCP call
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "y3", "name": "run_command", "title": "mcp_tool_check --all", "kind": "execute", "status": "pending" }));
  expect_absent(block(&s, 0, 9), "mcpServer");
  // agy acp 1.5.13: the server of a second session request in one process is registered as `<server>_<8 hex>`
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "y4", "name": "mcp_blockbench_9defe6f0_get_project_state",
    "title": "Run mcp_blockbench_9defe6f0_get_project_state", "kind": "other", "status": "in_progress" }));
  expect_match(block(&s, 0, 10), json!({ "mcpServer": "blockbench", "target": "get project state" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "y5", "name": "mcp_acpira_0db272fb_show_image", "title": "Run mcp_acpira_0db272fb_show_image",
    "kind": "other", "status": "in_progress", "rawInput": { "path": "/tmp/a.png" } }));
  expect_match(block(&s, 0, 11), json!({ "verbKey": "verb.showImage" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "y6", "name": "mcp_acpira_b3cf0d51_read_session", "title": "Run mcp_acpira_b3cf0d51_read_session",
    "kind": "other", "status": "in_progress", "rawInput": { "session": "acpira://session/68e65628-76be-4748-ae17-8d1d7ea1ffb1" } }));
  expect_match(block(&s, 0, 12), json!({ "verbKey": "verb.readSession" }));
  expect_absent(block(&s, 0, 12), "mcpServer");
}

#[test]
fn the_host_show_image_tool_becomes_an_image_row_for_claude_and_codex_wire_shapes() {
  let mut s = state();
  s.ctx.save_image_file = Some(Arc::new(|p: &str| Some(format!("b-{}", p.rsplit('/').next().unwrap()))));
  // claude-agent-acp 0.83: `name` mcp__acpira__show_image, the arguments as rawInput, no image until completion
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "name": "mcp__acpira__show_image", "title": "mcp__acpira__show_image",
    "kind": "other", "status": "pending", "rawInput": { "path": "/tmp/a.png", "caption": "Account menu" } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.showImage", "kind": "other", "target": "Account menu", "locations": [{ "path": "/tmp/a.png" }] }));
  expect_absent(block(&s, 0, 0), "content");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "Shown to the user: a.png." } }] }));
  expect_match(block(&s, 0, 0), json!({ "contents": [
    { "type": "image", "blob": "b-a.png", "uri": "/tmp/a.png" }, { "type": "text", "text": "Shown to the user: a.png." },
  ] }));
  // codex-acp 1.13: `mcp.acpira.show_image`, kind execute, rawInput { server, tool, arguments }
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "x1", "title": "mcp.acpira.show_image", "kind": "execute", "status": "in_progress",
    "rawInput": { "server": "acpira", "tool": "show_image", "arguments": { "paths": ["/tmp/b.png", "/tmp/c.jpg"] } } }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.showImage", "kind": "other", "target": "b.png, c.jpg" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "x1", "status": "completed" }));
  expect_match(block(&s, 0, 1), json!({ "contents": [{ "blob": "b-b.png" }, { "blob": "b-c.jpg", "mimeType": "image/jpeg" }] }));
  // A failed call (the server refused the path) shows no image
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "f1", "title": "acpira_show_image", "status": "failed", "rawInput": { "path": "/tmp/d.png" } }));
  expect_match(block(&s, 0, 2), json!({ "verbKey": "verb.showImage", "status": "failed" }));
  expect_absent(block(&s, 0, 2), "content");
}

#[test]
fn a_shown_file_with_no_saved_copy_keeps_its_path_through_a_repeated_completion() {
  // A session on an SSH host saves nothing at show time: the row names the host file and the page reads it from there
  let mut s = state();
  s.ctx.save_image_file = Some(Arc::new(|_: &str| None));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "mcp__acpira__show_image", "kind": "other", "status": "pending",
    "rawInput": { "path": "/srv/app/shot.png" } }));
  let done = json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "Shown to the user: shot.png." } }] });
  let shown = json!({ "contents": [{ "type": "image", "mimeType": "image/png", "uri": "/srv/app/shot.png" }, { "type": "text" }] });
  apply(&mut s, done.clone());
  expect_match(block(&s, 0, 0), shown.clone());
  // The adapter repeats the result: the receipt replaces the contents, and the path comes back ahead of it
  apply(&mut s, done);
  expect_match(block(&s, 0, 0), shown);
  expect_absent(&block(&s, 0, 0)["contents"][0], "blob");
}

#[test]
fn show_video_survives_adapter_shapes_and_titleless_completion_without_becoming_an_image() {
  for (title, nested) in [("mcp__acpira__show_video", false), ("mcp.acpira.show_video", true), ("acpira_show_video", false), ("show_video", false)] {
    let mut s = state();
    s.ctx.save_image_file = Some(Arc::new(|_: &str| Some("saved.mp4".into())));
    let args = json!({ "path": "/tmp/recording.mp4", "caption": "Panel interaction" });
    let raw = if nested { json!({ "server": "acpira", "tool": "show_video", "arguments": args }) } else { args };
    apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": title, "kind": "execute", "status": "pending", "rawInput": raw }));
    expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.showVideo", "kind": "other", "target": "Panel interaction" }));
    expect_absent(block(&s, 0, 0), "content");
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "v", "status": "completed", "kind": "execute", "locations": [],
      "content": [{ "type": "content", "content": { "type": "text", "text": "Shown." } }] }));
    expect_match(block(&s, 0, 0), json!({ "kind": "other", "contents": [
      { "type": "image", "blob": "saved.mp4", "mimeType": "video/mp4", "uri": "/tmp/recording.mp4" }, { "type": "text", "text": "Shown." },
    ] }));
    // Unknown verbs are plain strings and the content shape remains readable by older engines.
    let row: acpira_shared::transcript::ToolCallBlock = serde_json::from_value(block(&s, 0, 0).clone()).unwrap();
    assert_eq!(row.verb_key.as_deref(), Some("verb.showVideo"));
    apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "failed", "title": title, "status": "failed", "rawInput": { "path": "/tmp/no.mp4" } }));
    expect_absent(block(&s, 0, 1), "content");
  }
}

#[test]
fn the_host_show_widget_tool_reads_its_html_into_the_row_in_every_adapter_naming() {
  let mut s = state();
  s.ctx.save_image_file = Some(Arc::new(|p: &str| Some(format!("b-{}", p.rsplit('/').next().unwrap()))));
  // claude-agent-acp: mcp__acpira__show_widget, the title names the row; the file is read only once the call completes
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "name": "mcp__acpira__show_widget", "title": "mcp__acpira__show_widget",
    "kind": "other", "status": "pending", "rawInput": { "path": "/tmp/tuner.html", "title": "Latency tuner" } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.showWidget", "kind": "other", "target": "Latency tuner", "locations": [{ "path": "/tmp/tuner.html" }] }));
  expect_absent(block(&s, 0, 0), "mcpServer");
  expect_absent(block(&s, 0, 0), "content");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "Shown to the user: tuner.html." } }] }));
  // Stored as an image content of type text/html, which engines that predate widgets still read
  expect_match(block(&s, 0, 0), json!({ "contents": [
    { "type": "image", "blob": "b-tuner.html", "mimeType": "text/html", "uri": "/tmp/tuner.html" }, { "type": "text", "text": "Shown to the user: tuner.html." },
  ] }));
  // codex-acp: mcp.acpira.show_widget filed as execute with nested arguments; without a title the file names the row
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "x1", "title": "mcp.acpira.show_widget", "kind": "execute", "status": "in_progress",
    "rawInput": { "server": "acpira", "tool": "show_widget", "arguments": { "path": "/tmp/w/plan.htm" } } }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.showWidget", "kind": "other", "target": "plan.htm", "targetMono": true }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "x1", "status": "completed", "kind": "execute" }));
  expect_match(block(&s, 0, 1), json!({ "kind": "other", "content": { "type": "image", "blob": "b-plan.htm", "mimeType": "text/html" } }));
  // OpenCode's acpira_show_widget; a failed call (the server refused the file) reads nothing
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "f1", "title": "acpira_show_widget", "status": "failed", "rawInput": { "path": "/tmp/x.html" } }));
  expect_match(block(&s, 0, 2), json!({ "verbKey": "verb.showWidget", "status": "failed" }));
  expect_absent(block(&s, 0, 2), "content");
  // A file that is not HTML is never read, whatever the call claimed
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "n1", "title": "show_widget", "status": "completed", "rawInput": { "path": "/tmp/notes.txt" } }));
  expect_match(block(&s, 0, 3), json!({ "verbKey": "verb.showWidget" }));
  expect_absent(block(&s, 0, 3), "content");
}

#[test]
fn a_skill_load_is_one_row_naming_the_skill_whichever_agent_ran_it() {
  let mut s = state();
  // claude-agent-acp: "Load skill" before the input is known, then the name; the receipt is for the model
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Load skill", "kind": "other", "status": "pending",
    "rawInput": {}, "_meta": { "claudeCode": { "toolName": "Skill" } } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.skill", "kind": "other" }));
  expect_absent(block(&s, 0, 0), "target");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "title": "Load skill: lab-shot", "rawInput": { "skill": "lab-shot" },
    "_meta": { "claudeCode": { "toolName": "Skill" }, "jetbrains": { "air": { "version": 1, "skill": { "name": "lab-shot", "path": "/repo/.claude/skills/lab-shot/SKILL.md" } } } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "Launching skill: lab-shot" } }] }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.skill", "kind": "other", "target": "lab-shot", "targetMono": false,
    "locations": [{ "path": "/repo/.claude/skills/lab-shot/SKILL.md" }], "status": "completed" }));
  expect_absent(block(&s, 0, 0), "content");
  // The built-in agent's `skill`: a read of the SKILL.md that answers with the instructions
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "b1", "title": "Skill ui-pick", "kind": "read", "status": "completed",
    "rawInput": { "name": "ui-pick" }, "locations": [{ "path": "/repo/.agents/skills/ui-pick/SKILL.md" }],
    "content": [{ "type": "content", "content": { "type": "text", "text": "# UI pick\n..." } }] }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.skill", "kind": "other", "target": "ui-pick", "locations": [{ "path": "/repo/.agents/skills/ui-pick/SKILL.md" }] }));
  expect_absent(block(&s, 0, 1), "content");
  // Codex: a command that only prints the SKILL.md, its output streamed through _meta; a later packet repeating the kind
  // and command keeps the skill row
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "x1", "title": "sed -n '1,200p' /home/u/.agents/skills/dig/SKILL.md", "kind": "execute",
    "status": "in_progress", "rawInput": { "command": "sed -n '1,200p' /home/u/.agents/skills/dig/SKILL.md" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "x1", "kind": "execute", "status": "completed",
    "_meta": { "terminal_output_delta": { "data": "---\nname: dig\n---\n" } } }));
  expect_match(block(&s, 0, 2), json!({ "verbKey": "verb.skill", "kind": "other", "target": "dig", "targetMono": false,
    "locations": [{ "path": "/home/u/.agents/skills/dig/SKILL.md" }] }));
  expect_absent(block(&s, 0, 2), "content");
  // A failed load keeps its message
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "f1", "title": "Skill nope", "kind": "read", "status": "failed", "rawInput": { "name": "nope" },
    "content": [{ "type": "content", "content": { "type": "text", "text": "Unknown skill \"nope\"." } }] }));
  expect_match(block(&s, 0, 3), json!({ "verbKey": "verb.skill", "target": "nope", "content": { "type": "text", "text": "Unknown skill \"nope\"." } }));
  // Anything more than printing the one file stays what it is
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "g1", "title": "grep -n name /home/u/.agents/skills/dig/SKILL.md", "kind": "execute",
    "status": "completed", "rawInput": { "command": "grep -n name /home/u/.agents/skills/dig/SKILL.md" } }));
  expect_absent(block(&s, 0, 4), "verbKey");
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "r1", "title": "Read README.md", "kind": "read", "status": "completed",
    "locations": [{ "path": "/repo/.agents/skills/dig/README.md" }] }));
  expect_absent(block(&s, 0, 5), "verbKey");
}

#[test]
fn the_host_session_tools_name_what_they_read_in_every_adapter_naming() {
  let mut s = state();
  // claude-agent-acp: mcp__acpira__read_session with the arguments as rawInput; a vscode link is stored canonical
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "r1", "name": "mcp__acpira__read_session", "title": "mcp__acpira__read_session",
    "kind": "other", "status": "pending", "rawInput": { "session": "vscode://hotic.acpira/session/abc", "from": 3, "to": 5 } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.readSession", "kind": "other", "target": "acpira://session/abc 3-5" }));
  expect_absent(block(&s, 0, 0), "mcpServer");
  // codex-acp: mcp.acpira.search_sessions, kind execute, rawInput { server, tool, arguments }
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "s1", "title": "mcp.acpira.search_sessions", "kind": "execute", "status": "in_progress",
    "rawInput": { "server": "acpira", "tool": "search_sessions", "arguments": { "query": "lexer speed", "scope": "all" } } }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.searchSessions", "kind": "other", "target": "lexer speed" }));
  // OpenCode: acpira_search_sessions; inside one session the session leads the target, an outline read has no range
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "s2", "title": "acpira_search_sessions", "status": "pending",
    "rawInput": { "session": "abc", "query": "parser" } }));
  expect_match(block(&s, 0, 2), json!({ "verbKey": "verb.searchSessions", "target": "acpira://session/abc parser" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "r2", "title": "read_session", "status": "pending", "rawInput": { "session": "019a-native", "from": "2" } }));
  expect_match(block(&s, 0, 3), json!({ "verbKey": "verb.readSession", "target": "acpira://session/019a-native 2-" }));
  // A later packet without the title keeps the identity
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "r1", "status": "completed", "kind": "execute",
    "content": [{ "type": "content", "content": { "type": "text", "text": "# Title" } }] }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.readSession", "kind": "other", "target": "acpira://session/abc 3-5" }));
}

// A historical edit continues on a fresh agent session, and the built-in agent numbers its calls per session: its
// `call-1` arrives again while an earlier turn still holds the finished `call-1` (acpira 1.10.7, 2026-10-22)
#[test]
fn a_reused_tool_call_id_opens_a_new_row_instead_of_rewriting_an_earlier_turn() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "Read", "kind": "read", "status": "pending", "locations": [{ "path": "/repo/old.ts" }] }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed" }));
  end_turn(&mut s, TurnStop::EndTurn);
  let before = block(&s, 0, 0);
  s.turns.push(turn(json!({ "role": "user", "text": "edited" })));
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 2000 })));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "work_Edit", "kind": "other", "status": "pending" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "rawInput": { "file_path": "/repo/new.ts" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "failed" }));
  // A re-announcement inside the live turn still merges into that turn's row
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "work_Edit", "kind": "other", "status": "failed" }));
  expect_eq(block(&s, 0, 0), before);
  let rows = s.turns[2].as_agent().unwrap().blocks.iter().filter(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.id == "call-1")).count();
  assert_eq!(rows, 1);
  expect_match(block(&s, 2, 0), json!({ "type": "tool_call", "id": "call-1", "status": "failed" }));
}

#[test]
fn a_scheduled_wakeup_names_the_time_the_session_resumes() {
  let mut s = state();
  // claude-agent-acp 0.84.0: the generic row under the tool's name; the result's instructions are for the model
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "w1", "title": "ScheduleWakeup", "kind": "other", "status": "pending",
    "rawInput": { "delaySeconds": 1800, "prompt": "go on", "reason": "waiting on agents" }, "_meta": { "claudeCode": { "toolName": "ScheduleWakeup" } } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.wakeup", "kind": "other" }));
  expect_absent(block(&s, 0, 0), "target");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "w1", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "Next wakeup scheduled for 02:22:00 (in 1860s). If you owe the user a status update…" } }] }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.wakeup", "target": "02:22", "targetMono": false, "status": "completed" }));
  expect_absent(block(&s, 0, 0), "content");
  // `stop: true` ends the wakeups and names no time
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "w2", "title": "ScheduleWakeup", "kind": "other", "status": "completed",
    "rawInput": { "stop": true }, "_meta": { "claudeCode": { "toolName": "ScheduleWakeup" } },
    "content": [{ "type": "content", "content": { "type": "text", "text": "Loop stopped" } }] }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.wakeupStop" }));
  expect_absent(block(&s, 0, 1), "target");
}

#[test]
fn a_mixed_read_attaches_image_locations_to_uri_less_image_results() {
  let mut s = state();
  s.ctx.save_image = saver(|_, _| Some("blob-a.png".into()));
  apply(&mut s, json!({
    "sessionUpdate": "tool_call",
    "toolCallId": "r-mix",
    "kind": "read",
    "title": "Read marks.tsx",
    "status": "in_progress",
    "locations": [
      { "path": "/repo/marks.tsx", "line": 180 },
      { "path": "/repo/SidekickRelay.tsx" },
      { "path": "/repo/.acpira/tmp/a.png" },
      { "path": "/repo/.acpira/tmp/b.png" }
    ]
  }));
  apply(&mut s, json!({
    "sessionUpdate": "tool_call_update",
    "toolCallId": "r-mix",
    "status": "completed",
    "content": [
      { "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } },
      { "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }
    ]
  }));
  expect_match(block(&s, 0, 0), json!({
    "contents": [
      { "type": "image", "blob": "blob-a.png", "uri": "/repo/.acpira/tmp/a.png" },
      { "type": "image", "blob": "blob-a.png", "uri": "/repo/.acpira/tmp/b.png" }
    ]
  }));
}
