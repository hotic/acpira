//! The hooks a plugin's resident process answers for every agent (`acp/session/plugin_hooks.rs`, docs/dev/plugins.md,
//! Hooks): prompts rewritten, blocked and sent to another agent, permission requests denied or carded under yolo, a
//! failed turn retried, the title, the MCP list and the agent's launch environment, all against the fake agent

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_host::relay::hub::RelayHub;
use acpira_host::relay::roster::Roster;

use crate::acp_session::{agent_blocks, prompt, view};
use crate::fake_or_skip;
use crate::session_manager::{Mgr, Opts};
use crate::support::{Disposing, Harness, until};

/// Answers every cross-agent hook and writes each call to calls.jsonl in its data folder
const HOOKER: &str = r#"
import { createInterface } from 'node:readline';
import { appendFileSync, mkdirSync } from 'node:fs';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let data = '';
const rl = createInterface({ input: process.stdin });
rl.on('close', () => process.exit(0));
const answer = (method, p) => {
  switch (method) {
    case 'hook/prompt':
      if (p.text.includes('block-me')) return { block: 'not today' };
      if (p.text.includes('send-away')) return { redirect: 'other' };
      if (p.text.includes('rewrite-me')) return { text: p.text.replace('rewrite-me', 'REWRITTEN'), context: 'CTX-FROM-PLUGIN' };
      return null;
    case 'hook/permission': {
      const path = String(p.toolCall?.rawInput?.path ?? '');
      if (path.includes('deny')) return { decision: 'deny', reason: 'protected file' };
      if (path.includes('ask')) return { decision: 'ask' };
      return { decision: 'allow' };
    }
    case 'hook/retry': return { retry: p.attempt < 2, delayMs: 50 };
    case 'hook/title': return p.source === 'prompt' ? { title: 'P: ' + p.title } : null;
    case 'hook/session.mcp': return { servers: [...p.servers, { name: 'extra', command: 'true', args: [], env: [] }] };
    case 'hook/agent.launch': return { env: { FAKE_MCP_LOG: data + '/mcp.log' } };
    case 'hook/tool.before':
      if (p.arguments.cwd === '/blocked') return { block: 'not this one' };
      return { arguments: { ...p.arguments, cwd: '/from-plugin' } };
    case 'hook/command': return p.name === 'ping' ? { text: 'echo-text pong ' + p.arguments } : {};
    case 'hook/tool.after': return { result: { content: [{ type: 'text', text: 'AFTER:' + p.result.content[0].text }], isError: false } };
  }
  return null;
};
rl.on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') { data = m.params.plugin.data; mkdirSync(data, { recursive: true }); return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } }); }
  if (m.id === undefined) return;
  appendFileSync(data + '/calls.jsonl', JSON.stringify({ method: m.method, params: m.params }) + '\n');
  send({ jsonrpc: '2.0', id: m.id, result: answer(m.method, m.params) });
});
"#;

fn calls(root: &Path, method: &str) -> Vec<Value> {
  let text = std::fs::read_to_string(root.join(".data/hooker/calls.jsonl")).unwrap_or_default();
  text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|c| c["method"] == method).map(|c| c["params"].clone()).collect()
}

fn last_agent_text(view: &Value) -> String {
  let Some(turn) = view["turns"].as_array().and_then(|t| t.iter().rev().find(|t| t["role"] == "agent")) else { return String::new() };
  turn["blocks"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["markdown"].as_str()).collect()
}

