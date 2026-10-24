//! Sessions whose folder is on an SSH host (docs/dev/remote.md, "Built-in agent"). The engine opens them with the
//! host's path as `cwd` and `_meta["acpira/remote"] = { alias }`. The loop, the model calls, the session log and the
//! session's own folder (spilled outputs, the plan) stay on this machine; the calls that read or change the project or
//! run a command are prepared and run on the host by the same tool code, through `_acpira/remoteTool` requests the
//! engine forwards to its helper there. A call whose connection drops comes back as an error saying its outcome is
//! unknown: nothing is replayed

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::tools::{self, Action, Ctx, Output};

/// The `_meta` key of session/new, load and resume naming the host
pub const META: &str = "acpira/remote";
/// Agent → client: prepare or run one call on the host
pub const TOOL: &str = "_acpira/remoteTool";
/// Client → agent: a running call's progress (`{ sessionId, toolCallId, update }`)
pub const PROGRESS: &str = "_acpira/remoteProgress";

type Sink = Arc<dyn Fn(Value) + Send + Sync>;

pub struct Remote {
  pub alias: String,
  /// The host's AGENTS.md, prompt overrides and skills for the folder, read there when the session opened (None: the
  /// host could not be asked, so the session has none)
  pub files: Option<Arc<crate::prompt::HostFiles>>,
  /// The progress sinks of the calls running on the host, by tool call id
  progress: Mutex<HashMap<String, Sink>>,
}

impl Remote {
  /// The host a session request names, if any
  pub fn of(params: &Value) -> Option<Arc<Remote>> {
    let meta = params.get("_meta")?.get(META)?;
    let alias = meta.get("alias")?.as_str().filter(|a| !a.is_empty())?;
    let files = meta.get("places").and_then(|p| serde_json::from_value(p.clone()).ok()).map(Arc::new);
    Some(Arc::new(Remote { alias: alias.to_owned(), files, progress: Default::default() }))
  }

  /// Whether a call of this tool goes to the host: everything that reads or changes the project or runs a command,
  /// unless every file it names is in the session's own folder (`own`: a spilled output, the plan)
  pub fn prepares_there(name: &str, args: &Value, cwd: &Path, own: &Path) -> bool {
    match name {
      tools::READ | tools::WRITE | tools::EDIT => {
        let files = tools::files_of(name, args, cwd);
        files.is_empty() || !files.iter().all(|f| f.starts_with(own))
      }
      tools::BASH | tools::JOB | tools::GREP | tools::GLOB | tools::LIST => true,
      _ => false,
    }
  }

  /// Whether a prepared call runs on the host (the same split as `prepares_there`)
  pub fn runs_there(action: &Action, own: &Path) -> bool {
    match action {
      Action::Read { files, .. } => !files.iter().all(|f| f.path.starts_with(own)),
      Action::Write { files } => !files.iter().all(|f| f.path.starts_with(own)),
      Action::Bash { .. } | Action::Job { .. } | Action::Grep { .. } | Action::Glob { .. } | Action::List { .. } => true,
      _ => false,
    }
  }

  /// Validate a call on the host; the error goes back to the model as the call's result
  pub async fn prepare(&self, conn: &acpira_rpc::rpc::Connection, session: &str, cwd: &Path, name: &str, args: &Value, images: bool) -> Result<Action, String> {
    let params = json!({ "sessionId": session, "phase": "prepare", "cwd": cwd, "name": name, "args": args, "images": images });
    let reply = conn.request_cancellable(TOOL, params).await.map_err(|e| format!("The call could not be prepared on {}: {}", self.alias, e.message))?;
    if let Some(e) = reply.get("error").and_then(Value::as_str) {
      return Err(e.to_owned());
    }
    serde_json::from_value(reply.get("action").cloned().unwrap_or(Value::Null)).map_err(|e| format!("{} answered with a call this agent does not know: {e}", self.alias))
  }

