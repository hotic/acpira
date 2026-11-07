//! Claude Code's own split of its context, the one its `/context` shows: the Agent SDK's `getContextUsage` control
//! request (`@anthropic-ai/claude-agent-sdk` under claude-agent-acp 0.84.0, Claude Code 2.1.299). claude-agent-acp keeps
//! the query that could answer it to itself, so a small node script resumes a fork of the session with the adapter's
//! config directory, setting sources and presets and asks with `detail: "summary"`: local estimates and the last
//! response's usage, no model call, and a fork that is sent nothing writes no session file. `detail: "full"` counts
//! every part through the API (billable `max_tokens: 1` calls on gateway logins) and is never asked.
//!
//! Its categories become the agent's parts (system prompt, built-in tools, MCP tools and server instructions, custom
//! agents, memory files, skills), memory files, skills and loaded MCP tools as items. Its messages are left to the log
//! (`context_claude.rs`): the last request there is newer, and the conversation is what the parts leave of it. What the
//! fork cannot see lands in the conversation: the MCP servers Acpira hands the adapter (deferred behind tool search by
//! default, so only their names and instructions reach the context) and a model switched in the session (the system
//! prompt differs by a few tokens).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use acpira_shared::context::{ContextCategory, ContextNode, ContextNodeKind};
use acpira_shared::transcript::StrMap;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use crate::acp::session::context_estimate::{DOC_PREFIX, Known};

/// One line, so a `.cmd` node shim run through cmd.exe passes it whole. argv ends in the SDK directory, the session id
/// and its cwd; the never-ending prompt keeps the fork from sending anything
const SCRIPT: &str = "import { pathToFileURL } from 'node:url'; import { join } from 'node:path'; \
  const [sdk, resume, cwd] = process.argv.slice(-3); \
  const { query } = await import(pathToFileURL(join(sdk, 'sdk.mjs')).href); \
  const exe = process.env.CLAUDE_CODE_EXECUTABLE; \
  const q = query({ prompt: (async function* () { await new Promise(() => {}); })(), options: { resume, forkSession: true, cwd, \
  settingSources: ['user', 'project', 'local'], systemPrompt: { type: 'preset', preset: 'claude_code' }, \
  tools: { type: 'preset', preset: 'claude_code' }, ...(exe ? { pathToClaudeCodeExecutable: exe } : {}) } }); \
  process.stdout.write(JSON.stringify(await q.getContextUsage({ detail: 'summary' }))); process.exit(0);";
const TIMEOUT: Duration = Duration::from_secs(20);
/// The answer is cut off here (a session with hundreds of skills stays far below)
const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
/// A split this old is asked again in the background (a skill or memory file changed), the old one shown meanwhile
const FRESH: Duration = Duration::from_secs(5 * 60);
/// The first ask waits this long before the report goes out with the log's split alone
const FIRST_WAIT: Duration = Duration::from_secs(12);

/// What resolves the probe's `Launch` (in the background, after the caller returned)
pub type LaunchFuture = Pin<Box<dyn Future<Output = Option<Launch>> + Send>>;
type SplitFuture = Pin<Box<dyn Future<Output = Option<Split>> + Send>>;

/// What runs the probe: node, the SDK package directory, the agent's env on top of the engine's
pub struct Launch {
  pub node: String,
  pub sdk: PathBuf,
  pub env: StrMap,
  pub cwd: String,
  pub session_id: String,
}

/// The agent's parts from Claude Code's categories, and what memory files, skills and MCP tools each holds
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Split {
  pub parts: Vec<(ContextCategory, u64)>,
  pub items: Vec<(ContextCategory, Vec<ContextNode>)>,
}

fn leaf(id: String, kind: ContextNodeKind, name: Option<String>, tokens: u64) -> ContextNode {
  ContextNode { id, kind, name, title: None, index: None, tool_kind: None, tokens, cached: None, children: None, call: None }
}

fn n(v: &Value, k: &str) -> u64 {
  v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn s(v: &Value, k: &str) -> Option<String> {
  v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned)
}

