//! The hub's calls: `ask_agent` for one persona or a group, `ask_caller` from a child, starting a round, and waiting on
//! a thread with progress lines

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use acpira_shared::subagents::{
  EXECUTOR_BRIEF, RelayMode, SIDEKICK_ID, SubagentDuty, SubagentGroup, SubagentHarness, SubagentPersona, SubagentState,
};

use super::controls::{caller_access, round_mode};
use super::personas::{
  NO_SIDEKICK, SIDEKICK_CATEGORY, SIDEKICK_CONFIG, SIDEKICK_EFFORT_CATEGORY, SIDEKICK_EFFORT_CONFIG, SIDEKICK_WHEN, call_fits, idle_thread,
  node_persona, thread_persona, who,
};
use super::texts::{
  answer_text, busy, caller_answers, clip, dispatched_text, final_reply, group_dispatched_text, norm, question_text, round_result,
};
use super::{
  DEFAULT_WAIT, Handoff, MAX_WAIT, Owed, Owing, PROGRESS_EVERY, Round, RoundEnd, Start, Starting, TITLE_MAX, Via, Waiting, caller_scope,
};
use crate::acp::session::controls::picked_controls;
use crate::acp::session::updates::current_turn_index;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::subagent_tree::{RelayOpen, RouteCtx};
use crate::handoff;
use crate::relay::roster::Roster;
use crate::relay::wire::{AskArgs, AskCallerArgs, HubReply};
use crate::relay::{MAX_DEPTH, MAX_RUNNING};

impl AcpSession {
  /// The sidekick this session's built-in agent has picked (its `sidekick` select, picks in flight and a reopened
  /// record's value included), as the executor the hub lists first for the root: the built-in agent on that model,
  /// following the caller's mode, at the effort picked for it, in one conversation that later calls continue
  pub(crate) fn sidekick_persona(&self) -> Option<SubagentPersona> {
    let (pick, effort) = {
      let c = self.core.lock();
      let options = picked_controls(&c).options;
      let value = |id: &str, category: &str| options.iter().find(|o| o.id == id && o.category.as_deref() == Some(category)).and_then(|o| o.value.clone());
      (value(SIDEKICK_CONFIG, SIDEKICK_CATEGORY), value(SIDEKICK_EFFORT_CONFIG, SIDEKICK_EFFORT_CATEGORY))
    };
    let pick = pick?;
    (!pick.is_empty() && pick != NO_SIDEKICK).then(|| SubagentPersona {
      id: SIDEKICK_ID.into(),
      name: "Sidekick".into(),
      agent: self.agent.clone(),
      model: Some(pick),
      effort: effort.filter(|e| !e.is_empty()),
      mode: RelayMode::Inherit,
      when: SIDEKICK_WHEN.into(),
      brief: Some(EXECUTOR_BRIEF.into()),
      enabled: true,
      duty: SubagentDuty::Executor,
      changes: false,
    })
  }

  /// A persona by id or name as this caller sees it: to the root, `sidekick` is its own sidekick while it has one
  fn find_persona(&self, roster: &Roster, caller: Option<&str>, id: &str) -> Option<SubagentPersona> {
    if caller.is_none()
      && id.trim().eq_ignore_ascii_case(SIDEKICK_ID)
      && let Some(p) = self.sidekick_persona()
    {
      return Some(p);
    }
    roster.find(id)
  }

  /// One `ask_agent` call from an agent of this session (the root, or a summoned child when `caller` names its thread)
  pub async fn relay_ask(self: Arc<Self>, roster: Arc<Roster>, caller: Option<String>, depth: u32, args: AskArgs, out: mpsc::UnboundedSender<HubReply>) {
    // A new task naming a group (and no persona of that name) goes to each of its members
    let new_task = args.thread.as_deref().is_none_or(|t| t.trim().is_empty()) && !args.prompt.trim().is_empty();
    if new_task
      && self.find_persona(&roster, caller.as_deref(), &args.agent).is_none()
      && let Some((group, members)) = roster.find_group(&args.agent)
    {
      return self.relay_group(roster, caller, depth, args, group, members, out).await;
    }
    let deadline = args.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    // A new task is sent off unless the call asks to wait: the ledger delivers the reply, to the root in an automatic
    // message, to a summoned caller in a follow-up within its own round (`relay_follow_ups`)
    let wait = args.wait.unwrap_or(false);
    match self.clone().relay_start(&roster, caller, depth, args, None).await {
      Ok(Start::Started { thread, persona }) if !wait => {
        let _ = out.send(HubReply::Done(dispatched_text(&thread, &persona)));
      }
      Ok(Start::Answered { thread, persona }) if !wait => {
        let _ = out.send(HubReply::Done(format!(
          "Your answer went to {} (thread: {thread}). It carries on; Acpira sends you its reply in a message when it is done.",
          who(&persona)
        )));
      }
      Ok(Start::Answered { thread, persona }) => self.relay_wait(&thread, &persona, deadline, &out).await,
      Ok(Start::Started { thread, persona }) | Ok(Start::Wait { thread, persona }) => self.relay_wait(&thread, &persona, deadline, &out).await,
      Ok(Start::Ended(reply)) => {
        let _ = out.send(reply);
      }
      Err(e) => {
        let _ = out.send(HubReply::Error(e.to_string()));
      }
    }
  }

