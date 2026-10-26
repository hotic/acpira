//! The hooks a session asks the plugins' resident processes about, whichever agent it drives (docs/dev/plugins.md,
//! Hooks): `prompt` (rewrite, add context, block or send elsewhere), `permission` (deny or insist on a card), `retry`
//! (try a failed turn again), `title`, `session.mcp` (the MCP servers a session gets) and `agent.launch` (the
//! command line and environment an agent starts with). Each call site checks `Runtimes::has_hook` first, so a session
//! no plugin hooks into pays one lock read
//!
//! No answer can loosen what the user decides: a permission hook may deny or ask for a card, never approve, and a launch
//! hook never sees the account layer's variables

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_shared::slash_commands::command_name;
use acpira_shared::transcript::{Attachment, CommandInput, Draft, SlashCommand, StrMap, Turn, TurnStop};

use super::AcpSession;
use super::attachments::encode_uri_component;
use super::prompt::Origin;
use crate::acp::agents::registry::AgentDef;
use crate::acp::transport::rpc::BoxFuture;

/// A prompt a plugin's `prompt` hook sent to another agent: the engine starts a session for it in the same folder
pub struct Redirect {
  pub from: String,
  pub plugin: String,
  pub agent: String,
  pub cwd: String,
  pub text: String,
  pub drafts: Vec<Draft>,
}

/// Start the redirected prompt's session (the session manager's); the new session's id
pub type RedirectFn = Arc<dyn Fn(Redirect) -> BoxFuture<Result<String, String>> + Send + Sync>;

/// Failed turns one prompt of the user's may be retried by plugins, and the longest wait before one
pub const RETRY_MAX: u32 = 5;
pub const RETRY_DELAY_MAX: Duration = Duration::from_secs(60);
const RETRY_DELAY_DEFAULT: Duration = Duration::from_secs(2);
/// Longest text a plugin may make of a prompt, a context or a title
const TEXT_MAX: usize = 256 * 1024;

#[derive(Debug, Default)]
pub(crate) struct PluginState {
  /// The prompt now staging was sent here by another session's plugin: it is not sent on again
  pub redirected: bool,
  /// Plugin retries since the user's last prompt
  pub retries: u32,
}

/// What the `prompt` hooks made of a prompt
#[derive(Debug, Default)]
pub(crate) struct PromptOutcome {
  /// The text the agent gets instead of the user's (the bubble keeps what the user wrote)
  pub text: Option<String>,
  /// (plugin, agent): send the prompt there instead
  pub redirect: Option<(String, String)>,
  /// A plugin's command its process answered without a prompt: nothing goes out
  pub dropped: bool,
}

/// How the `permission` hooks judged a request
#[derive(Debug, PartialEq)]
pub(crate) enum PermVerdict {
  Pass,
  /// Show the card even under yolo or a plan's auto-approval
  Ask,
  /// (plugin, reason): rejected without a card
  Deny(String, String),
}

fn clipped(s: &str) -> String {
  if s.len() <= TEXT_MAX {
    return s.to_owned();
  }
  let mut end = TEXT_MAX;
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  s.get(..end).unwrap_or_default().to_owned()
}

/// A command's template with `$ARGUMENTS` replaced, or the arguments after it when the template has no placeholder
fn expand(template: &str, args: &str) -> String {
  if template.contains("$ARGUMENTS") {
    template.replace("$ARGUMENTS", args)
  } else if args.is_empty() {
    template.to_owned()
  } else {
    format!("{template}\n\n{args}")
  }
}

/// The plugins' skills for an agent that does not load them itself: what each is for and where its instructions are
fn skills_block(dirs: &[std::path::PathBuf]) -> Option<Value> {
  let skills = acpira_agent::skills::scan_dirs(dirs);
  if skills.is_empty() {
    return None;
  }
  let mut text = String::from("<plugin-skills>\nSkills from Acpira's plugins. When a task matches one, read its SKILL.md first and follow it.\n");
  for s in &skills {
    text.push_str(&format!("- {}: {} ({})\n", s.name, s.description, s.path.display()));
  }
  text.push_str("</plugin-skills>");
  Some(json!({ "type": "text", "text": text }))
}

fn text_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
  v.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

