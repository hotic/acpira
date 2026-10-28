//! The server's way back to the sidecar (`relay/hub.rs`): `ask_agent`, `ask_caller`, the session and plugin tools,
//! `open_browser` and the plugins' tool hooks go over the loopback hub, and the roster watch tells the client when
//! `ask_agent`'s agents change

use std::sync::Arc;

use serde_json::{Map, Value, json};

use acpira_shared::host_tools::{SEARCH_SESSIONS as SEARCH_SESSIONS_TOOL, SHOW_WIDGET as WIDGET_TOOL, WORKSPACE_HOOKS as HOOKS_TOOL};

use super::tools::{show_image_with, show_widget_with, tool_error, workspace_hooks_with};

/// The way back to the sidecar (`relay/hub.rs`), from the env the session put on this server's entry
#[derive(Clone)]
pub(super) struct Relay {
  addr: String,
  token: String,
  /// How long one call may wait, picked from the MCP client at initialize
  wait_secs: Arc<std::sync::atomic::AtomicU64>,
  watch: Arc<Watch>,
}

/// What `tools/list` last told the client about the roster, and the change it was last notified of
#[derive(Default)]
struct Watch {
  started: std::sync::atomic::AtomicBool,
  /// One lock for both, so a listing between a poll's check and its notification cannot be told of again
  state: parking_lot::Mutex<WatchState>,
}

#[derive(Default)]
pub(super) struct WatchState {
  /// The roster's fingerprint at the client's latest `tools/list` (None before the first)
  pub(super) served: Option<String>,
  /// The fingerprint a `list_changed` went out for since then, so a client that never lists again is told once
  pub(super) notified: Option<String>,
}

impl WatchState {
  /// One poll's roster fingerprint: true when the client should be told its list changed
  pub(super) fn poll(&mut self, print: String) -> bool {
    let stale = self.served.as_ref().is_some_and(|served| *served != print);
    if !stale {
      // Back to what the client holds: a later change away from it is news again (A → B → A → B)
      self.notified = None;
      return false;
    }
    if self.notified.as_deref() == Some(print.as_str()) {
      return false;
    }
    self.notified = Some(print);
    true
  }
}

/// How often the server compares the roster with what it listed: one loopback request and one small file read
const ROSTER_POLL: std::time::Duration = std::time::Duration::from_secs(2);
/// Failed polls in a row after which the watch ends: the grant is gone (session closed) or the sidecar with it
const ROSTER_POLL_FAILURES: u32 = 3;

/// The part of the roster `ask_agent` lists, as compared between polls. The built-in agent's sidekick is left out:
/// that agent keeps its tool table as first sent and tells a sidekick change in a note instead (`builtin-agent.md`)
fn roster_fingerprint(personas: &[acpira_shared::subagents::SubagentPersona], groups: &[acpira_shared::subagents::SubagentGroup]) -> String {
  let personas: Vec<_> = personas.iter().filter(|p| p.id != acpira_shared::subagents::SIDEKICK_ID).collect();
  serde_json::to_string(&(personas, groups)).unwrap_or_default()
}

/// What a listing depends on besides the fixed tools: the roster (`roster_fingerprint`), the plugin tools in effect and
/// the tool groups switched off. An unreachable hub offered none of them, which counts as empty
pub(super) fn listing_print(
  roster: Option<&(Vec<acpira_shared::subagents::SubagentPersona>, Vec<acpira_shared::subagents::SubagentGroup>)>,
  plugins: Option<&(Vec<Value>, Vec<String>)>,
) -> String {
  let roster = roster.map(|(p, g)| roster_fingerprint(p, g)).unwrap_or_else(|| roster_fingerprint(&[], &[]));
  let plugins = plugins.map(|p| serde_json::to_string(p).unwrap_or_default()).unwrap_or_default();
  format!("{roster}\n{plugins}")
}

