//! What a host's engine pushes: its list, auth state, subagents and session views, rebuilt into this engine's own views

use std::sync::{Arc, Weak};

use serde_json::value::RawValue;
use serde_json::{Value, json};

use acpira_shared::protocol::{HostMsg, RawJson};
use acpira_shared::session_patch::{LastTurnParts, ViewParts};
use acpira_shared::transcript::SessionSummary;

use super::blobs::blob_refs;
use super::{HostRun, LIST_VIEW, SessionManager, Shown, host_folder_selected, local_folder};

impl SessionManager {
  /// One envelope from a host's engine
  pub(super) fn on_host_envelope(self: &Arc<Self>, run: &Arc<HostRun>, env: Value) {
    match env.get("type").and_then(Value::as_str) {
      Some("hostMessage") => {
        let view_id = env.get("viewId").and_then(Value::as_str).unwrap_or("").to_owned();
        let msg = env.get("message").cloned().unwrap_or(Value::Null);
        let key = match msg.get("type").and_then(Value::as_str) {
          Some("init") if view_id.starts_with("lookup-") => Some(view_id.clone()),
          Some("sessionHits") => msg.get("seq").and_then(Value::as_u64).map(|seq| format!("search-{seq}")),
          Some("controls") if view_id == LIST_VIEW => msg.get("agent").and_then(Value::as_str).map(|a| format!("controls-{a}")),
          Some("sessionMarkdown") if view_id == LIST_VIEW => msg.get("seq").and_then(Value::as_u64).map(|seq| format!("record-{seq}")),
          Some("editTurnResult") if view_id == LIST_VIEW => msg.get("requestId").and_then(Value::as_str).map(str::to_owned),
          _ => None,
        };
        if let Some(key) = key {
          if let Some(tx) = run.requests.lock().remove(&key) {
            let _ = tx.send(msg);
          }
          return;
        }
        self.on_host_auth(run, &msg);
        if msg.get("type").and_then(Value::as_str) == Some("deleteSessionResult") {
          let Some(request_id) = msg.get("requestId").and_then(Value::as_str) else { return };
          let Some(undo) = msg.get("undo").and_then(Value::as_bool) else { return };
          let error = msg.get("error").and_then(Value::as_str).map(str::to_owned);
          let waiting = run.deletions.lock().remove(request_id);
          if let Some((id, viewer)) = waiting {
            if undo && error.is_none() {
              run.deleted.lock().insert(id, tokio::time::Instant::now() + crate::session_manager::TRASH_TTL);
            }
            if let Some(v) = viewer.upgrade() {
              v.emit(HostMsg::DeleteSessionResult { request_id: request_id.to_owned(), undo, error });
            }
          }
        } else if view_id == LIST_VIEW {
          self.on_host_list(run, &msg);
        } else {
          self.on_host_view(run, &view_id, &msg);
        }
      }
      Some("platformRequest") => {
        let request = env.get("request").cloned().unwrap_or(Value::Null);
        match request.get("method").and_then(Value::as_str) {
          Some("toast") => {
            let text = request.get("text").and_then(Value::as_str).unwrap_or("");
            let level = request.get("level").and_then(Value::as_str).unwrap_or("info");
            (self.deps.toast)(level, &format!("{}: {text}", run.alias));
          }
          Some("runInTerminal") => {
            if let (Some(remote), Some(command)) = (&self.deps.remote, request.get("command").and_then(Value::as_str)) {
              let title = request.get("title").and_then(Value::as_str).unwrap_or(command);
              let args: Vec<String> = request
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
              let (prog, ssh_args) = remote.terminal_command(&run.alias, command, &args, request.get("env").and_then(Value::as_object));
              (self.deps.run_in_terminal)(format!("{title} ({})", run.alias), prog, ssh_args, None);
            }
          }
          _ => {}
        }
        if let Some(id) = env.get("requestId").and_then(Value::as_str) {
          run.send(json!({ "type": "platformResponse", "requestId": id, "error": "not available to a peer" }));
        }
      }
      _ => {}
    }
  }

