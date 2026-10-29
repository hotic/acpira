#![cfg(unix)]

//! Claude Code in a folder on an SSH host under local run (`vendors/claude_remote.rs`): the fake agent plays
//! claude-agent-acp (`claude-remote-call:`), the lab's fake ssh plays the host, and Acpira's own MCP server carries the
//! project tools to it through a live hub

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_host::acp::session::AcpSession;
use acpira_host::relay::hub::RelayHub;
use acpira_host::relay::roster::Roster;
use acpira_shared::remote::{ConnectionOp, RunMode};

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
    let logs = tempfile::tempdir().unwrap();
    let mut h = Harness::for_agent(fake, "claude", json!({ "env": { "FAKE_META_LOG": logs.path().join("meta.log") } }));
    let lab = Lab::new();
    let real = lab.path().join("hostfs");
    std::fs::create_dir_all(real.join("work")).unwrap();
    lab.map_root(VIRT, &real);
    let connections = lab.engine("engine");
    connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
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
    self.h.dir.path().join("remote/hosts/gen1").join(VIRT.trim_start_matches('/')).join("work")
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
  // The system prompt section itself is checked in `claude_remote`'s unit tests: the fake's ACP SDK drops the field.
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
