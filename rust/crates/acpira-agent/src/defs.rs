//! Shared leaf definitions for `acpira-agent`: wire/tool/mode constants, path helpers, URL checks, and pure text utilities.
//! Keeps foundational definitions below all runtime modules so the crate's internal module graph is strictly acyclic.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::llm::{Item, Part};

/// The context size reported when a model has none configured
pub const DEFAULT_CONTEXT: u64 = 128_000;

/// The value of `ProvidersFile::compaction` that picks chunk compaction (absent or anything else is summary)
pub const CHUNK_STRATEGY: &str = "chunk";

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const EDIT: &str = "edit";
pub const BASH: &str = "bash";
pub const JOB: &str = "job";
pub const GREP: &str = "grep";
pub const GLOB: &str = "glob";
pub const LIST: &str = "list";
pub const TODO: &str = "todo";
pub const SKILL: &str = "skill";
pub const EXIT_PLAN: &str = "exit_plan";
pub const WEBFETCH: &str = "webfetch";
pub const WEBSEARCH: &str = "websearch";
pub const QUESTION: &str = "question";
pub const HISTORY_GREP: &str = "history_grep";
pub const HISTORY_LIST: &str = "history_list";

pub const MODE_AGENT: &str = "agent";
pub const MODE_PLAN: &str = "plan";
pub const MODE_TEAM: &str = "team";
pub const MODE_RAW: &str = "raw";
pub const MODE_READ_ONLY: &str = "read-only";

pub const GOAL_METHOD: &str = "_session/goal";
/// The prompt the client sends to go on with an active goal (`initialize` names it as `acpira/continue`)
pub const GOAL_CONTINUE: &str = "/goal continue";
pub const GOAL_GET: &str = "get_goal";
pub const GOAL_CREATE: &str = "create_goal";
pub const GOAL_UPDATE: &str = "update_goal";
/// Continuation turns in a row that called no tool, after which the goal is blocked (Codex: three empty turns)
pub const GOAL_IDLE_TURNS: u32 = 3;

/// What a steered message's note closes with (`steer.rs`)
pub const STEER_CLOSE: &str = "\n</steer>";

pub(crate) const DEEPSEEK_URL: &str = "https://api.deepseek.com";

pub fn now_ms() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Where a data root keeps this agent's sessions: one `<id>.jsonl` log each, with an `<id>/` folder beside it
pub fn sessions_root(home: &Path) -> PathBuf {
  home.join("agent").join("sessions")
}

/// The files one session owns under `sessions_root`: its log and its folder
pub fn session_paths(home: &Path, id: &str) -> [PathBuf; 2] {
  let root = sessions_root(home);
  [root.join(format!("{id}.jsonl")), root.join(id)]
}

pub fn user_home() -> Option<PathBuf> {
  std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The project a folder belongs to: the nearest ancestor with `.git`; the home folder is never one
pub fn project_root(cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
  cwd.ancestors().find(|d| d.join(".git").exists()).filter(|r| Some(*r) != home).map(Path::to_path_buf)
}

/// The project's override folder, at its root
pub fn project_prompts(root: &Path) -> PathBuf {
  root.join(".acpira").join("prompts")
}

pub fn shell() -> (String, Vec<&'static str>) {
  #[cfg(windows)]
  {
    ("powershell.exe".into(), vec!["-NoProfile", "-NonInteractive", "-Command"])
  }
  #[cfg(not(windows))]
  {
    let sh = if Path::new("/bin/bash").exists() { "/bin/bash" } else { "/bin/sh" };
    (sh.into(), vec!["-c"])
  }
}

/// Where new scratch folders go (`ProvidersFile::scratch_place`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
  /// Under the data root, one folder per project
  Data,
  /// In the project's `.acpira/tmp`
  Project,
}

/// What the settings page set up for `websearch` (`Config::web_search`): the keyed backends, all absent when the
/// default (free) backend is picked
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchSetup {
  pub jina_key: Option<String>,
  pub deepseek: Option<DeepSeekSearch>,
  /// DeepSeek was the pick, so it goes before Jina
  pub deepseek_first: bool,
}

