//! Summoned children (cross-harness subagents, see `relay/`): the session runs a persona's CLI as a child of its own.
//! The child gets its own agent process whose client handlers feed the parent session, so its updates, permission and
//! question requests route by native session id into a `session` subagent node exactly like a native child's.
//!
//! One `ask_agent` call = one round = one node. A thread is the sequence of rounds talking to the same native session;
//! its process stays up between rounds (and is resumed by session id after a restart). The call waits for the round
//! up to a deadline, then answers "still working" and the round goes on; a call with the thread and no prompt waits again.
//!
//! Ownership: a thread's process is registered on the session the moment it is spawned, so a failed open, a cancel or
//! the session closing always reaches it. The session's close hands every child process to the close future (and to the
//! hard-kill list of a host shutdown); a process that finishes spawning after the close is ended at once

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, watch};

use acpira_shared::subagents::{RelayMode, SubagentHarness, SubagentPersona, SubagentState};

use crate::acp::session::updates::current_turn_index;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::runtime_info_of;
use crate::acp::transcript::subagent_tree::{RelayOpen, RouteCtx};
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::process::{AgentProcess, ClientHandlers};
use crate::acp::transport::rpc::{BoxFuture, RpcError};
use crate::i18n::t;
use crate::relay::roster::Roster;
use crate::relay::wire::{AskArgs, HubReply};
use crate::relay::{MAX_DEPTH, MAX_RUNNING};

/// How long a call waits for the round when the MCP side names no deadline
const DEFAULT_WAIT: Duration = Duration::from_secs(50);
const MAX_WAIT: Duration = Duration::from_secs(30 * 60);
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
const TITLE_MAX: usize = 48;
/// Mode ids that mean "read only" across the built-in CLIs: codex-acp `read-only`, Claude / OpenCode / Grok `plan`
const READ_ONLY_MODES: [&str; 3] = ["read-only", "plan", "ask"];
const MODEL_KEYS: [&str; 1] = ["model"];
const EFFORT_KEYS: [&str; 3] = ["thought_level", "reasoning_effort", "effort"];
const MODE_KEYS: [&str; 1] = ["mode"];

/// The final word of a round: the reply text for the caller, or why there is none
type RoundEnd = Option<std::result::Result<String, String>>;

/// One thread's process and native session
pub(crate) struct RelayProc {
  pub proc: Arc<AgentProcess>,
  /// The native session id; empty while the session is still being opened
  pub peer: String,
  /// The session's modes and config options as last known: the open / load / resume answer, then every change
  opened: Value,
  /// The CLI mode the session opened in: a work round after a consult switches back to it
  start_mode: Option<String>,
  /// The mode the previous round ran in (None = not confirmed: the next round applies it again)
  mode: Option<RelayMode>,
  /// The persona's model / effort as last applied: a persona edited since gets them applied again
  applied: (Option<String>, Option<String>),
}

/// Per-session relay state (in `Core`)
pub(crate) struct Relays {
  pub procs: HashMap<String, RelayProc>,
  /// Rounds in flight, by thread: the call waiting on one watches its end
  pub rounds: HashMap<String, watch::Receiver<RoundEnd>>,
  /// Root `ask_agent` rows seen before their round existed
  calls: Vec<PendingCall>,
  /// Setup notes of a round (a model the CLI lacks, a refused mode), by node: every read of the round repeats them
  notes: HashMap<String, Vec<String>>,
  /// Starts between spawn and registration: the close waits until they are done
  starting: Arc<watch::Sender<usize>>,
  /// The session closed: no round starts, a process spawned afterwards is ended at once
  closed: bool,
}

impl Default for Relays {
  fn default() -> Self {
    Relays { procs: HashMap::new(), rounds: HashMap::new(), calls: vec![], notes: HashMap::new(), starting: Arc::new(watch::channel(0).0), closed: false }
  }
}

/// One start in flight (`Relays::starting`), counted for as long as it lives
struct Starting(Arc<watch::Sender<usize>>);

impl Starting {
  fn enter(count: &Arc<watch::Sender<usize>>) -> Starting {
    count.send_modify(|n| *n += 1);
    Starting(count.clone())
  }
}

impl Drop for Starting {
  fn drop(&mut self) {
    self.0.send_modify(|n| *n = n.saturating_sub(1));
  }
}

/// A root `ask_agent` row waiting for its node, with the arguments that tell which round it started
struct PendingCall {
  tool_call_id: String,
  agent: String,
  thread: Option<String>,
  prompt: String,
  turn_index: usize,
}

/// The client side of a summoned child's process: everything goes to the parent session, which routes by session id
struct RelayHandlers {
  session: Weak<AcpSession>,
  thread: String,
  /// A session/load replays the thread's earlier rounds; those are already in their own nodes
  quiet: Arc<AtomicBool>,
}

/// The routing key a thread's traffic carries into the parent session: two CLIs may mint the same session id (the
/// parent's own included), so the child's real id never reaches the parent's router
pub(crate) fn route_key(thread: &str) -> String {
  format!("relay:{thread}")
}

impl RelayHandlers {
  fn stamp(&self, mut v: Value) -> Value {
    if v.is_object() {
      v["sessionId"] = Value::from(route_key(&self.thread));
    }
    v
  }
}

impl ClientHandlers for RelayHandlers {
  fn on_update(&self, params: Value) {
    if self.quiet.load(Ordering::Relaxed) {
      return;
    }
    if let Some(s) = self.session.upgrade() {
      s.on_update(self.stamp(params));
    }
  }

  fn on_permission(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let req = self.stamp(req);
    Box::pin(async move {
      match s {
        Some(s) => s.on_permission(req, cancel).await,
        None => Ok(json!({ "outcome": { "outcome": "cancelled" } })),
      }
    })
  }