fn turns(view: &Value) -> usize {
  view["turns"].as_array().map_or(0, Vec::len)
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_rewrites_blocks_redirects_judges_retries_and_launches_for_every_agent() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let project = tempfile::tempdir().unwrap();
  let plugins = tempfile::tempdir().unwrap();
  let hooks = ["prompt", "permission", "retry", "title", "session.mcp", "agent.launch"];
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "hooker", HOOKER, &[], &hooks).await;
  // Synthesized modes, so yolo is the host's own auto-approval
  let modes = json!([{ "id": "default", "name": "Agent" }, { "id": "plan", "name": "Plan" }, { "id": "yolo", "name": "Auto accept" }]);
  let mut agents = fake.setting(json!({ "modes": modes }));
  agents["other"] = fake.setting_as("other", json!({ "name": "Other" }))["other"].clone();
  let mut opts = Opts::with_agents(agents, "fake").cwd(project.path().to_str().unwrap());
  opts.plugin_runtime = Some(rt.clone());
  let m = Mgr::new(dir.path(), opts);
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();

  // Rewritten for the agent, the bubble and the title kept from what the user wrote, then retitled by the plugin
  m.handle(json!({ "type": "send", "text": "echo-text rewrite-me" })).await;
  let view = m.view_of(&id).unwrap();
  let reply = last_agent_text(&view);
  assert!(reply.contains("echo-text REWRITTEN") && reply.contains("CTX-FROM-PLUGIN") && !reply.contains("rewrite-me"), "{reply}");
  assert_eq!(view["turns"][0]["text"], "echo-text rewrite-me");
  until(|| m.view_of(&id).is_some_and(|v| v["title"] == "P: echo-text rewrite-me"), 5000).await;
  // The launch hook's env reached the agent, which logged the MCP list the session hook extended
  let mcp = std::fs::read_to_string(plugins.path().join(".data/hooker/mcp.log")).unwrap();
  assert!(mcp.contains("\"extra\""), "{mcp}");
  let launch = calls(plugins.path(), "hook/agent.launch");
  assert_eq!((launch.len(), launch[0]["agent"].as_str()), (1, Some("fake")));

  // Blocked: the agent never gets it, the user sees why
  let before = turns(&m.view_of(&id).unwrap());
  m.handle(json!({ "type": "send", "text": "echo-text block-me" })).await;
  let view = m.view_of(&id).unwrap();
  assert!(view.to_string().contains("hooker: not today"), "{view}");
  assert!(!last_agent_text(&view).contains("block-me"));
  assert!(turns(&view) <= before + 1, "{view}");

  // Under yolo a denial still rejects without a card, and an ask still shows one
  m.handle(json!({ "type": "setMode", "id": "yolo" })).await;
  let denied = project.path().join("deny.txt");
  m.handle(json!({ "type": "send", "text": format!("hook-perm-write:{}", denied.display()) })).await;
  assert_eq!(last_agent_text(&m.view_of(&id).unwrap()), "rejected");
  assert!(!denied.exists());
  assert!(m.toasts().iter().any(|t| t.contains("hooker") && t.contains("protected file")), "{:?}", m.toasts());
  let asked = project.path().join("ask.txt");
  let sending = m.spawn_handle(json!({ "type": "send", "text": format!("hook-perm-write:{}", asked.display()) }));
  let perm = |v: &Value| v.to_string().contains("\"type\":\"permission\"") && v["turns"].as_array().and_then(|t| t.last()).is_some_and(|t| t["blocks"].as_array().into_iter().flatten().any(|b| b["type"] == "permission" && b["outcome"].is_null()));
  until(|| m.view_of(&id).is_some_and(|v| perm(&v)), 10_000).await;
  let view = m.view_of(&id).unwrap();
  let block = view["turns"].as_array().unwrap().last().unwrap()["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "permission").unwrap().clone();
  m.handle(json!({ "type": "permission", "sessionId": id, "blockId": block["id"], "optionId": "allow" })).await;
  sending.await.unwrap();
  until(|| asked.exists(), 5000).await;
  assert_eq!(calls(plugins.path(), "hook/permission").iter().map(|p| p["autoApprove"].as_bool()).collect::<Vec<_>>(), [Some(true), Some(true)]);
  m.handle(json!({ "type": "setMode", "id": "default" })).await;

  // An empty completion fails the turn; the plugin retries it and the second attempt answers
  m.handle(json!({ "type": "send", "text": "empty-response" })).await;
  until(|| m.view_of(&id).is_some_and(|v| v["turns"].as_array().and_then(|t| t.last()).is_some_and(|t| t["stop"] == "end_turn" && t["role"] == "agent")), 10_000).await;
  let retry = calls(plugins.path(), "hook/retry");
  assert_eq!(retry.len(), 1, "{retry:?}");
  assert_eq!((retry[0]["attempt"].as_u64(), retry[0]["error"]["kind"].as_str()), (Some(1), Some("empty_response")));

  // Sent to the other agent: a new session there gets it, the window follows, and it is not sent on again
  m.handle(json!({ "type": "send", "text": "echo-text send-away" })).await;
  until(|| m.active_id().is_some_and(|a| a != id), 10_000).await;
  let moved = m.active_id().unwrap();
  until(|| m.view_of(&moved).is_some_and(|v| last_agent_text(&v).contains("send-away")), 10_000).await;
  assert_eq!(m.view_of(&moved).unwrap()["agent"], "other");
  assert!(!last_agent_text(&m.view_of(&id).unwrap()).contains("send-away"));
  let prompts = calls(plugins.path(), "hook/prompt");
  assert_eq!(prompts.last().unwrap()["redirected"], true, "{prompts:?}");
  assert!(m.toasts().iter().any(|t| t.contains("hooker") && t.contains("Other")), "{:?}", m.toasts());
  m.dispose().await;
  rt.stop_all().await;
}

