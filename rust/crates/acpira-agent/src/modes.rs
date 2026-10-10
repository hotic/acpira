//! Modes as data: a mode is a tool set, a text the model is told when the session enters it, and permission rules that
//! hold over every other layer (`turn.rs` evaluates them last, so neither the approval level nor an earlier "always
//! allow" can widen them). `{plan}` in a text or a rule pattern stands for the session's plan file

use std::path::Path;

use crate::permission::{BASH, Decision, EDIT, Rule, path_target};
use crate::tools::{BASH as T_BASH, EDIT as T_EDIT, EXIT_PLAN, GLOB, GREP, LIST, READ, TODO, WRITE};

pub const AGENT: &str = "agent";
pub const PLAN: &str = "plan";

pub struct Mode {
  pub id: &'static str,
  pub name: &'static str,
  pub description: &'static str,
  /// The tools offered, in request order
  pub tools: &'static [&'static str],
  /// Told to the model at the start of the first turn after the session entered this mode
  pub overlay: &'static str,
  /// (permission, pattern, decision), evaluated after every other layer
  pub rules: &'static [(&'static str, &'static str, Decision)],
}

const AGENT_TOOLS: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, GREP, GLOB, LIST, TODO];
const PLAN_TOOLS: &[&str] = &[READ, WRITE, T_EDIT, T_BASH, GREP, GLOB, LIST, TODO, EXIT_PLAN];

pub const MODES: &[Mode] = &[
  Mode {
    id: AGENT,
    name: "Agent",
    description: "Reads, edits and runs commands, asking before each change",
    tools: AGENT_TOOLS,
    overlay: "Plan mode is off. You are in Agent mode again: you can edit files and run commands, within the permission rules.",
    rules: &[],
  },
  Mode {
    id: PLAN,
    name: "Plan",
    description: "Investigates and writes a plan for approval; changes nothing else",
    tools: PLAN_TOOLS,
    overlay: "Plan mode is on. Work out how to do the task, but change nothing yet.
- Read, search and list as much as needed. Run only commands that change nothing; each one needs the user's approval.
- The one file you may write is the plan: {plan}. Write it there, and edit it to revise.
- The plan names the changes file by file, in order, says why, and says how the result will be checked. Keep it concrete \
and no longer than the task needs. Leave open questions for the user in the plan, marked as such.
- When the plan is ready, call exit_plan. The user either approves it, and you then carry it out, or asks for changes.",
    rules: &[(EDIT, "*", Decision::Deny), (EDIT, "{plan}", Decision::Allow), (BASH, "*", Decision::Ask)],
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
    assert!(get(PLAN).overlay(plan).contains("/h/agent/sessions/s/plan.md"));
    assert!(get(AGENT).rules(plan, cwd).is_empty() && get("nope").id == AGENT);
    assert!(get(PLAN).tools.contains(&EXIT_PLAN) && !get(AGENT).tools.contains(&EXIT_PLAN));
  }
}
