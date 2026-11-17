//! A Claude Code or Antigravity session in a folder on an SSH host under local run (`vendors::claude_remote`,
//! docs/dev/remote.md): its stand-in folder, the session request that routes project work through `acpira mcp`
//! (`HubOp::RemoteTool`), and its rows shown as native tools

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use acpira_agent::tools::Output;
use serde_json::{Value, json};

use crate::acp::session::AcpSession;
use crate::acp::session::remote::Notify;
use crate::acp::transport::cancel::Cancel;
use crate::acp::vendors::claude_remote::{self, Stand};
use crate::store::data_dir::{acpira_home, home_dir};

/// How much of one instructions file goes into the system prompt
const MAX_INSTRUCTIONS: usize = 48 * 1024;

fn next_call_seq() -> u64 {
  static SEQ: AtomicU64 = AtomicU64::new(1);
  SEQ.fetch_add(1, Ordering::Relaxed)
}

/// What the engine knows of one call of a remote tool, by the model's call id, until its row completes
pub(crate) struct RemoteCall {
  seq: u64,
  name: &'static str,
  /// The model's input, as the adapter last reported it
  input: Value,
  /// The subagent the call belongs to (`_meta.claudeCode.parentToolUseId`), for the updates the engine adds to its row
  parent: Option<Value>,
  /// Bound to an incoming MCP `tools/call`
  claimed: bool,
  /// The run's content and raw output, for the row's completion
  card: Option<(Vec<Value>, Option<Value>)>,
  /// Output was streamed onto the row while it ran
  streamed: bool,
  /// Fired by the user's stop: drops the call, which kills its process group on the host
  stop: Cancel,
}

impl RemoteCall {
  fn new(name: &'static str) -> Self {
    Self { seq: next_call_seq(), name, input: Value::Null, parent: None, claimed: false, card: None, streamed: false, stop: Cancel::new() }
  }
}

/// How `decide_permission` handles a permission request in a remote stand-in session
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RemotePerm {
  /// Not a remote tool call that settles without a card
  None,
  /// A read-only remote tool (`Read`, `Grep`, `Glob`): `agy acp` has no `allowedTools` list and asks on every MCP tool
  ReadOnly,
  /// A write on the SSH host (`Write` / `Edit` / `MultiEdit`): auto-approved in accept-edits mode, card in default mode
  HostEdit,
  /// The CLI's own local plan or artifact file here (`~/.claude/plans`, `~/.gemini/.../brain`)
  AgentOwnFile,
  /// A built-in local workspace or shell tool outside `~/.gemini` in an Antigravity stand-in session: always denied
  BlockedLocalTool,
}

impl AcpSession {
  /// The engine's data root: where its session store lives (`<root>/sessions`)
  fn data_root(&self) -> PathBuf {
    self.deps.blobs.dir().parent().map(PathBuf::from).unwrap_or_else(acpira_home)
  }

  fn here_roots(&self, stand: &Stand) -> Vec<PathBuf> {
    claude_remote::here_roots(stand.dialect, &home_dir(), &self.data_root())
  }

  /// The stand-in of a Claude Code or Antigravity session in a folder on an SSH host; None for any other session
  pub(crate) fn remote_stand(&self) -> Option<Stand> {
    let dialect = self.vendor.remote_stand()?;
    let f = self.remote_folder()?;
    Some(Stand::for_dialect(dialect, &f.alias, &f.path, &self.data_root()))
  }

  /// Makes the stand-in folder if it is missing (`Stand::make`). A failure is only logged: the process that then cannot
  /// start there names the folder itself (`AgentSpawnError`)
  pub(crate) fn make_remote_stand(&self, stand: &Stand) {
    if let Err(e) = stand.make(&self.data_root()) {
      self.log(&format!("remote: the stand-in folder {} could not be made: {e}", stand.local.display()));
    }
  }

