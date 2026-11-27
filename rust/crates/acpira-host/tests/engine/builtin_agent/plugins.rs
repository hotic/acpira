//! Plugins shaping the built-in agent: prompt folders, model sources, skills, tool and shell hooks, specialised agents

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_s_prompt_folder_reaches_the_built_in_agent_s_system_prompt() {
  let mut env = Env::new();
  let layer = env.root.path().join("plugins/.versions/tone/abc/prompts");
  std::fs::create_dir_all(&layer).unwrap();
  std::fs::write(layer.join("system.md"), "## Answering\n- Answer like a pirate (PLUGIN-MARK).\n").unwrap();
  let sent = layer.clone();
  env.deps.plugin_prompts = Some(Arc::new(move || vec![sent.clone()]));
  let s = env.session().await;
  env.server.push(mock::text("Arr."));
  run(&s, "hello").await;
  let body = env.server.requests()[0].body.to_string();
  assert!(body.contains("PLUGIN-MARK"), "{body}");
}

/// Answers every hook of the built-in agent's loop and writes each call to calls.jsonl in its data folder
#[cfg(unix)]
const LOOPER: &str = r#"
import { createInterface } from 'node:readline';
import { appendFileSync, mkdirSync } from 'node:fs';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let data = '';
const answer = (method, p) => {
  switch (method) {
    case 'hook/tools': return { tools: p.tools.filter(t => t.name !== 'glob') };
    case 'hook/context': return { system: p.system + '\nPLUGIN-RULE' };
    case 'hook/model.request': return { headers: { 'x-plugin': 'on' } };
    case 'hook/tool.before': return p.tool === 'read' ? { result: { text: 'READ-BY-PLUGIN' } } : null;
    case 'hook/shell': return { command: 'echo "$PLUGIN_VAR"', env: { PLUGIN_VAR: "from 'plugin'" } };
    case 'hook/tool.after': return p.tool === 'bash' ? { result: { text: 'AFTER:' + (p.arguments.command.includes('PLUGIN_VAR') ? 'REWRITTEN:' : '') + p.result.text } } : null;
    case 'hook/websearch': return { text: 'RESULTS FOR ' + p.query };
    case 'hook/compaction': return { summary: 'PLUGIN SUMMARY' };
  }
  return null;
};
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') { data = m.params.plugin.data; mkdirSync(data, { recursive: true }); return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } }); }
  if (m.id === undefined) return;
  appendFileSync(data + '/calls.jsonl', JSON.stringify({ method: m.method }) + '\n');
  send({ jsonrpc: '2.0', id: m.id, result: answer(m.method, m.params) });
}).on('close', () => process.exit(0));
"#;

