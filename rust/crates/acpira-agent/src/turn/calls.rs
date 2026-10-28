//! One round of tool calls: each call prepared, approved, run (here or where the session's commands run) and
//! reported as its row and its result, with Plan mode's exit and the question tool handled by the turn itself

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::MAX_CORRECTIONS;
use super::approval::{Answer, ask};
use super::riders::{Riders, ride_command, ride_todo, todo_row_allowed};
use super::setup::Setup;
use crate::acp::{Server, Session};
use crate::goal;
use crate::llm::{Item, ToolCall};
use crate::modes;
use crate::permission::{Decision, Rule};
use crate::tools::names::{self, Resolved};
use crate::tools::{self, Action, Ctx, Output};

/// A call of a goal tool, which shows no row: the goal's own rows stand for it (`goal.rs`)
pub(super) fn hidden(name: &str, tools: &[&str]) -> bool {
  goal::is_tool(shown_name(name, tools))
}

/// The tool a model-given name stands for, for display before the call is prepared
fn shown_name<'a>(name: &'a str, tools: &[&'a str]) -> &'a str {
  match names::resolve(name, tools) {
    Resolved::Exact(n) | Resolved::Corrected(n) => n,
    Resolved::Unknown(_) => name,
  }
}

/// The engine's `acpira/mcp` note for a call of an MCP tool (by the name the model wrote, corrected like any other)
pub(super) fn mcp_meta(name: &str, s: &Setup) -> Option<Value> {
  let names = s.names();
  let resolved = shown_name(name, &names);
  s.mcp.iter().find(|t| t.name == resolved).map(crate::mcp::Tool::meta)
}

/// The title a tool call starts with: the tool's name, except Plan mode's exit, which shows as the plan step the
/// webview already knows (`Exit plan mode`, localized there) from the first update on
pub(super) fn call_title<'a>(name: &'a str, tools: &[&'a str]) -> &'a str {
  match shown_name(name, tools) {
    tools::EXIT_PLAN => EXIT_PLAN_TITLE,
    // Read as an ask row before the arguments are known
    tools::QUESTION => "Ask questions",
    n => n,
  }
}

const EXIT_PLAN_TITLE: &str = "Exit plan mode";

/// ACP ToolKind for a tool name, before its arguments are known
pub(super) fn kind_of(name: &str, tools: &[&str]) -> &'static str {
  match shown_name(name, tools) {
    tools::READ | tools::LIST | tools::SKILL => "read",
    tools::WRITE | tools::EDIT => "edit",
    tools::BASH | tools::JOB => "execute",
    tools::GREP | tools::GLOB => "search",
    tools::EXIT_PLAN => "switch_mode",
    _ => "other",
  }
}

/// A tool call after parsing and preparation
pub(super) struct Prepared {
  pub(super) call: ToolCall,
  pub(super) acp: String,
  pub(super) action: Result<Action, String>,
  /// Put before the result: the misnamed call was taken as another tool
  pub(super) note: Option<String>,
  /// The `acpira/mcp` note of an MCP call, on its permission card too
  pub(super) meta: Option<Value>,
  pub(super) riders: Riders,
  /// The note the call's result starts with when it carried the to-do list (`todos`), set while preparing
  pub(super) todo: Option<String>,
  /// A plugin answered the call before it ran (`plugin_hooks::Before::Answer`): this is its result, nothing runs
  pub(super) preset: Option<Output>,
  /// The arguments the call runs with, after the plugins' `tool.before` and `shell` hooks; None when they did not parse
  pub(super) ran_with: Option<Value>,
}

/// How a tool round leaves the turn
pub(super) enum Flow {
  Continue,
  /// The user rejected an action or did not approve the plan: the turn ends after the round
  Stop,
  /// A plan was approved: the session is in Agent mode and the turn goes on under it
  Switched,
}

/// Run one round of tool calls
pub(super) async fn run_tools(
  server: &Arc<Server>,
  session: &Arc<Session>,
  setup: &Setup,
  calls: &[ToolCall],
  ids: &mut HashMap<String, String>,
  corrections: &mut u32,
) -> Flow {
  let (rules, tool_names) = (&setup.rules, setup.names());
  let tool_names = &tool_names[..];
  let cwd = session.cwd.clone();
  let mut prepared = vec![];
  for (k, call) in calls.iter().enumerate() {
    let meta = mcp_meta(&call.name, setup);
    let announce = || {
      let a = session.next_tool_id();
      let mut first = json!({ "sessionUpdate": "tool_call", "toolCallId": a, "title": call_title(&call.name, tool_names), "kind": kind_of(&call.name, tool_names), "status": "pending" });
      if let Some(m) = &meta {
        first["_meta"] = m.clone();
      }
      server.update(&session.id, first);
      a
    };
    let streamed = ids.remove(&call.id);
    let args: Result<Value, String> = serde_json::from_str::<Value>(&call.arguments)
      .map_err(|e| format!("The arguments are not valid JSON ({e}). Received: {}", crate::budget::cut(&call.arguments, 2000)))
      .and_then(|v| if v.is_object() { Ok(v) } else { Err(format!("The arguments must be a JSON object. Received: {}", crate::budget::cut(&call.arguments, 2000))) });
    let (name, note) = match names::resolve(&call.name, tool_names) {
      Resolved::Exact(n) => (Ok(n), None),
      Resolved::Corrected(n) if *corrections < MAX_CORRECTIONS => {
        *corrections += 1;
        (Ok(n), Some(format!("(\"{}\" was taken as the {n} tool; call tools by their exact names.)", call.name)))
      }
      Resolved::Corrected(n) => (Err(format!("Unknown tool \"{}\" (did you mean \"{n}\"?). Tool names must match exactly: {}.", call.name, tool_names.join(", "))), None),
      Resolved::Unknown(message) => (Err(message), None),
    };
    // A goal tool's call has no row (`hidden`), unless the stream opened one for a name it took for another tool
    let rowless = streamed.is_none() && matches!(&name, Ok(n) if goal::is_tool(n));
    let riders = match (name.as_ref().ok().copied(), &args) {
      (Some(n), Ok(a)) if !rowless && !setup.mcp.iter().any(|t| t.name == n) => Riders::of(server, session, n, a).await,
      _ => Riders::default(),
    };
    // The list a call carries is set now, on a row above the call's own, so the transcript reads in the order things
    // happen. A row the stream opened for the reply's last call becomes the list's row and the call gets a new one
    // below it; any other streamed row stays the call's (a new row would land below the calls after it), so the list
    // follows it there
    let (acp, todo) = match streamed {
      _ if rowless => (String::new(), None),
      Some(row) if k + 1 == calls.len() && todo_row_allowed(session, rules, &riders) => {
        let todo = ride_todo(server, session, rules, &riders, Some(row));
        (announce(), todo)
      }
      Some(row) => (row, ride_todo(server, session, rules, &riders, None)),
      None => {
        let todo = ride_todo(server, session, rules, &riders, None);
        (announce(), todo)
      }
    };
    // The plugins may change the arguments, refuse the call or answer it themselves
    let mut preset = None;
    let args = match (&name, args) {
      (Ok(n), Ok(a)) if !rowless => match crate::plugin_hooks::before_call(server, session, n, a).await {
        crate::plugin_hooks::Before::Run(a) => Ok(a),
        crate::plugin_hooks::Before::Block(reason) => Err(reason),
        crate::plugin_hooks::Before::Answer(a, out) => {
          preset = Some(out);
          Ok(a)
        }
      },
      (_, a) => a,
    };
    // A bad argument goes back with what was received, so the model can see its own mistake
    let action = match (name, args.clone()) {
      (Err(e), _) | (Ok(_), Err(e)) => Err(e),
      // An MCP tool's arguments are the server's to check against its schema
      (Ok(n), Ok(a)) => match setup.mcp.iter().find(|t| t.name == n) {
        Some(t) => Ok(tools::mcp::prepare(t, &a, setup.images)),
        None => prepare_call(server, session, n, &a, setup.images).await.map_err(|e| format!("{e}. Received arguments: {}", crate::budget::cut(&a.to_string(), 2000))),
      },
    };
    let ran_with = args.as_ref().ok().cloned();
    let raw_input = args.unwrap_or_else(|_| Value::String(call.arguments.clone()));
    match &action {
      _ if rowless => {}
      Ok(a) => {
        let p = a.describe(&cwd);
        let mut update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "title": p.title, "kind": p.kind, "rawInput": raw_input,
          "locations": p.locations.iter().map(tools::Location::json).collect::<Vec<_>>(), "content": p.content });
        if let Some(m) = &meta {
          update["_meta"] = m.clone();
        }
        server.update(&session.id, update);
      }
      Err(_) => {
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "rawInput": raw_input }));
      }
    }
    prepared.push(Prepared { call: call.clone(), acp, action, note, meta, riders, todo, preset, ran_with });
  }

  let mut rejected = false;
  let mut switched = false;
  let mut i = 0;
  while i < prepared.len() {
    // Consecutive read-only calls the rules allow run together
    let start = i;
    while i < prepared.len() && !rejected && matches!(&prepared[i].action, Ok(a) if a.read_only() && rules.decide(session, a) == Decision::Allow) {
      i += 1;
    }
    if i > start {
      let mut handles = vec![];
      for p in &prepared[start..i] {
        // The loop above took only calls whose action prepared
        let Ok(action) = p.action.clone() else { continue };
        // A plugin answered it already
        if let Some(out) = &p.preset {
          let out = Output { model: out.model.clone(), is_error: out.is_error, content: out.content.clone(), raw_output: out.raw_output.clone() };
          handles.push(tokio::spawn(async move { out }));
          continue;
        }
        let ctx = ctx_for(server, session, &p.acp);
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
        // An MCP call waits on its server; the rest read files or the network on a blocking thread
        handles.push(match action {
          mcp @ Action::Mcp { .. } => tokio::spawn(mcp.run(ctx)),
          there if runs_remotely(server, session, &there) => {
            let (srv, s) = (server.clone(), session.clone());
            tokio::spawn(async move { run_action(&srv, &s, there, ctx).await })
          }
          other => tokio::task::spawn_blocking(move || other.run_sync(ctx)),
        });
      }
      for (p, h) in prepared[start..i].iter().zip(handles) {
        let out = h.await.unwrap_or_else(|e| Output::error(format!("The tool crashed: {e}")));
        finish(server, session, p, out, p.todo.clone(), p.riders.refused()).await;
      }
      continue;
    }
    let p = &prepared[i];
    i += 1;
    if rejected {
      finish(server, session, p, Output::error("Skipped: the user rejected an earlier action in this round."), p.todo.clone(), None).await;
      continue;
    }
    // The list was set while preparing: it moves on whatever comes of the call itself
    let todo = p.todo.clone();
    // A plugin answered it already: nothing runs, and nothing needs the user's permission
    if let Some(out) = &p.preset {
      let out = Output { model: out.model.clone(), is_error: out.is_error, content: out.content.clone(), raw_output: out.raw_output.clone() };
      finish(server, session, p, out, todo, None).await;
      continue;
    }
    let action = match &p.action {
      Ok(a) => a.clone(),
      Err(e) => {
        finish(server, session, p, Output::error(e.clone()), todo, p.riders.skipped("the change failed")).await;
        continue;
      }
    };
    // A goal tool works on the session itself: no permission to ask, and the result is the model's alone
    if let Action::Goal(call) = &action {
      finish(server, session, p, goal::run_tool(server, session, call), todo, None).await;
      continue;
    }
    // The plan approval card is this call's permission
    if let Action::ExitPlan { plan } = &action {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
      let (out, approved) = exit_plan(server, session, p, plan.as_deref()).await;
      finish(server, session, p, out, todo, p.riders.refused()).await;
      match approved {
        Some(true) => switched = true,
        Some(false) => rejected = true,
        None => {}
      }
      continue;
    }
    // The question card needs no permission; closing it ends the turn
    if let Action::Question { questions } = &action {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
      let (out, dismissed) = question(server, session, p, questions).await;
      finish(server, session, p, out, todo, p.riders.refused()).await;
      rejected |= dismissed;
      continue;
    }
    match rules.decide(session, &action) {
      Decision::Allow => {}
      Decision::Deny => {
        let (key, target) = rules.denied(session, &action);
        finish(server, session, p, Output::error(format!("Not allowed: the permission rules deny {key} on {target}. Do not retry it.")), todo, p.riders.skipped("the change was not allowed")).await;
        continue;
      }
      Decision::Ask => match ask(server, session, rules, p, &action).await {
        Answer::Once => {}
        Answer::Always(pattern) => {
          let (key, _) = action.permission(&cwd);
          session.state.lock().allowed.push(Rule::new(key, &pattern, Decision::Allow));
        }
        Answer::Reject => {
          rejected = true;
          finish(server, session, p, Output::error("The user rejected this action. Do not retry it; wait for the user's direction."), todo, None).await;
          continue;
        }
      },
    }
    server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
    // A write of what the file already holds is no change to check
    let unchanged = matches!(&action, Action::Write { files } if files.iter().all(tools::edit::Change::unchanged));
    let out = run_action(server, session, action, ctx_for(server, session, &p.acp)).await;
    let (command, line) = match &p.riders.then_run {
      Some(Ok(command @ Action::Bash { command: line, .. })) => (command, line),
      _ => {
        finish(server, session, p, out, todo, p.riders.refused()).await;
        continue;
      }
    };
    if out.is_error || unchanged {
      let why = if out.is_error { "the change failed" } else { "the file already held this content" };
      finish(server, session, p, out, todo, p.riders.skipped(why)).await;
      continue;
    }
    // The change is made: its row completes now, the command gets a row of its own, and the call's one result waits
    // for both (a turn cancelled meanwhile still tells the model the change was made)
    let images = images_of(p, &out);
    let mut out = out;
    let arguments = p.ran_with.clone().unwrap_or_else(|| serde_json::from_str(&p.call.arguments).unwrap_or_else(|_| Value::String(p.call.arguments.clone())));
    // The change's own `tool.after`, as on a call without a command after it
    if let Some((model, is_error)) = crate::plugin_hooks::after_call(server, session, &p.call.name, &arguments, &out.model, out.is_error).await {
      out.model = model;
      out.is_error = is_error;
    }
    let (model, is_error) = report(server, session, &p.acp, out);
    let cut = format!("then_run `{line}` was cancelled by the user before it finished.");
    session.state.lock().landed.push((p.call.id.clone(), joined(todo.as_deref(), &model, Some(&cut))));
    let (section, stop) = ride_command(server, session, rules, command, line).await;
    session.state.lock().landed.retain(|(id, _)| *id != p.call.id);
    rejected |= stop;
    record(session, p, joined(todo.as_deref(), &model, Some(&section)), is_error, images);
  }
  if rejected {
    Flow::Stop
  } else if switched {
    Flow::Switched
  } else {
    Flow::Continue
  }
}