  /// The session request of such a session (`claude_remote::with_stand`), with the host's instructions read now; the
  /// stand-in folder is made if it is missing
  pub(crate) async fn with_remote_stand(&self, req: Value) -> Value {
    let Some(stand) = self.remote_stand() else { return req };
    self.make_remote_stand(&stand);
    let places = req.pointer(&format!("/_meta/{}/places", crate::acp::session::remote::META.replace('/', "~1"))).cloned();
    let instructions = self.host_instructions(&stand, places.as_ref()).await;
    let files: Vec<String> = instructions.iter().map(|(p, t)| format!("{p} ({} bytes)", t.len())).collect();
    self.log(&format!(
      "remote: {} runs in {} for {}; instructions: {}",
      stand.dialect.label(),
      stand.local.display(),
      stand.alias,
      if files.is_empty() { "none".into() } else { files.join(", ") }
    ));
    claude_remote::with_stand(req, &stand, places.as_ref(), &instructions)
  }

  /// The project's instructions on the host as the CLI would read them there (`CLAUDE.md` / `GEMINI.md` of the project
  /// root and of the folder, else their `AGENTS.md`). A line that is only `@file` is that file's text, one level deep
  async fn host_instructions(&self, stand: &Stand, places: Option<&Value>) -> Vec<(String, String)> {
    let Some(client) = self.deps.remote.as_ref().and_then(|c| c.client(&stand.alias)) else { return vec![] };
    let read = |path: String| {
      let client = client.clone();
      async move { client.call("read", json!({ "path": path })).await.ok().map(|r| String::from_utf8_lossy(&r.data).into_owned()) }
    };
    let mut dirs: Vec<String> = places.and_then(|p| p.get("root")).and_then(Value::as_str).map(str::to_owned).into_iter().collect();
    if !dirs.contains(&stand.host) {
      dirs.push(stand.host.clone());
    }
    let mut out = vec![];
    for dir in dirs {
      let dir = dir.trim_end_matches('/').to_owned();
      let mut found = None;
      for name in stand.dialect.instruction_files() {
        let path = format!("{dir}/{name}");
        if let Some(text) = read(path.clone()).await {
          found = Some((path, text));
          break;
        }
      }
      let Some((path, text)) = found else { continue };
      let mut body = String::new();
      for line in text.lines() {
        let import = line.trim().strip_prefix('@').filter(|r| !r.is_empty() && !r.contains(char::is_whitespace));
        let expanded = match import {
          Some(rel) => read(if rel.starts_with('/') { rel.to_owned() } else { format!("{dir}/{rel}") }).await,
          None => None,
        };
        body.push_str(expanded.as_deref().unwrap_or(line));
        body.push('\n');
      }
      if body.len() > MAX_INSTRUCTIONS {
        let cut = acpira_agent::budget::cut(&body, MAX_INSTRUCTIONS).to_owned();
        body = format!("{cut}\n[Cut at {} KB; read {path} for the rest.]", MAX_INSTRUCTIONS / 1024);
      }
      if !body.trim().is_empty() {
        out.push((path, body));
      }
    }
    out
  }

