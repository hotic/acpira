//! Modes as data: a mode is a text the model is told when the session enters it, and permission rules that hold over
//! every other layer (`turn/approval.rs` evaluates them last, so an earlier "always allow" cannot widen them). A mode's `ask`
//! rules are the exception: the approval level sits above them, so full access drops them and a command of Plan mode
//! runs without a card (its `deny` rules hold whatever the level is). `{plan}` in a text or a rule pattern stands for
//! the session's plan file. Agent and Plan offer the same
//! tools, so a switch between them leaves the request prefix (and the provider's prompt cache) intact; `exit_plan`
//! outside Plan mode answers with an error. Raw mode is the exception: it talks to the model with nothing of the
//! agent's own around the conversation (no system prompt but the settings page's, no tools, no mode notes), so
//! entering or leaving it changes the prefix once

use std::path::Path;

use crate::permission::{Approval, BASH, Decision, EDIT, ENGINE_MCP, MCP, Rule, path_target};
use crate::goal::{CREATE as CREATE_GOAL, GET as GET_GOAL, UPDATE as UPDATE_GOAL};
use crate::tools::{BASH as T_BASH, EDIT as T_EDIT, EXIT_PLAN, GLOB, GREP, HISTORY_GREP, HISTORY_LIST, JOB, LIST, QUESTION, READ, SKILL, TODO, WEBFETCH, WEBSEARCH, WRITE};

pub const AGENT: &str = "agent";
pub const PLAN: &str = "plan";
pub const TEAM: &str = "team";
pub const RAW: &str = "raw";
/// The mode a summoned agent runs a consult in (`acp::SUMMONED_META`): Plan's limits without its plan and approval,
/// since the engine picks a CLI's read-only mode for a consult and a plan there would ask the user to approve it
pub const READ_ONLY: &str = "read-only";

pub struct Mode {
  pub id: &'static str,
  pub name: &'static str,
  pub description: &'static str,
  /// The prompt told to the model at the start of the first turn after the session entered this mode (`prompt::library`;
  /// empty: nothing is told)
  pub note: &'static str,
  /// (permission, pattern, decision), evaluated after every other layer
  pub rules: &'static [(&'static str, &'static str, Decision)],
}

/// The tools every mode offers, in request order (the web tools came last, 2026-10-14, then `question`, then the goal
/// tools on 2026-10-22, so the earlier specs kept their place)
pub const TOOLS: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, GET_GOAL, CREATE_GOAL, UPDATE_GOAL];
/// The same with `skill` behind them, for a session that has skills (the turn appends its spec, so it stays last)
pub const TOOLS_WITH_SKILL: &[&str] =
  &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, GET_GOAL, CREATE_GOAL, UPDATE_GOAL, SKILL];
/// The same two with the history tools of chunk compaction (`chunk.rs`) behind the goal tools, while the settings page
/// picks it; the other sessions' sets stay exactly as they were
pub const TOOLS_CHUNKED: &[&str] =
  &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, GET_GOAL, CREATE_GOAL, UPDATE_GOAL, HISTORY_GREP, HISTORY_LIST];
pub const TOOLS_CHUNKED_WITH_SKILL: &[&str] =
  &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, GET_GOAL, CREATE_GOAL, UPDATE_GOAL, HISTORY_GREP, HISTORY_LIST, SKILL];

pub const MODES: &[Mode] = &[
  Mode {
    id: AGENT,
    name: "Agent",
    description: "Reads, edits and runs commands; what asks first is the session's approval level",
    // Also the note after Team mode, so it says again that summoning is the user's call
    note: "mode/agent",
    rules: &[],
  },
  Mode {
    id: PLAN,
    name: "Plan",
    description: "Investigates and writes a plan for approval; changes nothing else",
    // The plan's shape follows Cursor's plan mode: sections in a fixed order, evidence for every problem, mermaid for
    // structure, and the steps on the to-do list before approval, so the to-do bar shows the plan's order while the user
    // reviews it and becomes the progress of the build after approval. The plan card renders the file with the chat's
    // Markdown (mermaid included) and takes its title from the leading `# heading`
    note: "mode/plan",
    // An MCP tool its server does not mark read-only may change anything, so it asks like a command (the engine's own
    // tools excepted)
    rules: &[(EDIT, "*", Decision::Deny), (EDIT, "{plan}", Decision::Allow), (BASH, "*", Decision::Ask), (MCP, "*", Decision::Ask), (MCP, ENGINE_MCP, Decision::Allow)],
  },
  // The lead of a team: the roster of `acpira_ask_agent` does the substantial work. The rules follow Cursor's Multitask
  // mode (one coherent worker before many narrow ones, end the turn instead of waiting) and Qoder's Experts (disjoint
  // ownership for parallel edits, one shared contract, a reply is a claim checked against the diff). Same tools and
  // rules as Agent mode, so a switch keeps the prompt cache; the host's tool description defers to this mode
  Mode {
    id: TEAM,
    name: "Team",
    description: "Leads the summoned agents: hands them the work, then checks and merges what they did",
    note: "mode/team",
    rules: &[],
  },
  // The plain endpoint: the turn sends the conversation alone (`turn::setup` leaves the tools, the composed prompt and
  // the variant's sampling out; compaction only runs on /compact). No overlay: the model is told nothing on entering
  // it, and Agent mode after it needs no note either, since its system prompt arrives with it. The rules only back up
  // the empty tool set
  Mode {
    id: RAW,
    name: "Raw",
    description: "Talks to the model directly: no system prompt, tools or project instructions",
    note: "",
    rules: &[(EDIT, "*", Decision::Deny), (BASH, "*", Decision::Deny), (MCP, "*", Decision::Deny)],
  },
  // Listed only to a summoned session (or one already in it), never in the user's picker. Same tools as Agent, so
  // the engine's switch into it and back keeps the prompt cache
  Mode {
    id: READ_ONLY,
    name: "Read-only",
    description: "Investigates and answers; changes nothing",
    note: "mode/read-only",
    rules: &[(EDIT, "*", Decision::Deny), (BASH, "*", Decision::Ask), (MCP, "*", Decision::Ask), (MCP, ENGINE_MCP, Decision::Allow)],
  },
];

