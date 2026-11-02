//! The hooks a session asks the plugins' resident processes about, whichever agent it drives (docs/dev/plugins.md,
//! Hooks): `prompt` (rewrite, add context, block or send elsewhere), `permission` (deny or insist on a card), `retry`
//! (try a failed turn again), `title`, `session.mcp` (the MCP servers a session gets) and `agent.launch` (the
//! command line and environment an agent starts with), `transcript` (split a settled thought or reply text into thought
//! and reply parts). Each call site checks `Runtimes::has_hook` first, so a session
//! no plugin hooks into pays one lock read
//!
//! No answer can loosen what the user decides: a permission hook may deny or ask for a card, never approve, and a launch
//! hook never sees the account layer's variables

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_shared::slash_commands::command_name;
use acpira_shared::transcript::{AgentBlock, Attachment, CommandInput, CommandMenuItem, Draft, SlashCommand, StrMap, Turn, TurnStop};

use super::{AcpSession, Core};
use super::attachments::encode_uri_component;
use super::prompt::Origin;
use crate::acp::agents::registry::AgentDef;
use crate::acp::transcript::normalize::reclassify::{Prose, parse_parts, parts_json, reclassify, same_words};
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
  /// The root transcript's thought or reply text now streaming, for the `transcript` hook once it settles
  pub streaming: Option<Streaming>,
  /// What the user picked in plugin command buttons' menus, saved with the record
  pub menus: crate::store::record::PluginMenus,
}

/// Where a streaming thought or reply text sits: the turn (and its start, in case the turns were rebuilt) and the block
#[derive(Debug)]
pub(crate) struct Streaming {
  turn: usize,
  started: Option<i64>,
  index: usize,
  thought: bool,
}

fn streams(b: &AgentBlock) -> bool {
  match b {
    AgentBlock::Thought(t) => t.streaming == Some(true),
    AgentBlock::Text(t) => t.streaming == Some(true),
    _ => false,
  }
}

