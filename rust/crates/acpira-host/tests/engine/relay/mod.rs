//! Cross-harness subagents (acp/session/relay.rs, relay/): a session summons a persona whose CLI is the fake agent, through
//! `relay_ask` directly and through the loopback hub the MCP server uses

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_host::relay::hub::{HostBrowser, RelayHub, call};
use acpira_shared::sidecar::{BrowserOpened, BrowserReuse};
use acpira_host::relay::roster::Roster;
use acpira_host::relay::wire::{AskArgs, AskCallerArgs, HubOp, HubReply, HubRequest};

use crate::acp_session::{agent_blocks, last_turn, prompt, view};
use crate::fake_or_skip;
use crate::support::{Disposing, Harness, expect_match, until, v};

mod rounds;
mod hub;
mod rows;
mod deliveries;
mod questions;
mod follow_ups;
mod packs;
mod groups;
mod login;

async fn roster(dir: &std::path::Path) -> Arc<Roster> {
  let r = Arc::new(Roster::new(dir));
  r.save(&json!([
    { "name": "Fake Review", "agent": "fake", "mode": "consult", "when": "second opinions", "brief": "Severity levels please." },
    { "name": "Fake Worker", "agent": "fake", "mode": "work", "when": "edits" },
  ]))
  .await
  .unwrap();
  r
}

/// A call that waits for the reply (the root's default is to send the task off)
fn ask(agent: &str, prompt: &str) -> AskArgs {
  AskArgs { agent: agent.into(), prompt: prompt.into(), wait_secs: Some(20), wait: Some(true), ..Default::default() }
}

/// Every reply line of one call, the final one last
async fn run(s: &Disposing, roster: &Arc<Roster>, depth: u32, args: AskArgs) -> Vec<HubReply> {
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, depth, args, tx));
  let mut out = vec![];
  while let Some(r) = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await.expect("reply in time") {
    out.push(r);
  }
  out
}

fn last_text(replies: &[HubReply]) -> (bool, String) {
  match replies.last() {
    Some(HubReply::Done(t)) => (true, t.clone()),
    Some(HubReply::Error(t)) => (false, t.clone()),
    other => panic!("no final reply: {other:?}"),
  }
}

fn nodes(s: &Disposing) -> Vec<Value> {
  v(s.view().subagents.unwrap_or_default()).as_array().cloned().unwrap_or_default()
}

fn thread_of(text: &str) -> String {
  let rest = text.split("thread: ").nth(1).expect("thread named");
  rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// The automatic user turns delivering summoned agents' replies
fn deliveries(s: &Disposing) -> Vec<Value> {
  v(s.view())["turns"].as_array().unwrap().iter().filter(|t| t["autoReason"] == "subagents").cloned().collect()
}

fn send_off(agent: &str, prompt: &str) -> AskArgs {
  AskArgs { agent: agent.into(), prompt: prompt.into(), ..Default::default() }
}

/// A session whose agents get Acpira's MCP entry with a live hub, so a fake CLI's `mcp-call:` reaches the relay for real
/// (the harness first, so it outlives the session when the bindings drop)
async fn hub_session(fake: &crate::support::FakeAgent) -> (Harness, Disposing, Arc<Roster>) {
  let mut h = Harness::new(fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  h.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub)));
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  (h, s, roster)
}

/// A git work tree holding a committed `a.txt`, for rounds whose changes are counted
fn repo() -> Option<(tempfile::TempDir, std::path::PathBuf)> {
  let git = |root: &std::path::Path, args: &[&str]| {
    std::process::Command::new("git").arg("-C").arg(root).args(args).output().is_ok_and(|o| o.status.success())
  };
  let dir = tempfile::tempdir().unwrap();
  let root = acpira_host::platform::paths::canonical_for_cli(dir.path()).unwrap();
  let ok = git(&root, &["init", "-q"]) && git(&root, &["config", "user.email", "t@t"]) && git(&root, &["config", "user.name", "t"]);
  std::fs::write(root.join("a.txt"), "one\n").unwrap();
  (ok && git(&root, &["add", "."]) && git(&root, &["commit", "-q", "-m", "base"])).then_some((dir, root))
}