/// Where DeepSeek-shaped search goes: DeepSeek's own API, or a model source's address and key
#[derive(Debug, Clone, PartialEq)]
pub struct DeepSeekSearch {
  /// The full `…/v1/tools/search` endpoint
  pub url: String,
  pub key: String,
}

impl DeepSeekSearch {
  /// DeepSeek's own API with the search key
  pub fn official(key: String) -> Self {
    DeepSeekSearch { url: tools_search_url(DEEPSEEK_URL), key }
  }
}

/// The `/v1/tools/search` endpoint under an API root
pub fn tools_search_url(base: &str) -> String {
  let base = base.trim().trim_end_matches('/');
  let root = match base.find("/v1/") {
    Some(i) => base.get(..i + 3).unwrap_or(base),
    None => base,
  };
  if root.ends_with("/v1") { format!("{root}/tools/search") } else { format!("{root}/v1/tools/search") }
}

/// `scheme://authority` and the rest (path, query, fragment); None without `://` or a host
pub(crate) fn split_url(url: &str) -> Option<(&str, &str, &str)> {
  let (scheme, after) = url.split_once("://")?;
  if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) {
    return None;
  }
  let end = after.find(['/', '?', '#']).unwrap_or(after.len());
  let (authority, rest) = after.split_at_checked(end)?;
  (!host_of(authority).is_empty()).then_some((scheme, authority, rest))
}

/// The host of an authority, lowercase, without user info, port or IPv6 brackets
pub(crate) fn host_of(authority: &str) -> String {
  let a = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
  let host = if let Some(v6) = a.strip_prefix('[') {
    v6.split(']').next().unwrap_or("")
  } else {
    match a.rsplit_once(':') {
      Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
      _ => a,
    }
  };
  host.trim_end_matches('.').to_ascii_lowercase()
}

/// Whether the URL names this machine or a private network by itself
pub fn private_name(url: &str) -> bool {
  let Some((_, authority, _)) = split_url(url) else { return false };
  let host = host_of(authority);
  if let Ok(ip) = host.parse::<IpAddr>() {
    return private_ip(ip);
  }
  if host.split('.').next_back().is_some_and(|l| l.chars().all(|c| c.is_ascii_digit()) || l.starts_with("0x")) {
    return true;
  }
  !host.contains('.')
    || ["localhost", "local", "internal", "lan", "home", "corp", "intranet", "home.arpa"].iter().any(|s| host.ends_with(&format!(".{s}")))
}

pub(crate) fn private_ip(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => private_v4(v4),
    IpAddr::V6(v6) => {
      if let Some(v4) = v6.to_ipv4_mapped() {
        return private_v4(v4);
      }
      let first = v6.segments()[0];
      v6.is_loopback()
        || v6.is_unspecified()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || v6 == Ipv6Addr::LOCALHOST
    }
  }
}

fn private_v4(v4: Ipv4Addr) -> bool {
  let [a, b, ..] = v4.octets();
  v4.is_loopback()
    || v4.is_private()
    || v4.is_link_local()
    || v4.is_unspecified()
    || v4.is_broadcast()
    || (a == 100 && (64..128).contains(&b))
}

/// The goal tools, offered in Agent and Plan mode alike
pub fn goal_is_tool(name: &str) -> bool {
  matches!(name, GOAL_GET | GOAL_CREATE | GOAL_UPDATE)
}

/// The capability `initialize` advertises: codex-acp's, plus the prompt that continues an active goal
pub fn goal_capability() -> Value {
  json!({ "version": 1, "controlMethod": GOAL_METHOD, "actions": ["set", "pause", "resume", "clear"], "acpira/continue": GOAL_CONTINUE })
}

/// `/goal` in the session's slash commands
pub fn goal_slash_command() -> Value {
  json!({
    "name": "goal",
    "description": "Set a goal the agent keeps working toward across turns until it is achieved",
    "input": { "hint": "<objective> | pause | resume | clear" },
  })
}

/// The user's own words in a steered message (`Item::is_steer`): its text without the note (images left out)
pub fn steer_text_of(item: &Item) -> Option<String> {
  let Item::User(parts) = item else { return None };
  if !item.is_steer() {
    return None;
  }
  let text = parts
    .iter()
    .filter_map(|p| match p {
      Part::Text(t) => Some(t.as_str()),
      Part::Image { .. } => None,
    })
    .collect::<Vec<_>>()
    .join("\n\n");
  let words = text.split_once(STEER_CLOSE).map_or(text.as_str(), |(_, rest)| rest);
  Some(words.trim().to_owned())
}