  /// Run a prepared call on the host. Its spilled outputs come back into `ctx.outputs`, and the model's copy names them
  /// there
  pub async fn run(&self, conn: &acpira_rpc::rpc::Connection, session: &str, action: Action, ctx: Ctx) -> Output {
    let call = ctx.call_id.clone();
    self.progress.lock().insert(call.clone(), ctx.progress.clone());
    let params = json!({ "sessionId": session, "phase": "run", "cwd": ctx.cwd, "callId": call, "action": action });
    let reply = conn.request_cancellable(TOOL, params).await;
    self.progress.lock().remove(&call);
    let reply = match reply {
      Ok(r) => r,
      Err(e) => {
        return Output::error(format!(
          "The call did not complete on {}: {}. Whether it took effect there is unknown: check before doing it again.",
          self.alias, e.message
        ));
      }
    };
    let mut out: Output = match serde_json::from_value(reply.get("output").cloned().unwrap_or(Value::Null)) {
      Ok(o) => o,
      Err(e) => return Output::error(format!("{} answered with a result this agent cannot read: {e}", self.alias)),
    };
    let spills = reply.get("spills").and_then(Value::as_array).cloned().unwrap_or_default();
    if !spills.is_empty() {
      let _ = std::fs::create_dir_all(&ctx.outputs);
      for s in &spills {
        let (Some(name), Some(text)) = (s.get("name").and_then(Value::as_str), s.get("text").and_then(Value::as_str)) else { continue };
        // A name the host made from the call id; never a path
        if name.contains(['/', '\\']) || name.starts_with('.') {
          continue;
        }
        let _ = std::fs::write(ctx.outputs.join(name), text);
      }
      if let Some(dir) = reply.get("spillDir").and_then(Value::as_str).filter(|d| !d.is_empty()) {
        out.model = out.model.replace(dir, &ctx.outputs.to_string_lossy());
      }
    }
    out
  }

  /// The text files among `paths` as they are on the host now (compaction's file restore); a path that is gone, not
  /// text or too large is missing from the answer, as is everything when the host cannot be asked
  pub async fn read_files(&self, conn: &acpira_rpc::rpc::Connection, session: &str, cwd: &Path, paths: &[std::path::PathBuf]) -> HashMap<std::path::PathBuf, String> {
    let params = json!({ "sessionId": session, "phase": "files", "cwd": cwd, "paths": paths });
    let Ok(reply) = conn.request_cancellable(TOOL, params).await else { return HashMap::new() };
    let files = reply.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    files
      .iter()
      .filter_map(|f| Some((std::path::PathBuf::from(f.get("path")?.as_str()?), f.get("text")?.as_str()?.to_owned())))
      .collect()
  }

  /// A progress notification for a call running on the host
  pub fn progress(&self, params: &Value) {
    let Some(call) = params.get("toolCallId").and_then(Value::as_str) else { return };
    let sink = self.progress.lock().get(call).cloned();
    if let (Some(sink), Some(update)) = (sink, params.get("update")) {
      sink(update.clone());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  #[test]
  fn the_session_folder_stays_here_and_the_project_goes_there() {
    let (cwd, own) = (Path::new("/home/u/work"), Path::new("/Users/me/.acpira/agent/sessions/s1"));
    let spill = own.join("outputs/call-3.txt").to_string_lossy().into_owned();
    assert!(!Remote::prepares_there(tools::READ, &json!({ "path": spill }), cwd, own));
    assert!(Remote::prepares_there(tools::READ, &json!({ "path": "src/a.rs" }), cwd, own));
    assert!(Remote::prepares_there(tools::READ, &json!({ "paths": [spill, "src/a.rs"] }), cwd, own));
    assert!(Remote::prepares_there(tools::BASH, &json!({ "command": "ls" }), cwd, own));
    assert!(!Remote::prepares_there(tools::TODO, &json!({}), cwd, own));
    let local = Action::Read { files: vec![tools::read::Slice { path: PathBuf::from(&spill), offset: 1, limit: 10 }], images: false };
    assert!(!Remote::runs_there(&local, own));
    let there = Action::Bash { command: "ls".into(), workdir: cwd.into(), timeout_ms: 1000, background: false, interactive: false };
    assert!(Remote::runs_there(&there, own));
    assert!(Remote::of(&json!({ "_meta": { "acpira/remote": { "alias": "gen1" } } })).is_some_and(|r| r.alias == "gen1"));
    assert!(Remote::of(&json!({ "cwd": "/x" })).is_none());
  }
}