  /// A new task for a group: a round per enabled member, each with the same task and the same handoff pack (built once,
  /// before any round starts), all started by this one call so their replies are delivered together. Refused whole
  /// when the members would not all fit beside the rounds already running. With `wait` the call answers once every
  /// member has (or the deadline came), all the replies in one answer
  #[allow(clippy::too_many_arguments, reason = "one round's whole brief, passed once per group call")]
  async fn relay_group(
    self: Arc<Self>,
    roster: Arc<Roster>,
    caller: Option<String>,
    depth: u32,
    args: AskArgs,
    group: SubagentGroup,
    members: Vec<SubagentPersona>,
    out: mpsc::UnboundedSender<HubReply>,
  ) {
    let deadline = args.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    if depth >= MAX_DEPTH {
      let _ = out.send(HubReply::Error(format!("Summoned agents may not summon further than {MAX_DEPTH} levels deep.")));
      return;
    }
    let running = self.core.lock().tree.relay_running().len();
    if running + members.len() > MAX_RUNNING {
      let _ = out.send(HubReply::Error(format!(
        "{} has {} members, but {running} summoned agents are already running in this session (at most {MAX_RUNNING} at once). \
Wait for some of them first: call ask_agent with their thread and an empty prompt.",
        group.name,
        members.len()
      )));
      return;
    }
    let pack = if members.iter().any(|p| args.changes.unwrap_or(p.changes)) {
      let scope = caller_scope(&self.core.lock(), caller.as_deref());
      let recipe = handoff::recipe_in(&roster.dir());
      let built = self.handoff_build(scope.as_ref(), &recipe).await.unwrap_or_else(|e| {
        self.log(&format!("relay group {}: handoff pack: {e:#}", group.id));
        None
      });
      Some(Arc::new(built))
    } else {
      None
    };
    let via = Via { group: group.id.clone(), pack };
    let mut started: Vec<(String, SubagentPersona)> = vec![];
    let mut refused: Vec<String> = vec![];
    for p in &members {
      let one = AskArgs { agent: p.id.clone(), thread: None, answers: None, ..args.clone() };
      match self.clone().relay_start(&roster, caller.clone(), depth, one, Some(via.clone())).await {
        Ok(Start::Started { thread, persona }) => started.push((thread, persona)),
        Ok(_) => {}
        Err(e) => refused.push(format!("{}: {e}", p.name)),
      }
    }
    let refused_text = (!refused.is_empty()).then(|| format!("Could not start: {}", refused.join(" ")));
    if started.is_empty() {
      let _ = out.send(HubReply::Error(format!("No member of {} could start. {}", group.name, refused.join(" "))));
      return;
    }
    if !args.wait.unwrap_or(false) {
      let text = group_dispatched_text(&group, &started);
      let _ = out.send(HubReply::Done(refused_text.map(|r| format!("{text} {r}")).unwrap_or(text)));
      return;
    }
    let Some(replies) = self.relay_wait_all(&started, deadline, &out).await else { return };
    let mut parts = vec![format!("The replies of {} ({} of its members):", group.name, started.len())];
    for ((_, persona), r) in started.iter().zip(replies) {
      let text = match r {
        HubReply::Done(t) | HubReply::Error(t) => t,
        _ => continue,
      };
      parts.push(format!("## {}\n\n{text}", who(persona)));
    }
    parts.extend(refused_text);
    let _ = out.send(HubReply::Done(parts.join("\n\n")));
  }

