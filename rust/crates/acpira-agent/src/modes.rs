//! Modes as data: a mode is a text the model is told when the session enters it, and permission rules that hold over
//! every other layer (`turn.rs` evaluates them last, so neither the approval level nor an earlier "always allow" can
//! widen them). `{plan}` in a text or a rule pattern stands for the session's plan file. Every mode offers the same
//! tools, so a mode switch leaves the request prefix (and the provider's prompt cache) intact; `exit_plan` outside
//! Plan mode answers with an error

use std::path::Path;

use crate::permission::{BASH, Decision, EDIT, ENGINE_MCP, MCP, Rule, path_target};
use crate::tools::{BASH as T_BASH, EDIT as T_EDIT, EXIT_PLAN, GLOB, GREP, HISTORY_GREP, HISTORY_LIST, JOB, LIST, QUESTION, READ, SKILL, TODO, WEBFETCH, WEBSEARCH, WRITE};

pub const AGENT: &str = "agent";
pub const PLAN: &str = "plan";

pub struct Mode {
  pub id: &'static str,
  pub name: &'static str,
  pub description: &'static str,
  /// Told to the model at the start of the first turn after the session entered this mode
  pub overlay: &'static str,
  /// (permission, pattern, decision), evaluated after every other layer
  pub rules: &'static [(&'static str, &'static str, Decision)],
}

/// The tools every mode offers, in request order (the web tools came last, 2026-10-14, then `question`, so the earlier
/// specs kept their place)
pub const TOOLS: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION];
/// The same with `skill` behind them, for a session that has skills (the turn appends its spec, so it stays last)
pub const TOOLS_WITH_SKILL: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, SKILL];
/// The same two with the history tools of chunk compaction (`chunk.rs`) behind `question`, while the settings page
/// picks it; the other sessions' sets stay exactly as they were
pub const TOOLS_CHUNKED: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, HISTORY_GREP, HISTORY_LIST];
pub const TOOLS_CHUNKED_WITH_SKILL: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, JOB, GREP, GLOB, LIST, TODO, EXIT_PLAN, WEBFETCH, WEBSEARCH, QUESTION, HISTORY_GREP, HISTORY_LIST, SKILL];

pub const MODES: &[Mode] = &[
  Mode {
    id: AGENT,
    name: "Agent",
    description: "Reads, edits and runs commands; what asks first is the approval level in settings",
    overlay: "Plan mode is off. You are in Agent mode again: you can edit files and run commands, within the permission rules.",
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
- Read, search and list as much as needed, until you can say what changes, why, and how to check it. Run only commands \
that change nothing; each one needs the user's approval.
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

  pub fn rules(&self, plan: &Path, cwd: &Path) -> Vec<Rule> {
    let plan = path_target(plan, cwd);
    self.rules.iter().map(|(p, pattern, d)| Rule::new(p, &pattern.replace("{plan}", &plan), *d)).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::permission::{Approval, evaluate};

  #[test]
  fn plan_mode_allows_only_the_plan_file_under_any_approval() {
    let (plan, cwd) = (Path::new("/h/agent/sessions/s/plan.md"), Path::new("/w"));
    let rules = get(PLAN).rules(plan, cwd);
    let full = Approval::Full.rules();
    let allowed = [Rule::new(EDIT, "*", Decision::Allow), Rule::new(BASH, "ls *", Decision::Allow)];
    let ev = |p: &str, t: &str| evaluate(&[&full, &allowed, &rules], p, t);
    assert_eq!(ev(EDIT, "src/a.rs"), Decision::Deny);
    assert_eq!(ev(EDIT, "/h/agent/sessions/s/plan.md"), Decision::Allow);
    assert_eq!(ev(BASH, "ls -la"), Decision::Ask);
    assert_eq!(ev(MCP, "blockbench/remove_cube"), Decision::Ask);
    assert_eq!(ev(MCP, "acpira/show_image"), Decision::Allow);
    assert_eq!(ev(crate::permission::MCP_READ, "blockbench/list_outline"), Decision::Allow);
    assert!(get(PLAN).overlay(plan).contains("/h/agent/sessions/s/plan.md"));
    assert!(get(AGENT).rules(plan, cwd).is_empty() && get("nope").id == AGENT);
    assert!(TOOLS.contains(&EXIT_PLAN) && TOOLS.contains(&QUESTION));
  }
}