  fn on_elicitation(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let req = self.stamp(req);
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_elicitation(req, cancel).await),
        None => Ok(json!({ "action": "cancel" })),
      }
    })
  }

  fn on_grok_question(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    let req = self.stamp(req);
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_grok_question(req, cancel).await),
        None => Ok(json!({ "outcome": "skip_interview" })),
      }
    })
  }

  fn on_stderr(&self, line: &str) {
    if let Some(s) = self.session.upgrade() {
      s.log(&format!("relay {}: stderr: {line}", short(&self.thread)));
    }
  }

  fn on_exit(&self, code: Option<i32>, signal: Option<String>) {
    let Some(s) = self.session.upgrade() else { return };
    s.log(&format!("relay {}: exit code={code:?} signal={signal:?}", short(&self.thread)));
    s.core.lock().relays.procs.remove(&self.thread);
  }
}

fn short(id: &str) -> String {
  id.chars().take(8).collect()
}

fn clip(s: &str, max: usize) -> String {
  let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
  let mut out: String = line.chars().take(max).collect();
  if line.chars().count() > max {
    out.push('…');
  }
  out
}

/// The task the child receives: the caller's prompt, the persona's standing brief, the read-only rule for a consult
fn compose_brief(prompt: &str, persona: &SubagentPersona, mode: RelayMode) -> String {
  let mut parts = vec![prompt.trim().to_owned()];
  if let Some(b) = persona.brief.as_deref().filter(|b| !b.trim().is_empty()) {
    parts.push(b.trim().to_owned());
  }
  if mode == RelayMode::Consult {
    parts.push("You are consulted for an opinion: read whatever you need, but do not modify any file.".into());
  }
  parts.push(
    "(Another agent working in this same repository asked you this. It reads only your final reply, so end with a self-contained answer.)"
      .into(),
  );
  parts.join("\n\n")
}

/// The config option whose category or id is one of `keys`
fn find_option<'a>(options: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
  options?.as_array()?.iter().find(|o| {
    let id = o.get("id").and_then(Value::as_str).unwrap_or("");
    let cat = o.get("category").and_then(Value::as_str).unwrap_or("");
    keys.iter().any(|k| *k == id || *k == cat)
  })
}

/// A select's `(value, name)` pairs; grouped selects nest their values one level down
fn option_values(o: &Value) -> Vec<(String, String)> {
  let mut values = vec![];
  for v in o.get("options").and_then(Value::as_array).into_iter().flatten() {
    let inner = v.get("options").and_then(Value::as_array);
    for x in inner.map(|l| l.iter().collect::<Vec<_>>()).unwrap_or_else(|| vec![v]) {
      if let Some(val) = x.get("value").and_then(Value::as_str) {
        values.push((val.to_owned(), x.get("name").and_then(Value::as_str).unwrap_or(val).to_owned()));
      }
    }
  }
  values
}

/// `(config id, chosen value)` for a select whose category or id is one of `keys`, matching `want` by value or name
fn pick_option(options: Option<&Value>, keys: &[&str], want: &str) -> Option<(String, String)> {
  let o = find_option(options, keys)?;
  let id = o.get("id").and_then(Value::as_str).unwrap_or("");
  let want = want.trim().to_lowercase();
  let values = option_values(o);
  let exact = values.iter().find(|(v, n)| v.to_lowercase() == want || n.to_lowercase() == want);
  let loose = || values.iter().find(|(v, n)| v.to_lowercase().contains(&want) || n.to_lowercase().contains(&want));
  exact.or_else(loose).map(|(v, _)| (id.to_owned(), v.clone()))
}

/// The display name of a select's current value
fn current_name(options: Option<&Value>, keys: &[&str]) -> Option<String> {
  let o = find_option(options, keys)?;
  let cur = o.get("currentValue").and_then(Value::as_str)?;
  Some(option_values(o).into_iter().find(|(v, _)| v == cur).map(|(_, n)| n).unwrap_or_else(|| cur.to_owned()))
}

/// The session's mode ids: the `modes` list, else the values of a `mode` select
fn mode_ids(opened: &Value) -> Vec<String> {
  let listed: Vec<String> = opened
    .pointer("/modes/availableModes")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned))
    .collect();
  if !listed.is_empty() {
    return listed;
  }
  find_option(opened.get("configOptions"), &MODE_KEYS).map(|o| option_values(o).into_iter().map(|(v, _)| v).collect()).unwrap_or_default()
}

fn current_mode(opened: &Value) -> Option<String> {
  opened
    .pointer("/modes/currentModeId")
    .and_then(Value::as_str)
    .map(str::to_owned)
    .or_else(|| find_option(opened.get("configOptions"), &MODE_KEYS).and_then(|o| o.get("currentValue")).and_then(Value::as_str).map(str::to_owned))
}

fn is_read_only(mode: &str) -> bool {
  READ_ONLY_MODES.contains(&mode)
}

/// One `session/set_config_option`; the answer's option list (model-dependent effort values) replaces the known one
async fn set_config(proc: &AgentProcess, sid: &str, opened: &mut Value, id: &str, value: &str) -> std::result::Result<(), String> {
  let r = proc
    .request("session/set_config_option", json!({ "sessionId": sid, "configId": id, "value": value }))
    .await
    .map_err(|e| e.to_string())?;
  if !opened.is_object() {
    *opened = json!({});
  }
  match r.get("configOptions") {
    Some(list) if list.is_array() => opened["configOptions"] = list.clone(),
    _ => {
      if let Some(o) = opened.get_mut("configOptions").and_then(Value::as_array_mut).and_then(|l| l.iter_mut().find(|o| o.get("id").and_then(Value::as_str) == Some(id))) {
        o["currentValue"] = Value::from(value);
      }
    }
  }
  Ok(())
}

