//! A stand-in CLI's session in a folder on an SSH host under local run (`acp::local_run`, docs/dev/remote.md): its
//! stand-in folder, the session request that routes project work through `acpira mcp` (`HubOp::RemoteTool`), its rows
//! shown as native tools, and the permission requests its gates settle

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use acpira_agent::tools::Output;
use serde_json::{Value, json};

use crate::acp::session::AcpSession;
use crate::acp::session::remote::Notify;
use crate::acp::transport::cancel::Cancel;
use crate::acp::local_run::stand::{self as stand_in, Stand};
use crate::acp::local_run::{LocalRun, brief, tools};
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
  /// A permission request named the call: the CLI's own, or the engine's for a CLI that asks nothing (`ask_remote`)
  asked: bool,
}

impl RemoteCall {
  fn new(name: &'static str) -> Self {
    Self { seq: next_call_seq(), name, input: Value::Null, parent: None, claimed: false, card: None, streamed: false, stop: Cancel::new(), asked: false }
  }
}

/// Takes a call out of a session's list when its holder goes, a dropped one too (the CLI cancelled its `tools/call`)
struct Listed<'a, V>(&'a parking_lot::Mutex<std::collections::HashMap<String, V>>, &'a str);

impl<V> Drop for Listed<'_, V> {
  fn drop(&mut self) {
    self.0.lock().remove(self.1);
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
  /// A native tool that would write, read or run on this machine instead of the host (`stand::blocked_native`): always
  /// denied, full access included
  BlockedLocalTool,
  /// A native tool the engine cannot tell safe, in a stand-in no gate guards (`stand::unjudged`): its card stays under
  /// the engine's own full access
  Unjudged,
}

impl AcpSession {
  /// The engine's data root: where its session store lives (`<root>/sessions`)
  fn data_root(&self) -> PathBuf {
    self.deps.blobs.dir().parent().map(PathBuf::from).unwrap_or_else(acpira_home)
  }

  fn here_roots(&self, stand: &Stand) -> Vec<PathBuf> {
    stand_in::here_roots(stand, &home_dir(), &self.data_root())
  }

  /// How the session's agent runs here for a folder on an SSH host (`AgentDef::local_run`); None runs it in the host's
  /// engine. The built-in agent's id keeps its own way even under an agents.json entry of that id
  pub(crate) fn local_run(&self) -> Option<LocalRun> {
    let def = self.deps.registry.try_get(&self.agent).and_then(|d| d.local_run.clone());
    def.or_else(|| (self.agent == crate::acp::agents::registry::SELF_AGENT_ID).then_some(LocalRun::Native))
  }

  /// The stand-in of a stand-in CLI's session in a folder on an SSH host; None for any other session
  pub(crate) fn remote_stand(&self) -> Option<Stand> {
    let f = self.remote_folder()?;
    let profile = self.local_run()?.stand()?.clone();
    let mut stand = Stand::new(profile, &f.alias, &f.path, &self.data_root());
    stand.own = self.own_dirs.lock().clone();
    Some(stand)
  }

  /// Whether picking mode `id` stays off the wire, the engine approving instead: in a gateless stand-in (the generic
  /// profile) the CLI's own full access would run its native tools here without asking, past the engine's refusal of
  /// native writes and commands. `set_mode` and an edit's fresh process keep the CLI in its asking mode
  pub(crate) fn hosts_full_access(&self, controls: &acpira_shared::transcript::SessionControls, id: &str) -> bool {
    crate::acp::session::controls::is_full_access(controls, id) && self.remote_stand().is_some_and(|s| s.profile.gates.is_empty())
  }

  /// Makes the stand-in folder if it is missing (`Stand::make`). A failure is only logged: `gated_remote_stand` stops
  /// the start that needs it
  pub(crate) fn make_remote_stand(&self, stand: &Stand) {
    if let Err(e) = stand.make(&self.data_root()) {
      self.log(&format!("remote: the stand-in folder {} could not be made: {e}", stand.local.display()));
    }
  }