/// `getContextUsage` as parts and items; none when it names no part of its own
pub fn split_of(v: &Value) -> Option<Split> {
  let mut out = Split::default();
  let mut add = |cat: ContextCategory, tokens: u64| match out.parts.iter_mut().find(|p| p.0 == cat) {
    Some(p) => p.1 += tokens,
    None if tokens > 0 => out.parts.push((cat, tokens)),
    None => {}
  };
  for c in v.get("categories").and_then(Value::as_array).into_iter().flatten() {
    // Deferred tools are only named in the conversation, the buffer and free space are no context
    if c.get("isDeferred").and_then(Value::as_bool) == Some(true) || c.get("kind").and_then(Value::as_str).is_some_and(|k| k != "used") {
      continue;
    }
    let cat = match c.get("name").and_then(Value::as_str).unwrap_or_default() {
      "Messages" => continue,
      "System prompt" => ContextCategory::System,
      "System tools" => ContextCategory::Tools,
      "MCP tools" | "MCP server instructions" => ContextCategory::Mcp,
      "Memory files" => ContextCategory::Docs,
      "Skills" => ContextCategory::Skills,
      // Custom agents, and whatever a later Claude Code adds
      _ => ContextCategory::Agent,
    };
    add(cat, n(c, "tokens"));
  }

  let docs: Vec<ContextNode> = v
    .get("memoryFiles")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .enumerate()
    .filter_map(|(i, f)| Some(leaf(format!("{DOC_PREFIX}{i}"), ContextNodeKind::Doc, Some(s(f, "path")?), n(f, "tokens"))))
    .collect();
  let skills: Vec<ContextNode> = v
    .pointer("/skills/skillFrontmatter")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|f| {
      let name = s(f, "name")?;
      Some(leaf(format!("skill:{name}"), ContextNodeKind::Skill, Some(name), n(f, "tokens")))
    })
    .collect();
  // Loaded MCP tools under their server, named without the `mcp__<server>__` prefix
  let mut servers: Vec<ContextNode> = vec![];
  for t in v.get("mcpTools").and_then(Value::as_array).into_iter().flatten() {
    if t.get("isLoaded").and_then(Value::as_bool) == Some(false) {
      continue;
    }
    let (Some(name), Some(server)) = (s(t, "name"), s(t, "serverName")) else { continue };
    let short = name.strip_prefix(&format!("mcp__{server}__")).unwrap_or(&name).to_owned();
    let tool = leaf(format!("mcp:{name}"), ContextNodeKind::McpTool, Some(short), n(t, "tokens"));
    let id = format!("srv:{server}");
    match servers.iter_mut().find(|g| g.id == id) {
      Some(g) => {
        g.tokens += tool.tokens;
        g.children.get_or_insert_with(Vec::new).push(tool);
      }
      None => servers.push(ContextNode { tokens: tool.tokens, children: Some(vec![tool]), ..leaf(id, ContextNodeKind::Server, Some(server), 0) }),
    }
  }
  for (cat, nodes) in [(ContextCategory::Docs, docs), (ContextCategory::Skills, skills), (ContextCategory::Mcp, servers)] {
    if nodes.is_empty() {
      continue;
    }
    // A part is never smaller than what it names
    let named: u64 = nodes.iter().map(|x| x.tokens).sum();
    match out.parts.iter_mut().find(|p| p.0 == cat) {
      Some(p) => p.1 = p.1.max(named),
      None if named > 0 => out.parts.push((cat, named)),
      None => {}
    }
    out.items.push((cat, nodes));
  }
  (!out.parts.is_empty()).then_some(out)
}

/// The log's split with the agent's own part split as Claude Code splits it; the conversation is what the parts leave
/// of the log's last request
pub fn merge(log: Known, split: &Split) -> Known {
  let last = log.parts.iter().map(|p| p.1).sum::<u64>() + log.chat;
  let own: u64 = split.parts.iter().map(|p| p.1).sum();
  Known { parts: split.parts.clone(), items: split.items.clone(), chat: last.saturating_sub(own), ..log }
}