  /// Account pushes belong to the host, not the local settings. Reissue each shown view's head when they change so
  /// login progress and keychain locks update even while the transcript stands still.
  fn on_host_auth(self: &Arc<Self>, run: &Arc<HostRun>, msg: &Value) {
    let changed = {
      let mut auth = run.auth.lock();
      let before = auth.clone();
      match msg.get("type").and_then(Value::as_str) {
        Some("init") => {
          if let Some(state) = msg.get("state") {
            for (key, source) in [
              ("agents", "agents"),
              ("accounts", "accounts"),
              ("actions", "accountActions"),
              ("installs", "agentInstalls"),
            ] {
              if let Some(v) = state.get(source) {
                auth[key] = v.clone();
              }
            }
          }
        }
        Some("agents") => auth["agents"] = msg.get("agents").cloned().unwrap_or_else(|| json!([])),
        Some("accounts") => auth["accounts"] = msg.get("accounts").cloned().unwrap_or_else(|| json!([])),
        Some("accountActions") => auth["actions"] = msg.get("actions").cloned().unwrap_or_else(|| json!([])),
        Some("agentInstalls") => auth["installs"] = msg.get("installs").cloned().unwrap_or_else(|| json!([])),
        _ => return,
      }
      *auth != before
    };
    if !changed {
      return;
    }
    let messages: Vec<(String, HostMsg)> = {
      let mut shown = run.shown.lock();
      shown.iter_mut().filter_map(|(id, s)| {
        let prev = s.parts.as_ref()?;
        let mut head: Value = serde_json::from_str(prev.0.head.get()).ok()?;
        head["remoteAuth"] = remote_auth(&run.auth.lock(), head.get("agent")?.as_str()?);
        let head = serde_json::value::to_raw_value(&head).ok()?;
        let last = prev.0.last.as_ref().map(|l| LastTurnParts { head: l.head.clone(), blocks: l.blocks.clone() });
        let turns = prev.0.turns.clone();
        s.publish(id, &head, self.next_mirror_rev(), turns, last).map(|msg| (id.clone(), msg))
      }).collect()
    };
    for (id, msg) in messages {
      for v in self.viewers_on(&id, None) {
        v.emit(msg.clone());
      }
    }
  }

  fn on_host_list(self: &Arc<Self>, run: &Arc<HostRun>, msg: &Value) {
    let list = match msg.get("type").and_then(Value::as_str) {
      Some("init") => msg.pointer("/state/sessions"),
      Some("sessions") => msg.get("sessions"),
      _ => None,
    };
    let Some(list) = list.and_then(|l| serde_json::from_value::<Vec<SessionSummary>>(l.clone()).ok()) else { return };
    // Sessions the host's engine lists from hosts of its own are theirs, not this host's
    let scope = run.folders.lock();
    let items: Vec<SessionSummary> = list
      .into_iter()
      .filter(|s| !acpira_shared::remote::is_remote_folder(&s.cwd))
      .map(|mut s| {
        s.cwd = local_folder(&run.alias, &s.cwd);
        s.category = None;
        s
      })
      .filter(|s| host_folder_selected(&run.alias, &s.cwd, scope.as_deref()))
      .collect();
    if *run.items.lock() == items {
      return;
    }
    run.deleted.lock().retain(|id, until| *until > tokio::time::Instant::now() && !items.iter().any(|s| &s.id == id));
    self.save_host_list(&run.alias, &items);
    *run.items.lock() = items;
    drop(scope);
    self.emit_sessions();
  }

  /// A subagent transcript the host pushed through a session's view (a page here observes it: `observeSubagent` went to that
  /// view like the session's other actions). The host's rev is the session's own counter, so it passes through
  fn on_host_subagent(&self, run: &Arc<HostRun>, view_id: &str, msg: &Value) {
    let Some(id) = run.shown.lock().iter().find(|(_, s)| s.view_id == view_id).map(|(id, _)| id.clone()) else { return };
    let (Some(sub), Some(rev), Some(turns)) = (msg.get("subagentId").and_then(Value::as_str), msg.get("rev").and_then(Value::as_i64), msg.get("turns")) else {
      return;
    };
    let Ok(turns) = serde_json::value::to_raw_value(turns) else { return };
    let running = msg.get("running").and_then(Value::as_bool).unwrap_or(false);
    let m = HostMsg::Subagent { session_id: id.clone(), subagent_id: sub.to_owned(), rev, running, turns: RawJson(Arc::from(turns)) };
    for v in self.viewers_on(&id, None) {
      v.emit(m.clone());
    }
  }

