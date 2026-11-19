#![cfg(unix)]

//! Stand-in CLIs in a folder on an SSH host under local run (`acp/local_run`): the fake agent plays claude-agent-acp
//! (`claude-remote-call:`), `agy acp` (`agy-remote-call:`) or any CLI a custom agent declares `localRun` for, the lab's
//! fake ssh plays the host, and Acpira's own MCP server carries the project tools to it through a live hub

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_host::acp::session::AcpSession;
use acpira_host::relay::hub::RelayHub;
use acpira_host::relay::roster::Roster;
use acpira_shared::remote::ConnectionOp;

use crate::acp_session::{agent_blocks, find_block, spawn_prompt, view};
use crate::remote_connections::Lab;
use crate::support::{Disposing, FakeAgent, Harness, expect_match};

const VIRT: &str = "/acpira-virtual-host";

struct Remote {
  h: Harness,
  _lab: Lab,
  /// The host's file system root for VIRT
  real: PathBuf,
  /// Holds the fake agent's request log
  logs: tempfile::TempDir,
  connections: Arc<acpira_host::remote::connections::Connections>,
}

impl Drop for Remote {
  fn drop(&mut self) {
    self.connections.dispose();
  }
}

impl Remote {
  async fn new(fake: &FakeAgent) -> Remote {
    Self::for_agent(fake, "claude").await
  }

  async fn for_agent(fake: &FakeAgent, agent: &str) -> Remote {
    Self::with(fake, agent, json!({})).await
  }

  /// `extra`: more of the agent's agents.json entry
  async fn with(fake: &FakeAgent, agent: &str, extra: Value) -> Remote {
    let logs = tempfile::tempdir().unwrap();
    let mut setting = json!({ "env": { "FAKE_META_LOG": logs.path().join("meta.log") } });
    if let (Some(s), Some(e)) = (setting.as_object_mut(), extra.as_object()) {
      s.extend(e.clone());
    }
    let mut h = Harness::for_agent(fake, agent, setting);
    let lab = Lab::new();
    let real = lab.path().join("hostfs");
    std::fs::create_dir_all(real.join("work")).unwrap();
    lab.map_root(VIRT, &real);
    let connections = lab.engine("engine");
    connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
    connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();
    h.deps.remote = Some(connections.clone());
    let roster = Arc::new(Roster::new(h.dir.path()));
    let hub = RelayHub::start(roster, Arc::new(|_: &str| {})).await.unwrap();
    h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub));
    Remote { h, _lab: lab, real, logs, connections }
  }

  async fn session(&self) -> Disposing {
    let s = Disposing(self.h.session(&format!("ssh://gen1{VIRT}/work")));
    s.start().await;
    s
  }

  /// The session requests the fake agent received: `{ method, meta, cwd, systemPrompt }`
  fn requests(&self) -> Vec<Value> {
    let text = std::fs::read_to_string(self.logs.path().join("meta.log")).unwrap_or_default();
    text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
  }

  /// The stand-in the session runs in: under the engine's data root, which is where its session store lives
  fn stand_in(&self) -> PathBuf {
    self.stand_in_under("remote/hosts")
  }

  fn stand_in_under(&self, root: &str) -> PathBuf {
    self.h.dir.path().join(root).join("gen1").join(VIRT.trim_start_matches('/')).join("work")
  }
}

