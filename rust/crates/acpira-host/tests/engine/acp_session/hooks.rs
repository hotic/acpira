//! Workspace hooks (`.agents/hooks.json`): context on the first prompt, `beforeEdit` on permission requests and on the
//! turn's own edits, `afterTurn` sending its findings back as automatic follow-ups

use std::path::{Path, PathBuf};

use super::*;

/// A git work tree with a hooks file and two node hook scripts that log every payload next to themselves:
/// - before.cjs denies edits of `guarded.txt` until a tool has looked at RULES.md;
/// - after.cjs blocks while a file the turn changed holds "bad\n", naming it with `hook-fix:` (the fake agent's cue);
/// - always.cjs blocks every time, through exit code 2
struct Project {
  _dir: tempfile::TempDir,
  root: PathBuf,
}

const BEFORE: &str = r#"
const fs = require('fs'), path = require('path');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
fs.appendFileSync(path.join(__dirname, 'before.log'), JSON.stringify(input) + '\n');
const guarded = (input.files || []).some(f => f.endsWith('guarded.txt'));
const read = (input.read_files || []).some(r => r.includes('RULES.md'));
if (guarded && !read) {
  process.stdout.write(JSON.stringify({ hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: 'read RULES.md first' } }));
}
"#;

const AFTER: &str = r#"
const fs = require('fs'), path = require('path');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
fs.appendFileSync(path.join(__dirname, 'after.log'), JSON.stringify(input) + '\n');
const bad = (input.turn_files || []).filter(f => fs.existsSync(f) && fs.readFileSync(f, 'utf8') === 'bad\n');
if (bad.length) process.stdout.write(JSON.stringify({ decision: 'block', reason: bad.map(f => `${path.basename(f)} is bad; hook-fix:${f}`).join('\n') }));
"#;

const ALWAYS: &str = r#"
require('fs').readFileSync(0);
process.stderr.write('nope');
process.exit(2);
"#;

fn git(root: &Path, args: &[&str]) {
  let ok = std::process::Command::new("git").arg("-C").arg(root).args(args).output().map(|o| o.status.success());
  assert_eq!(ok.ok(), Some(true), "git {args:?}");
}