  /// One project tool call of the session's agent (`HubOp::RemoteTool`): run on the host by the built-in agent's tool
  /// code, or here when it names the CLI's own files. Ok is the MCP result (a failed call included), Err a call that
  /// cannot be made in this session at all
  pub(crate) async fn on_mcp_remote_tool(self: &Arc<Self>, name: &str, args: Value, tool_use_id: Option<String>) -> Result<Value, String> {
    let stand = self.remote_stand().ok_or("This session's agent has no project tools on an SSH host.")?;
    let data_root = self.data_root();
    let roots = self.here_roots(&stand);
    let here = claude_remote::runs_here(name, &args, &stand, &data_root, &roots);
    let (tool, call_args) = match claude_remote::builtin_call(name, &args, &stand, here) {
      Ok(x) => x,
      Err(e) => return Ok(claude_remote::mcp_result(&Output::error(e))),
    };
    let mut stop = Cancel::new();
    let mut bound_id = tool_use_id;
    if let Some(static_name) = acpira_shared::host_tools::REMOTE.iter().find(|t| **t == name).copied() {
      let mut calls = self.remote_calls.lock();
      if let Some(id) = &bound_id {
        let entry = calls.entry(id.clone()).or_insert_with(|| RemoteCall::new(static_name));
        entry.claimed = true;
        stop = entry.stop.clone();
      } else {
        // Antigravity sends ACP `tool_call` before MCP `tools/call` without a call id in `_meta`: bind to the earliest
        // unclaimed row of the same tool, preferring an exact canonical-argument match when calls run in parallel
        let exact = calls
          .iter()
          .filter(|(_, c)| !c.claimed && c.card.is_none() && c.name == static_name)
          .filter(|(_, c)| {
            let entry_here = claude_remote::runs_here(static_name, &c.input, &stand, &data_root, &roots);
            claude_remote::builtin_call(static_name, &c.input, &stand, entry_here).ok().as_ref() == Some(&(tool, call_args.clone()))
          })
          .min_by_key(|(_, c)| c.seq)
          .map(|(id, _)| id.clone());
        let matched = exact.or_else(|| {
          calls
            .iter()
            .filter(|(_, c)| !c.claimed && c.card.is_none() && c.name == static_name)
            .min_by_key(|(_, c)| c.seq)
            .map(|(id, _)| id.clone())
        });
        if let Some(id) = matched
          && let Some(entry) = calls.get_mut(&id)
        {
          entry.claimed = true;
          stop = entry.stop.clone();
          bound_id = Some(id);
        }
      }
    }
    let call = bound_id.clone().unwrap_or_else(crate::util::random_uuid);
    let mut out = if here { self.run_here(tool, call_args, &call).await } else { self.run_on_host(tool, call_args, &call, stop).await };
    if name == "Bash" {
      claude_remote::name_job_tools(&mut out, stand.dialect);
    }
    if let Some(id) = &bound_id
      && let Some(c) = self.remote_calls.lock().get_mut(id)
    {
      c.card = Some((out.content.clone(), out.raw_output.clone()));
    }
    Ok(claude_remote::mcp_result(&out))
  }

  /// Prepare and run a call on the host's helper (`on_remote_tool`, the built-in agent's path): the edit gate asks
  /// before a write no card showed, a command's output streams onto the row, and outputs over the budget stay on the
  /// host for the model to read there
  async fn run_on_host(self: &Arc<Self>, tool: &str, args: Value, call: &str, stop: Cancel) -> Output {
    let quiet: Notify = Arc::new(|_, _| {});
    // The agent's session id, which closing the session stops the host's jobs and kept outputs by (`close_host_jobs`)
    let sid = self.core.lock().acp_session_id.clone().unwrap_or_else(|| self.id.clone());
    let prepare = json!({ "sessionId": sid, "phase": "prepare", "name": tool, "args": args, "images": true });
    let prepared = match self.on_remote_tool(prepare, stop.clone(), quiet).await {
      Ok(v) => v,
      Err(e) => return Output::error(format!("The call could not be prepared on the host: {}", e.message)),
    };
    if let Some(e) = prepared.get("error").and_then(Value::as_str) {
      return Output::error(e);
    }
    let (me, id) = (self.me.clone(), call.to_owned());
    let progress: Notify = Arc::new(move |_, p| {
      if let (Some(s), Some(u)) = (me.upgrade(), p.get("update")) {
        s.remote_progress(&id, u.clone());
      }
    });
    let run = json!({ "sessionId": sid, "phase": "run", "callId": call, "action": prepared.get("action"), "keep": true });
    match self.on_remote_tool(run, stop, progress).await {
      Ok(v) => match v.get("output").cloned().map(serde_json::from_value::<Output>) {
        Some(Ok(out)) => out,
        _ => Output::error("The host answered with a result this engine cannot read."),
      },
      Err(e) => Output::error(format!(
        "The call did not complete on the host: {}. Whether it took effect there is unknown: check before doing it again.",
        e.message
      )),
    }
  }

