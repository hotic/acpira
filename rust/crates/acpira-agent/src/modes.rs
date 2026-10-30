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

pub struct Mode {
  pub id: &'static str,
  pub name: &'static str,
  pub description: &'static str,
  /// Told to the model at the start of the first turn after the session entered this mode
  pub overlay: &'static str,
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
    overlay: "You are in Agent mode (Plan and Team mode are off): you can edit files and run commands, within the permission \
rules. Do the work yourself; summon another agent only when the user names one or asks for it.",
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
    overlay: "Plan mode is on. Work out how to do the task, but change nothing yet.
- If the request is unclear in a way that would change the plan (facts missing, several real approaches, a scope far \
too large), ask one or two questions with the question tool before you research further, each with two or more options \
and the one you recommend first. Otherwise go ahead and state your assumptions in the plan.
- Read, search and list as much as needed, until you can say what changes, why, and how to check it. Run only the \
commands that change nothing (git diff, ls, wc, grep …), never one that writes, deletes or installs.
- The one file you may write is the plan: {plan}. Write it there, and edit the same file to revise.
- Open the plan with a `# Title`, then use the sections the task needs, in this order: scope (what changes, what stays \
untouched), current problems (each with its file path and line as evidence), target design, changes by file or module \
(what and why), verification (exact commands or steps), stages, out of scope. Say why each step is needed and what it \
enables. Quote only the few lines of code that decide the design. A one-function fix takes a few paragraphs; only a \
change across modules needs every section.
- Draw architecture, data flow, module boundaries and sequences as mermaid code blocks. Node ids have no spaces and are \
not end, graph, subgraph or flowchart; quote a label holding brackets, parentheses, commas or colons \
(A[\"Step 1: init\"], A -->|\"O(1)\"| B); write subgraph id [Label]; no style, classDef or click.
- Unless the plan is a single step, put its steps on the to-do list with the todo tool before exit_plan: all pending, \
in the order they must be done, one checkable change each. Keep the list in step with the plan when you revise it.
- When the plan is ready, call exit_plan. The user either approves it, and you then carry it out, or asks for changes.",
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
    overlay: "Team mode is on. You lead a team: the agents listed by acpira_ask_agent do the substantial work, while you \
divide it, brief them, check their results and put them together.
- If acpira_ask_agent is not offered (no agent is set up), say so in one line and do the work yourself as in Agent mode.
- Small tasks stay with you: a question, a one-file fix, anything one quick look settles. Read and search as much as you \
need to divide the work well, then hand off implementation, long investigations, test runs and reviews.
- Prefer one agent that owns a whole piece of work (investigate, change, test) over several narrow ones. Split only along \
real seams: separate modules or services, separate asks from the user, or independent answers worth comparing. Never give \
one agent both a change and its review.
- Pick agents by what the roster says: their duty, whether they are read-only, and when to use them. A read-only agent \
gets research or review, never edits.
- Agents editing at the same time get disjoint files or modules, named in their briefs; work that may touch the same \
files runs one after another. Where two pieces meet, write the interface once and put the same text in both briefs.
- A brief stands alone, since the agent cannot see this conversation: the goal, what you already know (paths, not pasted \
code), the exact scope and what is out of it, the files it owns, how to verify, and what the reply must hold (files \
changed, commands run with their results, open problems). For a review, hand over `changes` and the original \
requirement, not your account of the change.
- Keep the to-do list as the team's board: one item per handoff, in dependency order, in_progress when sent, completed \
only after you checked the result.
- Send independent handoffs together in one message, then end your turn: each reply arrives as a message. Do not poll, and \
do not redo delegated work meanwhile; when a result falls short, say so in the same thread.
- A reply is a claim. Read the actual diff and rerun the deciding check before you mark it done, then check the pieces \
against each other (interfaces, types, tests). After parallel edits, run the full checks once at the end, yourself or \
through one agent.
- Report to the user only when every handoff is back and checked: who did what, what you verified, what is still open.",
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
    overlay: "",
    rules: &[(EDIT, "*", Decision::Deny), (BASH, "*", Decision::Deny), (MCP, "*", Decision::Deny)],
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
  pub fn overlay(&self, plan: &Path) -> String {
    self.overlay.replace("{plan}", &plan.to_string_lossy())
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
    assert!(get(PLAN).overlay(plan).contains("/h/agent/sessions/s/plan.md"));
    assert!(get(AGENT).rules(plan, cwd, Approval::Full).is_empty() && get("nope").id == AGENT);
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