/// Sets one model source from its process, the way a plugin that fetched a gateway's models would
#[cfg(unix)]
const SOURCER: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') {
    send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
    return send({ jsonrpc: '2.0', id: 'set', method: 'providers/set', params: { providers: [
      { id: 'gw', name: 'Gateway', baseUrl: 'BASE_URL', keyEnv: 'PATH', models: [{ id: 'm2', name: 'Gateway Two' }] },
    ] } });
  }
  if (m.method && m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, result: null });
}).on('close', () => process.exit(0));
"#;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugins_model_sources_reach_the_picker_and_serve_its_turns() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  // The user's own file is left without sources, so the plugin's model is the session's default
  std::fs::write(env.root.path().join("providers.json"), json!({ "version": 1, "fastModel": "off", "providers": [] }).to_string()).unwrap();
  let plugins = env.root.path().join("plugins");
  let script = SOURCER.replace("BASE_URL", &env.server.base_url());
  let rt = crate::acp_session::plugin_runtime::plugin(&plugins, "sourcer", &script, &[], &[]).await;
  // A manifest's source lands beside the process's, ahead of it (switched off here, so the default model is the process's)
  let fixed = acpira_host::plugins::manifest::provider_of(&json!({ "id": "fixed", "baseUrl": "http://127.0.0.1:9", "enabled": false, "models": [{ "id": "f1" }] }), "fixed").unwrap();
  rt.set_contributed(acpira_host::plugins::runtime::Contributed { providers: vec![fixed], ..Default::default() });
  let file = acpira_shared::providers::plugin_providers_path(env.root.path());
  until(|| std::fs::read_to_string(&file).is_ok_and(|t| t.contains("\"gw\"") && t.contains("\"fixed\"")), 10_000).await;
  let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
  let ids: Vec<&str> = written["providers"].as_array().unwrap().iter().filter_map(|p| p["id"].as_str()).collect();
  assert_eq!(ids, ["fixed", "gw"]);
  assert!(!written.to_string().contains(&std::env::var("PATH").unwrap()), "the key stays out of the file");
  env.deps.plugin_runtime = Some(rt.clone());
  let s = env.session().await;
  assert!(view(&s).to_string().contains("Gateway Two"), "the picker lists the plugin's model");
  env.server.push(mock::text("Hi from the gateway."));
  s.prompt("hello".into(), vec![], false, None, None).await;
  let reqs = env.server.requests();
  assert_eq!(reqs.len(), 1);
  assert_eq!(reqs[0].body["model"], "m2");
  assert_eq!(reqs[0].header("authorization").map(str::to_owned), std::env::var("PATH").ok().map(|k| format!("Bearer {k}")));
  // The plugin leaving takes its sources along
  rt.sync(&[]).await;
  rt.set_contributed(Default::default());
  let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
  assert_eq!(written["providers"], json!([]));
  rt.stop_all().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugins_skill_loads_through_the_built_in_agents_skill_tool() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  let plugins = tempfile::tempdir().unwrap();
  let skills = plugins.path().join("kit/skills");
  std::fs::create_dir_all(skills.join("tidy")).unwrap();
  std::fs::write(skills.join("tidy/SKILL.md"), "---\nname: tidy\ndescription: Tidy a folder\n---\n\nPLUGIN SKILL BODY\n").unwrap();
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "kit", RIDER, &[], &["shell"]).await;
  rt.set_contributed(acpira_host::plugins::runtime::Contributed { skills: vec![skills], ..Default::default() });
  env.deps.plugin_runtime = Some(rt.clone());
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "skill", json!({ "name": "tidy" }))]));
  env.server.push(mock::text("Done."));
  s.prompt("tidy up".into(), vec![], false, None, None).await;
  let results = tool_results(&env.server.requests()[1].body);
  assert!(results[0].contains("PLUGIN SKILL BODY"), "{results:?}");
  // The session's slash commands list it too
  assert!(view(&s).to_string().contains("tidy"), "the menu lists the plugin's skill");
  rt.stop_all().await;
}

/// Leaves `bash` out of every turn's tools
#[cfg(unix)]
const NO_SHELL: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'hook/tools') return send({ jsonrpc: '2.0', id: m.id, result: { tools: m.params.tools.filter(t => t.name !== 'bash') } });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, result: null });
}).on('close', () => process.exit(0));
"#;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_the_tools_hook_left_out_does_not_run_when_the_model_calls_it() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  let path = env.root.path().join("providers.json");
  let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  p["approval"] = json!("full");
  std::fs::write(path, p.to_string()).unwrap();
  let plugins = tempfile::tempdir().unwrap();
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "noshell", NO_SHELL, &[], &["tools"]).await;
  env.deps.plugin_runtime = Some(rt.clone());
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "touch ran.txt" }))]));
  env.server.push(mock::text("Done."));
  s.prompt("go".into(), vec![], false, None, None).await;
  let reqs = env.server.requests();
  let names: Vec<&str> = reqs[0].body["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
  assert!(!names.contains(&"bash"), "{names:?}");
  assert!(!env.cwd().join("ran.txt").exists(), "the left-out tool ran: {:?}", tool_results(&reqs[1].body));
  rt.stop_all().await;
}