impl AcpSession {
  fn plugin_payload(&self, mut data: Value) -> Value {
    if let Some(o) = data.as_object_mut() {
      o.insert("session".into(), json!(self.id));
      o.insert("agent".into(), json!(self.agent));
      o.insert("cwd".into(), json!(self.cwd));
    }
    data
  }

  /// `prompt`: asked about every prompt the user (or their goal) sends, after the workspace's `beforePrompt`. An answer
  /// `{ text?, context?, attachments?, block?, redirect? }`: `text` replaces what the agent reads, `context` and
  /// `attachments` (`[{ name, text }]`) ride behind it, `block` keeps the prompt back with that reason, `redirect`
  /// (an agent id) sends it to a new session of that agent. Err(reason): blocked
  pub(crate) async fn plugin_prompt(&self, origin: Origin, user_text: &str, attachments: &[Attachment], first: bool, tail: &mut Vec<Value>) -> Result<PromptOutcome, String> {
    let redirected = std::mem::take(&mut self.core.lock().plugin_state.redirected);
    if !matches!(origin, Origin::User | Origin::Goal) {
      return Ok(PromptOutcome::default());
    }
    let Some(rt) = self.deps.plugin_runtime.clone() else { return Ok(PromptOutcome::default()) };
    // A plugin's slash command: its prompt goes out in the command's place (the bubble keeps `/name …`)
    let expanded = match self.plugin_command(user_text).await {
      Some(Ok(Some(t))) => Some(t),
      Some(Ok(None)) => return Ok(PromptOutcome { dropped: true, ..Default::default() }),
      Some(Err(reason)) => return Err(reason),
      None => None,
    };
    let text = expanded.as_deref().unwrap_or(user_text);
    let command = command_name(text).is_some();
    // The plugins' skills, offered once to an agent that does not load them itself
    if first
      && !command
      && self.agent != crate::acp::agents::registry::SELF_AGENT_ID
      && let Some(block) = skills_block(&rt.contributed().skills)
    {
      tail.push(block);
    }
    if !rt.has_hook("prompt") {
      return Ok(PromptOutcome { text: expanded, ..Default::default() });
    }
    let embedded = self.caps(&self.core.lock()).embedded_context;
    let payload = self.plugin_payload(json!({
      "origin": if origin == Origin::Goal { "goal" } else { "user" },
      "text": text,
      "attachments": attachments,
      "first": first,
      "command": command_name(text),
      "redirected": redirected,
    }));
    let mut blocked = None;
    let mut redirect = None;
    let out = rt
      .chain("prompt", payload, |plugin, answer, p| {
        if let Some(reason) = text_of(&answer, "block") {
          blocked = Some(format!("{plugin}: {}", clipped(reason)));
          return false;
        }
        if let Some(agent) = text_of(&answer, "redirect") {
          if redirected {
            self.log(&format!("plugin {plugin}: redirect to {agent} ignored, the prompt was redirected here"));
          } else if agent != self.agent {
            redirect = Some((plugin.to_owned(), agent.to_owned()));
            return false;
          }
        }
        if let Some(t) = answer.get("text").and_then(Value::as_str) {
          p["text"] = json!(clipped(t));
        }
        let mut extra = vec![];
        if let Some(context) = text_of(&answer, "context") {
          extra.push(json!({ "type": "text", "text": format!("<plugin-context plugin=\"{plugin}\">\n{}\n</plugin-context>", clipped(context)) }));
        }
        for a in answer.get("attachments").and_then(Value::as_array).into_iter().flatten() {
          let (Some(name), Some(body)) = (text_of(a, "name"), a.get("text").and_then(Value::as_str)) else { continue };
          let body = clipped(body);
          extra.push(if embedded {
            let uri = format!("attachment:///{}/{}", encode_uri_component(plugin), encode_uri_component(name));
            json!({ "type": "resource", "resource": { "uri": uri, "mimeType": "text/plain", "text": body } })
          } else {
            json!({ "type": "text", "text": format!("[Attachment: {name}]\n{body}\n[End of attachment: {name}]") })
          });
        }
        // A slash command must stay the prompt's only text, or the agent no longer reads it as one
        if command && !extra.is_empty() {
          self.log(&format!("plugin {plugin}: prompt context left out of a slash command"));
        } else {
          tail.extend(extra);
        }
        true
      })
      .await;
    if let Some(reason) = blocked {
      return Err(reason);
    }
    let rewritten = out.get("text").and_then(Value::as_str).filter(|t| *t != user_text).map(str::to_owned);
    if let Some(t) = &rewritten {
      self.log(&format!("plugins: prompt rewritten ({} → {} bytes)", user_text.len(), t.len()));
    }
    Ok(PromptOutcome { text: rewritten, redirect, dropped: false })
  }