  /// The session's stand-in with its gate files in place, before a process starts or is borrowed there: a CLI whose
  /// hook or configuration could not be written would run its native tools on this machine
  pub(crate) fn gated_remote_stand(&self) -> anyhow::Result<Option<Stand>> {
    let Some(stand) = self.remote_stand() else { return Ok(None) };
    stand.make(&self.data_root()).map_err(|e| anyhow::anyhow!("the stand-in folder {} could not be made: {e}", stand.local.display()))?;
    Ok(Some(stand))
  }

  /// The session request of such a session (`local_run::brief`), with the host's instructions read now; the stand-in
  /// folder is made if it is missing, and a note the CLI reads from the stand-in is written there before it is asked
  pub(crate) async fn with_remote_stand(&self, req: Value) -> Value {
    let Some(stand) = self.remote_stand() else { return req };
    self.make_remote_stand(&stand);
    let places = req.pointer(&format!("/_meta/{}/places", crate::acp::session::remote::META.replace('/', "~1"))).cloned();
    let instructions = self.host_instructions(&stand, places.as_ref()).await;
    let files: Vec<String> = instructions.iter().map(|(p, t)| format!("{p} ({} bytes)", t.len())).collect();
    self.log(&format!(
      "remote: {} runs in {} for {}; instructions: {}",
      stand.profile.label,
      stand.local.display(),
      stand.alias,
      if files.is_empty() { "none".into() } else { files.join(", ") }
    ));
    let note = brief::note(&stand, places.as_ref(), &instructions);
    if let Some((path, text)) = brief::stand_file(&stand, &note)
      && std::fs::read(&path).ok().as_deref() != Some(text.as_bytes())
      && let Err(e) = crate::store::file_lock::write_atomic(&path, text.as_bytes(), None).await
    {
      self.log(&format!("remote: the note {} could not be written: {e}", path.display()));
    }
    brief::with_stand(req, &stand, &note)
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
      for name in &stand.profile.instructions {
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
    let here = stand_in::runs_here(name, &args, &stand, &data_root, &roots);
    let (tool, call_args) = match tools::builtin_call(name, &args, &stand, here) {
      Ok(x) => x,
      Err(e) => return Ok(tools::mcp_result(&Output::error(e))),
    };
    let known = acpira_shared::host_tools::REMOTE.iter().find(|t| **t == name).copied();
    let mut stop = Cancel::new();
    let mut bound_id = tool_use_id;
    // The row is this very call's: named by its id, or found by its arguments. One found by its tool alone is good for
    // the row's looks, never for what was approved on it
    let mut sure = bound_id.is_some();
    if let Some(static_name) = known {
      let mut calls = self.remote_calls.lock();
      if let Some(id) = &bound_id {
        let entry = calls.entry(id.clone()).or_insert_with(|| RemoteCall::new(static_name));
        entry.claimed = true;
        stop = entry.stop.clone();
      } else {
        // A CLI other than claude-agent-acp (Antigravity) sends ACP `tool_call` before MCP `tools/call` without a call id in
        // `_meta`: bind to the earliest
        // unclaimed row of the same tool, preferring an exact canonical-argument match when calls run in parallel
        let exact = calls
          .iter()
          .filter(|(_, c)| !c.claimed && c.card.is_none() && c.name == static_name)
          .filter(|(_, c)| {
            let entry_here = stand_in::runs_here(static_name, &c.input, &stand, &data_root, &roots);
            tools::builtin_call(static_name, &c.input, &stand, entry_here).ok().as_ref() == Some(&(tool, call_args.clone()))
          })
          .min_by_key(|(_, c)| c.seq)
          .map(|(id, _)| id.clone());
        sure = exact.is_some();
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
    // A call no row names is the stop's by this list, from before its question to its end
    let _loose = (bound_id.is_none() && !here).then(|| {
      self.loose_calls.lock().insert(call.clone(), stop.clone());
      Listed(&self.loose_calls, &call)
    });
    // A CLI that calls an MCP tool without asking would write and run on the host with no card at all
    let asked = stand.profile.own_cards || tools::READ_ONLY.contains(&name) || (sure && self.remote_calls.lock().get(&call).is_some_and(|c| c.asked));
    if !asked {
      // A call without a row is held as claimed while its card shows: the request makes an entry for it, which another
      // call of the same tool would else take for its row, approval and all. No row will ever complete it
      let _held = known.filter(|_| bound_id.is_none()).map(|n| {
        self.remote_calls.lock().insert(call.clone(), RemoteCall { claimed: true, stop: stop.clone(), ..RemoteCall::new(n) });
        Listed(&self.remote_calls, &call)
      });
      if !self.ask_remote(name, &args, &call, stop.clone()).await || stop.is_cancelled() {
        return Ok(tools::mcp_result(&Output::error("The user declined this call. Do not repeat it unless they ask.")));
      }
    }
    if bound_id.is_some() {
      self.remote_started(&call);
    }
    let mut out = if here { self.run_here(tool, call_args, &call).await } else { self.run_on_host(tool, call_args, &call, stop).await };
    if name == "Bash" {
      tools::name_job_tools(&mut out, &stand.profile.prefix);
    }
    if let Some(id) = &bound_id
      && let Some(c) = self.remote_calls.lock().get_mut(id)
    {
      c.card = Some((out.content.clone(), out.raw_output.clone()));
    }
    Ok(tools::mcp_result(&out))
  }

  /// The engine's own permission request for a remote tool's write or command the CLI did not ask about: decided like
  /// one of the CLI's (`on_permission`), so full access and accept-edits answer it and anything else shows a card. In a
  /// task of its own: a caller that goes away mid-question (the CLI cancelled its `tools/call`) drops this future, and
  /// the card would stay up with no one to answer to
  async fn ask_remote(self: &Arc<Self>, name: &str, args: &Value, call: &str, stop: Cancel) -> bool {
    struct Fire(Cancel);
    impl Drop for Fire {
      fn drop(&mut self) {
        self.0.cancel();
      }
    }
    let (sid, auto_approve) = {
      let c = self.core.lock();
      // A non-split agent whose `_permission_mode` select is on a `full_access` option (outside read-only modes such as
      // plan): `decide_permission` intentionally ignores that select on the CLI's own requests (so protected-file asks
      // stay up), whereas `ask_remote` is the engine's own card for a stand-in CLI that did not ask at all
      let in_read_only = c.state.controls.mode_id.as_deref().is_some_and(|m| crate::acp::session::relay::READ_ONLY_MODES.contains(&m));
      let select_full = c.access.offered.is_empty()
        && !in_read_only
        && c.state.controls.options.iter().any(|o| {
          o.category.as_deref() == Some("_permission_mode")
            && o.value.as_deref().is_some_and(|v| o.options.iter().any(|opt| opt.id == v && opt.kind.as_deref() == Some("full_access")))
        });
      (c.acp_session_id.clone(), select_full)
    };
    let mut req = json!({
      "sessionId": sid,
      "toolCall": { "toolCallId": call, "title": format!("{}{name}", crate::acp::local_run::DEFAULT_PREFIX), "rawInput": args },
      "options": [{ "optionId": "allow", "name": "Allow", "kind": "allow_once" }, { "optionId": "reject", "name": "Reject", "kind": "reject_once" }],
    });
    if auto_approve {
      req["_meta"] = json!({ "acpira/autoApprove": true });
    }
    let gone = Fire(stop.child());
    let (me, cancel) = (self.clone(), gone.0.clone());
    let answer = tokio::spawn(async move { me.on_permission(req, cancel).await }).await;
    matches!(answer, Ok(Ok(v)) if v.pointer("/outcome/optionId").and_then(Value::as_str) == Some("allow"))
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

  /// The row of a call the engine starts now reads as running: Droid and Grok leave theirs pending until the result,
  /// which showed a command at work on the host as one still waiting
  fn remote_started(self: &Arc<Self>, call: &str) {
    let Some(parent) = self.remote_calls.lock().get(call).map(|c| c.parent.clone()) else { return };
    let mut u = json!({ "sessionUpdate": "tool_call_update", "toolCallId": call, "status": "in_progress" });
    if let Some(p) = parent {
      u["_meta"]["claudeCode"]["parentToolUseId"] = p;
    }
    let sid = self.core.lock().acp_session_id.clone();
    self.on_update(json!({ "sessionId": sid, "update": u }));
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
    let name = match tools::remote_name(&stand.profile, u) {
      Some(n) => n,
      None => match calls.get(&id) {
        Some(c) => c.name,
        None => return,
      },
    };
    let entry = calls.entry(id.clone()).or_insert_with(|| RemoteCall::new(name));
    if let Some(raw) = u.get("rawInput").or_else(|| u.get("args")).filter(|r| r.as_object().is_some_and(|o| !o.is_empty())) {
      entry.input = tools::clean_input(tools::call_input(raw));
    }
    if let Some(p) = u.pointer("/_meta/claudeCode/parentToolUseId") {
      entry.parent = Some(p.clone());
    }
    let view = tools::view(name, &entry.input, &stand, &data_root, &roots);
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
      m.remove("acpira/mcp");
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
    for stop in self.loose_calls.lock().values() {
      stop.cancel();
    }
  }

  /// The user's stop of one remote call's row; false when the call is not one of them
  pub(crate) fn stop_remote_call(&self, tool_call_id: &str) -> bool {
    self.remote_calls.lock().get(tool_call_id).map(|c| c.stop.cancel()).is_some()
  }

  /// Whether a permission request's toolCall is one of the remote tools: a call the engine already holds, or one named so
  fn is_remote_call(&self, stand: &Stand, tc: &Value) -> bool {
    tc.get("toolCallId").and_then(Value::as_str).is_some_and(|id| self.remote_calls.lock().contains_key(id)) || tools::remote_name(&stand.profile, tc).is_some()
  }

  /// A permission request's toolCall of a remote tool, rewritten like its row
  pub(crate) fn nativize_permission(&self, req: &mut Value) {
    let Some(stand) = self.remote_stand() else { return };
    let Some(tc) = req.get_mut("toolCall") else { return };
    if !self.is_remote_call(&stand, tc) {
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
  /// the CLI's own plan / artifact writes without a card, approves host edits in accept-edits mode, and refuses a native
  /// tool that would work on this machine (`stand::blocked_native`)
  pub(crate) fn remote_perm(&self, tool_call_id: &str, tool_call: &Value) -> RemotePerm {
    let Some(stand) = self.remote_stand() else { return RemotePerm::None };
    let home = home_dir();
    let data_root = self.data_root();
    if !self.is_remote_call(&stand, tool_call) {
      return if stand_in::blocked_native(&stand, tool_call, &home, &data_root) {
        RemotePerm::BlockedLocalTool
      } else if stand_in::unjudged(&stand, tool_call) {
        RemotePerm::Unjudged
      } else {
        RemotePerm::None
      };
    }
    let mut calls = self.remote_calls.lock();
    let Some(c) = calls.get_mut(tool_call_id) else { return RemotePerm::None };
    c.asked = true;
    if tools::READ_ONLY.contains(&c.name) {
      return RemotePerm::ReadOnly;
    }
    if !matches!(c.name, "Write" | "Edit" | "MultiEdit") {
      return RemotePerm::None;
    }
    let roots = stand_in::here_roots(&stand, &home, &data_root);
    if !stand_in::runs_here(c.name, &c.input, &stand, &data_root, &roots) {
      return RemotePerm::HostEdit;
    }
    let path = c.input.get("file_path").or_else(|| c.input.get("path")).and_then(Value::as_str).unwrap_or("");
    if stand_in::is_quiet_write(&stand, path, &home) { RemotePerm::AgentOwnFile } else { RemotePerm::None }
  }
}