/// One turn, answering every permission card with `option` (an allow_once or reject_once kind); returns the cards and
/// the reply's JSON
async fn turn(s: &Arc<AcpSession>, text: &str, option: &str) -> (Vec<Value>, Value) {
  let task = spawn_prompt(s, text);
  let (mut cards, mut answered) = (vec![], HashSet::new());
  let t0 = Instant::now();
  while !task.is_finished() {
    assert!(t0.elapsed() < Duration::from_secs(30), "the turn did not end: {}", view(s));
    for card in agent_blocks(&view(s)).into_iter().filter(|b| b["type"] == "permission" && b["resolved"].is_null()) {
      if !answered.insert(card["id"].to_string()) {
        continue;
      }
      let id = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == option).expect("the option")["id"].clone();
      s.resolve_permission(card["id"].as_str().unwrap(), id.as_str().unwrap());
      cards.push(card);
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  task.await.unwrap();
  let vw = view(s);
  let reply = agent_blocks(&vw).into_iter().rev().find(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap_or("").to_owned()).unwrap_or_default();
  (cards, serde_json::from_str(&reply).unwrap_or(Value::String(reply)))
}

fn result_text(reply: &Value) -> String {
  reply["result"]["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

/// The session runs in its stand-in with Claude's own tools off and the host's instructions in its system prompt
#[tokio::test(flavor = "multi_thread")]
async fn claude_code_opens_in_a_stand_in_with_its_own_tools_off() {
  let fake = crate::fake_or_skip!();
  let r = Remote::new(&fake).await;
  std::fs::write(r.real.join("work/CLAUDE.md"), "@AGENTS.md\n").unwrap();
  std::fs::write(r.real.join("work/AGENTS.md"), "Run the tests with cargo nextest.\n").unwrap();
  let s = r.session().await;
  assert!(view(&s)["error"].is_null(), "{} {:?}", view(&s)["error"], r.h.logs());
  let req = r.requests().into_iter().find(|q| q["method"] == "new").expect("a session/new");
  assert_eq!(req["cwd"], json!(r.stand_in()));
  assert!(r.stand_in().is_dir(), "the stand-in exists for the CLI to start in");
  let options = &req["meta"]["claudeCode"]["options"];
  for native in ["Read", "Write", "Edit", "Bash", "Grep", "Glob", "Monitor"] {
    assert!(options["disallowedTools"].as_array().unwrap().iter().any(|t| t == native), "{native} is off: {options}");
  }
  assert_eq!(options["toolAliases"]["Bash"], "mcp__acpira__Bash");
  assert!(req["meta"].get("acpira/remote").is_none(), "the built-in agent's host meta is not Claude's");
  // The system prompt section itself is checked in `local_run::brief`'s unit tests: the fake's ACP SDK drops the field.
  // CLAUDE.md is read, with its `@AGENTS.md` line as that file's text
  let logs = r.h.logs();
  let line = logs.iter().find(|l| l.contains("Claude Code runs in")).expect("the stand-in is logged");
  let expected = format!("{VIRT}/work/CLAUDE.md ({} bytes)", "Run the tests with cargo nextest.\n".len() + 1);
  assert!(line.contains(&expected), "{line}");
}

/// A write asks on a card that shows the diff, lands on the host, and its row reads as Claude's own Write; a command
/// runs in the host's folder; a read asks nothing
#[tokio::test(flavor = "multi_thread")]
async fn the_project_tools_run_on_the_host_and_read_as_the_native_ones() {
  let fake = crate::fake_or_skip!();
  let r = Remote::new(&fake).await;
  std::fs::write(r.real.join("work/notes.txt"), "host notes\n").unwrap();
  let s = r.session().await;
  let (cards, reply) = turn(&s, r#"claude-remote-call:Write {"file_path":"hello.txt","content":"hi from the host\n"}"#, "allow_once").await;
  assert_eq!(cards.len(), 1, "the write asks");
  let title = cards[0]["title"].as_str().unwrap_or_default();
  assert!(title.contains("hello.txt") && !title.contains("mcp__"), "the card reads as Claude's own Write: {title}");
  assert_eq!(std::fs::read_to_string(r.real.join("work/hello.txt")).unwrap(), "hi from the host\n");
  assert!(!r.stand_in().join("hello.txt").exists(), "nothing lands in the stand-in");
  for t in ["Read", "Write", "Edit", "Bash", "Grep", "Glob"] {
    assert!(reply["tools"].as_array().unwrap().iter().any(|x| x == t), "{t} is offered: {reply}");
  }
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "edit", "status": "completed", "diffStat": { "add": 1 } }));
  assert!(row.get("mcpServer").is_none_or(Value::is_null), "the row is Claude's Write, not an MCP call: {row}");

  let (cards, reply) = turn(&s, r#"claude-remote-call:Bash {"command":"cat hello.txt; pwd"}"#, "allow_once").await;
  assert_eq!(cards.len(), 1, "the command asks");
  let text = result_text(&reply);
  assert!(text.contains("hi from the host"), "{reply}");
  assert!(text.contains(&format!("{VIRT}/work")), "it ran in the host's folder: {reply}");
  assert!(!text.contains(&r.real.to_string_lossy().into_owned()), "{reply}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().rfind(|b| b["type"] == "tool_call").unwrap();
  // The output streamed onto the row as it ran (the model's copy has no trailing newline)
  expect_match(&row, json!({ "kind": "execute", "status": "completed", "target": "cat hello.txt; pwd" }));
  assert!(row["content"]["text"].as_str().is_some_and(|t| t.starts_with("hi from the host\n") && t.ends_with("/work\n")), "{row}");

  // A path under the stand-in means the host folder
  let stand = r.stand_in().join("notes.txt");
  let (cards, reply) = turn(&s, &format!(r#"claude-remote-call:Read {{"file_path":"{}"}}"#, stand.display()), "allow_once").await;
  assert!(cards.is_empty(), "a read asks nothing");
  assert!(result_text(&reply).contains("host notes"), "{reply}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().rfind(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "read", "status": "completed" }));
}

/// The tools beyond the first six: several edits in one call, a folder's tree, and a command that keeps running on the
/// host, read and stopped with tools named in its own start notice
#[tokio::test(flavor = "multi_thread")]
async fn several_edits_a_listing_and_a_background_command_run_on_the_host() {
  let fake = crate::fake_or_skip!();
  let r = Remote::new(&fake).await;
  std::fs::write(r.real.join("work/a.txt"), "one\ntwo\nthree\n").unwrap();
  let s = r.session().await;
  let edits = r#"{"file_path":"a.txt","edits":[{"old_string":"one","new_string":"1"},{"old_string":"three","new_string":"3"}]}"#;
  let (cards, reply) = turn(&s, &format!("claude-remote-call:MultiEdit {edits}"), "allow_once").await;
  assert_eq!(cards.len(), 1, "the edit asks: {reply}");
  let title = cards[0]["title"].as_str().unwrap_or_default();
  assert!(title.contains("a.txt") && !title.contains("mcp__"), "{title}");
  assert_eq!(std::fs::read_to_string(r.real.join("work/a.txt")).unwrap(), "1\ntwo\n3\n", "{reply}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().rfind(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "edit", "status": "completed", "diffStat": { "add": 2, "del": 2 } }));

  let (cards, reply) = turn(&s, r#"claude-remote-call:LS {"path":"."}"#, "allow_once").await;
  assert!(cards.is_empty(), "a listing asks nothing");
  assert!(result_text(&reply).contains("a.txt"), "{reply}");

  let (cards, reply) = turn(&s, r#"claude-remote-call:Bash {"command":"echo up; exec sleep 30","run_in_background":true}"#, "allow_once").await;
  assert_eq!(cards.len(), 1, "the command asks");
  let text = result_text(&reply);
  assert!(text.starts_with("Started background job ") && text.contains("mcp__acpira__BashOutput") && !text.contains("with job ("), "{reply}");
  let id = text.trim_start_matches("Started background job ").split('.').next().unwrap().to_owned();
  let (cards, reply) = turn(&s, &format!(r#"claude-remote-call:BashOutput {{"bash_id":"{id}"}}"#), "allow_once").await;
  assert!(cards.is_empty(), "reading the job asks nothing");
  assert!(result_text(&reply).contains("still running"), "{reply}");
  let (cards, reply) = turn(&s, &format!(r#"claude-remote-call:KillShell {{"shell_id":"{id}"}}"#), "allow_once").await;
  assert!(cards.is_empty(), "stopping the job asks nothing");
  assert!(result_text(&reply).contains("was stopped"), "{reply}");
}

/// A rejected write never reaches the host; Claude's own files on this machine stay here
#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_write_stays_off_the_host_and_claudes_own_files_stay_here() {
  let fake = crate::fake_or_skip!();
  let r = Remote::new(&fake).await;
  let s = r.session().await;
  let (_, reply) = turn(&s, r#"claude-remote-call:Write {"file_path":"no.txt","content":"x"}"#, "reject_once").await;
  assert_eq!(reply["rejected"], true, "{reply}");
  assert!(!r.real.join("work/no.txt").exists());
  // Under the engine's data root, as a saved output would be
  let own = r.h.dir.path().join("scratch/own.txt");
  let (_, reply) = turn(&s, &format!(r#"claude-remote-call:Write {{"file_path":"{}","content":"kept here\n"}}"#, own.display()), "allow_once").await;
  assert!(!result_text(&reply).is_empty(), "{reply}");
  assert_eq!(std::fs::read_to_string(&own).unwrap(), "kept here\n");
  assert!(find_block(&view(&s), "tool_call").is_some());
}

/// Stop ends a command still running on the host, whatever the CLI does with its MCP request
#[tokio::test(flavor = "multi_thread")]
async fn stop_ends_a_command_running_on_the_host() {
  let fake = crate::fake_or_skip!();
  let r = Remote::new(&fake).await;
  let s = r.session().await;
  let task = spawn_prompt(&s, r#"claude-remote-call:Bash {"command":"echo $$ > pid.txt; exec sleep 30"}"#);
  let pid_file = r.real.join("work/pid.txt");
  let t0 = Instant::now();
  let pid = loop {
    assert!(t0.elapsed() < Duration::from_secs(20), "the command did not start: {}", view(&s));
    for card in agent_blocks(&view(&s)).into_iter().filter(|b| b["type"] == "permission" && b["resolved"].is_null()) {
      let id = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").unwrap()["id"].clone();
      s.resolve_permission(card["id"].as_str().unwrap(), id.as_str().unwrap());
    }
    if let Some(pid) = std::fs::read_to_string(&pid_file).ok().and_then(|t| t.trim().parse::<i32>().ok()) {
      break pid;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  };
  s.cancel().await;
  let t0 = Instant::now();
  // SAFETY: signal 0 only checks the process exists
  while unsafe { libc::kill(pid, 0) } == 0 {
    assert!(t0.elapsed() < Duration::from_secs(5), "the command outlived the stop");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  tokio::time::timeout(Duration::from_secs(10), task).await.expect("the turn ends").unwrap();
}

/// Antigravity (`agy acp`) runs locally in its guarded stand-in with `.agents/hooks.json`, appends remote instructions +
/// `GEMINI.md` / `AGENTS.md` via `_meta.antigravity.systemPrompt.append`, correlates MCP calls without `toolUseId`,
/// strips `toolSummary`, auto-approves remote reads, and blocks built-in local workspace tools even in full access.
#[tokio::test(flavor = "multi_thread")]
async fn antigravity_runs_in_its_stand_in_and_routes_mcp_tools_to_the_host() {
  let fake = crate::fake_or_skip!();
  let r = Remote::for_agent(&fake, "antigravity").await;
  std::fs::write(r.real.join("work/GEMINI.md"), "Follow GEMINI rules.\n@AGENTS.md\n").unwrap();
  std::fs::write(r.real.join("work/AGENTS.md"), "Follow AGENTS rules.\n").unwrap();
  std::fs::write(r.real.join("work/notes.txt"), "host notes\n").unwrap();
  let s = r.session().await;
  assert!(view(&s)["error"].is_null(), "{} {:?}", view(&s)["error"], r.h.logs());

  let stand = r.stand_in_under("remote/antigravity");
  let req = r.requests().into_iter().find(|q| q["method"] == "new").expect("a session/new");
  assert_eq!(req["cwd"], json!(stand));
  let hooks = std::fs::read_to_string(stand.join(".agents/hooks.json")).expect("PreToolUse guard written");
  assert!(hooks.contains("PreToolUse") && hooks.contains("read_file"), "{hooks}");
  let append = req["meta"]["antigravity"]["systemPrompt"]["append"].as_str().expect("antigravity systemPrompt.append");
  assert!(append.contains(&format!("{VIRT}/work")) && append.contains("mcp_acpira_Bash"), "{append}");
  assert!(append.contains("Follow GEMINI rules.") && append.contains("Follow AGENTS rules."), "{append}");
  assert!(req["meta"].get("claudeCode").is_none() && req["meta"].get("acpira/remote").is_none());

  let (cards, _reply) = turn(&s, r#"agy-remote-call:Write {"file_path":"hello.txt","content":"hi from agy\n"}"#, "allow_once").await;
  assert_eq!(cards.len(), 1, "the write asks");
  let title = cards[0]["title"].as_str().unwrap_or_default();
  assert!(title.contains("hello.txt") && !title.contains("mcp_acpira_") && !title.contains("toolSummary"), "{title}");
  assert_eq!(std::fs::read_to_string(r.real.join("work/hello.txt")).unwrap(), "hi from agy\n");
  assert!(!stand.join("hello.txt").exists());
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "edit", "status": "completed", "diffStat": { "add": 1 } }));
  assert!(row.get("mcpServer").is_none_or(Value::is_null), "{row}");
  assert!(!row.to_string().contains("toolSummary"), "agy's synthetic toolSummary is stripped: {row}");

  let (cards, reply) = turn(&s, r#"agy-remote-call:Bash {"command":"cat hello.txt; pwd"}"#, "allow_once").await;
  assert_eq!(cards.len(), 1, "the command asks");
  assert!(result_text(&reply).contains("hi from agy") && result_text(&reply).contains(&format!("{VIRT}/work")), "{reply}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().rfind(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "execute", "status": "completed", "target": "cat hello.txt; pwd" }));
  assert!(row["content"]["text"].as_str().is_some_and(|t| t.starts_with("hi from agy\n") && t.ends_with("/work\n")), "{row}");

  // `agy` asks permission on every MCP tool, including Read; RemotePerm::ReadOnly approves it without a card
  let stand_notes = stand.join("notes.txt");
  let (cards, reply) = turn(&s, &format!(r#"agy-remote-call:Read {{"file_path":"{}"}}"#, stand_notes.display()), "allow_once").await;
  assert!(cards.is_empty(), "remote Read is auto-approved: {cards:?}");
  assert!(result_text(&reply).contains("host notes"), "{reply}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().rfind(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "read", "status": "completed" }));

  // Even under full access, built-in local workspace tools outside `~/.gemini` are auto-rejected before reaching disk
  let _ = s.set_mode("bypassPermissions".into()).await;
  let (cards, reply) = turn(&s, "agy-local-tool:read_file /etc/hosts", "allow_once").await;
  assert!(cards.is_empty(), "blocked local tool never opens a permission card: {cards:?}");
  assert_eq!(reply["rejected"], true, "{reply}");
}

/// Any ACP CLI a custom agent declares `localRun` for (`local_run::generic`): it starts in a stand-in of its own with the
/// remote note and the host's instructions as the stand-in's `AGENTS.md`, Acpira's project tools reach the host however
/// the CLI names and binds its calls, and a native write is refused without a card
#[tokio::test(flavor = "multi_thread")]
async fn a_custom_agent_runs_in_a_stand_in_of_its_own() {
  let fake = crate::fake_or_skip!();
  let r = Remote::with(&fake, "mine", json!({ "localRun": true })).await;
  std::fs::write(r.real.join("work/AGENTS.md"), "Build with make.\n").unwrap();
  let s = r.session().await;
  assert!(view(&s)["error"].is_null(), "{} {:?}", view(&s)["error"], r.h.logs());

  let stand = r.stand_in_under("remote/stand/mine");
  let req = r.requests().into_iter().find(|q| q["method"] == "new").expect("a session/new");
  assert_eq!(req["cwd"], json!(stand));
  for key in ["claudeCode", "antigravity", "acpira/remote"] {
    assert!(req["meta"].get(key).is_none(), "{key} is not this CLI's: {}", req["meta"]);
  }
  let note = std::fs::read_to_string(stand.join("AGENTS.md")).expect("the note is in the stand-in");
  assert!(note.contains(&format!("{VIRT}/work")) && note.contains("mcp__acpira__Bash") && note.contains("Build with make."), "{note}");

  // A call without an id of its own binds to its row by its input
  let (cards, _) = turn(&s, r#"agy-remote-call:Write {"file_path":"hello.txt","content":"hi from mine\n"}"#, "allow_once").await;
  assert_eq!(cards.len(), 1, "the write asks");
  assert_eq!(std::fs::read_to_string(r.real.join("work/hello.txt")).unwrap(), "hi from mine\n");
  assert!(!stand.join("hello.txt").exists(), "nothing lands in the stand-in");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "kind": "edit", "status": "completed", "diffStat": { "add": 1 } }));

  let (_, reply) = turn(&s, r#"claude-remote-call:Bash {"command":"pwd"}"#, "allow_once").await;
  assert!(result_text(&reply).contains(&format!("{VIRT}/work")), "it ran in the host's folder: {reply}");

  // The CLI's own write tool would write on this machine: refused before any card
  let elsewhere = tempfile::tempdir().unwrap();
  let target = elsewhere.path().join("native.txt");
  let (cards, reply) = turn(&s, &format!("hook-perm-write:{}", target.display()), "allow_once").await;
  assert!(cards.is_empty(), "a refused native write shows no card: {cards:?}");
  assert_eq!(reply, json!("rejected"));
  assert!(!target.exists());
}

/// A custom agent's own full-access mode would run its native tools here unasked: picking it stays the engine's, the
/// CLI keeps its asking mode, and a native write is still refused
#[tokio::test(flavor = "multi_thread")]
async fn a_gateless_stand_in_keeps_full_access_with_the_engine() {
  let fake = crate::fake_or_skip!();
  let logs = tempfile::tempdir().unwrap();
  let sent = logs.path().join("modes.log");
  let r = Remote::with(&fake, "mine", json!({ "localRun": true, "env": { "FAKE_MIXED_MODES": "claude", "FAKE_MODE_LOG": sent } })).await;
  let s = r.session().await;
  assert!(view(&s)["error"].is_null(), "{} {:?}", view(&s)["error"], r.h.logs());

  s.select_mode("bypassPermissions".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "bypassPermissions");
  let elsewhere = tempfile::tempdir().unwrap();
  let target = elsewhere.path().join("native.txt");
  let (cards, reply) = turn(&s, &format!("hook-perm-write:{}", target.display()), "allow_once").await;
  assert!(cards.is_empty(), "a refused native write shows no card: {cards:?}");
  assert_eq!(reply, json!("rejected"));
  assert!(!target.exists());

  s.select_mode("acceptEdits".into()).await.unwrap();
  let wire = std::fs::read_to_string(&sent).unwrap_or_default();
  assert!(!wire.contains("bypassPermissions") && wire.contains("acceptEdits"), "modes on the wire: {wire}");
}