/// The thread's display: persona name and CLI
fn who(persona: &SubagentPersona) -> String {
  format!("{} ({})", persona.name, persona.agent)
}

/// Whether a call (its `agent` and `thread` arguments) is the one that opened a round with this harness / persona name
fn call_fits(agent: &str, thread: Option<&str>, h: &SubagentHarness, role: Option<&str>) -> bool {
  let agent = agent.trim();
  match thread.map(str::trim).filter(|t| !t.is_empty()) {
    Some(th) => h.thread == th && h.round > 1,
    None => h.round == 1 && (h.persona.as_deref() == Some(agent) || role.is_some_and(|r| r.eq_ignore_ascii_case(agent))),
  }
}

/// What a finished or running round is shown as, from its node: no roster lookup
fn node_persona(h: &SubagentHarness, name: Option<&str>) -> SubagentPersona {
  SubagentPersona {
    id: h.persona.clone().unwrap_or_default(),
    name: name.map(str::to_owned).unwrap_or_else(|| h.agent.clone()),
    agent: h.agent.clone(),
    model: None,
    effort: None,
    mode: h.mode,
    when: String::new(),
    brief: None,
    enabled: true,
  }
}

/// The persona a thread continues with: its current settings, in the CLI the thread was started in
fn thread_persona(roster: &Roster, h: &SubagentHarness, name: Option<&str>) -> Result<SubagentPersona> {
  let label = name.unwrap_or(h.persona.as_deref().unwrap_or(&h.agent));
  let Some(p) = roster.find(h.persona.as_deref().unwrap_or("")) else {
    return Err(anyhow!("{label} is no longer an enabled subagent, so thread {} cannot continue.", h.thread));
  };
  if p.agent != h.agent {
    return Err(anyhow!(
      "{} now runs in {}, but thread {} was started in {}. Leave `thread` out to start a new conversation.",
      p.name,
      p.agent,
      h.thread,
      h.agent
    ));
  }
  Ok(p)
}

impl AcpSession {
  /// One `ask_agent` call from an agent of this session (the root, or a summoned child when `caller` names its thread)
  pub async fn relay_ask(self: Arc<Self>, roster: Arc<Roster>, caller: Option<String>, depth: u32, args: AskArgs, out: mpsc::UnboundedSender<HubReply>) {
    let deadline = args.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    match self.clone().relay_start(&roster, caller, depth, args).await {
      Ok(Start::Wait { thread, persona }) => self.relay_wait(&thread, &persona, deadline, &out).await,
      Ok(Start::Ended(reply)) => {
        let _ = out.send(reply);
      }
      Err(e) => {
        let _ = out.send(HubReply::Error(e.to_string()));
      }
    }
  }

