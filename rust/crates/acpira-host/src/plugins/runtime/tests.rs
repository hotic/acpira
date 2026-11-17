use super::*;
use super::kv::{KV_MAX_BYTES, Kv};
use crate::plugins::manifest;
use chrono::TimeZone;

// A resident plugin in node: answers initialize (narrowing its hooks to those it handles), echoes `hook/prompt` with
// its version and a stored counter, sleeps through `hook/permission`, exits on `hook/tool.before`, writes every event
// it hears to events.ndjson in its data folder, and exits when stdin closes
const PLUGIN: &str = r#"
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let next = 1000, data = '';
const waits = new Map();
const ask = (method, params) => new Promise(r => { const id = next++; waits.set(id, r); send({ jsonrpc: '2.0', id, method, params }); });
const rl = createInterface({ input: process.stdin });
rl.on('close', () => process.exit(0));
rl.on('line', async line => {
  const m = JSON.parse(line);
  if (m.id !== undefined && !m.method) { waits.get(m.id)?.(m.result); return; }
  if (m.method === 'event') { appendFileSync(data + '/events.ndjson', JSON.stringify(m.params) + '\n'); return; }
  if (m.method === 'shutdown') return;
  if (m.method === 'initialize') {
    data = m.params.plugin.data;
    const { mkdirSync } = await import('node:fs'); mkdirSync(data, { recursive: true });
    return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: Number(process.env.PROTO ?? 1), hooks: ['prompt', 'permission', 'tool.before'] } });
  }
  if (m.method === 'hook/prompt') {
    const n = ((await ask('storage/get', { key: 'n' })).value ?? 0) + 1;
    await ask('storage/set', { key: 'n', value: n });
    return send({ jsonrpc: '2.0', id: m.id, result: { ...m.params, version: process.env.VERSION ?? 'v1', n, pid: process.pid } });
  }
  if (m.method === 'hook/permission') return setTimeout(() => send({ jsonrpc: '2.0', id: m.id, result: {} }), 5000);
  if (m.method === 'hook/tool.before') process.exit(3);
  send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
"#;

fn active(dir: &Path, hash: &str, extra: &str) -> Arc<Active> {
  let json = format!(
    r#"{{ "schema": 1, "id": "rt", "version": "1.0.0", "description": "d", "contributes": {{ "runtime": {{ "command": ["node", "plugin.mjs"], "hooks": ["prompt", "permission", "tool.before", "title"], "events": ["session.*"], "timeout": 0.5 {extra} }} }} }}"#
  );
  let (m, _) = manifest::parse(json.as_bytes()).unwrap();
  Arc::new(Active { id: "rt".into(), hash: hash.into(), dir: dir.to_path_buf(), manifest: m })
}