  /// A call on the CLI's own files here (a plan, an artifact, a memory file, a saved output)
  async fn run_here(&self, tool: &'static str, args: Value, call: &str) -> Output {
    let cwd = home_dir();
    let prepared = {
      let cwd = cwd.clone();
      tokio::task::spawn_blocking(move || acpira_agent::tools::prepare(tool, &args, &cwd, true)).await
    };
    let action = match prepared {
      Ok(Ok(a)) => a,
      Ok(Err(e)) => return Output::error(e),
      Err(e) => return Output::error(e.to_string()),
    };
    let safe: String = call.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let ctx = acpira_agent::tools::Ctx {
      cwd,
      outputs: self.data_root().join("run").join("claude-remote").join(&self.id).join(safe),
      call_id: call.to_owned(),
      progress: Arc::new(|_| {}),
      jobs: Default::default(),
      search: Default::default(),
      mcp: Arc::new(acpira_agent::mcp::Hub::default()),
      history: PathBuf::new(),
      // A folder on an SSH host takes no scratch folder (`acpira_agent::scratch`)
      scratch: None,
      path: None,
    };
    action.run(ctx).await
  }

  /// A running command's output (`{ _meta: { terminal_output_delta } }`) onto its row, as the adapter's own deltas would
  fn remote_progress(self: &Arc<Self>, call: &str, mut u: Value) {
    if !u.is_object() {
      return;
    }
    let parent = {
      let mut calls = self.remote_calls.lock();
      let Some(c) = calls.get_mut(call) else { return };
      c.streamed = true;
      c.parent.clone()
    };
    u["sessionUpdate"] = json!("tool_call_update");
    u["toolCallId"] = json!(call);
    if let Some(p) = parent {
      u["_meta"]["claudeCode"]["parentToolUseId"] = p;
    }
    let sid = self.core.lock().acp_session_id.clone();
    self.on_update(json!({ "sessionId": sid, "update": u }));
  }

  /// A `tool_call` / `tool_call_update` of a remote tool, rewritten as the adapter reports the native tool (kind, title,
  /// locations, the host's paths, a write's diff), and its completion filled with what the run gave the built-in
  /// agent's row: the whole file's diff, a command's exit code. Anything else passes untouched
  pub(crate) fn nativize_remote(&self, kind: &str, u: &mut Value) {
    if !matches!(kind, "tool_call" | "tool_call_update") {
      return;
    }
    let Some(stand) = self.remote_stand() else { return };
    let Some(id) = u.get("toolCallId").and_then(Value::as_str).map(str::to_owned) else { return };
    let data_root = self.data_root();
    let roots = self.here_roots(&stand);
    let mut calls = self.remote_calls.lock();
    let name = match claude_remote::remote_name(u) {
      Some(n) => n,
      None => match calls.get(&id) {
        Some(c) => c.name,
        None => return,
      },
    };
    let entry = calls.entry(id.clone()).or_insert_with(|| RemoteCall::new(name));
    if let Some(raw) = u.get("rawInput").or_else(|| u.get("args")).filter(|r| r.as_object().is_some_and(|o| !o.is_empty())) {
      entry.input = claude_remote::clean_input(raw);
    }
    if let Some(p) = u.pointer("/_meta/claudeCode/parentToolUseId") {
      entry.parent = Some(p.clone());
    }
    let view = claude_remote::view(name, &entry.input, &stand, &data_root, &roots);
    let first = kind == "tool_call";
    u["kind"] = json!(view.kind);
    if first || u.get("title").is_some() {
      u["title"] = json!(view.title);
    }
    if !view.locations.is_empty() && (first || u.get("locations").is_some()) {
      u["locations"] = json!(view.locations);
    }
    if u.get("args").is_some() {
      u["args"] = view.input.clone();
    }
    if u.get("rawInput").is_some() {
      u["rawInput"] = view.input;
    }
    if let Some(n) = u.pointer_mut("/_meta/claudeCode/toolName") {
      *n = json!(name);
    }
    // claude-agent-acp and agy acp also name the call at the top level; claude-agent-acp flags it as MCP for AIR
    if u.get("name").is_some() {
      u["name"] = json!(name);
    }
    if let Some(m) = u.get_mut("_meta").and_then(Value::as_object_mut) {
      m.remove("is_mcp_tool_call");
      if let Some(cc) = m.get_mut("claudeCode").and_then(Value::as_object_mut) {
        cc.remove("mcpServer");
      }
    }
    match u.get("status").and_then(Value::as_str) {
      Some("completed") => {
        let Some(c) = calls.remove(&id) else { return };
        match c.card {
          Some((content, raw)) => {
            if !content.is_empty() {
              u["content"] = json!(content);
            } else if c.streamed && let Some(o) = u.as_object_mut() {
              // The output is on the row already; the adapter's copy is the model's text of it
              o.remove("content");
            }
            if let Some(code) = raw.as_ref().and_then(|r| r.get("exitCode")).and_then(Value::as_i64).filter(|c| *c != 0) {
              u["_meta"]["terminal_exit"] = json!({ "exit_code": code });
            }
            if let Some(r) = raw {
              u["rawOutput"] = r;
            }
          }
          None => {
            if !view.diffs.is_empty() {
              u["content"] = json!(view.diffs);
            }
          }
        }
      }
      Some("failed") => {
        calls.remove(&id);
      }
      _ => {
        if !view.diffs.is_empty() && (first || u.get("rawInput").is_some() || u.get("args").is_some()) {
          u["content"] = json!(view.diffs);
        }
      }
    }
  }