  /// The menu's commands with the plugins' after the agent's own (an agent's command of the same name wins); None when
  /// no plugin adds one
  pub(crate) fn plugin_commands(&self, own: &[SlashCommand]) -> Option<Vec<SlashCommand>> {
    let contributed = self.deps.plugin_runtime.as_ref()?.contributed();
    let extra: Vec<SlashCommand> = contributed
      .commands
      .iter()
      .filter(|c| !own.iter().any(|o| o.name == c.def.name))
      .map(|c| SlashCommand { name: c.def.name.clone(), description: c.def.description.clone(), input: c.def.hint.clone().map(|hint| CommandInput { hint }) })
      .collect();
    if extra.is_empty() {
      return None;
    }
    let mut all = own.to_vec();
    all.extend(extra);
    Some(all)
  }

  /// `/name …` naming a plugin's command (and none of the agent's): Some(Ok(Some(text))) is the prompt to send in its
  /// place, Some(Ok(None)) its process answered without one, Some(Err) the reason nothing was sent
  async fn plugin_command(&self, text: &str) -> Option<Result<Option<String>, String>> {
    let rt = self.deps.plugin_runtime.clone()?;
    let name = command_name(text)?.to_owned();
    if self.core.lock().state.commands.iter().any(|c| c.name == name) {
      return None;
    }
    let contributed = rt.contributed();
    let cmd = contributed.commands.iter().find(|c| c.def.name == name)?;
    let args = text.get(name.len() + 1..).unwrap_or_default().trim();
    if let Some(template) = cmd.def.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
      return Some(Ok(Some(expand(template, args))));
    }
    let payload = self.plugin_payload(json!({ "name": name, "arguments": args }));
    Some(match rt.hook_of(&cmd.plugin, "command", payload).await {
      None => Err(format!("{}: /{name} needs its process, which is not running", cmd.plugin)),
      Some(Err(e)) => Err(format!("{}: {e}", cmd.plugin)),
      Some(Ok(answer)) => Ok(answer.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(clipped)),
    })
  }

  /// A plugin's command its process answered without a prompt: the claim is undone and the queue moves on
  pub(crate) fn plugin_dropped(self: &Arc<Self>) {
    let mut c = self.core.lock();
    c.phase.staging = false;
    c.phase.running = false;
    self.touch(&mut c);
    drop(c);
    self.log("plugins: command answered without a prompt");
    self.flush_queue();
  }

  /// This session's next prompt came from another session's plugin and is not sent on again
  pub fn mark_redirected(&self) {
    self.core.lock().plugin_state.redirected = true;
  }

  /// `permission`: asked about every permission request before yolo, a plan's auto-approval or the card. An answer
  /// `{ decision: "deny" | "ask", reason? }`; any other decision is no opinion, so a plugin can never approve
  pub(crate) async fn plugin_permission(&self, req: &Value, row: Option<Value>, auto: bool) -> PermVerdict {
    let Some(rt) = self.deps.plugin_runtime.clone().filter(|rt| rt.has_hook("permission")) else { return PermVerdict::Pass };
    let payload = self.plugin_payload(json!({
      "toolCall": req.get("toolCall").cloned().unwrap_or(Value::Null),
      "options": req.get("options").cloned().unwrap_or(Value::Null),
      "row": row,
      "autoApprove": auto,
    }));
    let mut verdict = PermVerdict::Pass;
    rt.chain("permission", payload, |plugin, answer, _| match answer.get("decision").and_then(Value::as_str) {
      Some("deny") => {
        let reason = text_of(&answer, "reason").map(clipped).unwrap_or_default();
        verdict = PermVerdict::Deny(plugin.to_owned(), reason);
        false
      }
      Some("ask") => {
        verdict = PermVerdict::Ask;
        true
      }
      _ => true,
    })
    .await;
    verdict
  }

  /// `retry`: a turn ended with an error. True when a plugin's process is asked (it then retries the turn after the
  /// wait it named, or lets the session go on as it would have); an answer `{ retry: true, delayMs? }`
  pub(crate) fn plugin_retry(self: &Arc<Self>, agent_idx: usize, started_at: i64, auto: bool) -> bool {
    let Some(rt) = self.deps.plugin_runtime.clone().filter(|rt| rt.has_hook("retry")) else { return false };
    let (error, attempt) = {
      let c = self.core.lock();
      let Some(Turn::Agent(a)) = c.state.turns.get(agent_idx) else { return false };
      if a.started_at != Some(started_at) || a.stop != Some(TurnStop::Error) {
        return false;
      }
      let Some(e) = a.error.clone() else { return false };
      (e, c.plugin_state.retries)
    };
    if attempt >= RETRY_MAX {
      self.log(&format!("plugins: retry limit ({RETRY_MAX}) reached"));
      return false;
    }
    let me = self.clone();
    tokio::spawn(async move {
      let payload = me.plugin_payload(json!({
        "error": { "message": error.message, "code": error.code, "kind": error.kind, "retryable": error.retryable },
        "attempt": attempt + 1,
        "max": RETRY_MAX,
      }));
      let mut wait = None;
      rt.chain("retry", payload, |plugin, answer, _| {
        if answer.get("retry").and_then(Value::as_bool) == Some(true) {
          let ms = answer.get("delayMs").and_then(Value::as_u64).map(Duration::from_millis).unwrap_or(RETRY_DELAY_DEFAULT);
          me.log(&format!("plugin {plugin}: retry in {} ms", ms.min(RETRY_DELAY_MAX).as_millis()));
          wait = Some(ms.min(RETRY_DELAY_MAX));
          return false;
        }
        true
      })
      .await;
      let Some(wait) = wait else {
        me.after_prompt(auto, TurnStop::Error);
        return;
      };
      tokio::time::sleep(wait).await;
      // Only the same failed turn, still the last one: the user may have moved on meanwhile
      let same = {
        let mut c = me.core.lock();
        let same = c.state.turns.len() == agent_idx + 1 && matches!(c.state.turns.get(agent_idx), Some(Turn::Agent(a)) if a.started_at == Some(started_at));
        if same {
          c.plugin_state.retries += 1;
        }
        same
      };
      if !same {
        return;
      }
      if let Err(e) = me.retry_turn().await {
        me.log(&format!("plugin retry failed: {e}"));
        me.after_prompt(auto, TurnStop::Error);
      }
    });
    true
  }

  /// A prompt of the user's opened a turn: plugin retries start over
  pub(crate) fn plugin_prompt_opened(&self, origin: Origin) {
    if origin == Origin::User {
      self.core.lock().plugin_state.retries = 0;
    }
  }

  /// `title`: the session was titled after the user's first prompt (`source: "prompt"`) or by the agent (`"agent"`).
  /// An answer `{ title }` replaces it, unless the title changed again meanwhile
  pub(crate) fn plugin_title(self: &Arc<Self>, title: String, source: &'static str, prompt: Option<String>) {
    let Some(rt) = self.deps.plugin_runtime.clone().filter(|rt| rt.has_hook("title")) else { return };
    let me = self.clone();
    tokio::spawn(async move {
      let payload = me.plugin_payload(json!({ "title": title, "source": source, "prompt": prompt }));
      let out = rt
        .chain("title", payload, |_, answer, p| {
          if let Some(t) = text_of(&answer, "title") {
            p["title"] = json!(crate::util::tidy_title(t, crate::limits::TITLE_MAX));
          }
          true
        })
        .await;
      let Some(next) = out.get("title").and_then(Value::as_str).filter(|t| !t.is_empty() && *t != title) else { return };
      let mut c = me.core.lock();
      if c.state.title.as_deref() != Some(title.as_str()) {
        return;
      }
      c.state.title = Some(next.to_owned());
      me.touch(&mut c);
      drop(c);
      me.log("plugins: session retitled");
      let next = next.to_owned();
      me.emit("session.renamed", || json!({ "title": next }));
    });
  }

  /// `tool.before` / `tool.after` around a call to one of Acpira's own MCP tools (`host_mcp.rs`, through the hub):
  /// before, an answer `{ arguments }` replaces the call's arguments and `{ block }` refuses it with that reason;
  /// after, `{ result }` replaces the MCP result. The answer the MCP server reads (`after`: whether to ask again once
  /// the call is done)
  pub async fn plugin_tool(&self, phase: &str, name: &str, arguments: Value, result: Option<Value>) -> Value {
    // The built-in agent asks its own `tool.before` / `tool.after` about every call, these included
    if self.agent == crate::acp::agents::registry::SELF_AGENT_ID {
      return json!({});
    }
    let Some(rt) = self.deps.plugin_runtime.clone() else { return json!({}) };
    let after = rt.has_hook("tool.after");
    if phase == "before" {
      if !rt.has_hook("tool.before") {
        return json!({ "after": after });
      }
      let mut block = None;
      let out = rt
        .chain("tool.before", self.plugin_payload(json!({ "tool": name, "arguments": arguments })), |plugin, answer, p| {
          if let Some(reason) = text_of(&answer, "block") {
            block = Some(format!("{plugin}: {}", clipped(reason)));
            return false;
          }
          if let Some(args) = answer.get("arguments").filter(|a| a.is_object()) {
            p["arguments"] = args.clone();
          }
          true
        })
        .await;
      return json!({ "arguments": out["arguments"], "block": block, "after": after });
    }
    if !after {
      return json!({});
    }
    let out = rt
      .chain("tool.after", self.plugin_payload(json!({ "tool": name, "arguments": arguments, "result": result })), |_, answer, p| {
        if let Some(r) = answer.get("result").filter(|r| r.get("content").is_some_and(Value::is_array)) {
          p["result"] = r.clone();
        }
        true
      })
      .await;
    json!({ "result": out["result"] })
  }

  /// The hooks into the built-in agent's own loop some plugin's process declares (`acpira_agent::plugin_hooks::NAMES`)
  pub(crate) fn plugin_loop_hooks(&self) -> Vec<String> {
    let Some(rt) = &self.deps.plugin_runtime else { return vec![] };
    rt.declared().into_iter().filter(|h| acpira_agent::plugin_hooks::NAMES.contains(&h.as_str())).collect()
  }

  /// One hook of the built-in agent's loop: each answer's fields replace the payload's (`session`, `agent` and `cwd`
  /// stay), `block` stops the chain with the plugin's name in front, and `by` names the last plugin that answered
  pub(crate) async fn plugin_loop_hook(&self, hook: &str, payload: Value) -> Value {
    let Some(rt) = self.deps.plugin_runtime.clone().filter(|rt| rt.has_hook(hook)) else { return payload };
    let mut out = rt
      .chain(hook, self.plugin_payload(payload), |plugin, answer, p| {
        if let Some(reason) = text_of(&answer, "block") {
          p["block"] = json!(format!("{plugin}: {}", clipped(reason)));
          return false;
        }
        let Some(fields) = answer.as_object() else { return true };
        for (k, v) in fields {
          match k.as_str() {
            "session" | "agent" | "cwd" | "by" => {}
            // Headers and variables add up across plugins: a later one replaces a name, never the whole set
            "headers" | "env" if v.is_object() && p[k].is_object() => {
              for (name, value) in v.as_object().into_iter().flatten() {
                p[k][name] = value.clone();
              }
            }
            _ => p[k] = v.clone(),
          }
        }
        p["by"] = json!(plugin);
        true
      })
      .await;
    if let Some(o) = out.as_object_mut() {
      for k in ["session", "agent", "cwd"] {
        o.remove(k);
      }
    }
    out
  }

  /// `session.mcp`: the MCP servers about to go out with session/new, load or resume. An answer `{ servers }` replaces
  /// the list; Acpira's own server stays whatever a plugin answers
  pub(crate) async fn plugin_mcp(&self, servers: Vec<Value>) -> Vec<Value> {
    let Some(rt) = self.deps.plugin_runtime.clone().filter(|rt| rt.has_hook("session.mcp")) else { return servers };
    let own: Vec<Value> = servers.iter().filter(|s| crate::host_mcp::is_host_entry(s)).cloned().collect();
    // The grant in our own entry's env stays in the engine
    let shown: Vec<Value> = servers.iter().filter(|s| !crate::host_mcp::is_host_entry(s)).cloned().collect();
    let out = rt
      .chain("session.mcp", self.plugin_payload(json!({ "servers": shown })), |plugin, answer, p| {
        match answer.get("servers").and_then(Value::as_array) {
          Some(list) => {
            let list: Vec<Value> = list.iter().filter(|s| s.get("name").and_then(Value::as_str).is_some_and(|n| !n.is_empty())).cloned().collect();
            self.log(&format!("plugin {plugin}: MCP servers now {}", list.len()));
            p["servers"] = json!(list);
          }
          None => self.log(&format!("plugin {plugin}: session.mcp answer without `servers` ignored")),
        }
        true
      })
      .await;
    let mut next: Vec<Value> = out.get("servers").and_then(Value::as_array).cloned().unwrap_or_default();
    next.retain(|s| !crate::host_mcp::is_host_entry(s) && s.get("name").and_then(Value::as_str) != Some(crate::host_mcp::SERVER_NAME));
    next.extend(own);
    next
  }
}

