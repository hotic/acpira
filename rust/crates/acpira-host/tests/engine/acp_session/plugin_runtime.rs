//! Plugins' resident processes as a session reaches them (`plugins/runtime.rs`, `acp/session/plugin_events.rs`):
//! the events of a turn in order, and nothing for families a process did not subscribe to

use std::path::Path;

use acpira_host::plugins::host::Active;
use acpira_host::plugins::manifest;
use acpira_host::plugins::runtime::Runtimes;

use super::*;

/// A resident plugin in node that writes every event it hears to events.ndjson in its data folder
const RECORDER: &str = r#"
import { createInterface } from 'node:readline';
import { appendFileSync, mkdirSync } from 'node:fs';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let data = '';
const rl = createInterface({ input: process.stdin });
rl.on('close', () => process.exit(0));
rl.on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') { data = m.params.plugin.data; mkdirSync(data, { recursive: true }); return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } }); }
  if (m.method === 'event') appendFileSync(data + '/events.ndjson', JSON.stringify(m.params) + '\n');
});
"#;

/// The recorder running for `events`, with its folder
pub async fn recorder(root: &Path, events: &[&str]) -> Arc<Runtimes> {
  let dir = root.join("rec");
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("plugin.mjs"), RECORDER).unwrap();
  let m = json!({ "schema": 1, "id": "rec", "version": "1.0.0", "description": "d", "contributes": { "runtime": { "command": ["node", "plugin.mjs"], "events": events } } });
  let (manifest, _) = manifest::parse(m.to_string().as_bytes()).unwrap();
  let rt = Runtimes::new(root.join(".data"), Arc::new(|_: &str| {}));
  rt.enable(None);
  rt.sync(&[Arc::new(Active { id: "rec".into(), hash: "h".into(), dir, manifest })]).await;
  assert_eq!(rt.running(), ["rec"]);
  rt
}

pub fn heard(root: &Path) -> Vec<Value> {
  std::fs::read_to_string(root.join(".data/rec/events.ndjson")).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_s_events_reach_a_plugin_in_order() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let root = tempfile::tempdir().unwrap();
  let rt = recorder(root.path(), &["turn.*", "tool.*", "permission.*", "files.*", "usage.*"]).await;
  let mut h = Harness::new(&fake, json!({}));
  h.deps.plugin_runtime = Some(rt.clone());
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "use tool");
  let perm = wait_block(&s, "permission").await;
  s.resolve_permission(perm["id"].as_str().unwrap(), "allow");
  p.await.unwrap();
  until(|| heard(root.path()).iter().any(|e| e["type"] == "turn.ended"), 5000).await;
  let events = heard(root.path());
  let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
  let at = |t: &str| types.iter().position(|x| *x == t).unwrap_or_else(|| panic!("no {t} in {types:?}"));
  assert!(at("turn.started") < at("tool.started") && at("permission.asked") < at("permission.replied"), "{types:?}");
  assert!(at("permission.replied") < at("tool.completed") && at("tool.completed") < at("turn.ended"), "{types:?}");
  // Every event names its session; the run row settles once although its update repeats the status
  assert!(events.iter().all(|e| e["data"]["session"] == s.id.as_str() && e["data"]["agent"] == "fake"), "{events:?}");
  let completed: Vec<&Value> = events.iter().filter(|e| e["type"] == "tool.completed").map(|e| &e["data"]).collect();
  assert_eq!(completed.iter().filter(|d| d["toolCallId"] == "tc1").count(), 1, "{completed:?}");
  expect_match(completed[0], json!({ "toolCallId": "tc1", "kind": "execute", "target": "pnpm test" }));
  let files = events.iter().find(|e| e["type"] == "files.changed").expect("files.changed");
  assert_eq!(files["data"]["paths"], json!(["/repo/a.ts"]));
  let replied = events.iter().find(|e| e["type"] == "permission.replied").unwrap();
  assert_eq!(replied["data"]["outcome"]["optionId"], "allow");
  let ended = events.iter().find(|e| e["type"] == "turn.ended").unwrap();
  assert_eq!((ended["data"]["stop"].as_str(), ended["data"]["usage"]["used"].as_f64()), (Some("end_turn"), Some(1234.0)));
  assert_eq!(events.iter().find(|e| e["type"] == "turn.started").unwrap()["data"]["origin"], "user");
  // Not subscribed: no streamed text, no mode changes
  assert!(!types.iter().any(|t| t.ends_with(".delta") || t.starts_with("model.")), "{types:?}");
  rt.stop_all().await;
}