impl Project {
  /// `hooks` names scripts by file name; they become `node "<abs path>"` commands
  fn new(hooks: Value) -> Project {
    let dir = tempfile::tempdir().unwrap();
    let root = acpira_host::platform::paths::canonical_for_cli(dir.path()).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("RULES.md"), "Rule one: never write bad files.\n").unwrap();
    std::fs::write(root.join("guarded.txt"), "ok\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let hooks_dir = root.join(".agents");
    std::fs::create_dir_all(&hooks_dir).unwrap();
    for (name, body) in [("before.cjs", BEFORE), ("after.cjs", AFTER), ("always.cjs", ALWAYS)] {
      std::fs::write(hooks_dir.join(name), body).unwrap();
    }
    let command = |v: &Value| -> Value {
      match v {
        Value::String(name) => json!(format!("node \"{}\"", hooks_dir.join(name).display())),
        Value::Object(o) => {
          let mut o = o.clone();
          let name = o["run"].as_str().unwrap().to_owned();
          o.insert("run".into(), json!(format!("node \"{}\"", hooks_dir.join(name).display())));
          Value::Object(o)
        }
        other => other.clone(),
      }
    };
    let mut cfg = hooks.as_object().unwrap().clone();
    for key in ["beforeEdit", "afterTurn"] {
      if let Some(v) = cfg.get(key).cloned() {
        cfg.insert(key.into(), command(&v));
      }
    }
    std::fs::write(hooks_dir.join("hooks.json"), serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
    // The hook scripts and their logs are not the agent's work
    std::fs::write(root.join(".git/info/exclude"), ".agents/\n").unwrap();
    Project { _dir: dir, root }
  }

  fn cwd(&self) -> String {
    self.root.to_string_lossy().into_owned()
  }

  fn file(&self, name: &str) -> String {
    self.root.join(name).to_string_lossy().into_owned()
  }

  fn payloads(&self, log: &str) -> Vec<Value> {
    std::fs::read_to_string(self.root.join(".agents").join(log))
      .unwrap_or_default()
      .lines()
      .map(|l| serde_json::from_str(l).unwrap())
      .collect()
  }
}

fn git_or_skip() -> bool {
  std::process::Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn count(h: &Harness, line: &str) -> usize {
  h.logs().iter().filter(|l| l.contains(line)).count()
}

/// Wait until the gate has decided `n` times in total (passed, gave up, or sent findings back). Generous: every
/// decision starts a node script, and the full suite runs hundreds of agents at once
async fn decisions(h: &Harness, n: usize) {
  let decided = || count(h, "hooks: gate passed") + count(h, "hooks: gate gave up") + count(h, "hooks: gate blocked");
  let t0 = std::time::Instant::now();
  while decided() < n {
    // The session log is what tells a slow gate from one that never ran
    assert!(t0.elapsed() < std::time::Duration::from_secs(60), "{} of {n} gate decisions after 60 s; log:\n{}", decided(), h.logs().join("\n"));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
}

fn notices(view: &Value) -> Vec<Value> {
  agent_blocks(view).into_iter().filter(|b| b["type"] == "notice").collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_prompt_carries_the_context_files_and_later_prompts_do_not() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({ "context": ["RULES.md"] }));
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  prompt(&s, "echo-blocks").await;
  prompt(&s, "echo-blocks").await;
  let texts: Vec<Value> = view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "agent").map(|t| t["blocks"][0]["markdown"].clone()).collect();
  assert_eq!(texts, [json!("resource,text"), json!("text")]);
  // The user's bubble is the message alone: the context rides on the wire only
  expect_absent(&view(&s)["turns"][0], "attachments");
  assert!(h.logs().iter().any(|l| l.contains("hooks: context") && l.contains("RULES.md")), "{:?}", h.logs());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_the_hook_denies_is_rejected_and_its_reason_goes_back_to_the_agent() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({ "beforeEdit": "before.cjs" }));
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  let guarded = p.file("guarded.txt");
  // No card: the gate answers the request with the agent's own reject before a person is asked
  prompt(&s, &format!("hook-perm-write:{guarded}")).await;
  decisions(&h, 2).await;
  assert_eq!(std::fs::read_to_string(&guarded).unwrap(), "ok\n");
  let vw = view(&s);
  let reply = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned();
  assert_eq!(reply.unwrap()["markdown"], "rejected", "{vw}");
  let n = notices(&vw);
  assert_eq!(n.len(), 2, "{n:?}");
  expect_match(&n[0], json!({ "severity": "warning", "details": "read RULES.md first" }));
  assert!(n[0]["title"].as_str().unwrap().contains("guarded.txt"));
  // The rejected edit becomes the gate's finding: one automatic follow-up carries it to the agent
  expect_match(&vw["turns"][2], json!({ "role": "user", "auto": true, "autoReason": "gate" }));
  let follow_up = vw["turns"][2]["text"].as_str().unwrap();
  assert!(follow_up.contains("read RULES.md first") && follow_up.contains("guarded.txt"), "{follow_up}");
  assert_eq!(turns_in(&vw), 4);
  let first = &p.payloads("before.log")[0];
  expect_match(first, json!({ "hook_event_name": "PreToolUse", "tool_name": "Edit", "phase": "permission", "tool_input": { "file_path": guarded } }));
  assert_eq!(first["files"], json!([guarded]));

