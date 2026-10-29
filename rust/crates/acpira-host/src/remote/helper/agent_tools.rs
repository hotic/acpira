//! The built-in agent's tool calls run on this host, and its background jobs here by session

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::files::{blocking, host_path, mapped, path_arg, unmapped};
use super::{MAX_READ, Out};
use crate::remote::wire::WireError;

/// The built-in agent's background jobs on this host, by session (`acpira_agent::tools::jobs`); they end with the
/// helper, so a dropped connection never leaves a session's dev server running unseen
static JOBS: std::sync::LazyLock<Mutex<HashMap<String, Arc<acpira_agent::tools::jobs::Jobs>>>> = std::sync::LazyLock::new(Default::default);

pub fn stop_all_jobs() {
  let jobs: Vec<_> = JOBS.lock().drain().map(|(_, j)| j).collect();
  for j in jobs {
    j.stop_all();
  }
}

/// A file-system safe name for a session or call id
fn safe_name(s: &str) -> String {
  s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// A tool call of the built-in agent, the same code it runs on its own machine (docs/dev/remote.md, "Built-in agent").
/// `prepare` validates the call against this host's files (an edit's resulting text, so the card shows the real
/// diff) and marks the files this host's guard protects; `run` carries it out. Output over the budget is written to a
/// folder of the call and sent back as `spill:<name>` chunks, since the session's own files stay on the engine's
/// machine; progress (a command's output so far) goes out as `progress` chunks, one JSON value per line. With `keep`
/// (Claude Code's calls, `claude_remote.rs`) the spilled outputs stay in that folder for the model to read on this host
/// instead. `close` stops a session's background jobs and drops its kept outputs
pub(super) async fn tool(a: &Value, id: u64, out: &Out) -> Result<Value, WireError> {
  use acpira_agent::tools::{self, Action, Ctx};
  let session = a.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
  match a.get("phase").and_then(Value::as_str).unwrap_or("") {
    "prepare" => {
      let cwd = path_arg(a, "cwd")?;
      let name = a.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
      let args = a.get("args").cloned().unwrap_or(Value::Null);
      let images = a.get("images").and_then(Value::as_bool).unwrap_or(false);
      blocking(move || {
        Ok(match tools::prepare(&name, &args, &cwd, images) {
          Ok(mut action) => {
            if let Action::Write { files } = &mut action {
              let home = std::env::var_os("HOME").map(PathBuf::from);
              let guard = acpira_agent::permission::Guard::new(&home.clone().unwrap_or_default().join(".acpira"), &cwd, home.as_deref());
              for f in files.iter_mut() {
                f.protected = guard.protects(&f.path);
              }
            }
            json!({ "action": action })
          }
          Err(e) => json!({ "error": e }),
        })
      })
      .await
      .map(unmapped)
    }
    // Files compaction brings back after its summary (`acpira_agent::compact`): the text ones, read as they are now
    "files" => {
      let mut paths = vec![];
      for p in a.get("paths").and_then(Value::as_array).into_iter().flatten() {
        paths.push(host_path(p.as_str().unwrap_or(""))?);
      }
      blocking(move || {
        let files: Vec<Value> = paths
          .iter()
          .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= MAX_READ))
          .filter_map(|p| {
            let bytes = std::fs::read(p).ok()?;
            if bytes[..bytes.len().min(8192)].contains(&0) {
              return None;
            }
            Some(json!({ "path": p, "text": String::from_utf8(bytes).ok()? }))
          })
          .collect();
        Ok(json!({ "files": files }))
      })
      .await
      .map(unmapped)
    }
    "run" => {
      let cwd = path_arg(a, "cwd")?;
      let call = a.get("callId").and_then(Value::as_str).unwrap_or("call").to_owned();
      let action: Action = serde_json::from_value(mapped(a.get("action").cloned().unwrap_or(Value::Null)))
        .map_err(|e| WireError::new("invalid", format!("not a tool action this host runs: {e}")))?;
      let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
      let dir = home.join(".acpira/remote/spill").join(safe_name(&session)).join(safe_name(&call));
      let keep = a.get("keep").and_then(Value::as_bool).unwrap_or(false);
      let jobs = JOBS.lock().entry(session.clone()).or_default().clone();
      let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
      let ctx = Ctx {
        cwd,
        outputs: dir.clone(),
        call_id: call,
        progress: Arc::new(move |v: Value| {
          let _ = tx.send(v);
        }),
        jobs,
        search: Default::default(),
        mcp: Arc::new(acpira_agent::mcp::Hub::default()),
        history: PathBuf::new(),
      };
      let run = action.run(ctx);
      tokio::pin!(run);
      let line = |v: Value| {
        let mut b = serde_json::to_vec(&v).unwrap_or_default();
        b.push(b'\n');
        b
      };
      let output = loop {
        tokio::select! {
          o = &mut run => break o,
          Some(v) = rx.recv() => out.chunks(id, "progress", &line(unmapped(v))).await,
        }
      };
      while let Ok(v) = rx.try_recv() {
        out.chunks(id, "progress", &line(unmapped(v))).await;
      }
      if keep {
        return Ok(unmapped(json!({ "output": output })));
      }
      let spilled = {
        let dir = dir.clone();
        blocking(move || {
          let mut files = vec![];
          if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
              if let Ok(bytes) = std::fs::read(e.path()) {
                files.push((e.file_name().to_string_lossy().into_owned(), bytes));
              }
            }
          }
          let _ = std::fs::remove_dir_all(&dir);
          Ok(files)
        })
        .await?
      };
      for (name, bytes) in spilled {
        out.chunks(id, &format!("spill:{name}"), &bytes).await;
      }
      Ok(unmapped(json!({ "output": output, "spillDir": dir })))
    }
    "close" => {
      if let Some(j) = JOBS.lock().remove(&session) {
        j.stop_all();
      }
      if let Some(home) = std::env::var_os("HOME").filter(|_| !session.is_empty()) {
        let dir = PathBuf::from(home).join(".acpira/remote/spill").join(safe_name(&session));
        let _ = blocking(move || Ok(std::fs::remove_dir_all(dir))).await;
      }
      Ok(json!({}))
    }
    other => Err(WireError::new("invalid", format!("unknown tool phase \"{other}\""))),
  }
}