/// Plan mode's exit: the plan file goes to the user on the host's plan approval card (`acpira/planApproval`, and the
/// `switch_mode` + `rawInput.plan` shape with `planFilePath`, which `transcript/plans.rs` takes as the whole document).
/// Some(true) when approved, Some(false) when not, None when there was nothing to ask about
async fn exit_plan(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, plan: Option<&str>) -> (Output, Option<bool>) {
  if session.state.lock().mode != modes::PLAN {
    return (Output::error("The session is not in Plan mode, so there is no plan to submit."), None);
  }
  let path = server.plan_file(&session.id);
  if let Some(text) = plan {
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, text));
    if let Err(e) = written {
      return (Output::error(format!("Could not write the plan to {}: {e}", path.display())), None);
    }
  }
  let text = std::fs::read_to_string(&path).unwrap_or_default();
  if text.trim().is_empty() {
    return (Output::error(format!("There is no plan yet. Write it to {}, then call exit_plan again.", path.display())), None);
  }
  let raw_input = json!({ "plan": text, "planFilePath": path });
  let meta = json!({ "acpira/planApproval": true });
  let title = EXIT_PLAN_TITLE;
  server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "title": title, "kind": "switch_mode", "rawInput": raw_input, "_meta": meta }));
  let request = json!({
    "sessionId": session.id,
    "toolCall": { "toolCallId": p.acp, "title": title, "kind": "switch_mode", "status": "pending", "rawInput": raw_input, "_meta": meta },
    // The webview localizes these two labels on a plan card
    "options": [
      { "optionId": "approved", "name": "Build", "kind": "allow_once" },
      { "optionId": "rejected", "name": "Revise", "kind": "reject_once" },
    ],
  });
  let answer = server.conn().request("session/request_permission", request).await;
  let approved = answer.is_ok_and(|r| {
    r.pointer("/outcome/outcome").and_then(Value::as_str) == Some("selected") && r.pointer("/outcome/optionId").and_then(Value::as_str) == Some("approved")
  });
  if !approved {
    let out = Output::error("The user did not approve the plan yet. Stop here and wait for their feedback, then revise the plan file and call exit_plan again.");
    return (out, Some(false));
  }
  {
    let mut st = session.state.lock();
    st.mode = modes::AGENT.to_owned();
    st.announced = modes::AGENT.to_owned();
    session.save_state(&st);
  }
  server.update(&session.id, json!({ "sessionUpdate": "current_mode_update", "currentModeId": modes::AGENT }));
  let model = format!("The user approved the plan. {} Carry out the plan now, item by item in the to-do list's order: mark an item in_progress when you start it and \
completed once it is checked. If the plan turns out wrong, correct the plan file before going on.", modes::get(modes::AGENT).overlay(&path));
  (Output { model, is_error: false, content: vec![], raw_output: None }, Some(true))
}