fn alive(pid: u64) -> bool {
  // SAFETY: signal 0 only checks that the pid exists
  unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

async fn until(f: impl Fn() -> bool, ms: u64) {
  let t = Instant::now();
  while !f() {
    assert!(t.elapsed() < Duration::from_millis(ms), "timed out");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

fn setup() -> Option<(tempfile::TempDir, Arc<Runtimes>)> {
  if !crate::plugins::host::on_path("node") {
    return None;
  }
  let root = tempfile::tempdir().unwrap();
  for v in ["a", "b"] {
    let d = root.path().join(v);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("plugin.mjs"), PLUGIN).unwrap();
  }
  let rt = Runtimes::new(root.path().join(".data"), Arc::new(|_: &str| {}));
  rt.enable(None);
  Some((root, rt))
}

// Offers `rates.get`; on `rpc/relay` it calls another plugin's method through the engine and returns the answer
const RPC: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let next = 1000;
const waits = new Map();
const ask = (method, params) => new Promise(r => { const id = next++; waits.set(id, r); send({ jsonrpc: '2.0', id, method, params }); });
createInterface({ input: process.stdin }).on('close', () => process.exit(0)).on('line', async line => {
  const m = JSON.parse(line);
  if (m.id !== undefined && !m.method) { waits.get(m.id)?.(m.error ? { error: m.error.message } : m.result); return; }
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'rpc/rates.get') return send({ jsonrpc: '2.0', id: m.id, result: { eur: 0.92, from: m.params.from, asked: m.params.params } });
  if (m.method === 'rpc/relay') return send({ jsonrpc: '2.0', id: m.id, result: await ask('plugin/call', m.params.params) });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
"#;

#[tokio::test]
async fn a_call_made_while_the_process_starts_waits_for_it() {
  let Some((root, rt)) = setup() else { return };
  let dir = root.path().join("rates");
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("rpc.mjs"), RPC).unwrap();
  let json = r#"{ "schema": 1, "id": "rates", "version": "1.0.0", "description": "d", "contributes": { "runtime": { "command": ["node", "rpc.mjs"], "methods": ["rates.get"] } } }"#;
  let (m, _) = manifest::parse(json.as_bytes()).unwrap();
  let active = Arc::new(Active { id: "rates".into(), hash: "h".into(), dir, manifest: m });
  let syncing = { let rt = rt.clone(); tokio::spawn(async move { rt.sync(&[active]).await }) };
  while !rt.wanted.lock().iter().any(|w| w.id == "rates") {
    tokio::task::yield_now().await;
  }
  // Asked before the process answered initialize (a page frame opened with its plugin)
  let page = rt.rpc("rates", "rates.get", json!({}), "page").await.unwrap();
  assert_eq!(page["eur"], json!(0.92));
  syncing.await.unwrap();
  // A plugin nobody wants is refused at once
  assert!(rt.rpc("gone", "x", json!({}), "page").await.unwrap_err().contains("no running process"));
}

#[tokio::test]
async fn a_runtime_offers_its_declared_methods_to_pages_and_other_plugins() {
  let Some((root, rt)) = setup() else { return };
  let plugin = |id: &str, methods: &str, deps: &str| {
    let dir = root.path().join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rpc.mjs"), RPC).unwrap();
    let json = format!(r#"{{ "schema": 1, "id": "{id}", "version": "1.0.0", "description": "d", "dependencies": {deps}, "contributes": {{ "runtime": {{ "command": ["node", "rpc.mjs"], "methods": {methods} }} }} }}"#);
    let (m, mut f) = manifest::parse(json.as_bytes()).unwrap();
    manifest::check(&m, id, &dir, &|p| p == "node", &mut f);
    assert!(f.ok(), "{f:?}");
    Arc::new(Active { id: id.into(), hash: "h".into(), dir, manifest: m })
  };
  rt.sync(&[plugin("rates", r#"["rates.get"]"#, "{}"), plugin("caller", r#"["relay"]"#, r#"{ "rates": "^1" }"#), plugin("loner", r#"["relay"]"#, "{}")]).await;
  let page = rt.rpc("rates", "rates.get", json!({ "to": "EUR" }), "page").await.unwrap();
  assert_eq!(page, json!({ "eur": 0.92, "from": "page", "asked": { "to": "EUR" } }));
  // Through the engine from another plugin, which is named as the caller
  let relayed = rt.rpc("caller", "relay", json!({ "plugin": "rates", "method": "rates.get", "params": 1 }), "page").await.unwrap();
  assert_eq!(relayed, json!({ "eur": 0.92, "from": "caller", "asked": 1 }));
  // Only declared methods, only running plugins
  assert!(rt.rpc("rates", "relay", json!({}), "page").await.unwrap_err().contains("no method `relay`"));
  let refused = rt.rpc("caller", "relay", json!({ "plugin": "rates", "method": "secret" }), "page").await.unwrap();
  assert!(refused["error"].as_str().is_some_and(|e| e.contains("no method `secret`")), "{refused}");
  assert!(rt.rpc("gone", "x", json!({}), "page").await.unwrap_err().contains("no running process"));
  // A plugin calls only what its manifest lists in `dependencies`
  let stranger = rt.rpc("loner", "relay", json!({ "plugin": "rates", "method": "rates.get" }), "page").await.unwrap();
  assert!(stranger["error"].as_str().is_some_and(|e| e.contains("does not list `rates`")), "{stranger}");
  rt.stop_all().await;
}

#[tokio::test]
async fn a_scaffolded_runtime_answers_hooks_events_and_methods_through_the_sdk() {
  let Some((root, rt)) = setup() else { return };
  let mut langs = vec![("kit", "node")];
  if crate::plugins::host::on_path("python3") {
    langs.push(("py-kit", "python"));
  }
  let mut active = vec![];
  for (id, lang) in &langs {
    let dir = crate::plugins::sdk::scaffold(root.path(), id, &["runtime"], lang).unwrap();
    let (m, _) = manifest::parse(&std::fs::read(dir.join(manifest::FILE)).unwrap()).unwrap();
    active.push(Arc::new(Active { id: (*id).into(), hash: "h".into(), dir, manifest: m }));
  }
  rt.sync(&active).await;
  assert_eq!(rt.running().len(), langs.len(), "{:?}", rt.errors.read());
  let ctx = |text: &str| json!({ "session": "s", "agent": "a", "cwd": "/", "text": text });
  for r in rt.hook("prompt", &ctx("fix the TODO")).await {
    assert_eq!(r.result.as_ref().unwrap(), &json!({ "context": "List the TODOs you leave behind." }), "{}", r.plugin);
  }
  for r in rt.hook("prompt", &ctx("hello")).await {
    assert_eq!(r.result.as_ref().unwrap(), &Value::Null, "{}", r.plugin);
  }
  rt.emit("turn.ended", || json!({ "session": "s" }));
  rt.emit("turn.started", || json!({ "session": "s" }));
  for (id, _) in &langs {
    let ping = || rt.rpc(id, "ping", json!({}), "page");
    let t = Instant::now();
    while ping().await.unwrap()["turns"] != 1 {
      assert!(t.elapsed() < Duration::from_secs(5), "{id}: the event never counted");
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(ping().await.unwrap(), json!({ "pong": true, "turns": 1 }), "{id}");
  }
  rt.stop_all().await;
}

#[tokio::test]
async fn a_runtime_answers_only_what_it_declared_and_keeps_its_store() {
  let Some((root, rt)) = setup() else { return };
  rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
  assert_eq!(rt.running(), ["rt"]);
  // Declared and handled; declared but narrowed away by initialize; never declared
  assert!(rt.has_hook("prompt") && !rt.has_hook("title") && !rt.has_hook("context"));
  let r = rt.hook("prompt", &json!({ "text": "hi" })).await;
  let v = r[0].result.as_ref().unwrap();
  assert_eq!((v["text"].as_str(), v["n"].as_u64()), (Some("hi"), Some(1)));
  assert_eq!(rt.hook("prompt", &json!({})).await[0].result.as_ref().unwrap()["n"], 2);
  let kv: Value = serde_json::from_slice(&std::fs::read(root.path().join(".data/rt.kv.json")).unwrap()).unwrap();
  assert_eq!(kv, json!({ "n": 2 }));
  // Events: only the families it subscribed to, built only when someone hears them
  assert!(rt.hears("session.created") && !rt.hears("turn.ended") && !rt.hears("text.delta"));
  rt.emit("turn.ended", || panic!("built for no one"));
  rt.emit("session.created", || json!({ "id": "s1" }));
  let events = root.path().join(".data/rt/events.ndjson");
  until(|| events.exists(), 3000).await;
  let ev: Value = serde_json::from_str(std::fs::read_to_string(&events).unwrap().lines().next().unwrap()).unwrap();
  assert_eq!((ev["type"].as_str(), ev["data"]["id"].as_str()), (Some("session.created"), Some("s1")));
  rt.stop_all().await;
}

#[tokio::test]
async fn a_new_version_takes_over_before_the_old_one_stops_and_nothing_runs_once_it_is_gone() {
  let Some((root, rt)) = setup() else { return };
  rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
  let old = rt.hook("prompt", &json!({})).await[0].result.as_ref().unwrap()["pid"].as_u64().unwrap();
  std::fs::write(root.path().join("b/plugin.mjs"), PLUGIN.replace("process.env.VERSION ?? 'v1'", "'v2'")).unwrap();
  rt.sync(&[active(&root.path().join("b"), "h2", "")]).await;
  let v = rt.hook("prompt", &json!({})).await[0].result.clone().unwrap();
  assert_eq!(v["version"], "v2");
  let new = v["pid"].as_u64().unwrap();
  assert_ne!(old, new);
  until(|| !alive(old), 3000).await;
  // The store outlives the version
  assert_eq!(v["n"], 2);
  rt.sync(&[]).await;
  assert!(rt.running().is_empty());
  until(|| !alive(new), 3000).await;
}

#[tokio::test]
async fn a_new_version_that_does_not_start_leaves_the_running_one_serving() {
  let Some((root, rt)) = setup() else { return };
  rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
  let old = rt.hook("prompt", &json!({})).await[0].result.as_ref().unwrap()["pid"].as_u64().unwrap();
  // v2 speaks another protocol, so its initialize is refused
  std::fs::write(root.path().join("b/plugin.mjs"), PLUGIN.replace("Number(process.env.PROTO ?? 1)", "2")).unwrap();
  rt.sync(&[active(&root.path().join("b"), "h2", "")]).await;
  let v = rt.hook("prompt", &json!({})).await[0].result.clone().unwrap();
  assert_eq!((v["version"].as_str(), v["pid"].as_u64()), (Some("v1"), Some(old)));
  assert!(rt.errors.read()["rt"].contains("previous one keeps running"));
  rt.stop_all().await;
}

#[tokio::test]
async fn two_stores_on_one_file_keep_each_others_keys() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("p.kv.json");
  // Two engines: each its own store over the same file
  let (a, b) = (Kv::new(path.clone()), Kv::new(path.clone()));
  assert_eq!(b.get("x"), Value::Null);
  a.change(|m| {
    m.insert("x".into(), json!(1));
  })
  .await
  .unwrap();
  b.change(|m| {
    m.insert("y".into(), json!(2));
  })
  .await
  .unwrap();
  assert_eq!((a.get("x"), a.get("y"), b.get("x")), (json!(1), json!(2), json!(1)));
  let big = "x".repeat(KV_MAX_BYTES);
  assert!(a.change(move |m| { m.insert("big".into(), json!(big)); }).await.unwrap_err().contains("more than"));
  assert_eq!(std::fs::read_dir(dir.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("tmp")).count(), 0);
}

#[tokio::test]
async fn slow_calls_pause_the_plugin_and_a_crash_starts_it_again() {
  let Some((root, rt)) = setup() else { return };
  rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
  for _ in 0..TRIP_AFTER {
    let e = rt.hook("permission", &json!({})).await[0].result.clone().unwrap_err();
    assert!(e.contains("timed out"), "{e}");
  }
  let e = rt.hook("prompt", &json!({})).await[0].result.clone().unwrap_err();
  assert!(e.contains("paused"), "{e}");
  assert!(rt.error("rt").unwrap().contains("paused"));
  // A crash: gone at once, back after the first pause
  rt.workers.read().get("rt").unwrap().tripped.lock().take();
  assert!(rt.hook("tool.before", &json!({})).await[0].result.is_err());
  until(|| rt.running().is_empty(), 3000).await;
  assert!(rt.error("rt").unwrap().contains("exited"));
  until(|| !rt.running().is_empty(), 5000).await;
  assert!(rt.hook("prompt", &json!({})).await[0].result.is_ok());
  assert!(rt.error("rt").is_none());
  rt.stop_all().await;
}

// A plugin written as a TypeScript definition: compiled by the engine, run with Node through the SDK's `start`
const DEFINED: &str = r#"import { definePlugin } from '@acpira/plugin';
import { label } from './label';

type Ctx = { storage: { get(k: string): Promise<unknown>; set(k: string, v: unknown): Promise<unknown> } };

let active = false;
export default definePlugin({
  async activate(ctx: Ctx) {
    active = true;
    await ctx.storage.set('activated', true);
    return () => { active = false; };
  },
  migrate: async (from: string, to: string, ctx: Ctx) => ctx.storage.set('migrated', `${from}->${to}`),
  hooks: { prompt: ({ text }: { text: string }) => (text.includes('deploy') ? { context: label('ship') } : null) },
  methods: {
    ping: async (_p: unknown, ctx: Ctx) => ({ active, migrated: await ctx.storage.get('migrated') }),
  },
  tools: { stat: async (args: { n?: number }) => `n=${args.n ?? 0}` },
  schedules: { daily: async (ctx: { due: number }) => ({ ran: ctx.due }) },
  commands: { review: (args: string) => `Review ${args || 'everything'}` },
});
"#;

#[tokio::test]
async fn a_typescript_definition_runs_compiled_through_the_sdk() {
  let Some((root, rt)) = setup() else { return };
  let dir = root.path().join("defined");
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("plugin.ts"), DEFINED).unwrap();
  std::fs::write(dir.join("label.ts"), "export const label = (s: string): string => `Use make ${s}.`;\n").unwrap();
  let manifest_for = |version: &str| {
    let json = format!(
      r#"{{ "schema": 1, "id": "defined", "version": "{version}", "description": "d",
          "contributes": {{
            "runtime": {{ "main": "plugin.ts", "hooks": ["prompt"], "methods": ["ping"] }},
            "tools": [{{ "name": "stat", "description": "Stat", "input": {{ "type": "object" }} }}],
            "schedules": [{{ "id": "daily", "cron": "0 9 * * *" }}],
            "commands": [{{ "name": "review", "description": "Review" }}]
          }} }}"#
    );
    let (m, mut f) = manifest::parse(json.as_bytes()).unwrap();
    manifest::check(&m, "defined", &dir, &|p| p == "node", &mut f);
    assert!(f.ok(), "{f:?}");
    m
  };
  // A first version, then a second one, so `migrate` is told where it came from
  for (hash, version) in [("v1", "1.0.0"), ("v2", "1.1.0")] {
    rt.sync(&[Arc::new(Active { id: "defined".into(), hash: hash.into(), dir: dir.clone(), manifest: manifest_for(version) })]).await;
    assert_eq!(rt.running().len(), 1, "{:?}", rt.errors.read());
  }
  assert!(root.path().join(crate::plugins::build::BUILD).join("defined").is_dir(), "compiled under the plugins root");
  let ping = rt.rpc("defined", "ping", json!(null), "page").await.unwrap();
  assert_eq!(ping, json!({ "active": true, "migrated": "1.0.0->1.1.0" }));
  let replies = rt.hook("prompt", &json!({ "text": "deploy it" })).await;
  assert_eq!(replies[0].result.as_ref().unwrap(), &json!({ "context": "Use make ship." }));
  let ctx = crate::plugins::run::CallContext::default();
  assert_eq!(rt.tool("defined", "stat", &json!({ "n": 3 }), &ctx).await.unwrap(), "n=3");
  assert_eq!(rt.run_schedule("defined", "daily", 42).await.unwrap(), json!({ "ran": 42 }));
  // The loop: at 9:00 the day's run starts once; the next morning at 9:30 the missed 9:00 catches up
  let records = Arc::new(crate::plugins::schedule::Records::new(root.path()));
  let log: LogFn = Arc::new(|_: &str| {});
  let nine = chrono::Local.with_ymd_and_hms(2026, 3, 4, 9, 0, 5).earliest().unwrap();
  for (now, runs) in [(nine, 1), (nine, 0), (nine + chrono::Duration::minutes(24 * 60 + 30), 1)] {
    let started = crate::plugins::schedule::tick(&rt, &records, now, &log).await;
    assert_eq!(started.len(), runs, "at {now}");
    for h in started {
      h.await.unwrap();
    }
  }
  let r = records.read().remove(&crate::plugins::schedule::key("defined", "daily")).unwrap();
  assert_eq!((r.last_due, r.error, r.failures, r.running_until), (Some((nine + chrono::Duration::days(1)).timestamp_millis() - 5000), None, 0, None));
  // A command without a prompt is answered in code, though the manifest does not list the `command` hook
  let answer = rt.hook_of("defined", "command", json!({ "name": "review", "arguments": "the diff" })).await.unwrap().unwrap();
  assert_eq!(answer, json!({ "text": "Review the diff" }));
  rt.stop_all().await;
}

#[tokio::test]
async fn two_engines_starting_a_new_version_migrate_its_data_once() {
  let Some((root, rt)) = setup() else { return };
  let dir = root.path().join("mig");
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(
    dir.join("plugin.ts"),
    "import { definePlugin } from '@acpira/plugin';\nexport default definePlugin({\n  migrate: async (_from: string, _to: string, ctx: any) => {\n    await new Promise(r => setTimeout(r, 300));\n    await ctx.storage.set('ran-' + process.pid, true);\n  },\n});\n",
  )
  .unwrap();
  let active = |hash: &str, version: &str| {
    let json = format!(r#"{{ "schema": 1, "id": "mig", "version": "{version}", "description": "d", "contributes": {{ "runtime": {{ "main": "plugin.ts" }} }} }}"#);
    Arc::new(Active { id: "mig".into(), hash: hash.into(), dir: dir.clone(), manifest: manifest::parse(json.as_bytes()).unwrap().0 })
  };
  rt.sync(&[active("v1", "1.0.0")]).await;
  rt.stop_all().await;
  // A second engine on the same data folder
  let (a, b) = (Runtimes::new(root.path().join(".data"), Arc::new(|_: &str| {})), Runtimes::new(root.path().join(".data"), Arc::new(|_: &str| {})));
  a.enable(None);
  b.enable(None);
  let v2 = [active("v2", "2.0.0")];
  tokio::join!(a.sync(&v2), b.sync(&v2));
  assert_eq!((a.running().len(), b.running().len()), (1, 1), "{:?} {:?}", a.errors.read(), b.errors.read());
  let kv: Value = serde_json::from_slice(&std::fs::read(root.path().join(".data/mig.kv.json")).unwrap()).unwrap();
  let runs = kv.as_object().unwrap().keys().filter(|k| k.starts_with("ran-")).count();
  assert_eq!((runs, kv["__acpira.version"].clone(), kv.get("__acpira.migrating")), (1, json!("2.0.0"), None), "{kv}");
  a.stop_all().await;
  b.stop_all().await;
}

#[tokio::test]
async fn the_check_refuses_a_runtime_that_speaks_another_protocol() {
  let Some((root, _rt)) = setup() else { return };
  let a = active(&root.path().join("a"), "h1", "");
  let with = |command: Option<Vec<String>>| {
    let mut m = a.manifest.clone();
    if let (Some(rt), Some(c)) = (m.contributes.runtime.as_mut(), command) {
      rt.command = c;
    }
    m
  };
  probe("rt", "h1", &a.dir, &with(None), &root.path().join(".data"), Arc::new(|_: &str| {})).await.unwrap();
  let bad = with(Some(vec!["sh".into(), "-c".into(), "PROTO=2 exec node plugin.mjs".into()]));
  let e = probe("rt", "h1", &a.dir, &bad, &root.path().join(".data"), Arc::new(|_: &str| {})).await.unwrap_err();
  assert!(e.contains("protocol 2"), "{e}");
  let silent = with(Some(vec!["sleep".into(), "30".into()]));
  let t = Instant::now();
  let e = probe("rt", "h1", &a.dir, &silent, &root.path().join(".data"), Arc::new(|_: &str| {})).await.unwrap_err();
  assert!(e.contains("no answer") && t.elapsed() < INIT_TIMEOUT + Duration::from_secs(4), "{e}");
}