/// Claude Code waits for an MCP tool call far longer than a child round takes, and so does the built-in agent (client
/// name `acpira`, `CALL_TIMEOUT` 30 min in `acpira-agent/src/mcp`); Codex gives up after 60 s by default
/// (`tool_timeout_sec`), and the others are not known, so they get an answer within the shorter bound
const SHORT_WAIT: u64 = 50;
const LONG_WAIT: u64 = 25 * 60;

impl Relay {
  pub(super) fn from_env() -> Option<Relay> {
    use crate::relay::{ENV_ADDR, ENV_TOKEN};
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    Some(Relay {
      addr: var(ENV_ADDR)?,
      token: var(ENV_TOKEN)?,
      wait_secs: Arc::new(SHORT_WAIT.into()),
      watch: Default::default(),
    })
  }

  pub(super) fn note_client(&self, params: Option<&Value>) {
    let name = params.and_then(|p| p.get("clientInfo")).and_then(|c| c.get("name")).and_then(Value::as_str).unwrap_or("");
    let lower = name.to_lowercase();
    let secs = if lower.contains("claude") || lower == "acpira" { LONG_WAIT } else { SHORT_WAIT };
    self.wait_secs.store(secs, std::sync::atomic::Ordering::Relaxed);
  }

  fn request(&self, op: crate::relay::wire::HubOp) -> crate::relay::wire::HubRequest {
    crate::relay::wire::HubRequest { token: self.token.clone(), op }
  }

  /// The host tools the serving shell can carry out now; none when the hub cannot be reached
  pub(super) fn features(&self) -> crate::relay::wire::HostFeatures {
    use crate::relay::wire::{HubOp, HubReply};
    let mut features = Default::default();
    let _ = crate::relay::hub::call(&self.addr, &self.request(HubOp::Features), |r| {
      if let HubReply::Features(f) = r {
        features = f;
      }
      false
    });
    features
  }

  /// `open_browser`: checked here for shape, carried out by the sidecar and its shell
  pub(super) fn open_browser(&self, params: &Value) -> Value {
    use crate::relay::wire::HubOp;
    let url = params.get("arguments").and_then(|a| a.get("url")).and_then(Value::as_str).map(str::trim).unwrap_or("");
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    if !matches!(scheme.as_deref(), Some("http" | "https")) {
      return tool_error("Pass `url`: an absolute http:// or https:// address.".into());
    }
    self.answer(HubOp::OpenBrowser { url: url.to_owned() })
  }

  /// `show_image` / `show_widget` / `workspace_hooks` for a session folder on an SSH host: the hub checks (and reads)
  /// the files there (`AcpSession::fetch_shown`, `describe_host_hooks`), then the answer is the local one
  pub(super) fn show_on_host(&self, params: &Value) -> Value {
    use crate::relay::wire::HubOp;
    let empty = Map::new();
    let args = params.get("arguments").and_then(Value::as_object).unwrap_or(&empty);
    if params.get("name").and_then(Value::as_str) == Some(HOOKS_TOOL) {
      return workspace_hooks_with(args, &|cwd| {
        let checked = self.answer(HubOp::HostHooks { cwd: cwd.to_owned() });
        checked.pointer("/content/0/text").and_then(Value::as_str).unwrap_or("").to_owned()
      });
    }
    let widget = params.get("name").and_then(Value::as_str) == Some(WIDGET_TOOL);
    let one = args.get("path").and_then(Value::as_str).into_iter();
    let many = args.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter(|_| !widget);
    let paths: Vec<String> = one.chain(many).map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect();
    if !paths.is_empty() {
      let checked = self.answer(HubOp::HostFiles { paths, widget });
      if checked.get("isError").and_then(Value::as_bool) == Some(true) {
        return checked;
      }
    }
    let fine = |_: &str| Ok(());
    if widget { show_widget_with(args, &fine) } else { show_image_with(args, &fine) }
  }