/// The `question` tool: an `elicitation/create` form on the host's question card. The bool is true when the user
/// closed the card (or the client refused the form), which ends the turn like a rejected action
async fn question(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, questions: &[tools::question::Question]) -> (Output, bool) {
  if !server.forms() {
    return (Output::error("This client cannot show questions. Ask in plain text instead and end your turn."), false);
  }
  let request = tools::question::form(&session.id, &p.acp, questions);
  let out = match server.conn().request("elicitation/create", request).await {
    Ok(r) => tools::question::reply(questions, &r),
    Err(e) => return (Output::error(format!("The questions could not be shown ({}). Ask in plain text instead and end your turn.", e.message)), false),
  };
  match out {
    // A skip is an answer too, so the row does not read as failed
    tools::question::Reply::Answered(text) => (answered(text), false),
    tools::question::Reply::Skipped => (answered("The user skipped the questions without answering. Go on with your own judgement and say what you assumed.".into()), false),
    tools::question::Reply::Dismissed => (Output::error("The user closed the questions. Stop here and wait for their direction."), true),
  }
}

/// The answers are the model's copy only: the question card below the row already shows them
pub(super) fn answered(text: String) -> Output {
  Output { content: vec![], model: text, is_error: false, raw_output: None }
}

/// Validate a call of a built-in tool: on the session's SSH host when the call reads or changes the project there or runs
/// a command (`remote.rs`), else here
pub(super) async fn prepare_call(server: &Arc<Server>, session: &Arc<Session>, name: &str, args: &Value, images: bool) -> Result<Action, String> {
  // Skills come from where the session's prompt files are, the host's included
  if name == tools::SKILL {
    return tools::skill::prepare(args, &server.places_of(&session.cwd, session.remote.as_deref(), session.layers.clone()));
  }
  if let Some(r) = &session.remote
    && crate::remote::Remote::prepares_there(name, args, &session.cwd, server.config.home())
  {
    return r.prepare(server.conn(), &session.id, &session.cwd, name, args, images).await;
  }
  tools::prepare(name, args, &session.cwd, images)
}