/// The JSON the fake CLI answers an `mcp-call:` prompt with: the tools it listed and the call's result
fn mcp_reply(s: &Disposing) -> Value {
  let text = agent_blocks(&view(s)).into_iter().filter(|b| b["type"] == "text").filter_map(|b| b["markdown"].as_str().map(str::to_owned)).next_back().unwrap_or_default();
  serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_changes_and_refuses_calls_to_acpiras_own_tools() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let plugins = tempfile::tempdir().unwrap();
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "hooker", HOOKER, &[], &["tool.before", "tool.after"]).await;
  let mut h = Harness::new(&fake, json!({}));
  let hub = RelayHub::start(Arc::new(Roster::new(h.dir.path())), Arc::new(|_: &str| {})).await.unwrap();
  h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub));
  h.deps.plugin_runtime = Some(rt.clone());
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The arguments as the plugin set them reach the tool, and the result as the plugin changed it reaches the agent
  prompt(&s, r#"mcp-call:workspace_hooks {"cwd":"/from-agent"}"#).await;
  let reply = mcp_reply(&s);
  let text = reply["result"]["content"][0]["text"].as_str().unwrap_or_default();
  assert!(text.starts_with("AFTER:") && text.contains("/from-plugin") && !text.contains("/from-agent"), "{reply}");
  let after = calls(plugins.path(), "hook/tool.after");
  assert_eq!((after[0]["tool"].as_str(), after[0]["arguments"]["cwd"].as_str(), after[0]["session"].as_str()), (Some("workspace_hooks"), Some("/from-plugin"), Some(s.id.as_str())));
  // Refused before it runs: the agent reads why
  prompt(&s, r#"mcp-call:workspace_hooks {"cwd":"/blocked"}"#).await;
  let reply = mcp_reply(&s);
  assert_eq!(reply["result"]["isError"], true, "{reply}");
  assert!(reply["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("hooker: not this one")), "{reply}");
  assert_eq!(calls(plugins.path(), "hook/tool.after").len(), 1);
  rt.stop_all().await;
}

fn command(plugin: &str, name: &str, prompt: Option<&str>) -> acpira_host::plugins::runtime::PluginCommand {
  let def = serde_json::from_value(json!({ "name": name, "description": format!("{name} it"), "prompt": prompt })).unwrap();
  acpira_host::plugins::runtime::PluginCommand { plugin: plugin.into(), def }
}

fn menu(view: &Value) -> Vec<String> {
  view["commands"].as_array().into_iter().flatten().filter_map(|c| c["name"].as_str().map(str::to_owned)).collect()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugins_commands_join_every_menu_and_its_skills_reach_an_agent_that_does_not_load_them() {
  use acpira_host::plugins::runtime::Contributed;
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let plugins = tempfile::tempdir().unwrap();
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "hooker", HOOKER, &[], &["command"]).await;
  let skills = plugins.path().join("hooker/skills");
  std::fs::create_dir_all(skills.join("demo")).unwrap();
  std::fs::write(skills.join("demo/SKILL.md"), "---\nname: demo\ndescription: Do demo things\n---\nSteps.\n").unwrap();
  let commands = vec![command("hooker", "review", Some("echo-text Please review $ARGUMENTS")), command("hooker", "ping", None), command("hooker", "quiet", None)];
  rt.set_contributed(Contributed { commands: commands.clone(), skills: vec![skills.clone()], ..Default::default() });
  let mut opts = Opts::fake(&fake);
  opts.plugin_runtime = Some(rt.clone());
  let m = Mgr::new(dir.path(), opts);
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  until(|| m.view_of(&id).is_some_and(|v| menu(&v).contains(&"review".to_owned())), 5000).await;
  assert!(["ping", "quiet"].iter().all(|c| menu(&m.view_of(&id).unwrap()).contains(&c.to_string())));

  // The template goes out in the command's place; the first prompt also tells of the plugin's skills
  m.handle(json!({ "type": "send", "text": "/review src/x.rs" })).await;
  let view = m.view_of(&id).unwrap();
  let reply = last_agent_text(&view);
  assert!(reply.starts_with("echo-text Please review src/x.rs"), "{reply}");
  assert!(reply.contains("<plugin-skills>") && reply.contains("- demo: Do demo things") && reply.contains("demo/SKILL.md"), "{reply}");
  assert_eq!(view["turns"][0]["text"], "/review src/x.rs");
  // The process answers a command without a template; an answer without a prompt sends nothing
  m.handle(json!({ "type": "send", "text": "/ping now" })).await;
  let reply = last_agent_text(&m.view_of(&id).unwrap());
  assert_eq!(reply, "echo-text pong now");
  let before = turns(&m.view_of(&id).unwrap());
  m.handle(json!({ "type": "send", "text": "/quiet" })).await;
  assert_eq!(turns(&m.view_of(&id).unwrap()), before);
  assert_eq!(calls(plugins.path(), "hook/command").len(), 2);

  // A command's button with `draft`: the text comes back for the composer, the template filled in or the process's
  // answer (told it is for the draft), and nothing is sent
  let entry = m.view_of(&id).unwrap()["commands"].as_array().unwrap().iter().find(|c| c["name"] == "review").cloned().unwrap();
  assert_eq!(entry["plugin"], "hooker");
  m.handle(json!({ "type": "pluginCommand", "id": "c1", "sessionId": id, "name": "review", "arguments": "my draft" })).await;
  m.handle(json!({ "type": "pluginCommand", "id": "c2", "sessionId": id, "name": "ping", "arguments": "soon" })).await;
  m.handle(json!({ "type": "pluginCommand", "id": "c3", "sessionId": id, "name": "nope", "arguments": "" })).await;
  let called = |call: &str| m.events.lock().unwrap().iter().find(|e| e["type"] == "pluginCalled" && e["id"] == call).cloned();
  until(|| ["c1", "c2", "c3"].iter().all(|c| called(c).is_some()), 5000).await;
  assert_eq!(called("c1").unwrap()["result"]["text"], "echo-text Please review my draft");
  assert_eq!(called("c2").unwrap()["result"]["text"], "echo-text pong soon");
  assert!(called("c3").unwrap()["error"].as_str().is_some_and(|e| e.contains("/nope")));
  assert_eq!(calls(plugins.path(), "hook/command").last().unwrap()["draft"], true);
  assert_eq!(turns(&m.view_of(&id).unwrap()), before);

  // A command added while the session is open shows in its menu at once
  let mut more = commands;
  more.push(command("hooker", "later", Some("echo-text later")));
  rt.set_contributed(Contributed { commands: more, skills: vec![skills], ..Default::default() });
  until(|| m.view_of(&id).is_some_and(|v| menu(&v).contains(&"later".to_owned())), 5000).await;
  m.dispose().await;
  rt.stop_all().await;
}
