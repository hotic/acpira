//! What a built-in tool's call carries beside its own arguments (the to-do list, an edit's `then_run` command), each
//! shown and decided as a row of its own

use std::sync::Arc;

use serde_json::{Value, json};

use super::approval::{Answer, Rules, ask};
use super::calls::{Prepared, ctx_for, prepare_call, report, run_action};
use crate::acp::{Server, Session};
use crate::llm::ToolCall;
use crate::permission::{self, Decision, Rule};
use crate::tools::{self, Action, Output};

/// What a call of a built-in tool carries beside its own arguments, so a model that makes one call per reply needs no
/// request of its own for it (`tools::todo::rider`, `tools::edit::then_run`). Each rider shows as a row of its own,
/// under the rules its own tool would be; the model gets one result for the call
#[derive(Default)]
pub(super) struct Riders {
  /// `todos`: the to-do list, set before the call runs
  todos: Option<Result<Vec<tools::todo::Todo>, String>>,
  /// `then_run` of an edit or a write: the command to run once the change is made, or why it will not run
  pub(super) then_run: Option<Result<Action, String>>,
}

impl Riders {
  /// The riders of a call of this built-in tool (an MCP tool's arguments are its server's; the todo tool's list is its
  /// own argument). A then_run command is prepared where the session's commands run (`prepare_call`)
  pub(super) async fn of(server: &Arc<Server>, session: &Arc<Session>, name: &str, args: &Value) -> Riders {
    let todos = if name == tools::TODO { None } else { tools::todo::rider(args) };
    let then_run = match tools::edit::then_run(args) {
      Some(command) => Some(match name {
        tools::EDIT | tools::WRITE => prepare_call(server, session, tools::BASH, &json!({ "command": command }), false).await,
        _ => Err(format!("then_run is taken only by edit and write, so `{command}` was not run.")),
      }),
      None => None,
    };
    Riders { todos, then_run }
  }

  /// The note on a then_run that cannot run whatever the call does
  pub(super) fn refused(&self) -> Option<String> {
    match &self.then_run {
      Some(Err(e)) => Some(e.clone()),
      _ => None,
    }
  }

  /// The note on a then_run left out, and why: the change was never made
  pub(super) fn skipped(&self, why: &str) -> Option<String> {
    match &self.then_run {
      Some(Ok(Action::Bash { command, .. })) => Some(format!("then_run `{command}` was not run: {why}.")),
      _ => self.refused(),
    }
  }
}

/// Whether a call's to-do list gets a row: it parses and the rules allow setting it
pub(super) fn todo_row_allowed(session: &Arc<Session>, rules: &Rules, riders: &Riders) -> bool {
  match &riders.todos {
    Some(Ok(todos)) => rules.decide(session, &Action::Todo { todos: todos.clone() }) == Decision::Allow,
    _ => false,
  }
}

/// The to-do list a call carries (`todos`), set before the call runs: a `todo` row of its own, the way the todo tool's
/// call shows, so the host's to-do bar takes it unchanged; on `row`, a row the stream opened, when given. The note in
/// front of the call's result
pub(super) fn ride_todo(server: &Arc<Server>, session: &Arc<Session>, rules: &Rules, riders: &Riders, row: Option<String>) -> Option<String> {
  let todos = match riders.todos.as_ref()? {
    Ok(todos) => todos.clone(),
    Err(e) => return Some(format!("The to-do list was not updated: {e}.")),
  };
  if !todo_row_allowed(session, rules, riders) {
    return Some("The to-do list was not updated: the permission rules do not allow it.".into());
  }
  let raw = tools::todo::raw(&todos);
  let acp = match row {
    // The streamed row was the call's: every field it showed is set again
    Some(row) => {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": row, "title": tools::TODO, "kind": "other", "status": "pending", "rawInput": raw }));
      row
    }
    None => {
      let acp = session.next_tool_id();
      server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": tools::TODO, "kind": "other", "status": "pending", "rawInput": raw }));
      acp
    }
  };
  report(server, session, &acp, tools::todo::run(&todos));
  Some(tools::todo::note(&todos))
}

/// The command an edit or a write carries (`then_run`), once the change is made: on a command row of its own, under
/// the rules a `bash` call would be, with its own card when they ask. The section for the call's result, and whether
/// the user rejected the command, which ends the turn after the round as a rejected call does. `command` is the
/// command line of `action`, a `bash` one
pub(super) async fn ride_command(server: &Arc<Server>, session: &Arc<Session>, rules: &Rules, action: &Action, command: &str) -> (String, bool) {
  // The plugins' `shell` hook sees the command as it sees a `bash` call's
  let (rewritten, refused) = match crate::plugin_hooks::shell_rider(server, session, action).await {
    Ok(a) => (a, None),
    Err(reason) => (action.clone(), Some(reason)),
  };
  let action = &rewritten;
  let command = match action {
    Action::Bash { command, .. } => command.as_str(),
    _ => command,
  };
  let acp = session.next_tool_id();
  let raw_input = json!({ "command": command });
  server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": command, "kind": "execute", "status": "pending", "rawInput": raw_input }));
  let header = format!("$ {command}");
  if let Some(reason) = refused {
    report(server, session, &acp, Output::error(reason.clone()));
    return (format!("{header}\nNot run: {reason}"), false);
  }
  match rules.decide(session, action) {
    Decision::Allow => {}
    Decision::Deny => {
      let (key, target) = rules.denied(session, action);
      report(server, session, &acp, Output::error(format!("Not allowed: the permission rules deny {key} on {target}.")));
      return (format!("{header}\nNot run: the permission rules deny {key} on {target}. Do not retry it."), false);
    }
    Decision::Ask => {
      let call = ToolCall { id: acp.clone(), name: tools::BASH.into(), arguments: raw_input.to_string() };
      let p = Prepared { call, acp: acp.clone(), action: Ok(action.clone()), note: None, meta: None, riders: Riders::default(), todo: None, preset: None, ran_with: None };
      match ask(server, session, rules, &p, action).await {
        Answer::Once => {}
        Answer::Always(pattern) => session.state.lock().allowed.push(Rule::new(permission::BASH, &pattern, Decision::Allow)),
        Answer::Reject => {
          report(server, session, &acp, Output::error("The user rejected this action."));
          return (format!("{header}\nNot run: the user rejected it. Do not retry it; wait for the user's direction."), true);
        }
      }
    }
  }
  server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "status": "in_progress" }));
  let out = run_action(server, session, action.clone(), ctx_for(server, session, &acp)).await;
  let section = format!("{header}\n{}", out.model);
  report(server, session, &acp, out);
  (section, false)
}