  async fn relay_start(self: Arc<Self>, roster: &Roster, caller: Option<String>, depth: u32, args: AskArgs) -> Result<Start> {
    if depth >= MAX_DEPTH {
      return Err(anyhow!("Summoned agents may not summon further than {MAX_DEPTH} levels deep."));
    }
    let thread = args.thread.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
    let prompt = args.prompt.trim().to_owned();
    let latest = thread.as_deref().and_then(|th| self.core.lock().tree.relay_latest(th));
    if thread.is_some() && latest.is_none() {
      return Err(anyhow!("Unknown thread {}. Start a new one by leaving `thread` out.", thread.unwrap_or_default()));
    }
    // Waiting on a thread or reading its last round goes by the node alone: a persona edited or removed since then
    // does not take a finished answer away
    if let Some((node, h, name, _, state)) = &latest {
      let th = thread.clone().unwrap_or_default();
      let shown = node_persona(h, name.as_deref());
      if *state == SubagentState::Running {
        if prompt.is_empty() {
          return Ok(Start::Wait { thread: th, persona: shown });
        }
        return Err(busy(&shown, &th));
      }
      if prompt.is_empty() {
        let end = round_result(&self.core.lock(), node);
        return Ok(Start::Ended(final_reply(end, &th, &shown)));
      }
    }
    // A new round: the thread's persona as it is set now, else the named one
    let persona = match &latest {
      Some((_, h, name, _, _)) => thread_persona(roster, h, name.as_deref())?,
      None if args.agent.trim().is_empty() => return Err(anyhow!("Name the agent to summon (`agent`), or a `thread` to continue.")),
      None => roster.find(&args.agent).ok_or_else(|| {
        let ids: Vec<String> = roster.enabled().into_iter().map(|p| p.id).collect();
        anyhow!("No subagent named {:?}. Available: {}.", args.agent, if ids.is_empty() { "none".into() } else { ids.join(", ") })
      })?,
    };
    if prompt.is_empty() {
      return Err(anyhow!("`prompt` is empty: say what {} should do.", who(&persona)));
    }
    let mode = match args.mode.as_deref() {
      Some("work") => RelayMode::Work,
      Some("consult") => RelayMode::Consult,
      _ => persona.mode,
    };
    if self.deps.registry.try_get(&persona.agent).is_none() {
      return Err(anyhow!("{} runs in {:?}, which is not an agent Acpira knows.", persona.name, persona.agent));
    }
    let brief = compose_brief(&prompt, &persona, mode);
    let title = args.title.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(|x| clip(x, TITLE_MAX)).unwrap_or_else(|| clip(&prompt, TITLE_MAX));
    let round = latest.as_ref().map(|(_, h, ..)| h.round + 1).unwrap_or(1);
    let peer = latest.as_ref().and_then(|(_, _, _, p, _)| p.clone());
    let (tx, rx) = watch::channel::<RoundEnd>(None);
    // Everything that decides whether the round may start, and the round's registration, happen under one lock: two
    // calls on one thread cannot both start a round, and a waiter always finds the round it was told about
    let (node, thread, starting) = {
      let mut c = self.core.lock();
      if c.relays.closed {
        return Err(anyhow!("This Acpira session is closing."));
      }
      if let (Some(th), Some((was, ..))) = (&thread, &latest) {
        let now = c.tree.relay_latest(th);
        if !now.as_ref().is_some_and(|(id, .., state)| id == was && *state != SubagentState::Running) {
          return Err(busy(&persona, th));
        }
      }
      let running = c.tree.relay_running().len();
      if running >= MAX_RUNNING {
        return Err(anyhow!(
          "{running} summoned agents are already running in this session (at most {MAX_RUNNING}). Wait for one first: call ask_agent with its thread and an empty prompt."
        ));
      }
      let parent = caller.as_deref().and_then(|th| c.tree.relay_latest(th)).map(|(id, ..)| id);
      let turn_index = current_turn_index(&c);
      let harness = SubagentHarness { agent: persona.agent.clone(), persona: Some(persona.id.clone()), mode, thread: thread.clone().unwrap_or_default(), round, session_id: peer.clone() };
      let model = persona.model.clone();
      let node = c.tree.relay_open(RelayOpen { parent, turn_index, harness, role: persona.name.clone(), title, task: prompt.clone(), model });
      let thread = c.tree.relay_thread_of(&node).unwrap_or_else(|| node.clone());
      // A root row announced before the round existed takes the node now
      let opened = c.tree.relay_latest(&thread).map(|(_, h, ..)| h);
      let pending = opened.and_then(|h| {
        c.relays.calls.iter().position(|p| p.turn_index == turn_index && norm(&p.prompt) == norm(&prompt) && call_fits(&p.agent, p.thread.as_deref(), &h, Some(&persona.name)))
      });
      if let Some(i) = pending {
        let call = c.relays.calls.remove(i);
        let Core { tree, state, .. } = &mut *c;
        tree.relay_link_call(&node, &call.tool_call_id, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      }
      c.relays.rounds.insert(thread.clone(), rx);
      // Counted from here until its process is registered or ended, so a close right now still waits for it
      let starting = Starting::enter(&c.relays.starting);
      self.touch(&mut c);
      (node, thread, starting)
    };
    let me = self.clone();
    let round = Round { thread: thread.clone(), node: node.clone(), persona: persona.clone(), mode, peer, depth, brief };
    tokio::spawn(async move {
      let end = me.clone().relay_round(&round, starting).await;
      let reply = {
        let mut c = me.core.lock();
        match &end {
          Ok(stop) => {
            let cancelled = stop == "cancelled" || c.tree.relay_cancel_requested(&round.node);
            c.tree.relay_end(&round.node, if cancelled { SubagentState::Cancelled } else { SubagentState::Completed }, None);
          }
          Err(e) => c.tree.relay_end(&round.node, SubagentState::Failed, Some(e.to_string())),
        }
        me.drain_terminal(&mut c);
        c.relays.rounds.remove(&round.thread);
        me.touch(&mut c);
        round_result(&c, &round.node)
      };
      let _ = tx.send(Some(reply));
    });
    Ok(Start::Wait { thread, persona })
  }

  /// Wait for the thread's running round up to the deadline, with progress lines meanwhile
  async fn relay_wait(&self, thread: &str, persona: &SubagentPersona, deadline: Duration, out: &mpsc::UnboundedSender<HubReply>) {
    let rx = self.core.lock().relays.rounds.get(thread).cloned();
    let Some(mut rx) = rx else {
      // The round ended between the call's check and now: its outcome is on the node
      let end = {
        let c = self.core.lock();
        match c.tree.relay_latest(thread) {
          Some((id, ..)) => round_result(&c, &id),
          None => Err("is gone.".into()),
        }
      };
      let _ = out.send(final_reply(end, thread, persona));
      return;
    };
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    loop {
      let ended = rx.borrow().clone();
      if let Some(end) = ended {
        let _ = out.send(final_reply(end, thread, persona));
        return;
      }
      tokio::select! {
        r = rx.changed() => if r.is_err() { let _ = out.send(HubReply::Error(format!("{} stopped.", who(persona)))); return; },
        _ = tick.tick() => {
          if out.send(HubReply::Progress(self.relay_activity(thread).unwrap_or_else(|| "Working".into()))).is_err() {
            return;
          }
        }
        _ = tokio::time::sleep_until(until) => {
          let _ = out.send(HubReply::Done(format!(
            "{} is still working (thread: {thread}{}). Call ask_agent again with this thread and an empty prompt to keep waiting for its reply.",
            who(persona),
            self.relay_activity(thread).map(|a| format!("; latest: {a}")).unwrap_or_default()
          )));
          return;
        }
      }
    }
  }

  fn relay_activity(&self, thread: &str) -> Option<String> {
    let c = self.core.lock();
    c.tree.relay_latest(thread).and_then(|(id, ..)| c.tree.activity(&id))
  }

  /// Run one round: connect the thread (spawn, then resume or create its native session), send the brief, wait for the
  /// prompt's answer. Answers the stop reason; the setup notes the caller should hear (a model the CLI lacks …) stay
  /// with the node, so a later read of the round says them too
  async fn relay_round(self: Arc<Self>, r: &Round, starting: Starting) -> Result<String> {
    let mut notes = vec![];
    let (proc, sid, model) = self.relay_connect(r, &mut notes, starting).await?;
    for n in &notes {
      self.log(&format!("relay {}: {n}", short(&r.thread)));
    }
    {
      let mut c = self.core.lock();
      if c.relays.closed {
        return Err(anyhow!("the session closed"));
      }
      if !notes.is_empty() {
        c.relays.notes.insert(r.node.clone(), notes);
      }
      let turn_index = current_turn_index(&c);
      let Core { tree, state, .. } = &mut *c;
      tree.relay_bind(&r.node, &route_key(&r.thread), &sid, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      tree.relay_set_model(&r.node, model);
      // Cancelled while connecting: the prompt never goes out
      let cancelled = tree.relay_cancel_requested(&r.node);
      self.touch(&mut c);
      if cancelled {
        return Ok("cancelled".into());
      }
    }
    let answer = proc
      .request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": r.brief }] }))
      .await
      .map_err(anyhow::Error::new)?;
    Ok(answer.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_owned())
  }

  /// The thread's live process and native session (with the round's settings applied), started when there is none.
  /// Answers the model the session really runs
  /// `_starting` keeps the session's close waiting until this returns (the process registered, or ended)
  async fn relay_connect(self: &Arc<Self>, r: &Round, notes: &mut Vec<String>, _starting: Starting) -> Result<(Arc<AgentProcess>, String, Option<String>)> {
    let thread = r.thread.as_str();
    let wanted = (r.persona.model.clone(), r.persona.effort.clone());
    let live = {
      let c = self.core.lock();
      c.relays
        .procs
        .get(thread)
        .filter(|rp| rp.proc.alive() && !rp.peer.is_empty())
        .map(|rp| (rp.proc.clone(), rp.peer.clone(), rp.opened.clone(), rp.start_mode.clone(), rp.mode, rp.applied.clone()))
    };
    // A live thread keeps its process and session; a persona whose model / effort changed since is applied again, and
    // the mode follows the round
    if let Some((proc, sid, mut opened, start_mode, last, applied)) = live {
      if applied != wanted {
        relay_controls(&proc, &sid, &mut opened, &r.persona, notes).await;
      }
      let mode = if last == Some(r.mode) || relay_mode(&proc, &sid, &mut opened, start_mode.as_deref(), r.mode, &r.persona.agent, notes).await { Some(r.mode) } else { None };
      let model = current_name(opened.get("configOptions"), &MODEL_KEYS);
      if let Some(rp) = self.core.lock().relays.procs.get_mut(thread) {
        rp.opened = opened;
        rp.mode = mode;
        rp.applied = wanted;
      }
      return Ok((proc, sid, model));
    }
    let def = self.deps.registry.get(&r.persona.agent)?.clone();
    let bin = self.deps.registry.resolve_binary(&r.persona.agent).await.ok_or_else(|| anyhow!("{} is not installed", def.name))?;
    let quiet = Arc::new(AtomicBool::new(false));
    let handlers: Arc<dyn ClientHandlers> = Arc::new(RelayHandlers { session: self.me.clone(), thread: thread.to_owned(), quiet: quiet.clone() });
    self.log(&format!("relay {}: spawn {bin} {} for {}", short(thread), def.args.join(" "), r.persona.name));
    let proc = AgentProcess::spawn(&def, &bin, &self.cwd, handlers, None, None).await?;
    // Registered before anything else can fail, so the close or a failed open always ends it
    let closed = {
      let mut c = self.core.lock();
      if !c.relays.closed {
        let rp = RelayProc { proc: proc.clone(), peer: String::new(), opened: Value::Null, start_mode: None, mode: None, applied: (None, None) };
        c.relays.procs.insert(thread.to_owned(), rp);
      }
      c.relays.closed
    };
    if closed {
      proc.kill().await;
      return Err(anyhow!("the session closed"));
    }
    let opened = self.relay_open_session(&proc, r, &quiet, notes).await;
    let kept = {
      let mut c = self.core.lock();
      let closed = c.relays.closed;
      match (&opened, c.relays.procs.get_mut(thread).filter(|rp| Arc::ptr_eq(&rp.proc, &proc))) {
        (Ok((sid, opened, start_mode, mode_ok)), Some(rp)) if !closed => {
          rp.peer = sid.clone();
          rp.opened = opened.clone();
          rp.start_mode = start_mode.clone();
          rp.mode = mode_ok.then_some(r.mode);
          rp.applied = wanted;
          true
        }
        _ => false,
      }
    };
    if !kept {
      // Ended while still registered (a close meanwhile takes it over), then let go
      proc.kill().await;
      let mut c = self.core.lock();
      if c.relays.procs.get(thread).is_some_and(|rp| Arc::ptr_eq(&rp.proc, &proc)) {
        c.relays.procs.remove(thread);
      }
    }
    let (sid, opened, ..) = opened?;
    if !kept {
      return Err(anyhow!("the session closed"));
    }
    Ok((proc, sid, current_name(opened.get("configOptions"), &MODEL_KEYS)))
  }

  /// Open the thread's native session on a fresh process: resume / load the earlier one (never silently a new one in
  /// its place), else create it; then apply the persona's model, effort and the round's mode. Answers the session id,
  /// what it offers, the mode it opened in and whether the round's mode took
  async fn relay_open_session(&self, proc: &AgentProcess, r: &Round, quiet: &AtomicBool, notes: &mut Vec<String>) -> Result<(String, Value, Option<String>, bool)> {
    let req = self.relay_session_request(proc, &r.persona.agent, &r.thread, r.depth + 1).await;
    let (sid, mut opened) = match &r.peer {
      Some(peer) => self.relay_restore(proc, req, peer, &r.thread, quiet).await?,
      None => {
        let v = proc.request("session/new", req).await.map_err(anyhow::Error::new)?;
        (v.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned(), v)
      }
    };
    if sid.is_empty() {
      return Err(anyhow!("{} answered session/new without a session id", r.persona.agent));
    }
    let start_mode = current_mode(&opened);
    relay_controls(proc, &sid, &mut opened, &r.persona, notes).await;
    let mode_ok = relay_mode(proc, &sid, &mut opened, start_mode.as_deref(), r.mode, &r.persona.agent, notes).await;
    Ok((sid, opened, start_mode, mode_ok))
  }

  /// Reopen the thread's earlier native session; failing that is an error the caller reads, since a fresh session would
  /// have lost everything the thread said so far
  async fn relay_restore(&self, proc: &AgentProcess, req: Value, peer: &str, thread: &str, quiet: &AtomicBool) -> Result<(String, Value)> {
    let caps = proc.caps().clone();
    let mut r = req;
    r["sessionId"] = json!(peer);
    let mut why = vec![];
    if crate::json::truthy(caps.get("sessionCapabilities").and_then(|s| s.get("resume"))) {
      match proc.request("session/resume", r.clone()).await {
        Ok(v) => return Ok((peer.to_owned(), v)),
        Err(e) => why.push(format!("session/resume: {e}")),
      }
    }
    if crate::json::truthy(caps.get("loadSession")) {
      quiet.store(true, Ordering::Relaxed);
      let loaded = proc.request("session/load", r).await;
      quiet.store(false, Ordering::Relaxed);
      match loaded {
        Ok(v) => return Ok((peer.to_owned(), v)),
        Err(e) => why.push(format!("session/load: {e}")),
      }
    }
    if why.is_empty() {
      why.push("the CLI cannot reopen an earlier session".into());
    }
    self.log(&format!("relay {}: earlier conversation not restorable: {}", short(thread), why.join("; ")));
    Err(anyhow!(
      "The earlier conversation of thread {thread} could not be reopened ({}). Leave `thread` out to start a new conversation; it will not remember this one.",
      why.join("; ")
    ))
  }

  /// session/new params for a child: the shared MCP servers for its agent, plus Acpira's own leading back to this
  /// session one level deeper
  async fn relay_session_request(&self, proc: &AgentProcess, agent: &str, thread: &str, depth: u32) -> Value {
    let mut servers = match &self.deps.shared_mcp {
      Some(provider) => provider(agent.to_owned(), self.cwd.clone(), runtime_info_of(&proc.init).mcp).await.0,
      None => vec![],
    };
    if depth < MAX_DEPTH {
      servers.extend(self.deps.host_mcp.as_ref().and_then(|h| h.entry_for_session(agent, &self.arc(), Some(thread), depth)));
    } else {
      servers.extend(self.deps.host_mcp.as_ref().and_then(|h| h.entry_for(agent)));
    }
    json!({ "cwd": self.cwd, "mcpServers": servers })
  }

  /// A root `ask_agent` row (after the normalizer applied it): tie it to its round's node, or remember it until the
  /// round exists. A wait call (thread, no prompt) stays a visible "waiting" row
  pub(crate) fn relay_annotate(&self, c: &mut Core, u: &Value, turn_index: usize) {
    let tool_call_id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    // A call ends only after the hub answered it, so one still waiting here never started a round (refused, or the hub
    // was unreachable): a retry with the same arguments must not find this row first. The ending update rarely
    // repeats the tool's name, hence before the name check
    if matches!(u.get("status").and_then(Value::as_str), Some("completed" | "failed")) && !c.relays.calls.is_empty() {
      c.relays.calls.retain(|p| p.tool_call_id != tool_call_id);
      return;
    }
    let Some(args) = ask_agent_args(u) else { return };
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_owned();
    let (prompt, agent, thread) = (text("prompt"), text("agent"), Some(text("thread")).filter(|t| !t.is_empty()));
    if prompt.is_empty() {
      let name = thread.as_deref().and_then(|th| c.tree.relay_latest(th)).and_then(|(_, _, name, ..)| name);
      if let Some(block) = crate::acp::transcript::normalize::find_tool_mut(&mut c.state.turns, &tool_call_id) {
        block.verb_key = Some("verb.awaitSubagent".into());
        block.verb = t("verb.awaitSubagent");
        if let Some(name) = name {
          block.target = Some(name);
          block.target_mono = None;
        }
      }
      return;
    }
    if c.tree.relay_linked(&tool_call_id) {
      return;
    }
    match c.tree.relay_unlinked(&prompt, turn_index, |h, role| call_fits(&agent, thread.as_deref(), h, role)) {
      Some(node) => {
        c.relays.calls.retain(|p| p.tool_call_id != tool_call_id);
        let Core { tree, state, .. } = &mut *c;
        tree.relay_link_call(&node, &tool_call_id, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      }
      None => {
        // Calls of earlier turns never got a round: they are dropped rather than matched against a later one
        c.relays.calls.retain(|p| p.turn_index >= turn_index);
        if !c.relays.calls.iter().any(|p| p.tool_call_id == tool_call_id) {
          c.relays.calls.push(PendingCall { tool_call_id, agent, thread, prompt, turn_index });
        }
      }
    }
  }

  /// The session closes: no round starts from now on. Answers its children's processes, for the caller to end, and the
  /// count of starts still in flight (each ends its own process once it sees the close), for the caller to wait on
  pub(crate) fn relay_close(&self, c: &mut Core) -> (Vec<Arc<AgentProcess>>, watch::Receiver<usize>) {
    c.relays.closed = true;
    c.relays.rounds.clear();
    c.relays.calls.clear();
    c.relays.notes.clear();
    (c.relays.procs.drain().map(|(_, rp)| rp.proc).collect(), c.relays.starting.subscribe())
  }

  /// The process and native session a summoned node's cancel goes to (None = not summoned, or still connecting)
  pub(crate) fn relay_target(c: &Core, node: &str) -> Option<(Arc<AgentProcess>, String)> {
    let thread = c.tree.relay_thread_of(node)?;
    c.relays.procs.get(&thread).filter(|rp| !rp.peer.is_empty()).map(|rp| (rp.proc.clone(), rp.peer.clone()))
  }

  /// Cancel summoned rounds (and their pending cards): each child's own process gets the `session/cancel`; a round still
  /// connecting sees the request before it sends its prompt
  pub(crate) fn relay_cancel(&self, c: &mut Core, nodes: &[String]) {
    for id in nodes {
      if !c.tree.relay_cancel(id) {
        continue;
      }
      self.cancel_permissions_for(c, id);
      self.cancel_questions_for(c, id);
      if let Some((proc, sid)) = Self::relay_target(c, id) {
        proc.notify("session/cancel", json!({ "sessionId": sid }));
      }
    }
  }
}

/// The persona's model, then its effort, picked from the options the model switch answered (effort values depend on
/// the model); what the CLI does not offer or refuses becomes a note
async fn relay_controls(proc: &AgentProcess, sid: &str, opened: &mut Value, persona: &SubagentPersona, notes: &mut Vec<String>) {
  if let Some(m) = &persona.model {
    match pick_option(opened.get("configOptions"), &MODEL_KEYS, m) {
      Some((id, value)) => {
        if let Err(e) = set_config(proc, sid, opened, &id, &value).await {
          notes.push(format!("model {m:?} was refused ({e}); the CLI's default model runs"));
        }
      }
      None => notes.push(format!("{} offers no model matching {m:?}; its default model runs", persona.agent)),
    }
  }
  if let Some(e) = &persona.effort {
    match pick_option(opened.get("configOptions"), &EFFORT_KEYS, e) {
      Some((id, value)) => {
        if let Err(err) = set_config(proc, sid, opened, &id, &value).await {
          notes.push(format!("reasoning effort {e:?} was refused ({err})"));
        }
      }
      None => notes.push(format!("{} offers no reasoning effort {e:?} for this model; its default applies", persona.agent)),
    }
  }
}

/// The round's mode: a consult switches to the CLI's read-only mode where it has one; a work round leaves a read-only
/// mode for the one the session opened in (or the first writable one). Answers false when the CLI refused the switch,
/// so the next round tries again instead of taking the mode as set
async fn relay_mode(proc: &AgentProcess, sid: &str, opened: &mut Value, start: Option<&str>, mode: RelayMode, agent: &str, notes: &mut Vec<String>) -> bool {
  let ids = mode_ids(opened);
  let current = current_mode(opened);
  let want = match mode {
    RelayMode::Consult => match READ_ONLY_MODES.iter().find(|r| ids.iter().any(|m| m == *r)) {
      Some(m) => (*m).to_owned(),
      None => {
        notes.push(format!("{agent} has no read-only mode; it follows its own permission settings, so nothing stops a write"));
        return true;
      }
    },
    RelayMode::Work => match current.as_deref() {
      Some(c) if is_read_only(c) => {
        match start.filter(|s| !is_read_only(s)).map(str::to_owned).or_else(|| ids.iter().find(|m| !is_read_only(m)).cloned()) {
          Some(m) => m,
          None => return true,
        }
      }
      _ => return true,
    },
  };
  if current.as_deref() == Some(want.as_str()) {
    return true;
  }
  let listed = opened.pointer("/modes/availableModes").and_then(Value::as_array).is_some_and(|l| l.iter().any(|m| m.get("id").and_then(Value::as_str) == Some(want.as_str())));
  let r = if listed {
    match proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": want })).await {
      Ok(_) => {
        if let Some(m) = opened.get_mut("modes").filter(|m| m.is_object()) {
          m["currentModeId"] = Value::from(want.as_str());
        }
        Ok(())
      }
      Err(e) => Err(e.to_string()),
    }
  } else {
    let id = find_option(opened.get("configOptions"), &MODE_KEYS).and_then(|o| o.get("id")).and_then(Value::as_str).map(str::to_owned);
    match id {
      Some(id) => set_config(proc, sid, opened, &id, &want).await,
      None => return true,
    }
  };
  match r {
    Ok(()) => true,
    Err(e) => {
      notes.push(format!("switching {agent} to its {want} mode was refused ({e})"));
      false
    }
  }
}

/// What one round runs with
struct Round {
  thread: String,
  node: String,
  persona: SubagentPersona,
  mode: RelayMode,
  /// The thread's native session from its earlier rounds, to resume
  peer: Option<String>,
  depth: u32,
  brief: String,
}

enum Start {
  Wait { thread: String, persona: SubagentPersona },
  Ended(HubReply),
}

fn norm(s: &str) -> String {
  s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn busy(persona: &SubagentPersona, thread: &str) -> anyhow::Error {
  anyhow!("{} is still working on the previous round of thread {thread}. Call ask_agent with this thread and an empty prompt to wait for it.", who(persona))
}

/// A finished round as the caller reads it, the same live and later: the reply (and the round's setup notes), or why
/// there is none
fn round_result(c: &Core, node: &str) -> std::result::Result<String, String> {
  let Some((state, reply, result)) = c.tree.relay_outcome(node) else { return Err("is gone.".into()) };
  let notes = c.relays.notes.get(node).filter(|n| !n.is_empty());
  match state {
    SubagentState::Completed => Ok(match notes {
      Some(n) => format!("{}\n\n(Acpira: {}.)", reply.trim(), n.join("; ")),
      None => reply,
    }),
    SubagentState::Cancelled if reply.trim().is_empty() => Err("was cancelled.".into()),
    SubagentState::Cancelled => Err(format!("was cancelled. Its last words:\n\n{}", reply.trim())),
    SubagentState::Failed | SubagentState::Disconnected => Err(format!("failed: {}", result.as_deref().map(str::trim).filter(|r| !r.is_empty()).unwrap_or("no reason given"))),
    SubagentState::Running => Err("is still working.".into()),
  }
}

fn final_reply(end: std::result::Result<String, String>, thread: &str, persona: &SubagentPersona) -> HubReply {
  match end {
    Ok(reply) => HubReply::Done(done_text(&reply, thread, persona)),
    Err(e) => HubReply::Error(format!("{} {e}", who(persona))),
  }
}

fn done_text(reply: &str, thread: &str, persona: &SubagentPersona) -> String {
  let body = if reply.trim().is_empty() { "(no reply text)" } else { reply.trim() };
  format!("{body}\n\n---\nthread: {thread} — {}. Pass this thread to ask_agent to follow up in the same conversation.", who(persona))
}

/// The `ask_agent` arguments of a root tool update, when it is Acpira's tool (named like `show_image` per adapter)
fn ask_agent_args(u: &Value) -> Option<Map<String, Value>> {
  static ASK: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(?:^|acpira[_.:/-]{1,2})ask_agent$").unwrap());
  let raw = u.get("rawInput").and_then(Value::as_object);
  let named = |x: Option<&str>| x.map(str::trim).is_some_and(|x| ASK.is_match(x));
  if !(named(u.get("title").and_then(Value::as_str)) || named(u.get("name").and_then(Value::as_str)) || named(raw.and_then(|r| r.get("tool")).and_then(Value::as_str))) {
    return None;
  }
  let raw = raw?;
  Some(raw.get("arguments").and_then(Value::as_object).unwrap_or(raw).clone())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn picks_a_select_value_by_value_or_name_including_grouped_ones() {
    let options = json!([
      { "id": "model", "category": "model", "currentValue": "gpt-5.6", "options": [{ "value": "gpt-6-astra", "name": "GPT-6 Astra" }, { "value": "gpt-5.6", "name": "GPT-5.6 Terra" }] },
      { "id": "x", "category": "thought_level", "options": [{ "group": "g", "name": "G", "options": [{ "value": "max", "name": "Max" }] }] },
      { "id": "mode", "category": "mode", "options": [{ "value": "build", "name": "Build" }, { "value": "plan", "name": "Plan" }] },
    ]);
    assert_eq!(pick_option(Some(&options), &["model"], "GPT-6 Astra"), Some(("model".into(), "gpt-6-astra".into())));
    assert_eq!(pick_option(Some(&options), &["model"], "terra"), Some(("model".into(), "gpt-5.6".into())));
    assert_eq!(pick_option(Some(&options), &["thought_level"], "max"), Some(("x".into(), "max".into())));
    assert_eq!(pick_option(Some(&options), &["mode"], "plan"), Some(("mode".into(), "plan".into())));
    assert_eq!(pick_option(Some(&options), &["model"], "claude"), None);
    assert_eq!(current_name(Some(&options), &["model"]).as_deref(), Some("GPT-5.6 Terra"));
    assert_eq!(mode_ids(&json!({ "configOptions": options })), ["build", "plan"]);
  }

  #[test]
  fn recognizes_ask_agent_rows_across_adapter_namings() {
    let claude = json!({ "toolCallId": "t", "title": "mcp__acpira__ask_agent", "rawInput": { "agent": "codex-review", "prompt": "look" } });
    let codex = json!({ "toolCallId": "t", "title": "mcp.acpira.ask_agent", "rawInput": { "server": "acpira", "tool": "ask_agent", "arguments": { "prompt": "look" } } });
    let other = json!({ "toolCallId": "t", "title": "mcp__acpira__show_image", "rawInput": {} });
    assert_eq!(ask_agent_args(&claude).unwrap()["agent"], "codex-review");
    assert_eq!(ask_agent_args(&codex).unwrap()["prompt"], "look");
    assert!(ask_agent_args(&other).is_none());
  }

  #[test]
  fn a_call_fits_only_the_round_it_opened() {
    let h = |persona: &str, thread: &str, round: u32| SubagentHarness {
      agent: "codex".into(),
      persona: Some(persona.into()),
      mode: RelayMode::Consult,
      thread: thread.into(),
      round,
      session_id: None,
    };
    assert!(call_fits("codex-review", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(call_fits("Codex Review", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(!call_fits("claude-planner", None, &h("codex-review", "n1", 1), Some("Codex Review")));
    assert!(!call_fits("codex-review", None, &h("codex-review", "n1", 2), Some("Codex Review")));
    assert!(call_fits("", Some("n1"), &h("codex-review", "n1", 2), None));
    assert!(!call_fits("", Some("n2"), &h("codex-review", "n1", 2), None));
  }

  #[test]
  fn the_brief_carries_the_standing_rules_and_the_read_only_line_for_a_consult() {
    let p = SubagentPersona {
      id: "r".into(),
      name: "R".into(),
      agent: "codex".into(),
      model: None,
      effort: None,
      mode: RelayMode::Consult,
      when: String::new(),
      brief: Some("Three severity levels.".into()),
      enabled: true,
    };
    let consult = compose_brief(" review the diff ", &p, RelayMode::Consult);
    assert!(consult.starts_with("review the diff\n\nThree severity levels."));
    assert!(consult.contains("do not modify any file"));
    assert!(!compose_brief("fix it", &p, RelayMode::Work).contains("do not modify"));
  }
}