/// `agent.launch`: an agent process is about to start for a session. The payload names the agent, its command, args and
/// the definition's own env (the account layer's variables are never shown); an answer `{ args?, env? }` replaces the
/// args and sets (a string) or removes (null) variables. The definition as it will start
pub(crate) async fn plugin_launch(rt: Option<&Arc<crate::plugins::runtime::Runtimes>>, def: &AgentDef, session: &str, cwd: &str) -> AgentDef {
  let Some(rt) = rt.filter(|rt| rt.has_hook("agent.launch")) else { return def.clone() };
  let payload = json!({ "session": session, "agent": def.id, "cwd": cwd, "command": def.command, "args": def.args, "env": def.env });
  let out = rt
    .chain("agent.launch", payload, |_, answer, p| {
      if let Some(args) = answer.get("args").and_then(Value::as_array) {
        p["args"] = json!(args.iter().filter_map(Value::as_str).collect::<Vec<_>>());
      }
      if let Some(env) = answer.get("env").and_then(Value::as_object) {
        if !p["env"].is_object() {
          p["env"] = json!({});
        }
        for (k, v) in env {
          match v {
            Value::String(s) => p["env"][k] = json!(s),
            Value::Null => {
              p["env"].as_object_mut().map(|o| o.remove(k));
            }
            _ => {}
          }
        }
      }
      true
    })
    .await;
  let mut next = def.clone();
  if let Some(args) = out.get("args").and_then(Value::as_array) {
    next.args = args.iter().filter_map(Value::as_str).map(str::to_owned).collect();
  }
  next.env = out.get("env").and_then(|e| serde_json::from_value(e.clone()).ok()).filter(|e: &StrMap| !e.is_empty());
  next
}