/// Run the probe; none when it fails, times out or answers nothing usable
async fn probe(l: Launch) -> Option<Split> {
  let args: Vec<String> = vec!["--input-type=module".into(), "-e".into(), SCRIPT.into(), l.sdk.to_string_lossy().into_owned(), l.session_id, l.cwd.clone()];
  let mut cmd = crate::acp::agents::launch::command(&l.node, &args);
  cmd.current_dir(&l.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
  // As the adapter runs (`AgentProcess::spawn`): workflows on unless set, then the agent's own env
  if std::env::var_os(crate::acp::vendors::claude_workflow::WORKFLOWS_ENV).is_none() {
    cmd.env(crate::acp::vendors::claude_workflow::WORKFLOWS_ENV, "1");
  }
  for (k, v) in &l.env {
    cmd.env(k, v);
  }
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  let (mut child, _job) = crate::platform::windows_process::spawn(&mut cmd).await.ok()?;
  #[cfg(not(windows))]
  let mut child = cmd.spawn().ok()?;
  #[cfg(unix)]
  let mut group = crate::acp::vendors::claude_auth::ProbeGroup(child.id());
  let mut stdout = child.stdout.take()?.take(OUTPUT_LIMIT + 1);
  let mut out = Vec::new();
  tokio::time::timeout(TIMEOUT, async {
    stdout.read_to_end(&mut out).await.ok()?;
    child.wait().await.ok()?;
    #[cfg(unix)]
    {
      group.0 = None;
    }
    Some(())
  })
  .await
  .ok()??;
  if out.len() as u64 > OUTPUT_LIMIT {
    return None;
  }
  split_of(&serde_json::from_slice(&out).ok()?)
}

struct Entry {
  /// The log's own part when asked: a compaction changes it and asks again
  own: u64,
  at: Instant,
  split: Option<Split>,
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, Entry>>> = LazyLock::new(Default::default);
/// A probe on its way: none until it answers, then what it answered
type Pending = watch::Receiver<Option<Option<Split>>>;
static RUNNING: LazyLock<Mutex<HashMap<PathBuf, Pending>>> = LazyLock::new(Default::default);

/// The log's split refined by Claude Code's own (`log` is the session log's, keyed by its path). A split still fresh
/// is reused; an old one is shown while a new one is asked in the background; the first one is waited for a while.
/// `launch` gives what runs the probe, or none when it cannot run (no node, no SDK): the log's split stands then
pub async fn refine(key: PathBuf, log: Known, launch: impl FnOnce() -> LaunchFuture) -> Known {
  refine_with(key, log, launch, FRESH, FIRST_WAIT, |l| Box::pin(probe(l))).await
}

async fn refine_with(
  key: PathBuf,
  log: Known,
  launch: impl FnOnce() -> LaunchFuture,
  fresh: Duration,
  wait: Duration,
  run: impl FnOnce(Launch) -> SplitFuture + Send + 'static,
) -> Known {
  let own = log.parts.iter().map(|p| p.1).sum::<u64>();
  let shown = {
    let cache = CACHE.lock().ok();
    let entry = cache.as_ref().and_then(|c| c.get(&key));
    if let Some(e) = entry.filter(|e| e.own == own && e.at.elapsed() < fresh) {
      return e.split.as_ref().map_or(log.clone(), |s| merge(log.clone(), s));
    }
    // A compaction since: the old split still names the same files and skills, shown until the new one comes
    entry.and_then(|e| e.split.clone())
  };
  let pending = RUNNING.lock().ok().and_then(|r| r.get(&key).cloned());
  let mut rx = match pending {
    Some(rx) => rx,
    None => {
      let (tx, rx) = watch::channel(None);
      if let Ok(mut r) = RUNNING.lock() {
        r.insert(key.clone(), rx.clone());
      }
      let ready = launch();
      let k = key.clone();
      tokio::spawn(async move {
        let split = match ready.await {
          Some(l) => run(l).await,
          None => None,
        };
        if let Ok(mut c) = CACHE.lock() {
          c.insert(k.clone(), Entry { own, at: Instant::now(), split: split.clone() });
        }
        if let Ok(mut r) = RUNNING.lock() {
          r.remove(&k);
        }
        let _ = tx.send(Some(split));
      });
      rx
    }
  };
  if let Some(s) = shown {
    return merge(log, &s);
  }
  let got = tokio::time::timeout(wait, rx.wait_for(Option::is_some)).await.ok().and_then(|r| r.ok().and_then(|v| v.clone().flatten()));
  match got {
    Some(s) => merge(log, &s),
    None => log,
  }
}

#[cfg(test)]
mod tests {
  #![allow(clippy::unwrap_used, reason = "test code")]
  use super::*;
  use serde_json::json;
  use std::sync::Arc;
  use std::sync::atomic::{AtomicUsize, Ordering};

  /// Trimmed from a real `getContextUsage({ detail: "summary" })` answer
  fn usage() -> Value {
    json!({
      "categories": [
        { "name": "System prompt", "tokens": 4337, "kind": "used" },
        { "name": "System tools", "tokens": 6703, "kind": "used" },
        { "name": "MCP tools", "tokens": 900, "kind": "used" },
        { "name": "System tools (deferred)", "tokens": 22596, "isDeferred": true, "kind": "deferred" },
        { "name": "Custom agents", "tokens": 120, "kind": "used" },
        { "name": "Memory files", "tokens": 5007, "kind": "used" },
        { "name": "Skills", "tokens": 120, "kind": "used" },
        { "name": "Messages", "tokens": 37688, "kind": "used" },
        { "name": "Autocompact buffer", "tokens": 33000, "kind": "buffer" },
        { "name": "Free space", "tokens": 900000, "kind": "free" }
      ],
      "memoryFiles": [
        { "path": "/home/u/.claude/CLAUDE.md", "type": "User", "tokens": 7 },
        { "path": "/w/AGENTS.md", "type": "Project", "tokens": 5000 }
      ],
      "mcpTools": [
        { "name": "mcp__acpira__show_image", "serverName": "acpira", "tokens": 500, "isLoaded": true },
        { "name": "mcp__acpira__open_browser", "serverName": "acpira", "tokens": 400, "isLoaded": true },
        { "name": "mcp__other__big", "serverName": "other", "tokens": 9000, "isLoaded": false }
      ],
      "skills": { "skillFrontmatter": [{ "name": "debug", "source": "userSettings", "tokens": 66 }, { "name": "dig", "source": "userSettings", "tokens": 54 }] }
    })
  }

  #[test]
  fn claude_codes_categories_become_parts_and_its_files_skills_and_tools_items() {
    let s = split_of(&usage()).unwrap();
    assert_eq!(
      s.parts,
      vec![
        (ContextCategory::System, 4337),
        (ContextCategory::Tools, 6703),
        (ContextCategory::Mcp, 900),
        (ContextCategory::Agent, 120),
        (ContextCategory::Docs, 5007),
        (ContextCategory::Skills, 120),
      ]
    );
    let items = |cat| &s.items.iter().find(|i| i.0 == cat).unwrap().1;
    let docs = items(ContextCategory::Docs);
    assert_eq!((docs[1].id.as_str(), docs[1].name.as_deref()), ("doc:1", Some("/w/AGENTS.md")));
    assert_eq!(items(ContextCategory::Skills).iter().map(|n| n.name.clone().unwrap()).collect::<Vec<_>>(), ["debug", "dig"]);
    let mcp = items(ContextCategory::Mcp);
    assert_eq!(mcp.len(), 1, "a server whose tools are deferred names nothing");
    assert_eq!((mcp[0].name.as_deref(), mcp[0].tokens), (Some("acpira"), 900));
    assert_eq!(mcp[0].children.as_ref().unwrap()[0].name.as_deref(), Some("show_image"));
  }

  #[test]
  fn the_conversation_is_what_the_parts_leave_of_the_logs_last_request() {
    let s = split_of(&usage()).unwrap();
    let log = Known { parts: vec![(ContextCategory::Agent, 20_000)], items: vec![], chat: 45_000, summary: Some("sum".into()), cached: Some(60_000) };
    let k = merge(log, &s);
    assert_eq!(k.chat, 65_000 - 17_187);
    assert_eq!((k.summary.as_deref(), k.cached), (Some("sum"), Some(60_000)));
    assert!(split_of(&json!({ "categories": [{ "name": "Messages", "tokens": 9, "kind": "used" }] })).is_none());
  }

  #[test]
  fn the_report_shows_the_items_scaled_with_their_part() {
    use crate::acp::session::context_estimate::report_with;
    let s = split_of(&usage()).unwrap();
    let log = Known { parts: vec![(ContextCategory::Agent, 20_000)], items: vec![], chat: 45_000, summary: None, cached: None };
    let k = merge(log, &s);
    let t: Vec<acpira_shared::transcript::Turn> =
      serde_json::from_value(json!([{ "role": "user", "text": "hi" }, { "role": "agent", "blocks": [{ "type": "text", "markdown": "hello" }] }])).unwrap();
    let u = acpira_shared::transcript::Usage { used: acpira_shared::num::Num(65_000.0), size: acpira_shared::num::Num(1_000_000.0), cost: None, compacted_from: None };
    let r = report_with(&t, Some(&u), true, Some(&k)).unwrap();
    assert_eq!(r.categories.iter().map(|c| c.tokens).sum::<u64>(), 65_000);
    let docs = r.categories.iter().find(|c| c.id == ContextCategory::Docs).unwrap();
    assert_eq!(docs.nodes.as_ref().unwrap().iter().map(|n| n.tokens).sum::<u64>(), docs.tokens);
    let mcp = r.categories.iter().find(|c| c.id == ContextCategory::Mcp).unwrap();
    assert_eq!(mcp.nodes.as_ref().unwrap()[0].kind, ContextNodeKind::Server);
    // Sums only for the card
    let r = report_with(&t, Some(&u), false, Some(&k)).unwrap();
    assert!(r.categories.iter().all(|c| c.nodes.is_none()));
  }

  fn launch() -> LaunchFuture {
    Box::pin(async { Some(Launch { node: String::new(), sdk: PathBuf::new(), env: StrMap::new(), cwd: String::new(), session_id: String::new() }) })
  }

  fn counted(runs: &Arc<AtomicUsize>, split: Option<Split>) -> impl FnOnce(Launch) -> SplitFuture + Send + 'static {
    let runs = runs.clone();
    move |_| {
      runs.fetch_add(1, Ordering::SeqCst);
      Box::pin(async move { split })
    }
  }

  #[tokio::test]
  async fn one_probe_per_compaction_and_the_old_split_shown_meanwhile() {
    let key = PathBuf::from(format!("/probe-test/{}", std::process::id()));
    let log = |own: u64| Known { parts: vec![(ContextCategory::Agent, own)], items: vec![], chat: 50_000, summary: None, cached: None };
    let runs = Arc::new(AtomicUsize::new(0));
    let fresh = Duration::from_secs(60);
    let wait = Duration::from_secs(5);
    let split = split_of(&usage());

    let first = refine_with(key.clone(), log(20_000), launch, fresh, wait, counted(&runs, split.clone())).await;
    assert_eq!(first.parts.len(), 6, "the first ask waits for the probe");
    let again = refine_with(key.clone(), log(20_000), launch, fresh, wait, counted(&runs, split.clone())).await;
    assert_eq!((again, runs.load(Ordering::SeqCst)), (first, 1));

    // Compacted: the old split right away, a new probe behind it
    let after = refine_with(key.clone(), log(25_000), launch, fresh, wait, counted(&runs, None)).await;
    assert_eq!(after.parts.len(), 6);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    // That probe failed: the log's split stands until the entry goes stale
    let failed = refine_with(key.clone(), log(25_000), launch, fresh, wait, counted(&runs, split)).await;
    assert_eq!((failed.parts, runs.load(Ordering::SeqCst)), (vec![(ContextCategory::Agent, 25_000)], 2));
  }
}