  /// Waits on several threads at once, each the way a single call would (`relay_wait`), with progress lines naming whose
  /// they are. Answers each thread's final line, in order; None once the call was abandoned
  async fn relay_wait_all(self: &Arc<Self>, threads: &[(String, SubagentPersona)], deadline: Duration, out: &mpsc::UnboundedSender<HubReply>) -> Option<Vec<HubReply>> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(usize, HubReply)>();
    for (i, (thread, persona)) in threads.iter().enumerate() {
      let (one, mut lines) = mpsc::unbounded_channel::<HubReply>();
      let (s, th, p) = (self.clone(), thread.clone(), persona.clone());
      tokio::spawn(async move { s.relay_wait(&th, &p, deadline, &one).await });
      let tx = tx.clone();
      tokio::spawn(async move {
        while let Some(r) = lines.recv().await {
          if tx.send((i, r)).is_err() {
            break;
          }
        }
      });
    }
    drop(tx);
    let mut finals: Vec<Option<HubReply>> = vec![None; threads.len()];
    while let Some((i, r)) = rx.recv().await {
      match r {
        HubReply::Progress(line) => {
          if out.send(HubReply::Progress(format!("{}: {line}", threads[i].1.name))).is_err() {
            return None;
          }
        }
        r => finals[i] = Some(r),
      }
    }
    Some(finals.into_iter().zip(threads).map(|(r, (_, p))| r.unwrap_or_else(|| HubReply::Error(format!("{} stopped.", who(p))))).collect())
  }

  /// `ask_caller` from the CLI of a summoned thread: the question becomes a card on the thread's running round and travels
  /// like one its CLI asked itself (a call waiting on the thread, the automatic message, the user's card). No answer
  /// within the client's bound settles it skipped, and the child goes on with an assumption it states
  pub async fn relay_ask_caller(self: Arc<Self>, thread: String, args: AskCallerArgs, out: mpsc::UnboundedSender<HubReply>) {
    use acpira_shared::transcript::{Question, QuestionKind, QuestionOption, QuestionOutcome};
    let text = args.question.trim().to_owned();
    if text.is_empty() {
      let _ = out.send(HubReply::Error("`question` is empty: say what you need decided.".into()));
      return;
    }
    let deadline = args.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    let mut options: Vec<QuestionOption> = vec![];
    for label in args.options.iter().map(|o| o.trim()).filter(|o| !o.is_empty()) {
      if !options.iter().any(|o| o.label == label) {
        options.push(QuestionOption { id: label.to_owned(), label: label.to_owned(), description: None });
      }
    }
    let kind = if options.is_empty() { QuestionKind::Text } else { QuestionKind::Single };
    let question = Question { id: "answer".into(), title: None, text, kind, options, other: None, numeric: None, required: None };
    let opened = {
      let mut c = self.core.lock();
      match c.tree.relay_latest(&thread) {
        Some((node, .., SubagentState::Running)) if Self::relay_question(&c, &node).is_none() => Ok(self.open_caller_question(&mut c, &node, vec![question])),
        Some((.., SubagentState::Running)) => Err("You already have a question waiting for an answer; wait for it before asking another."),
        _ => Err("Your round is over, so there is no caller waiting to answer."),
      }
    };
    let (mut rx, block) = match opened {
      Ok(o) => o,
      Err(e) => {
        let _ = out.send(HubReply::Error(e.into()));
        return;
      }
    };
    // With the root idle, the automatic message carries the question now
    self.relay_nudge();
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    let reply = loop {
      tokio::select! {
        r = &mut rx => break r.ok(),
        // A progress line that cannot be written: the tool call was abandoned
        _ = tick.tick() => if out.send(HubReply::Progress("Waiting for an answer".into())).is_err() {
          self.close_question(&block, QuestionOutcome::Cancelled);
          return;
        },
        _ = tokio::time::sleep_until(until) => {
          self.close_question(&block, QuestionOutcome::Skipped);
          let _ = out.send(HubReply::Done(
            "No answer came in time. Go on with the most reasonable assumption, and state it in your final reply.".into(),
          ));
          return;
        }
      }
    };
    let _ = out.send(match reply {
      Some(v) if v.get("cancelled").is_some() || v.is_null() => HubReply::Error("The question was withdrawn: your round is being stopped.".into()),
      Some(v) => match v.get("answers").and_then(|a| a.get("answer")).filter(|_| v.get("skipped") != Some(&Value::Bool(true))) {
        Some(a) => HubReply::Done(format!("Answer: {}", answer_text(a))),
        None => HubReply::Done("The question was skipped. Go on with the most reasonable assumption, and state it in your final reply.".into()),
      },
      None => HubReply::Error("The question was withdrawn: your round is being stopped.".into()),
    });
  }

  async fn relay_start(self: Arc<Self>, roster: &Roster, caller: Option<String>, depth: u32, args: AskArgs, via: Option<Via>) -> Result<Start> {
    if depth >= MAX_DEPTH {
      return Err(anyhow!("Summoned agents may not summon further than {MAX_DEPTH} levels deep."));
    }
    let mut thread = args.thread.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
    let prompt = args.prompt.trim().to_owned();
    // A scout or an executor keeps its conversation: a new task without `thread` continues the idle one this caller
    // started (unless the call asks for a fresh start); a reviewer, and any other persona, starts fresh
    if thread.is_none() && !prompt.is_empty() && args.new_thread != Some(true)
      && let Some(p) = self.find_persona(roster, caller.as_deref(), &args.agent).filter(|p| p.duty.keeps_thread())
    {
      thread = idle_thread(&self.core.lock(), &p, caller.as_deref());
    }
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
        // A child waiting on a question: `answers` (or a prompt, for a single question) answers it, like the user's card
        let open = { Self::relay_question(&self.core.lock(), node) };
        if let Some((block, questions)) = open
          && (args.answers.is_some() || !prompt.is_empty())
        {
          let answers = caller_answers(&questions, args.answers.as_deref(), &prompt).map_err(|e| anyhow!("{} {e}", who(&shown)))?;
          self.answer_questions(&block, &answers, false);
          self.core.lock().relays.told.insert(block);
          return Ok(Start::Answered { thread: th, persona: shown });
        }
        if args.answers.is_some() {
          return Err(anyhow!("{} has no open question now (the user may have answered it already).", who(&shown)));
        }
        if prompt.is_empty() {
          return Ok(Start::Wait { thread: th, persona: shown });
        }
        return Err(busy(&shown, &th));
      }
      if args.answers.is_some() {
        return Err(anyhow!("{} has no open question: its round is over.", who(&shown)));
      }
      if prompt.is_empty() {
        let mut c = self.core.lock();
        c.relays.read(node);
        let end = round_result(&c, node);
        return Ok(Start::Ended(final_reply(end, &th, &shown)));
      }
    }
    // A new round: the thread's persona as it is set now, else the named one
    let persona = match &latest {
      Some((_, h, name, _, _)) => thread_persona(roster, self.sidekick_persona(), h, name.as_deref())?,
      None if args.agent.trim().is_empty() => return Err(anyhow!("Name the agent to summon (`agent`), or a `thread` to continue.")),
      None => self.find_persona(roster, caller.as_deref(), &args.agent).ok_or_else(|| {
        let sidekick = caller.is_none().then(|| self.sidekick_persona()).flatten();
        let ids: Vec<String> = sidekick.into_iter().chain(roster.enabled()).map(|p| p.id).collect();
        anyhow!("No subagent named {:?}. Available: {}.", args.agent, if ids.is_empty() { "none".into() } else { ids.join(", ") })
      })?,
    };
    if prompt.is_empty() {
      return Err(anyhow!("`prompt` is empty: say what {} should do.", who(&persona)));
    }
    let mode = round_mode(persona.mode, args.mode.as_deref(), || caller_access(&self.core.lock(), caller.as_deref()));
    if self.deps.registry.try_get(&persona.agent).is_none() {
      return Err(anyhow!("{} runs in {:?}, which is not an agent Acpira knows.", persona.name, persona.agent));
    }
    // `changes` decides, else the persona's switch; a thread that was handed the pack keeps hearing what moved since
    let told = thread.as_deref().is_some_and(|th| self.core.lock().relays.packs.contains_key(th));
    let handoff = args.changes.unwrap_or(persona.changes || told).then(|| Handoff {
      scope: caller_scope(&self.core.lock(), caller.as_deref()),
      recipe: handoff::recipe_in(&roster.dir()),
      shared: via.as_ref().and_then(|v| v.pack.clone()),
    });
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
      let harness = SubagentHarness {
        agent: persona.agent.clone(),
        persona: Some(persona.id.clone()),
        mode,
        thread: thread.clone().unwrap_or_default(),
        round,
        session_id: peer.clone(),
        effort: None,
        group: via.as_ref().map(|v| v.group.clone()),
      };
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
      let cohort = match caller.as_deref() {
        None => turn_index,
        Some(th) => c.relays.follow_ups.get(th).copied().unwrap_or(0),
      };
      c.relays.ledger.push(Owing { node: node.clone(), owner: caller.clone(), cohort, state: Owed::Running });
      // Counted from here until its process is registered or ended, so a close right now still waits for it
      let starting = Starting::enter(&c.relays.starting);
      self.touch(&mut c);
      (node, thread, starting)
    };
    let me = self.clone();
    let round = Round { thread: thread.clone(), node, persona: persona.clone(), mode, peer, depth, prompt: prompt.clone(), handoff };
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
        let waited = c.relays.waiters.get(&round.thread).is_some_and(|n| *n > 0);
        if let Some(o) = c.relays.owing(&round.node).filter(|o| o.state == Owed::Running) {
          o.state = if waited { Owed::Read } else { Owed::Unread };
        }
        me.drain_terminal(&mut c);
        c.relays.rounds.remove(&round.thread);
        c.relays.ended.send_modify(|n| *n += 1);
        me.touch(&mut c);
        round_result(&c, &round.node)
      };
      let _ = tx.send(Some(reply));
      me.relay_settled();
    });
    Ok(Start::Started { thread, persona })
  }

  /// Wait for the thread's running round up to the deadline, with progress lines meanwhile
  async fn relay_wait(&self, thread: &str, persona: &SubagentPersona, deadline: Duration, out: &mpsc::UnboundedSender<HubReply>) {
    let rx = {
      let mut c = self.core.lock();
      let rx = c.relays.rounds.get(thread).cloned();
      if rx.is_some() {
        *c.relays.waiters.entry(thread.to_owned()).or_default() += 1;
      }
      rx
    };
    let Some(mut rx) = rx else {
      // The round ended between the call's check and now: its outcome is on the node
      let _ = out.send(self.relay_read(thread, persona));
      return;
    };
    let _waiting = Waiting { session: self, thread };
    let mut asked = self.core.lock().relays.asked.subscribe();
    asked.borrow_and_update();
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    loop {
      if rx.borrow().is_some() {
        let _ = out.send(self.relay_read(thread, persona));
        return;
      }
      // The child asks something: the call answers with the question, so the caller can answer it
      if let Some(text) = self.relay_asking(thread, persona) {
        let _ = out.send(HubReply::Done(text));
        return;
      }
      tokio::select! {
        r = asked.changed() => if r.is_err() { return; },
        r = rx.changed() => if r.is_err() { let _ = out.send(HubReply::Error(format!("{} stopped.", who(persona)))); return; },
        _ = tick.tick() => {
          if out.send(HubReply::Progress(self.relay_activity(thread).unwrap_or_else(|| "Working".into()))).is_err() {
            return;
          }
        }
        _ = tokio::time::sleep_until(until) => {
          let latest = self.relay_activity(thread).map(|a| format!("; latest: {a}")).unwrap_or_default();
          // The caller is owed the reply anyway, unless its entry was settled meanwhile
          let still_owed = {
            let c = self.core.lock();
            c.tree.relay_latest(thread).is_some_and(|(id, ..)| c.relays.ledger.iter().any(|o| o.node == id && o.state == Owed::Running))
          };
          let next = if still_owed {
            "Acpira sends you its reply in a message when it is done: carry on with other work or end your turn, and do not poll."
          } else {
            "Call ask_agent again with this thread and an empty prompt to keep waiting for its reply."
          };
          let _ = out.send(HubReply::Done(format!("{} is still working (thread: {thread}{latest}). {next}", who(persona))));
          return;
        }
      }
    }
  }

  /// The question the thread's running round waits on, as a call answers it (marked told)
  fn relay_asking(&self, thread: &str, persona: &SubagentPersona) -> Option<String> {
    let mut c = self.core.lock();
    let (node, ..) = c.tree.relay_latest(thread)?;
    let (block, questions) = Self::relay_question(&c, &node)?;
    c.relays.told.insert(block);
    Some(question_text(&questions, thread, persona))
  }

  /// The thread's latest round as a call answers it, marking it read
  fn relay_read(&self, thread: &str, persona: &SubagentPersona) -> HubReply {
    let mut c = self.core.lock();
    let end = match c.tree.relay_latest(thread) {
      Some((id, ..)) => {
        c.relays.read(&id);
        round_result(&c, &id)
      }
      None => Err("is gone.".into()),
    };
    final_reply(end, thread, persona)
  }

  fn relay_activity(&self, thread: &str) -> Option<String> {
    let c = self.core.lock();
    c.tree.relay_latest(thread).and_then(|(id, ..)| c.tree.activity(&id))
  }
}
