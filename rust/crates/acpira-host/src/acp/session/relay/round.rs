//! One round on a thread: connecting its process and native session (resumed, never silently a new one), the prompt
//! with what it hands over, and the follow-ups a child's own rounds hold it for

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::subagents::RelayMode;
use acpira_shared::transcript::Draft;

use super::controls::{current_mode, relay_controls, relay_mode, running_controls, stuck_in_bypass};
use super::texts::{change_note, compose_brief, round_edits, short, wake_text};
use super::{MAX_WAKES, NOTE_FILES, Owed, PACK_LINE, PACK_NAME, RelayHandlers, RelayProc, Round, Starting, route_key};
use crate::acp::session::attachments::{prepare_prompt, prompt_caps_of};
use crate::acp::session::updates::current_turn_index;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::runtime_info_of;
use crate::acp::transcript::subagent_tree::RouteCtx;
use crate::acp::transport::process::{AgentProcess, ClientHandlers};
use crate::handoff::{self, Level};

impl AcpSession {
  /// Run one round: connect the thread (spawn, then resume or create its native session), send the brief, wait for the
  /// prompt's answer. Answers the stop reason; the setup notes the caller should hear (a model the CLI lacks …) stay
  /// with the node, so a later read of the round says them too
  pub(super) async fn relay_round(self: Arc<Self>, r: &Round, starting: Starting) -> Result<String> {
    let mut notes = vec![];
    let (proc, sid, (model, effort)) = self.relay_connect(r, &mut notes, starting).await?;
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
      tree.relay_set_controls(&r.node, model, effort);
      // Cancelled while connecting: the prompt never goes out
      let cancelled = tree.relay_cancel_requested(&r.node);
      self.touch(&mut c);
      if cancelled {
        return Ok("cancelled".into());
      }
    }
    self.core.lock().relays.follow_ups.remove(&r.thread);
    // The work tree before the prompt: a round that runs commands is told what changed meanwhile, not only what its
    // edit rows name
    let watch = [PathBuf::from(self.hook_folder())];
    let host = self.hook_host().await;
    let before = match &host {
      Some(h) if h.in_work_tree(&watch[0]).await => Some(h.snapshot(&watch).await),
      _ => None,
    };
    let (blocks, handed) = self.relay_blocks(r, &proc).await;
    if let Some(n) = handed {
      self.core.lock().relays.notes.entry(r.node.clone()).or_default().push(n);
    }
    let end = self.relay_prompt(&proc, &sid, blocks).await;
    let end = match end {
      Ok(stop) => self.relay_follow_ups(&proc, &sid, r, stop).await,
      Err(e) => Err(e),
    };
    let (edits, anywhere) = round_edits(self.core.lock().tree.relay_turns(&r.node).unwrap_or(&[]));
    let changed = match &before {
      Some(b) if anywhere => match &host {
        Some(h) => Some(h.snapshot(&watch).await.changed_since(b)),
        None => Some(vec![]),
      },
      Some(_) => Some(vec![]),
      None => None,
    };
    // A read-only round is told about only when it did change something
    let clean = edits.is_empty() && changed.as_ref().is_none_or(|c| c.is_empty());
    let note = (r.mode != RelayMode::Consult || !clean).then(|| change_note(&self.cwd, &edits, anywhere, changed.as_deref()));
    // Whatever this thread sent off and has not had by now goes unread: its round is over
    let mut c = self.core.lock();
    if let Some(note) = note {
      c.relays.notes.entry(r.node.clone()).or_default().push(note);
    }
    c.relays.follow_ups.remove(&r.thread);
    for o in c.relays.ledger.iter_mut().filter(|o| o.owner.as_deref() == Some(r.thread.as_str())) {
      if matches!(o.state, Owed::Running | Owed::Unread) {
        o.state = Owed::Void;
      }
    }
    end
  }

  async fn relay_prompt(&self, proc: &AgentProcess, sid: &str, blocks: Vec<Value>) -> Result<String> {
    let answer = proc.request("session/prompt", json!({ "sessionId": sid, "prompt": blocks })).await.map_err(anyhow::Error::new)?;
    Ok(answer.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_owned())
  }

  /// The round's prompt: its brief, then what it hands over of the caller's changes. A thread's first handover is the
  /// whole pack, staged like a dropped text file (an embedded resource where the CLI takes them, inline text
  /// otherwise); a later round of the same thread hears in one line what moved since. Answers a note for the caller
  /// when there was nothing to hand over, or the pack had to be cut down to fit
  async fn relay_blocks(&self, r: &Round, proc: &AgentProcess) -> (Vec<Value>, Option<String>) {
    let plain = |handover: Option<&str>| vec![json!({ "type": "text", "text": compose_brief(&r.prompt, &r.persona, r.mode, handover) })];
    let Some(h) = &r.handoff else { return (plain(None), None) };
    let told = self.core.lock().relays.packs.get(&r.thread).cloned();
    if let Some((base, before)) = told {
      let now = match self.handoff_stats(&base, h.scope.as_ref()).await {
        Ok(Some(now)) => now,
        _ => return (plain(None), None),
      };
      let moved = handoff::changed_since(&before, &now);
      self.core.lock().relays.packs.insert(r.thread.clone(), (base, now));
      if moved.is_empty() {
        return (plain(None), None);
      }
      let shown = if moved.len() > NOTE_FILES {
        format!("{} and {} more", moved[..NOTE_FILES].join(", "), moved.len() - NOTE_FILES)
      } else {
        moved.join(", ")
      };
      return (plain(Some(&format!("Changed since your last round: {shown}."))), None);
    }
    let pack = match &h.shared {
      Some(p) => (**p).clone(),
      None => self.handoff_build(h.scope.as_ref(), &h.recipe).await.unwrap_or_else(|e| {
        self.log(&format!("relay {}: handoff pack: {e:#}", short(&r.thread)));
        None
      }),
    };
    let Some(pack) = pack else { return (plain(None), Some("there were no uncommitted changes to hand over".into())) };
    self.core.lock().relays.packs.insert(r.thread.clone(), (pack.base.clone(), pack.files.clone()));
    // Staged like a dropped text file in the child's own CLI: the recipe's cap keeps the pack under the attachment limit
    let def = self.deps.registry.try_get(&r.persona.agent).cloned().unwrap_or_default();
    let caps = prompt_caps_of(Some(&proc.init), &def);
    let draft = Draft::Text { name: PACK_NAME.into(), text: pack.text.clone(), context: None };
    let brief = compose_brief(&r.prompt, &r.persona, r.mode, Some(PACK_LINE));
    let files = self.file_reader(Some(std::slice::from_ref(&draft))).await;
    let prepared = prepare_prompt(&self.id, &brief, &[draft], &self.deps.blobs, &files, Some(caps)).await;
    let cut = match pack.level {
      Level::Full => None,
      Level::Trimmed => Some("some full texts left out"),
      Level::Plain => Some("plain hunks, no full texts"),
      Level::Stat => Some("paths and line counts, with only the smallest diffs"),
    };
    (prepared.blocks, cut.map(|c| format!("the handoff pack was cut down to fit its size cap ({c})")))
  }

  /// The child answered, but rounds it sent off are still out: its round goes on. Once a batch of them has ended (or one
  /// asks something), the results go to its native session as one more prompt, after its answer so far is sealed and an
  /// automatic message opens the next turn of its transcript; at most `MAX_WAKES` follow-ups, then the last answer is
  /// the round's reply
  async fn relay_follow_ups(&self, proc: &AgentProcess, sid: &str, r: &Round, mut stop: String) -> Result<String> {
    let mut sent = 0;
    loop {
      if stop == "cancelled" {
        return Ok(stop);
      }
      let (mut ended, mut asked) = {
        let c = self.core.lock();
        (c.relays.ended.subscribe(), c.relays.asked.subscribe())
      };
      let text = {
        let mut c = self.core.lock();
        if c.relays.closed || c.tree.relay_cancel_requested(&r.node) || !c.relays.owes(&r.thread) || sent >= MAX_WAKES {
          return Ok(stop);
        }
        let nodes = c.relays.deliverable(Some(&r.thread));
        let asks: Vec<(String, String)> = c
          .relays
          .ledger
          .iter()
          .filter(|o| o.owner.as_deref() == Some(r.thread.as_str()) && o.state == Owed::Running)
          .filter_map(|o| Self::relay_question(&c, &o.node).map(|(b, _)| (o.node.clone(), b)))
          .filter(|(_, b)| !c.relays.told.contains(b))
          .collect();
        if nodes.is_empty() && asks.is_empty() {
          c.tree.relay_waiting(&r.node);
          self.touch(&mut c);
          None
        } else {
          for n in &nodes {
            if let Some(o) = c.relays.owing(n) {
              o.state = Owed::Delivered;
            }
          }
          c.relays.told.extend(asks.iter().map(|(_, b)| b.clone()));
          let text = wake_text(&c, &nodes, &asks.into_iter().map(|(n, _)| n).collect::<Vec<_>>());
          sent += 1;
          c.relays.follow_ups.insert(r.thread.clone(), sent as usize);
          c.tree.relay_follow_up(&r.node, &text);
          self.touch(&mut c);
          Some(text)
        }
      };
      match text {
        Some(text) => {
          self.log(&format!("relay {}: follow-up {sent} with the rounds it sent off", short(&r.thread)));
          stop = self.relay_prompt(proc, sid, vec![json!({ "type": "text", "text": text })]).await?;
        }
        None => {
          tokio::select! {
            _ = ended.changed() => {}
            _ = asked.changed() => {}
          }
        }
      }
    }
  }

  /// The thread's live process and native session (with the round's settings applied), started when there is none.
  /// Answers the model and reasoning effort the session really runs
  /// `_starting` keeps the session's close waiting until this returns (the process registered, or ended)
  async fn relay_connect(self: &Arc<Self>, r: &Round, notes: &mut Vec<String>, _starting: Starting) -> Result<(Arc<AgentProcess>, String, (Option<String>, Option<String>))> {
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
      // Still in full access (a refused step down, or no mode to step down to): the prompt would run without the
      // approvals this round asked for
      let stuck = stuck_in_bypass(&opened, r.mode);
      let controls = running_controls(&opened);
      if let Some(rp) = self.core.lock().relays.procs.get_mut(thread) {
        rp.opened = opened;
        rp.mode = mode;
        rp.applied = wanted;
      }
      if stuck {
        return Err(anyhow!("{} stayed in its full-access mode, so this {:?} round was not sent: {}", r.persona.agent, r.mode, notes.join("; ")));
      }
      return Ok((proc, sid, controls));
    }
    let def = self.deps.registry.get(&r.persona.agent)?.clone();
    if let Some(f) = self.remote_folder().filter(|_| r.persona.agent != crate::acp::agents::registry::SELF_AGENT_ID) {
      return Err(anyhow!(crate::i18n::tp("host.remoteBuiltinOnly", &[("agent", &def.name), ("host", &f.alias)])));
    }
    let bin = self.deps.registry.resolve_binary(&r.persona.agent).await.ok_or_else(|| anyhow!("{} is not installed", def.name))?;
    let quiet = Arc::new(AtomicBool::new(false));
    let handlers: Arc<dyn ClientHandlers> = Arc::new(RelayHandlers { session: self.me.clone(), thread: thread.to_owned(), quiet: quiet.clone() });
    // The CLI signs in the way a new session of it would: with the parent's own account when the persona runs in the
    // same CLI, else that CLI's default saved account (Devin refuses its local credentials over ACP); with none saved,
    // the CLI's own credentials
    let account = self.deps.accounts.as_ref().and_then(|h| {
      let same = (r.persona.agent == self.agent).then(|| self.core.lock().account_id.clone()).flatten();
      same.or_else(|| h.default_account(r.persona.agent.clone()))
    });
    let env = match (&account, &self.deps.accounts) {
      (Some(a), Some(h)) => h.spawn_env(r.persona.agent.clone(), a.clone()).await,
      _ => None,
    };
    // A summoned agent starts the way the plugins' `agent.launch` hooks have it too, on the parent session's behalf
    let def = crate::acp::session::plugin_hooks::plugin_launch(self.deps.plugin_runtime.as_ref(), &def, &self.id, &self.process_cwd()).await;
    self.log(&format!("relay {}: spawn {bin} {} for {}", short(thread), def.args.join(" "), r.persona.name));
    let proc = AgentProcess::spawn(&def, &bin, &self.process_cwd(), handlers, env.as_ref(), None).await?;
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
    let signed_in = match (&account, &self.deps.accounts) {
      (Some(a), Some(h)) => {
        h.authenticate(r.persona.agent.clone(), a.clone(), proc.clone()).await.map_err(|e| anyhow!("{} could not sign in with its saved account: {e}", def.name))
      }
      _ => Ok(()),
    };
    let opened = match signed_in {
      Ok(()) => self.relay_open_session(&proc, r, &quiet, notes).await,
      Err(e) => Err(e),
    };
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
    Ok((proc, sid, running_controls(&opened)))
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
    // A CLI that starts in full access and cannot leave it gets no prompt for a round that wants approvals
    if stuck_in_bypass(&opened, r.mode) {
      return Err(anyhow!("{} stayed in its full-access mode, so this {:?} round was not sent: {}", r.persona.agent, r.mode, notes.join("; ")));
    }
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
      Some(provider) => provider(agent.to_owned(), self.process_cwd(), runtime_info_of(&proc.init).mcp, self.remote_folder().is_none()).await.0,
      None => vec![],
    };
    // Past the depth limit the entry still leads back here (`ask_caller`, the session tools); the hub lists no personas
    let entry = self.deps.host_mcp.as_ref().and_then(|h| h.entry_for_session(agent, &self.arc(), Some(thread), depth));
    crate::host_mcp::push_entry(&mut servers, entry);
    let mut req = self.with_remote(json!({ "cwd": self.cwd, "mcpServers": servers })).await;
    self.with_prompt_layers(&mut req, agent);
    req
  }
}
