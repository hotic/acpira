//! Which persona a call or a thread names: the roster as the caller sees it, the node's persona, an idle thread to
//! continue, and the built-in agent's sidekick

use anyhow::{Result, anyhow};

use acpira_shared::subagents::{SIDEKICK_ID, SubagentDuty, SubagentHarness, SubagentPersona, SubagentState};

use crate::acp::session::Core;
use crate::relay::roster::Roster;

/// The thread's display: persona name and CLI
pub(super) fn who(persona: &SubagentPersona) -> String {
  format!("{} ({})", persona.name, persona.agent)
}

/// Whether a call (its `agent` and `thread` arguments) is the one that opened a round with this harness / persona name.
/// A call without `thread` usually opens a thread's first round, but a scout's or an executor's continues its idle one
pub(super) fn call_fits(agent: &str, thread: Option<&str>, h: &SubagentHarness, role: Option<&str>) -> bool {
  let agent = agent.trim();
  match thread.map(str::trim).filter(|t| !t.is_empty()) {
    Some(th) => h.thread == th && h.round > 1,
    None => {
      h.persona.as_deref() == Some(agent) || role.is_some_and(|r| r.eq_ignore_ascii_case(agent)) || h.group.as_deref() == Some(agent)
    }
  }
}

/// What a finished or running round is shown as, from its node: no roster lookup
pub(super) fn node_persona(h: &SubagentHarness, name: Option<&str>) -> SubagentPersona {
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
    duty: SubagentDuty::General,
    changes: false,
  }
}

/// The persona's latest conversation this caller started in the session, when it is idle and its process still up: a
/// scout or an executor called without `thread` continues it. Otherwise the call starts fresh rather than fall back to
/// an older conversation (one the caller may have left with `new_thread`): a busy one is busy, reopening one whose
/// process is gone might fail, and one in a CLI the persona no longer runs in is another agent
pub(super) fn idle_thread(c: &Core, persona: &SubagentPersona, caller: Option<&str>) -> Option<String> {
  let latest = c
    .relays
    .ledger
    .iter()
    .rev()
    .filter(|o| o.owner.as_deref() == caller)
    .filter_map(|o| c.tree.relay_harness(&o.node).map(|(h, _)| h))
    .find(|h| h.persona.as_deref() == Some(persona.id.as_str()))?;
  let idle = c.tree.relay_latest(&latest.thread).is_some_and(|(.., state)| state != SubagentState::Running);
  let alive = c.relays.procs.get(&latest.thread).is_some_and(|rp| rp.proc.alive() && !rp.peer.is_empty());
  (latest.agent == persona.agent && idle && alive).then_some(latest.thread)
}

/// The persona a thread continues with: its current settings, in the CLI the thread was started in. A sidekick's thread
/// goes on with the sidekick picked now (`sidekick`), on whatever model that is
pub(super) fn thread_persona(roster: &Roster, sidekick: Option<SubagentPersona>, h: &SubagentHarness, name: Option<&str>) -> Result<SubagentPersona> {
  let label = name.unwrap_or(h.persona.as_deref().unwrap_or(&h.agent));
  let id = h.persona.as_deref().unwrap_or("");
  let found = if id == SIDEKICK_ID { sidekick } else { roster.find(id) };
  let Some(p) = found else {
    if id == SIDEKICK_ID {
      return Err(anyhow!("This session has no sidekick now, so thread {} cannot continue.", h.thread));
    }
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

/// The built-in agent's sidekick select (`acpira-agent/src/acp.rs`): its id, its custom category and the value for none
pub(super) const SIDEKICK_CONFIG: &str = "sidekick";
pub(super) const SIDEKICK_CATEGORY: &str = "_sidekick";
pub(super) const NO_SIDEKICK: &str = "none";
/// Its effort select, there while the sidekick's model has levels
pub(super) const SIDEKICK_EFFORT_CONFIG: &str = "sidekick_effort";
pub(super) const SIDEKICK_EFFORT_CATEGORY: &str = "_sidekick_effort";

/// The sidekick's line in ask_agent: Fusion's split of the work, where the lead plans, decides and checks while the
/// sidekick does the well-specified parts. The Fusion bench (docs/dev/evals.md, 2026-10-21) showed the lead waiting on
/// steps it would have finished sooner itself, while a second look at its finished change caught real defects
pub(super) const SIDEKICK_WHEN: &str = "Your sidekick, the second model picked for this session. It works at its own pace, often \
slower than you, so hand it work that runs while you do something else: edits you can describe precisely (across files, \
repetitive, boilerplate), scouting that does not decide the plan (where something lives, what a log says), or a second look \
at your finished change against the requirements, which catches what you missed. A step you would finish sooner yourself, \
such as a handful of tests, is faster done than handed over and waited for. Brief it with the goal, the constraints and \
what done means; keep the plan and the decisions yourself, check its diff before building on it, and when it falls short \
say so in the same thread.";