/// Whether a prepared call runs on the session's SSH host
fn runs_remotely(server: &Arc<Server>, session: &Arc<Session>, action: &Action) -> bool {
  session.remote.is_some() && crate::remote::Remote::runs_there(action, server.config.home())
}

/// Run a prepared call, on the session's SSH host when it belongs there
pub(super) async fn run_action(server: &Arc<Server>, session: &Arc<Session>, action: Action, ctx: Ctx) -> Output {
  if let Some(r) = session.remote.as_ref().filter(|_| runs_remotely(server, session, &action)) {
    return r.run(server.conn(), &session.id, action, ctx).await;
  }
  action.run(ctx).await
}

pub(super) fn ctx_for(server: &Arc<Server>, session: &Arc<Session>, acp: &str) -> Ctx {
  let (srv, sid, id) = (server.clone(), session.id.clone(), acp.to_owned());
  Ctx {
    cwd: session.cwd.clone(),
    outputs: server.session_dir(&session.id).join("outputs"),
    call_id: acp.to_owned(),
    jobs: session.jobs.clone(),
    search: server.config.get().web_search(),
    mcp: session.mcp.clone(),
    history: server.session_file(&session.id),
    progress: Arc::new(move |mut u: Value| {
      u["sessionUpdate"] = "tool_call_update".into();
      u["toolCallId"] = id.clone().into();
      srv.update(&sid, u);
    }),
  }
}