  /// The user's stop ends the calls still running on the host. Claude Code's MCP cancel cannot be relied on: a host
  /// command outlived a stop on CLI 2.1.300
  pub(crate) fn stop_remote_calls(&self) {
    for c in self.remote_calls.lock().values() {
      c.stop.cancel();
    }
  }

  /// The user's stop of one remote call's row; false when the call is not one of them
  pub(crate) fn stop_remote_call(&self, tool_call_id: &str) -> bool {
    self.remote_calls.lock().get(tool_call_id).map(|c| c.stop.cancel()).is_some()
  }

  /// A permission request's toolCall of a remote tool, rewritten like its row
  pub(crate) fn nativize_permission(&self, req: &mut Value) {
    let Some(tc) = req.get_mut("toolCall") else { return };
    let remote = tc.get("toolCallId").and_then(Value::as_str).is_some_and(|id| self.remote_calls.lock().contains_key(id)) || claude_remote::remote_name(tc).is_some();
    if !remote {
      return;
    }
    self.nativize_remote("tool_call_update", tc);
    // An AIR client also gets the card's heading in `_meta`, which is the tool call's title: the MCP name as it stands
    let Some(title) = tc.get("title").cloned() else { return };
    for at in ["/_meta/jetbrains/air/permission", "/_meta/permission"] {
      if let Some(p) = req.pointer_mut(at).and_then(Value::as_object_mut).filter(|p| p.contains_key("title")) {
        p.insert("title".into(), title.clone());
      }
    }
  }

  /// Classifies a permission request in a stand-in session so `decide_permission` approves read-only remote tools and
  /// the CLI's own plan / artifact writes without a card, approves host edits in accept-edits mode, and blocks local
  /// workspace/shell tools outside `~/.gemini`
  pub(crate) fn remote_perm(&self, tool_call_id: &str, tool_call: &Value) -> RemotePerm {
    let Some(stand) = self.remote_stand() else { return RemotePerm::None };
    let home = home_dir();
    let data_root = self.data_root();
    if claude_remote::is_blocked_local_tool(&stand, tool_call, &home, &data_root) {
      return RemotePerm::BlockedLocalTool;
    }
    let calls = self.remote_calls.lock();
    let Some(c) = calls.get(tool_call_id) else { return RemotePerm::None };
    if claude_remote::READ_ONLY.contains(&c.name) {
      return RemotePerm::ReadOnly;
    }
    if !matches!(c.name, "Write" | "Edit" | "MultiEdit") {
      return RemotePerm::None;
    }
    let roots = claude_remote::here_roots(stand.dialect, &home, &data_root);
    if !claude_remote::runs_here(c.name, &c.input, &stand, &data_root, &roots) {
      return RemotePerm::HostEdit;
    }
    let path = c.input.get("file_path").or_else(|| c.input.get("path")).and_then(Value::as_str).unwrap_or("");
    if claude_remote::is_agent_own_write(stand.dialect, path, &home, &roots) { RemotePerm::AgentOwnFile } else { RemotePerm::None }
  }
}