/// Rewrites every command, an edit's `then_run` included
#[cfg(unix)]
const RIDER: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'hook/shell') return send({ jsonrpc: '2.0', id: m.id, result: m.params.command.includes('blocked') ? { block: 'no blocked commands' } : { command: 'echo rider-$PLUGIN_VAR', env: { PLUGIN_VAR: 'ok' } } });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, result: null });
}).on('close', () => process.exit(0));
"#;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugins_shell_hook_also_rewrites_and_refuses_an_edits_then_run_command() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  let path = env.root.path().join("providers.json");
  let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  p["approval"] = json!("full");
  std::fs::write(path, p.to_string()).unwrap();
  let plugins = tempfile::tempdir().unwrap();
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "rider", RIDER, &[], &["shell"]).await;
  env.deps.plugin_runtime = Some(rt.clone());
  let s = env.session().await;
  env.server.push(mock::tools(&[
    ("c1", "write", json!({ "path": "a.txt", "content": "x", "then_run": "echo original" })),
    ("c2", "write", json!({ "path": "b.txt", "content": "y", "then_run": "echo blocked" })),
  ]));
  env.server.push(mock::text("Done."));
  s.prompt("go".into(), vec![], false, None, None).await;
  let reqs = env.server.requests();
  let results = tool_results(&reqs[1].body);
  assert!(results[0].contains("rider-ok") && !results[0].contains("echo original"), "{results:?}");
  assert!(results[1].contains("Not run: Refused by a plugin: rider: no blocked commands"), "{results:?}");
  // The change itself was made either way
  assert!(env.cwd().join("a.txt").is_file() && env.cwd().join("b.txt").is_file());
  rt.stop_all().await;
}

/// The text of every `tool` message of a model call
#[cfg(unix)]
fn tool_results(body: &Value) -> Vec<String> {
  body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").map(|m| m["content"].as_str().map(str::to_owned).unwrap_or_else(|| m["content"].to_string())).collect()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_reshapes_every_request_tool_call_and_summary_of_the_loop() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  let plugins = tempfile::tempdir().unwrap();
  let hooks = ["tools", "context", "model.request", "tool.before", "tool.after", "shell", "websearch", "compaction"];
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "looper", LOOPER, &[], &hooks).await;
  env.deps.plugin_runtime = Some(rt.clone());
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "read", json!({ "path": "missing.txt" }))]));
  env.server.push(mock::tools(&[("c2", "bash", json!({ "command": "echo hi" }))]));
  env.server.push(mock::tools(&[("c3", "websearch", json!({ "query": "acpira" }))]));
  env.server.push(mock::text("Done."));
  let task = spawn_prompt(&s, "go");
  let card = wait_block(&s, "permission").await;
  let allow = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").unwrap()["id"].clone();
  s.resolve_permission(card["id"].as_str().unwrap(), allow.as_str().unwrap());
  task.await.unwrap();
  let reqs = env.server.requests();
  assert_eq!(reqs.len(), 4);
  // Every call: the plugin's rule in the system prompt, its header, and the tool it left out missing
  for r in &reqs {
    assert!(r.body["messages"][0]["content"].as_str().is_some_and(|c| c.ends_with("\nPLUGIN-RULE")), "{}", r.body["messages"][0]);
    assert_eq!(r.header("x-plugin"), Some("on"));
    let names: Vec<&str> = r.body["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
    assert!(names.contains(&"read") && !names.contains(&"glob"), "{names:?}");
  }
  // The read answered by the plugin, the command run as it rewrote it with its variable and its result changed after,
  // the search answered by its backend
  let results = tool_results(&reqs[3].body);
  assert_eq!(results[0], "READ-BY-PLUGIN");
  // tool.after sees the command as it ran, after the shell hook rewrote it
  assert!(results[1].starts_with("AFTER:REWRITTEN:") && results[1].contains("from 'plugin'"), "{results:?}");
  assert_eq!(results[2], "RESULTS FOR acpira");
  // /compact: the plugin writes the summary, no model is called, and the next call starts from it
  s.prompt("/compact".into(), vec![], false, None, None).await;
  assert_eq!(env.server.requests().len(), 4);
  env.server.push(mock::text("Again."));
  s.prompt("next".into(), vec![], false, None, None).await;
  let reqs = env.server.requests();
  assert!(reqs[4].body["messages"].to_string().contains("PLUGIN SUMMARY"), "{}", reqs[4].body["messages"]);
  rt.stop_all().await;
}