/// A user message's text without the notes the agent put in front of it (mode, MCP servers, sidekick, goal)
pub(crate) fn strip_notes(mut t: &str) -> &str {
  const NOTES: [(&str, &str); 4] = [("<mode>\n", "</mode>"), ("<mcp>\n", "</mcp>"), ("<sidekick>\n", "</sidekick>"), ("<goal>\n", "</goal>")];
  while let Some(after) = NOTES.iter().find_map(|(open, close)| t.strip_prefix(open)?.split_once(close).map(|(_, a)| a)) {
    t = after.trim_start_matches('\n');
  }
  t
}

fn prompt_words(parts: &[Part]) -> String {
  let mut out = vec![];
  for p in parts {
    match p {
      Part::Text(t) => {
        let t = strip_notes(t);
        let t = t.split("\n\n<skill name=\"").next().unwrap_or(t);
        if !t.trim().is_empty() {
          out.push(t.to_owned());
        }
      }
      Part::Image { mime, .. } => out.push(format!("[Attached {mime}]")),
    }
  }
  out.join("\n\n")
}

/// The user's words in an item: a prompt's, or a message steered into the turn
pub(crate) fn user_words(item: &Item) -> Option<String> {
  if let Some(t) = steer_text_of(item) {
    return Some(t).filter(|t| !t.is_empty());
  }
  match item {
    Item::User(parts) => Some(prompt_words(parts)).filter(|t| !t.trim().is_empty()),
    _ => None,
  }
}

/// The prompt's text blocks, joined (attachments and images left out)
pub(crate) fn prompt_text(prompt: &Value) -> String {
  prompt
    .as_array()
    .into_iter()
    .flatten()
    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
    .filter_map(|b| b.get("text").and_then(Value::as_str))
    .collect::<Vec<_>>()
    .join("\n")
}

/// The prompt's content blocks as user message parts
pub fn prompt_parts(prompt: &Value, images: bool) -> Vec<Part> {
  let mut parts = vec![];
  for block in prompt.as_array().into_iter().flatten() {
    match block.get("type").and_then(Value::as_str) {
      Some("text") => {
        if let Some(t) = block.get("text").and_then(Value::as_str) {
          parts.push(Part::Text(t.to_owned()));
        }
      }
      Some("image") => {
        let data = block.get("data").and_then(Value::as_str).unwrap_or("");
        let mime = block.get("mimeType").and_then(Value::as_str).unwrap_or("image/png");
        if images && !data.is_empty() {
          parts.push(Part::Image { mime: mime.to_owned(), data: data.to_owned() });
        } else {
          parts.push(Part::Text("[An image was attached, but this model does not accept images.]".into()));
        }
      }
      Some("resource") => {
        let r = block.get("resource").unwrap_or(&Value::Null);
        let uri = r.get("uri").and_then(Value::as_str).unwrap_or("");
        match r.get("text").and_then(Value::as_str) {
          Some(text) => parts.push(Part::Text(format!("<attachment uri=\"{uri}\">\n{text}\n</attachment>"))),
          None => parts.push(Part::Text(format!("[Attached: {uri}]"))),
        }
      }
      Some("resource_link") => {
        let uri = block.get("uri").and_then(Value::as_str).unwrap_or("");
        let name = block.get("name").and_then(Value::as_str).unwrap_or(uri);
        parts.push(Part::Text(format!("[{name}]({uri})")));
      }
      _ => {}
    }
  }
  let mut merged = merge_text(parts);
  if merged.is_empty() {
    merged.push(Part::Text(String::new()));
  }
  merged
}

/// Adjacent text parts joined into one, as a message reads
pub(crate) fn merge_text(parts: Vec<Part>) -> Vec<Part> {
  let mut merged: Vec<Part> = vec![];
  for p in parts {
    match (merged.last_mut(), p) {
      (Some(Part::Text(prev)), Part::Text(t)) => {
        prev.push_str("\n\n");
        prev.push_str(&t);
      }
      (_, p) => merged.push(p),
    }
  }
  merged
}