/// What followed a settled block, for the `transcript` hook: its type, and for a tool call its kind, target and whether
/// it asks the user (a question tool such as Claude's AskUserQuestion)
fn next_of(b: &AgentBlock) -> Value {
  match b {
    AgentBlock::ToolCall(tc) => {
      json!({ "type": "tool_call", "kind": tc.kind, "target": tc.target, "asksUser": tc.verb_key.as_deref() == Some("verb.ask") })
    }
    AgentBlock::Question(_) => json!({ "type": "question", "asksUser": true }),
    other => json!({ "type": serde_json::to_value(other).ok().and_then(|v| v.get("type").cloned()) }),
  }
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

/// A command button's menu as the page draws it, the session's picks checked, and the chip's badge: the label of the
/// first group's pick
fn command_menu(def: &crate::plugins::manifest::CommandDef, stored: Option<&serde_json::Map<String, Value>>) -> (Vec<CommandMenuItem>, Option<String>) {
  if def.menu.is_empty() {
    return (vec![], None);
  }
  let picks = def.menu_picks(stored);
  let items: Vec<CommandMenuItem> = def
    .menu
    .iter()
    .map(|i| CommandMenuItem {
      id: i.id.clone(),
      kind: i.kind.clone(),
      group: i.group.clone(),
      label: i.label.clone(),
      description: i.description.clone(),
      checked: match (i.kind.as_str(), i.group.as_deref()) {
        ("radio", Some(g)) => picks.get(g).and_then(Value::as_str) == Some(i.id.as_str()),
        ("toggle", _) => picks.get(&i.id).and_then(Value::as_bool) == Some(true),
        _ => false,
      },
    })
    .collect();
  let first = def.menu.iter().find_map(|i| (i.kind == "radio").then_some(i.group.as_deref()).flatten());
  let badge = first.and_then(|g| items.iter().find(|i| i.checked && i.group.as_deref() == Some(g))).map(|i| i.label.clone());
  (items, badge)
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
      if let Some(menus) = self.plugin_menu_picks() {
        o.insert("menus".into(), menus);
      }
    }
    data
  }

  /// Every plugin command menu's picks in this session, defaults filled in, by command name; None when no plugin
  /// command has a menu. Takes the core lock, so never called with it held
  fn plugin_menu_picks(&self) -> Option<Value> {
    let contributed = self.deps.plugin_runtime.as_ref()?.contributed();
    let mut menus = contributed.commands.iter().filter(|c| !c.def.menu.is_empty()).peekable();
    menus.peek()?;
    let c = self.core.lock();
    Some(Value::Object(menus.map(|cmd| (cmd.def.name.clone(), Value::Object(cmd.def.menu_picks(c.plugin_state.menus.get(&cmd.def.name))))).collect()))
  }

  /// The user picked row `item` of command `name`'s menu: a radio or toggle is stored (and the view pushed) at once, then
  /// the plugin's `menu` hook hears `{ name, item, picks }`; an answer `{ picks }` replaces the stored ones (an action
  /// keeps nothing otherwise). A failed hook shows as a notice
  pub fn plugin_menu_pick(self: &Arc<Self>, name: &str, item: &str) {
    let Some(rt) = self.deps.plugin_runtime.clone() else { return };
    let contributed = rt.contributed();
    let Some(cmd) = contributed.commands.iter().find(|c| c.def.name == name && c.def.menu.iter().any(|i| i.id == item && i.kind != "separator")) else { return };
    let picks = {
      let mut c = self.core.lock();
      let current = cmd.def.menu_picks(c.plugin_state.menus.get(name));
      match cmd.def.menu_pick(&current, item) {
        Some(next) => {
          c.plugin_state.menus.insert(name.to_owned(), next.clone());
          self.touch(&mut c);
          next
        }
        None => current,
      }
    };
    let (plugin, def, me, item) = (cmd.plugin.clone(), cmd.def.clone(), self.clone(), item.to_owned());
    tokio::spawn(async move {
      let payload = me.plugin_payload(json!({ "name": def.name, "item": item, "picks": picks }));
      match rt.hook_of(&plugin, "menu", payload).await {
        None | Some(Ok(Value::Null)) => {}
        Some(Ok(answer)) => me.apply_menu_answer(&def, &answer),
        Some(Err(e)) => me.notify(&format!("{plugin}: {e}")),
      }
    });
  }

  /// An answer's `picks` (from the `menu` or `command` hook) for command `def`'s menu: what the menu has replaces the
  /// stored picks, the rest is dropped
  fn apply_menu_answer(&self, def: &crate::plugins::manifest::CommandDef, answer: &Value) {
    let Some(picks) = answer.get("picks").and_then(Value::as_object) else { return };
    if def.menu.is_empty() {
      return;
    }
    let mut c = self.core.lock();
    let mut next = def.menu_picks(c.plugin_state.menus.get(&def.name));
    let valid = def.menu_picks(Some(picks));
    for (k, v) in picks {
      // Kept only where the menu has that group or toggle and the value survived validation
      if valid.get(k) == Some(v) {
        next.insert(k.clone(), v.clone());
      }
    }
    if c.plugin_state.menus.get(&def.name) != Some(&next) {
      c.plugin_state.menus.insert(def.name.clone(), next);
      self.touch(&mut c);
    }
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
  /// no plugin adds one. `menus`: the session's picks, checked in each button's menu
  pub(crate) fn plugin_commands(&self, own: &[SlashCommand], menus: &crate::store::record::PluginMenus) -> Option<Vec<SlashCommand>> {
    let contributed = self.deps.plugin_runtime.as_ref()?.contributed();
    let extra: Vec<SlashCommand> = contributed
      .commands
      .iter()
      .filter(|c| !own.iter().any(|o| o.name == c.def.name))
      .map(|c| {
        let (menu, badge) = command_menu(&c.def, menus.get(&c.def.name));
        SlashCommand {
          name: c.def.name.clone(),
          description: c.def.description.clone(),
          input: c.def.hint.clone().map(|hint| CommandInput { hint }),
          plugin: Some(c.plugin.clone()),
          button: c.def.button.clone(),
          draft: c.def.draft,
          icon: c.def.icon.clone(),
          look: c.def.look.clone(),
          takes_draft: c.def.button.is_some() && c.def.menu.is_empty() && c.def.takes_draft(),
          menu,
          badge,
        }
      })
      .collect();
    if extra.is_empty() {
      return None;
    }
    let mut all = own.to_vec();
    all.extend(extra);
    Some(all)
  }

  /// Whether `text` runs a plugin's command: `/name` that a plugin contributes and the agent's own `commands` lack
  pub(crate) fn runs_plugin_command(&self, commands: &[SlashCommand], text: &str) -> bool {
    let (Some(rt), Some(name)) = (self.deps.plugin_runtime.as_ref(), command_name(text)) else { return false };
    !commands.iter().any(|c| c.name == name) && rt.contributed().commands.iter().any(|c| c.def.name == name)
  }

  /// `/name …` naming a plugin's command (and none of the agent's): Some(Ok(Some(text))) is the prompt to send in its
  /// place, Some(Ok(None)) its process answered without one, Some(Err) the reason nothing was sent
  async fn plugin_command(&self, text: &str) -> Option<Result<Option<String>, String>> {
    let name = command_name(text)?;
    let args = text.get(name.len() + 1..).unwrap_or_default().trim();
    self.run_plugin_command(name, args, false).await
  }

  /// A plugin's command `name` (one the agent has none of) run with `args`: its template filled in, or its process's
  /// answer. `draft`: run from its composer button for text that replaces the draft, which the hook is told
  pub async fn run_plugin_command(&self, name: &str, args: &str, draft: bool) -> Option<Result<Option<String>, String>> {
    let rt = self.deps.plugin_runtime.clone()?;
    if self.core.lock().state.commands.iter().any(|c| c.name == name) {
      return None;
    }
    let contributed = rt.contributed();
    let cmd = contributed.commands.iter().find(|c| c.def.name == name)?;
    if let Some(template) = cmd.def.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
      return Some(Ok(Some(expand(template, args))));
    }
    let mut call = json!({ "name": name, "arguments": args });
    if draft {
      call["draft"] = json!(true);
    }
    let payload = self.plugin_payload(call);
    Some(match rt.hook_of(&cmd.plugin, "command", payload).await {
      None => Err(format!("{}: /{name} needs its process, which is not running", cmd.plugin)),
      Some(Err(e)) => Err(format!("{}: {e}", cmd.plugin)),
      Some(Ok(answer)) => {
        // `/name off` typed by hand may move the button's menu too
        self.apply_menu_answer(&cmd.def, &answer);
        Ok(answer.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(clipped))
      }
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

  /// `transcript`: a thought or reply text of the root transcript settled, because the next block began or the turn
  /// ended. The payload `{ blocks: [{ type, text }], next, ended }` says what followed it (`next_of`, null at the turn's
  /// end); an answer `{ blocks }` splits it into thought and reply parts holding the same words (`reclassify.rs`), and
  /// anything else is ignored. Called after every streamed update and when the turn ends, never during a replay
  pub(crate) fn plugin_settled(&self, c: &mut Core, ended: bool) {
    if c.replaying {
      return;
    }
    let Some(rt) = self.deps.plugin_runtime.clone().filter(|rt| rt.has_hook("transcript")) else {
      c.plugin_state.streaming = None;
      return;
    };
    if let Some(w) = c.plugin_state.streaming.take() {
      let at = match c.state.turns.get(w.turn) {
        Some(Turn::Agent(t)) if t.started_at == w.started => t.blocks.get(w.index).map(|b| (b, t.blocks.get(w.index + 1))),
        _ => None,
      };
      match at {
        Some((b, _)) if streams(b) => c.plugin_state.streaming = Some(w),
        Some((b, next)) => {
          if let Some(p) = Prose::of(b).filter(|p| p.thought == w.thought) {
            let next = next.map(next_of).unwrap_or(Value::Null);
            self.offer_settled(rt, w.turn, w.started, p, next, ended);
          }
        }
        None => {}
      }
    }
    if c.plugin_state.streaming.is_none()
      && !ended
      && let Some(Turn::Agent(t)) = c.state.turns.last()
      && let Some(index) = t.blocks.iter().rposition(streams)
    {
      let thought = matches!(t.blocks[index], AgentBlock::Thought(_));
      c.plugin_state.streaming = Some(Streaming { turn: c.state.turns.len() - 1, started: t.started_at, index, thought });
    }
  }

  fn offer_settled(&self, rt: Arc<crate::plugins::runtime::Runtimes>, turn: usize, started: Option<i64>, original: Prose, next: Value, ended: bool) {
    let me = self.arc();
    tokio::spawn(async move {
      let payload = me.plugin_payload(json!({ "blocks": [original.to_json()], "next": next, "ended": ended }));
      let out = rt
        .chain("transcript", payload, |plugin, answer, p| {
          let Some(answer) = answer.get("blocks") else { return true };
          let current = parse_parts(&p["blocks"]).unwrap_or_default();
          match parse_parts(answer) {
            Some(parts) if same_words(&current, &parts) => p["blocks"] = parts_json(&parts),
            _ => me.log(&format!("plugin {plugin}: transcript answer ignored, it may only split and retype the same words")),
          }
          true
        })
        .await;
      let Some(parts) = parse_parts(&out["blocks"]).filter(|parts| parts.as_slice() != std::slice::from_ref(&original)) else { return };
      let mut c = me.core.lock();
      // The turn may be gone (a fork, a reload) or the block changed meanwhile: then nothing is replaced
      if !matches!(c.state.turns.get(turn), Some(Turn::Agent(t)) if t.started_at == started) || !reclassify(&mut c.state, turn, &original, &parts) {
        return;
      }
      me.touch(&mut c);
      drop(c);
      me.log(&format!("plugins: a settled {} became {} parts", if original.thought { "thought" } else { "reply text" }, parts.len()));
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