  // Once a tool has looked at RULES.md the same edit reaches the person as an ordinary card
  prompt(&s, &format!("hook-read:{}", p.file("RULES.md"))).await;
  let pending = spawn_prompt(&s, &format!("hook-perm-write:{guarded}"));
  // The card only appears once beforeEdit (a node process) has answered, which takes longer than wait_block's 5 s when
  // the whole suite runs in parallel
  until(|| has_block(&s, "permission"), 20_000).await;
  let perm = find_block(&view(&s), "permission").unwrap();
  s.resolve_permission(perm["id"].as_str().unwrap(), "allow");
  pending.await.unwrap();
  decisions(&h, 4).await;
  assert_eq!(std::fs::read_to_string(&guarded).unwrap(), "bad\n");
  assert_eq!(view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["autoReason"] == "gate").count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_made_without_asking_is_checked_when_the_turn_ends() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({ "beforeEdit": "before.cjs" }));
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  let guarded = p.file("guarded.txt");
  prompt(&s, &format!("hook-write:{guarded}")).await;
  decisions(&h, 2).await;
  let audits: Vec<Value> = p.payloads("before.log").into_iter().filter(|x| x["phase"] == "audit").collect();
  assert!(!audits.is_empty());
  let files: Vec<String> = audits[0]["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().replace('\\', "/")).collect();
  assert_eq!(files.len(), 1, "the snapshot and the edit row name one file once: {files:?}");
  assert!(files[0].ends_with("guarded.txt"));
  let vw = view(&s);
  expect_match(&vw["turns"][2], json!({ "role": "user", "auto": true, "autoReason": "gate" }));
  assert!(vw["turns"][2]["text"].as_str().unwrap().contains("read RULES.md first"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gate_sends_its_findings_back_until_the_agent_fixes_them() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({ "afterTurn": "after.cjs" }));
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  let file = p.file("a.txt");
  prompt(&s, &format!("hook-write:{file}")).await;
  decisions(&h, 2).await;
  assert_eq!(std::fs::read_to_string(&file).unwrap(), "fixed\n");
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 4, "{vw}");
  expect_match(&vw["turns"][2], json!({ "role": "user", "auto": true, "autoReason": "gate" }));
  assert_eq!(vw["turns"][3]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").unwrap()["markdown"], "fixed");
  let n = notices(&vw);
  assert_eq!(n.len(), 1);
  assert!(n[0]["details"].as_str().unwrap().contains("a.txt is bad"));
  let stops = p.payloads("after.log");
  assert_eq!(stops.len(), 2);
  expect_match(&stops[0], json!({ "hook_event_name": "Stop", "stop_hook_active": false, "round": 0 }));
  expect_match(&stops[1], json!({ "stop_hook_active": true, "round": 1 }));
  for stop in &stops {
    let turn: Vec<String> = stop["turn_files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().replace('\\', "/")).collect();
    assert_eq!(turn.len(), 1, "{turn:?}");
    assert!(turn[0].ends_with("/a.txt"));
  }
  assert_eq!(stops[1]["changed_files"].as_array().unwrap().len(), 1);
  // A new user prompt starts the rounds over
  prompt(&s, &format!("hook-write:{file}")).await;
  decisions(&h, 4).await;
  expect_match(&p.payloads("after.log")[2], json!({ "stop_hook_active": false, "round": 0 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gate_gives_up_after_its_rounds_and_says_so() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({ "afterTurn": { "run": "always.cjs", "rounds": 1 } }));
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  prompt(&s, "hello").await;
  until(|| count(&h, "hooks: gate gave up") == 1, 20_000).await;
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 4, "{vw}");
  let n = notices(&vw);
  expect_match(&n[0], json!({ "severity": "warning", "details": "nope" }));
  expect_match(&n[1], json!({ "severity": "error", "details": "nope" }));
  assert!(!s.is_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_while_the_gate_decides_waits_behind_its_follow_up() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({ "afterTurn": "after.cjs" }));
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  let file = p.file("a.txt");
  prompt(&s, &format!("hook-write:{file}")).await;
  // The first turn has ended and the gate is still running its script: this one queues
  prompt(&s, "say:queued").await;
  decisions(&h, 3).await;
  until(|| !s.is_running() && turns_in(&view(&s)) == 6, 20_000).await;
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["autoReason"].clone()).collect();
  assert_eq!(users, [Value::Null, json!("gate"), Value::Null], "{vw}");
  assert_eq!(vw["turns"][5]["blocks"][0]["markdown"], "queued");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_hooks_file_leaves_the_session_working_and_is_logged() {
  let fake = fake_or_skip!();
  if !git_or_skip() {
    return;
  }
  let p = Project::new(json!({}));
  std::fs::write(p.root.join(".agents/hooks.json"), r#"{ "afterturn": "x" }"#).unwrap();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, &p.cwd()).await;
  prompt(&s, "say:fine").await;
  prompt(&s, "say:still").await;
  assert_eq!(turns_in(&view(&s)), 4);
  assert_eq!(count(&h, "afterturn"), 2, "logged each turn: {:?}", h.logs());
}