impl AcpSession {
  /// A `prompt` hook sent the claimed prompt to `agent`: the claim is undone, a new session of that agent gets the
  /// prompt, and a toast says where it went (a failure shows the way a blocked prompt does)
  pub(crate) fn plugin_redirect(self: &Arc<Self>, text: String, drafts: Vec<Draft>, plugin: String, agent: String) {
    {
      let mut c = self.core.lock();
      c.phase.staging = false;
      c.phase.running = false;
      self.touch(&mut c);
    }
    let Some(f) = self.deps.plugin_runtime.as_ref().and_then(|rt| rt.redirect()) else {
      self.hooks_prompt_blocked(&text, &format!("{plugin}: nowhere to send the prompt"));
      return;
    };
    let me = self.clone();
    let r = Redirect { from: self.id.clone(), plugin: plugin.clone(), agent: agent.clone(), cwd: self.cwd.clone(), text: text.clone(), drafts };
    tokio::spawn(async move {
      match f(r).await {
        Ok(_) => {
          let name = me.deps.registry.get(&agent).map(|d| d.name.clone()).unwrap_or(agent);
          me.notify(&crate::i18n::tp("host.plugin.redirected", &[("name", &plugin), ("agent", &name)]));
          me.flush_queue();
        }
        Err(e) => me.hooks_prompt_blocked(&text, &format!("{plugin}: {e}")),
      }
    });
  }
}