  fn on_host_view(self: &Arc<Self>, run: &Arc<HostRun>, view_id: &str, msg: &Value) {
    if msg.get("type").and_then(Value::as_str) == Some("subagent") {
      self.on_host_subagent(run, view_id, msg);
      return;
    }
    let (whole, patch) = match msg.get("type").and_then(Value::as_str) {
      Some("init") => (msg.pointer("/state/active").filter(|a| !a.is_null()), None),
      Some("session") => (msg.get("session"), None),
      Some("sessionPatch") => (None, msg.get("patch")),
      _ => return,
    };
    // Whether this view showed this session already: a reused view's push of the session it is leaving is no answer
    let again = whole
      .and_then(|w| w.get("id"))
      .and_then(Value::as_str)
      .is_some_and(|id| run.shown.lock().get(id).is_some_and(|s| s.view_id == view_id));
    // A reused host viewer has moved to its new session. Move the local pages with it and retire the old patch base.
    if let Some(id) = whole.and_then(|w| w.get("id")).and_then(Value::as_str) {
      let previous = {
        let mut shown = run.shown.lock();
        let previous = shown.iter().find(|(old, s)| old.as_str() != id && s.view_id == view_id).map(|(old, _)| old.clone());
        if let Some(old) = &previous {
          shown.remove(old);
          shown.insert(id.to_owned(), Shown::new(view_id.to_owned()));
        }
        previous
      };
      if let Some(old) = previous {
        let asker = run.asked.lock().get(view_id).and_then(Weak::upgrade);
        for v in self.viewers_on(&old, None) {
          match &asker {
            // Another page on the old session keeps it, through a view of its own on the host
            Some(a) if !Arc::ptr_eq(a, &v) => self.show_host_session(run, &old, &v),
            _ => self.set_active(&v, Some(id.to_owned())),
          }
        }
      }
    }
    // A view that asked for a new session: the first one it shows is that session, and its viewer goes there
    if let Some(id) = whole.and_then(|w| w.get("id")).and_then(Value::as_str)
      && !again
      && let Some(viewer) = run.asked.lock().remove(view_id)
    {
      run.shown.lock().insert(id.to_owned(), Shown::new(view_id.to_owned()));
      if let Some(v) = viewer.upgrade() {
        self.set_active(&v, Some(id.to_owned()));
      }
    }
    let Some(id) = run.shown.lock().iter().find(|(_, s)| s.view_id == view_id).map(|(id, _)| id.clone()) else { return };
    // The blobs this step shows (a whole view's, or a patch's changed tail): they live in the host's store
    let mut refs = vec![];
    if let Some(step) = whole.or(patch) {
      blob_refs(step, &id, &mut refs);
    }
    // Those not on this machine: the view goes out without them now, and each comes back with its file
    refs.sort_unstable();
    refs.dedup();
    let store = &self.deps.store;
    refs.retain(|name| store.blob_path(&id, name).is_some_and(|p| !p.exists()));
    let built = {
      let mut shown = run.shown.lock();
      let Some(s) = shown.get_mut(&id) else { return };
      s.absent.extend(refs.iter().cloned());
      let step = match (whole, patch) {
        (Some(w), _) => split_view(w),
        (None, Some(p)) if p.get("id").and_then(Value::as_str) == Some(id.as_str()) => {
          let fits = s.host_rev.is_some() && p.get("base").and_then(Value::as_i64) == s.host_rev;
          match (fits, s.parts.as_ref()) {
            (true, Some(prev)) => apply_patch(prev, p).and_then(|(turns, last)| Some((p.get("view")?.clone(), turns, last))),
            _ => None,
          }
        }
        _ => return,
      };
      match step {
        None => None,
        Some((mut head, turns, last)) => {
          s.host_rev = head.get("rev").and_then(Value::as_i64);
          if let Some(cwd) = head.get("cwd").and_then(Value::as_str) {
            head["cwd"] = json!(local_folder(&run.alias, cwd));
          }
          if let Some(agent) = head.get("agent").and_then(Value::as_str) {
            head["remoteAuth"] = remote_auth(&run.auth.lock(), agent);
          }
          let rev = self.next_mirror_rev();
          let head = serde_json::value::to_raw_value(&head).ok();
          head.and_then(|h| s.publish(&id, &h, rev, turns, last))
        }
      }
    };
    match built {
      Some(msg) => {
        self.fetch_host_blobs(run, &id, refs);
        for v in self.viewers_on(&id, None) {
          v.emit(msg.clone());
        }
      }
      // Out of step with the host's view: it sends the whole view again
      None => {
        self.log(&format!("remote run: {id} on {} out of step, asking for the whole view", run.alias));
        run.send_to_view(view_id, json!({ "type": "resync" }));
      }
    }
  }
}