// A specialised agent's guard: refuses reading `secret.txt`, asked only in its own agent's sessions (`scope: agent`)
#[cfg(unix)]
const LEDGER_GUARD: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'hook/tool.before') {
    const secret = String(m.params.arguments?.path ?? '').endsWith('secret.txt');
    return send({ jsonrpc: '2.0', id: m.id, result: secret ? { block: `no secrets for ${m.params.agent}` } : null });
  }
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, result: null });
}).on('close', () => process.exit(0));
"#;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugins_specialised_agent_runs_its_own_prompt_tools_and_guard_and_leaves_the_general_agent_alone() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  let path = env.root.path().join("providers.json");
  let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  p["approval"] = json!("full");
  std::fs::write(path, p.to_string()).unwrap();
  std::fs::write(env.cwd().join("secret.txt"), "the vault code").unwrap();
  // The plugin: Acpira's loop with its own prompt and only `read`, and a guard for its own sessions
  let plugins = tempfile::tempdir().unwrap();
  let dir = plugins.path().join("ledger");
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("plugin.mjs"), LEDGER_GUARD).unwrap();
  std::fs::write(dir.join("prompt.md"), "You keep the ledger and nothing else.").unwrap();
  let manifest = json!({ "schema": 1, "id": "ledger", "version": "1.0.0", "description": "Ledger keeper", "contributes": {
    "agent": { "name": "Ledger", "builtin": { "prompt": "prompt.md", "tools": ["read"] } },
    "runtime": { "command": ["node", "plugin.mjs"], "hooks": ["tool.before"], "scope": "agent" },
  } });
  std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
  let (m, mut f) = acpira_host::plugins::manifest::parse(manifest.to_string().as_bytes()).unwrap();
  acpira_host::plugins::manifest::check(&m, "ledger", &dir, &|c| c == "node", &mut f);
  assert!(f.ok(), "{f:?}");
  let rt = acpira_host::plugins::runtime::Runtimes::new(plugins.path().join(".data"), Arc::new(|_: &str| {}));
  rt.enable(None);
  rt.sync(&[Arc::new(acpira_host::plugins::host::Active { id: "ledger".into(), hash: "h".into(), dir: dir.clone(), manifest: m })]).await;
  env.deps.plugin_runtime = Some(rt.clone());
  // What the engine's registry makes of the plugin's entry (`runtime.rs` registry_of)
  let exe = env!("CARGO_BIN_EXE_acpira");
  let root = env.root.path().to_string_lossy().into_owned();
  let custom = json!({ "ledger": { "name": "Ledger", "command": exe, "args": ["agent", "--home", root, "--profile", dir], "subagents": false, "terminalAuth": false } });
  env.deps.registry = Arc::new(AgentRegistry::new(&custom).with_self_agent(exe, env.root.path()));

  let s = Disposing(AcpSession::fresh("ledger", &env.cwd().to_string_lossy(), env.deps.clone(), None));
  s.start().await;
  env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "touch ran.txt" }))]));
  env.server.push(mock::tools(&[("c2", "read", json!({ "path": "secret.txt" }))]));
  env.server.push(mock::text("Done."));
  s.prompt("go".into(), vec![], false, None, None).await;
  let reqs = env.server.requests();
  let system = reqs[0].body["messages"][0]["content"].as_str().unwrap_or_default().to_owned();
  assert!(system.starts_with("You keep the ledger and nothing else."), "{system}");
  assert!(!system.contains("AGENTS.md"), "{system}");
  let names: Vec<&str> = reqs[0].body["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
  assert_eq!(names, ["read"]);
  // A tool outside the profile does not run when the model calls it anyway; the guard refuses its own agent's read
  assert!(!env.cwd().join("ran.txt").exists(), "the left-out tool ran: {:?}", tool_results(&reqs[1].body));
  let results = tool_results(&reqs[2].body).join("\n");
  assert!(results.contains("no secrets for ledger") && !results.contains("the vault code"), "{results}");
  // Its sessions offer Agent mode only
  let modes: Vec<String> = view(&s)["controls"]["modes"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect();
  assert_eq!(modes, ["agent"]);

  // The general agent next to it: the full tool table, the built-in prompt, and the guard never asked
  let g = env.session().await;
  env.server.push(mock::tools(&[("c3", "read", json!({ "path": "secret.txt" }))]));
  env.server.push(mock::text("Read."));
  g.prompt("go".into(), vec![], false, None, None).await;
  let reqs = env.server.requests();
  let general = &reqs[reqs.len() - 2];
  let names: Vec<&str> = general.body["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
  assert!(names.contains(&"bash") && names.contains(&"write"), "{names:?}");
  assert!(tool_results(&reqs[reqs.len() - 1].body).join("\n").contains("the vault code"));
  rt.stop_all().await;
}