/// A call's result with the riders' notes: the to-do note in front (a command's exit code stays its last line, where
/// the chunk index reads it), the command after the change behind it
fn joined(front: Option<&str>, model: &str, back: Option<&str>) -> String {
  let mut text = String::new();
  if let Some(f) = front {
    text.push_str(f);
    text.push_str("\n\n");
  }
  text.push_str(model);
  if let Some(b) = back {
    text.push_str("\n\n");
    text.push_str(b);
  }
  text
}

/// An MCP result's images, and the pictures `read` makes of image files, go to the model when it takes them (those it
/// may not get are marked on the card's content); the card shows them either way
fn images_of(p: &Prepared, out: &Output) -> Vec<crate::llm::Image> {
  match &p.action {
    Ok(Action::Mcp { images: true, .. } | Action::Read { images: true, .. }) => tools::mcp::model_images(&out.content),
    _ => vec![],
  }
}

/// Report a finished call on its row and add its result, with the riders' notes, to the history
async fn finish(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, mut out: Output, front: Option<String>, back: Option<String>) {
  // The plugins' `tool.after` may change what the model reads (the card keeps what the tool returned)
  let arguments = p.ran_with.clone().unwrap_or_else(|| serde_json::from_str(&p.call.arguments).unwrap_or_else(|_| Value::String(p.call.arguments.clone())));
  if let Some((model, is_error)) = crate::plugin_hooks::after_call(server, session, &p.call.name, &arguments, &out.model, out.is_error).await {
    out.model = model;
    out.is_error = is_error;
  }
  let images = images_of(p, &out);
  // A call without a row (a goal tool's) only goes into the history
  let (model, is_error) = if p.acp.is_empty() { (out.model, out.is_error) } else { report(server, session, &p.acp, out) };
  record(session, p, joined(front.as_deref(), &model, back.as_deref()), is_error, images);
}