/// A whole view as parts: its head without turns, every turn, and the live agent turn split into its head and blocks
pub(super) fn split_view(view: &Value) -> Option<(Value, Vec<Box<RawValue>>, Option<LastTurnParts>)> {
  let mut head = view.clone();
  let turns = std::mem::replace(head.get_mut("turns")?, json!([]));
  let turns = turns.as_array()?;
  let mut encoded: Vec<Box<RawValue>> = turns.iter().map(|t| serde_json::value::to_raw_value(t).ok()).collect::<Option<_>>()?;
  let last = turns.last().and_then(split_last);
  if let (Some(l), Some(slot)) = (&last, encoded.last_mut()) {
    *slot = crate::bus::splice(&l.head, "blocks", &l.blocks)?;
  }
  Some((head, encoded, last))
}

/// The live agent turn's head (its `blocks` emptied) and blocks; None for any other turn
fn split_last(turn: &Value) -> Option<LastTurnParts> {
  if turn.get("role").and_then(Value::as_str) != Some("agent") {
    return None;
  }
  let mut head = turn.clone();
  let blocks = std::mem::replace(head.get_mut("blocks")?, json!([]));
  let blocks = blocks.as_array()?.iter().map(|b| serde_json::value::to_raw_value(b).ok()).collect::<Option<Vec<_>>>()?;
  Some(LastTurnParts { head: serde_json::value::to_raw_value(&head).ok()?, blocks })
}

/// The host's `sessionPatch` on top of what this engine holds of that view: the new turns and live turn, or None when
/// it does not fit (another base), which asks the host for the whole view
pub(super) fn apply_patch(prev: &ViewParts, patch: &Value) -> Option<(Vec<Box<RawValue>>, Option<LastTurnParts>)> {
  let keep = patch.get("keep")?.as_u64()? as usize;
  let held = &prev.0.turns;
  if keep > held.len() {
    return None;
  }
  let raw = |v: &Value| serde_json::value::to_raw_value(v).ok();
  let sent: Vec<Value> = patch.get("turns")?.as_array()?.clone();
  let mut turns: Vec<Box<RawValue>> = held[..keep].to_vec();
  if let Some(kb) = patch.get("keepBlocks").and_then(Value::as_u64) {
    // The same live turn: its new head, the blocks it kept and the ones that changed
    let pl = prev.0.last.as_ref()?;
    let head = raw(sent.first()?)?;
    let mut blocks = pl.blocks.get(..kb as usize)?.to_vec();
    for b in patch.get("blocks")?.as_array()? {
      blocks.push(raw(b)?);
    }
    turns.push(crate::bus::splice(&head, "blocks", &blocks)?);
    return Some((turns, Some(LastTurnParts { head, blocks })));
  }
  for t in &sent {
    turns.push(raw(t)?);
  }
  // The live turn's parts come from the last turn as it now reads; one kept from before has no parts here
  let last = match sent.last() {
    Some(t) => split_last(t),
    None if keep == held.len() => prev.0.last.as_ref().map(|l| LastTurnParts { head: l.head.clone(), blocks: l.blocks.clone() }),
    None => None,
  };
  Some((turns, last))
}

/// Select public metadata for the session's agent. No local account may appear in a host session's menu.
fn remote_auth(auth: &Value, agent: &str) -> Value {
  let info = auth.get("agents").and_then(Value::as_array).and_then(|a| a.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(agent)))
    .cloned().unwrap_or_else(|| json!({ "id": agent, "name": agent }));
  let accounts: Vec<Value> = auth.get("accounts").and_then(Value::as_array).into_iter().flatten()
    .filter(|a| a.get("agent").and_then(Value::as_str) == Some(agent)).cloned().collect();
  let action = auth.get("actions").and_then(Value::as_array).into_iter().flatten()
    .find(|a| a.get("agent").and_then(Value::as_str) == Some(agent));
  let install = auth.get("installs").and_then(Value::as_array).into_iter().flatten()
    .find(|a| a.get("agent").and_then(Value::as_str) == Some(agent));
  let mut out = json!({ "agent": info, "accounts": accounts });
  // The start picker needs this host's CLI availability, not the viewing machine's installation state.
  if let Some(agents) = auth.get("agents").filter(|a| a.is_array()) {
    out["agents"] = agents.clone();
  }
  if let Some(action) = action {
    out["action"] = action.clone();
  }
  if let Some(install) = install {
    out["install"] = install.clone();
  }
  out
}