pub fn find(id: &str) -> Option<&'static Mode> {
  MODES.iter().find(|m| m.id == id)
}

/// The mode with this id, Agent when unknown
pub fn get(id: &str) -> &'static Mode {
  find(id).unwrap_or(&MODES[0])
}

impl Mode {
  /// What the model is told on entering this mode, in the session's prompts for its family
  pub fn note(&self, prompts: &crate::prompt::Library, family: &str, plan: &Path) -> String {
    if self.note.is_empty() {
      return String::new();
    }
    prompts.render(self.note, family, &[("plan", &plan.to_string_lossy())])
  }

  /// The mode's rules, evaluated after every other layer. The approval level widens a mode's `ask` rules only: full
  /// access runs Plan mode's commands without a card, while what the mode denies (any edit but the plan file) holds
  /// whatever the level is
  pub fn rules(&self, plan: &Path, cwd: &Path, approval: Approval) -> Vec<Rule> {
    let plan = path_target(plan, cwd);
    self
      .rules
      .iter()
      .filter(|(_, _, d)| !(approval == Approval::Full && *d == Decision::Ask))
      .flat_map(|(p, pattern, d)| match *pattern {
        ENGINE_MCP => crate::permission::engine_rules(*d).collect(),
        pattern => vec![Rule::new(p, &pattern.replace("{plan}", &plan), *d)],
      })
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::permission::{Approval, evaluate};

  #[test]
  fn plan_mode_writes_only_the_plan_and_full_access_runs_its_commands() {
    let (plan, cwd) = (Path::new("/h/agent/sessions/s/plan.md"), Path::new("/w"));
    let defaults = crate::permission::defaults(Path::new("/h/agent/sessions/s/outputs"));
    let ev = |approval: Approval, p: &str, t: &str| evaluate(&[&defaults, &approval.rules(), &get(PLAN).rules(plan, cwd, approval)], p, t);
    for approval in [Approval::Ask, Approval::AutoEdit, Approval::Full] {
      assert_eq!(ev(approval, EDIT, "src/a.rs"), Decision::Deny, "{approval:?}");
      assert_eq!(ev(approval, EDIT, "/h/agent/sessions/s/plan.md"), Decision::Allow, "{approval:?}");
      assert_eq!(ev(approval, MCP, "acpira/show_image"), Decision::Allow, "{approval:?}");
      assert_eq!(ev(approval, MCP, "acpira/ask_agent"), Decision::Allow, "{approval:?}");
      assert_eq!(ev(approval, crate::permission::MCP_READ, "blockbench/list_outline"), Decision::Allow, "{approval:?}");
    }
    // The mode's ask rules hold under the lower levels and drop out under full access
    assert_eq!(ev(Approval::Ask, BASH, "ls -la"), Decision::Ask);
    assert_eq!(ev(Approval::AutoEdit, BASH, "ls -la"), Decision::Ask);
    assert_eq!(ev(Approval::Full, BASH, "ls -la"), Decision::Allow);
    assert_eq!(ev(Approval::Full, MCP, "blockbench/remove_cube"), Decision::Allow);
    // A plugin's tool on the engine's server is no exception to the mode
    assert_eq!(ev(Approval::Ask, MCP, "acpira/currency_convert"), Decision::Ask);
    assert_eq!(ev(Approval::AutoEdit, MCP, "acpira/currency_convert"), Decision::Ask);
    // A session "always allow" cannot widen them either
    let allowed = [Rule::new(BASH, "ls *", Decision::Allow)];
    assert_eq!(evaluate(&[&defaults, &allowed, &get(PLAN).rules(plan, cwd, Approval::Ask)], BASH, "ls -la"), Decision::Ask);
    let prompts = crate::prompt::Library::builtin();
    assert!(get(PLAN).note(&prompts, crate::prompt::BASE, plan).contains("/h/agent/sessions/s/plan.md"));
    assert!(get(RAW).note(&prompts, crate::prompt::BASE, plan).is_empty());
    assert!(get(AGENT).rules(plan, cwd, Approval::Full).is_empty() && get("nope").id == AGENT);
    // Read-only holds Plan's limits with no file to write
    let ro = |p: &str, t: &str| evaluate(&[&defaults, &Approval::Ask.rules(), &get(READ_ONLY).rules(plan, cwd, Approval::Ask)], p, t);
    assert_eq!(ro(EDIT, "/h/agent/sessions/s/plan.md"), Decision::Deny);
    assert_eq!(ro(BASH, "ls -la"), Decision::Ask);
    assert!(get(READ_ONLY).note(&prompts, crate::prompt::BASE, plan).starts_with("Read-only mode is on"));
    // Team mode only changes what the model is told, never what it may do
    assert!(get(TEAM).rules(plan, cwd, Approval::Ask).is_empty());
    assert!(TOOLS.contains(&EXIT_PLAN) && TOOLS.contains(&QUESTION));
    // Every list carries the goal tools at the same place, behind question
    for list in [TOOLS, TOOLS_WITH_SKILL, TOOLS_CHUNKED, TOOLS_CHUNKED_WITH_SKILL] {
      let at = list.iter().position(|t| *t == QUESTION).unwrap();
      assert_eq!(list[at + 1..at + 4], [GET_GOAL, CREATE_GOAL, UPDATE_GOAL]);
    }
  }
}