/// The model's result of a call, into the history
pub(super) fn record(session: &Arc<Session>, p: &Prepared, model: String, is_error: bool, images: Vec<crate::llm::Image>) {
  let content = match &p.note {
    Some(note) => format!("{note}\n{model}"),
    None => model,
  };
  let item = Item::ToolResult { call_id: p.call.id.clone(), name: p.call.name.clone(), content, is_error, images };
  session.store.item(&item);
  session.state.lock().items.push(item);
}

/// Report a finished call on its row (`acp`); the model's copy and whether it failed come back
pub(super) fn report(server: &Arc<Server>, session: &Arc<Session>, acp: &str, mut out: Output) -> (String, bool) {
  for c in &mut out.content {
    let emptied = c.get_mut("_meta").and_then(Value::as_object_mut).is_some_and(|m| {
      m.remove("acpira/notForModel");
      m.is_empty()
    });
    if emptied && let Some(o) = c.as_object_mut() {
      o.remove("_meta");
    }
  }
  let mut update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "status": if out.is_error { "failed" } else { "completed" } });
  if !out.content.is_empty() {
    update["content"] = Value::Array(out.content);
  }
  if let Some(raw) = out.raw_output {
    update["rawOutput"] = raw;
  }
  // A failed command's exit code shows on its card
  if let Some(code) = update.pointer("/rawOutput/exitCode").and_then(Value::as_i64).filter(|c| *c != 0) {
    update["_meta"] = json!({ "terminal_exit": { "exit_code": code } });
  }
  server.update(&session.id, update);
  (out.model, out.is_error)
}