  /// `search_sessions` / `read_session`: the arguments go to the hub as they are, the sidecar reads and bounds the answer
  pub(super) fn session_tool(&self, params: &Value) -> Value {
    use crate::relay::wire::HubOp;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let op = match params.get("name").and_then(Value::as_str) {
      Some(SEARCH_SESSIONS_TOOL) => serde_json::from_value(args).map(HubOp::SearchSessions),
      _ => serde_json::from_value(args).map(HubOp::ReadSession),
    };
    match op {
      Ok(op) => self.answer(op),
      Err(e) => tool_error(format!("Bad arguments: {e}")),
    }
  }

  /// One request whose answer is a single `Done` / `Error` line, as a tool result
  pub(super) fn answer(&self, op: crate::relay::wire::HubOp) -> Value {
    use crate::relay::wire::HubReply;
    let mut end: Option<Value> = None;
    let r = crate::relay::hub::call(&self.addr, &self.request(op), |reply| {
      end = match reply {
        HubReply::Done(text) => Some(json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
        HubReply::Error(text) => Some(tool_error(text)),
        _ => return true,
      };
      false
    });
    match (end, r) {
      (Some(v), _) => v,
      (None, Err(e)) => tool_error(format!("Acpira could not be reached: {e}")),
      (None, Ok(())) => tool_error("Acpira closed the call without an answer.".into()),
    }
  }

  /// The personas and groups the hub offers this session; None when the hub cannot be reached or refused the grant
  pub(super) fn roster(&self) -> Option<(Vec<acpira_shared::subagents::SubagentPersona>, Vec<acpira_shared::subagents::SubagentGroup>)> {
    use crate::relay::wire::{HubOp, HubReply};
    let mut roster = None;
    let _ = crate::relay::hub::call(&self.addr, &self.request(HubOp::List), |r| {
      match r {
        HubReply::Personas(p) => roster = Some((p, vec![])),
        HubReply::Roster { personas, groups } => roster = Some((personas, groups)),
        _ => {}
      }
      false
    });
    roster
  }

  /// The plugins' answer about a call to one of this server's tools (`HubOp::ToolHook`); `{}` when the hub cannot be
  /// reached, so a call never waits on a missing engine
  pub(super) fn tool_hook(&self, phase: &str, name: &str, arguments: &Value, result: Option<&Value>) -> Value {
    use crate::relay::wire::{HubOp, HubReply};
    let op = HubOp::ToolHook { phase: phase.into(), name: name.into(), arguments: arguments.clone(), result: result.cloned() };
    let mut out = json!({});
    let _ = crate::relay::hub::call(&self.addr, &self.request(op), |r| {
      if let HubReply::Done(text) = r {
        out = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
      }
      false
    });
    out
  }

  /// The plugin tools in effect and the tool groups switched off; None when the hub cannot be reached
  pub(super) fn plugins(&self) -> Option<(Vec<Value>, Vec<String>)> {
    use crate::relay::wire::{HubOp, HubReply};
    let mut out = None;
    let _ = crate::relay::hub::call(&self.addr, &self.request(HubOp::Plugins), |r| {
      if let HubReply::Plugins { tools, off } = r {
        out = Some((tools, off));
      }
      false
    });
    out
  }

  /// `plugin_activate`: the hub freezes and checks the plugin, then waits for the user's card
  pub(super) fn plugin_activate(&self, params: &Value, progress: &mut dyn FnMut(Value)) -> Value {
    use crate::relay::wire::HubOp;
    let id = params.get("arguments").and_then(|a| a.get("id")).and_then(Value::as_str).map(str::trim).unwrap_or("");
    if id.is_empty() {
      return tool_error("Pass `id`: the plugin's folder name under ~/.acpira/plugins.".into());
    }
    let wait_secs = Some(self.wait_secs.load(std::sync::atomic::Ordering::Relaxed));
    self.streamed(HubOp::PluginActivate { id: id.to_owned(), wait_secs }, params, progress)
  }

  /// Note what a `tools/list` answer offered (`listing_print`)
  pub(super) fn served(&self, print: String) {
    *self.watch.state.lock() = WatchState { served: Some(print), notified: None };
  }

  /// Poll the roster for the rest of the process and send `notifications/tools/list_changed` when it no longer matches
  /// what the client last listed, so a persona added, edited or removed on the settings page reaches a running
  /// session's `ask_agent` (Claude Code 2.1.299 lists again on it, so does the built-in agent at its next turn); started
  /// once, at initialize. `send` writes one line to stdout and is false once it is closed
  pub(super) fn watch(&self, send: impl Fn(&Value) -> bool + Send + 'static) {
    if self.watch.started.swap(true, std::sync::atomic::Ordering::Relaxed) {
      return;
    }
    let relay = self.clone();
    std::thread::spawn(move || {
      let mut failures = 0;
      loop {
        std::thread::sleep(ROSTER_POLL);
        let Some(roster) = relay.roster() else {
          failures += 1;
          if failures >= ROSTER_POLL_FAILURES {
            return;
          }
          continue;
        };
        failures = 0;
        let print = listing_print(Some(&roster), relay.plugins().as_ref());
        // A listing already in flight may still be told once more; that costs the client one identical tools/list
        let notify = relay.watch.state.lock().poll(print);
        if notify && !send(&json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })) {
          return;
        }
      }
    });
  }

  pub(super) fn ask(&self, params: &Value, progress: &mut dyn FnMut(Value)) -> Value {
    use crate::relay::wire::{AskArgs, HubOp};
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
    let ask = AskArgs {
      agent: text("agent").unwrap_or_default(),
      prompt: text("prompt").unwrap_or_default(),
      title: text("title"),
      mode: text("mode"),
      thread: text("thread"),
      new_thread: args.get("new_thread").and_then(Value::as_bool),
      wait_secs: Some(self.wait_secs.load(std::sync::atomic::Ordering::Relaxed)),
      wait: args.get("wait").and_then(Value::as_bool),
      answers: args.get("answers").and_then(Value::as_array).map(|l| l.iter().filter_map(|a| a.as_str().map(str::to_owned)).collect()),
      changes: args.get("changes").and_then(Value::as_bool),
    };
    self.streamed(HubOp::Ask(ask), params, progress)
  }

  /// `ask_caller`: the question goes to the hub, which waits for the caller's (or the user's) answer
  pub(super) fn ask_caller(&self, params: &Value, progress: &mut dyn FnMut(Value)) -> Value {
    use crate::relay::wire::{AskCallerArgs, HubOp};
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let ask = AskCallerArgs {
      question: args.get("question").and_then(Value::as_str).unwrap_or("").to_owned(),
      options: args.get("options").and_then(Value::as_array).map(|l| l.iter().filter_map(|a| a.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
      wait_secs: Some(self.wait_secs.load(std::sync::atomic::Ordering::Relaxed)),
    };
    self.streamed(HubOp::AskCaller(ask), params, progress)
  }

  /// One request answered with progress lines (forwarded as MCP progress when the call has a token), then a final line
  fn streamed(&self, op: crate::relay::wire::HubOp, params: &Value, progress: &mut dyn FnMut(Value)) -> Value {
    use crate::relay::wire::HubReply;
    let token = params.get("_meta").and_then(|m| m.get("progressToken")).cloned();
    let mut n = 0u64;
    let mut end: Option<Value> = None;
    let r = crate::relay::hub::call(&self.addr, &self.request(op), |reply| match reply {
      HubReply::Progress(message) => {
        if let Some(t) = &token {
          n += 1;
          progress(json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": t, "progress": n, "message": message } }));
        }
        true
      }
      HubReply::Done(text) => {
        end = Some(json!({ "content": [{ "type": "text", "text": text }], "isError": false }));
        false
      }
      HubReply::Error(text) => {
        end = Some(tool_error(text));
        false
      }
      HubReply::Personas(_) | HubReply::Roster { .. } | HubReply::Features(_) | HubReply::Plugins { .. } => true,
    });
    match (end, r) {
      (Some(v), _) => v,
      (None, Err(e)) => tool_error(format!("Acpira could not be reached: {e}")),
      (None, Ok(())) => tool_error("Acpira closed the call without an answer.".into()),
    }
  }
}
