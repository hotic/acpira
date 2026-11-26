//! The usage page's index: one entry per agent turn ever seen, per session, kept in `<root>/usage-index.json`. The first
//! read after an update scans every saved record once (the backfill); later reads re-read only the records whose stamp
//! moved, the way history search revalidates its cache (`session_search.rs`). A turn stays in the index when its record
//! drops it (an edit or a retry replaced it, the session was deleted): its tokens were spent all the same.
//!
//! What counts: the turns a session ran itself. A fork's copied prefix is its source's (`forkedFrom.turnIndex`), so it
//! is left out. Subagent transcripts add their tokens, tool calls and edits under the parent session. Kimi Code and pi
//! report no per-turn tokens over ACP but log them locally, so their sessions take tokens from those logs instead
//! (Kimi's `wire.jsonl` `usage.record` lines of scope `turn`, pi's assistant `usage`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use acpira_shared::model_shapes::is_model_control;
use acpira_shared::models::{FUSION, parse_model_name};
use acpira_shared::time::ms_of_iso;
use acpira_shared::transcript::{SessionControls, TurnUsage};
use acpira_shared::usage::{UsageModel, UsageRow, UsageSession, UsageView};

use crate::acp::vendors::{kimi_failure, pi_usage};
use crate::store::data_dir::home_dir;
use crate::store::transcript_store::TranscriptStore;

pub const INDEX_FILE: &str = "usage-index.json";
/// Bumped when a digest would come out different for the same record: every session is read again
const VERSION: u32 = 1;
/// A turn clock running longer than this was never stopped (the engine went away mid-turn): not working time
const TURN_MAX_MS: i64 = 12 * 3600 * 1000;
const HOUR_MS: i64 = 3600 * 1000;

#[derive(Default, Serialize, Deserialize)]
struct IndexFile {
  version: u32,
  sessions: HashMap<String, SessionEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SessionEntry {
  /// The record's stamp when last read: modification time in ns, and the file's size or the database body's revision
  stamp: (Option<i64>, u64),
  agent: String,
  cwd: String,
  /// Every turn seen, by a key stable across reads (`turn_key`)
  turns: BTreeMap<String, TurnFact>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnFact {
  at: i64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name: Option<String>,
  #[serde(default)]
  tokens: u64,
  #[serde(default)]
  input: u64,
  #[serde(default)]
  output: u64,
  #[serde(default)]
  cached_read: u64,
  /// An agent turn (vendor log lines and subagent turns carry tokens only)
  #[serde(default)]
  turn: bool,
  #[serde(default)]
  prompt: bool,
  #[serde(default)]
  work_ms: u64,
  #[serde(default)]
  tools: u64,
  #[serde(default)]
  added: u64,
  #[serde(default)]
  removed: u64,
}

/// A record reduced to what the index reads; unknown roles and block types deserialize as `Other`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiteRecord {
  #[serde(default)]
  agent: String,
  #[serde(default)]
  cwd: String,
  #[serde(default)]
  acp_session_id: Option<String>,
  #[serde(default)]
  created_at: String,
  #[serde(default)]
  updated_at: String,
  #[serde(default)]
  turns: Vec<LiteTurn>,
  #[serde(default)]
  controls: SessionControls,
  #[serde(default)]
  forked_from: Option<LiteFork>,
  #[serde(default)]
  subagents: Option<Vec<LiteSubagent>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiteFork {
  #[serde(default, deserialize_with = "acpira_shared::num::lenient_i64")]
  turn_index: i64,
}

#[derive(Deserialize)]
struct LiteSubagent {
  #[serde(default)]
  id: String,
  #[serde(default)]
  model: Option<String>,
  #[serde(default)]
  turns: Vec<LiteTurn>,
}

#[derive(Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum LiteTurn {
  User {
    #[serde(default)]
    auto: Option<bool>,
    #[serde(default)]
    settings: Option<LiteSettings>,
  },
  Agent(Box<LiteAgentTurn>),
  #[serde(other)]
  Other,
}

#[derive(Deserialize)]
struct LiteSettings {
  #[serde(default)]
  config: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiteAgentTurn {
  #[serde(default)]
  blocks: Vec<LiteBlock>,
  #[serde(default)]
  usage: Option<TurnUsage>,
  #[serde(default)]
  started_at: Option<i64>,
  #[serde(default)]
  responded_at: Option<i64>,
  #[serde(default)]
  ended_at: Option<i64>,
  #[serde(default)]
  idle_ms: Option<i64>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LiteBlock {
  ToolCall {
    #[serde(default, rename = "diffStat")]
    diff_stat: Option<DiffStat>,
  },
  #[serde(other)]
  Other,
}

#[derive(Deserialize)]
struct DiffStat {
  #[serde(default)]
  add: u64,
  #[serde(default)]
  del: u64,
}

/// Where the vendor logs live: Kimi Code's home and pi's agent directory (a test points them at a temp folder)
#[derive(Clone)]
pub struct VendorDirs {
  pub kimi: PathBuf,
  pub pi_home: PathBuf,
  pub pi_agent: PathBuf,
}

impl VendorDirs {
  pub fn from_env() -> Self {
    Self { kimi: kimi_failure::home_of(std::env::var(kimi_failure::HOME_ENV).ok()), pi_home: home_dir(), pi_agent: pi_usage::agent_dir() }
  }
}

/// A session's turns as read now, before they join what the index already holds
struct Digest {
  agent: String,
  cwd: String,
  turns: Vec<(String, TurnFact)>,
}

/// The model a turn ran: the id the agent reported, else the one the user turn was sent with, else the session's current
/// pick; named like the picker names it (the family, without effort or speed), the id's last segment when no option does
fn model_of(controls: &SessionControls, reported: Option<&str>, sent: Option<&LiteSettings>) -> (Option<String>, Option<String>) {
  let control = controls.options.iter().find(|c| is_model_control(c));
  let id = reported
    .map(str::to_owned)
    .or_else(|| control.and_then(|c| sent?.config.get(&c.id).cloned()))
    .or_else(|| control.and_then(|c| c.value.clone()))
    .filter(|id| !id.is_empty());
  let Some(id) = id else { return (None, None) };
  let option = control.and_then(|c| acpira_shared::model_shapes::offered_model(c, &id).and_then(|o| c.options.iter().find(|x| x.id == o)));
  let name = match option {
    Some(o) => {
      let parsed = parse_model_name(&o.name);
      if parsed.family == FUSION || parsed.family.is_empty() { o.name.clone() } else { parsed.family }
    }
    None => id.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&id).to_owned(),
  };
  (Some(id), Some(name))
}

fn num(n: Option<acpira_shared::num::Num>) -> u64 {
  n.map_or(0.0, |n| n.get()).max(0.0) as u64
}

/// The turn's tokens the way its own card counts them: the whole prompt plus the output (`total` when it says more)
fn tokens_of(u: &TurnUsage) -> (u64, u64, u64, u64) {
  let mut u = u.clone();
  u.fold_cache_into_input();
  let (input, output) = (num(u.input), num(u.output));
  ((input + output).max(num(u.total)), input, output, num(u.cached_read))
}

fn tool_facts(blocks: &[LiteBlock]) -> (u64, u64, u64) {
  let (mut tools, mut added, mut removed) = (0, 0, 0);
  for b in blocks {
    if let LiteBlock::ToolCall { diff_stat } = b {
      tools += 1;
      if let Some(d) = diff_stat {
        added += d.add;
        removed += d.del;
      }
    }
  }
  (tools, added, removed)
}

/// Read one record. `vendor` fills Kimi's and pi's tokens from their own logs
fn digest(raw: &[u8], vendor: Option<&VendorDirs>) -> Option<Digest> {
  let r: LiteRecord = serde_json::from_slice(raw).ok()?;
  // A turn without a clock (records before `startedAt`) lands on the session's own time
  let fallback = ms_of_iso(&r.created_at).or_else(|| ms_of_iso(&r.updated_at)).unwrap_or(0);
  let copied = r.forked_from.as_ref().map_or(-1, |f| f.turn_index);
  let vendor_tokens = vendor.and_then(|dirs| vendor_facts(&r, dirs));
  let mut turns = vec![];
  let mut sent: Option<&LiteSettings> = None;
  let mut prompt = false;
  for (i, turn) in r.turns.iter().enumerate() {
    match turn {
      LiteTurn::User { auto, settings } => {
        sent = settings.as_ref();
        prompt = *auto != Some(true);
      }
      LiteTurn::Agent(a) => {
        let was_prompt = std::mem::take(&mut prompt);
        if (i as i64) <= copied {
          continue;
        }
        let at = a.started_at.or(a.ended_at).unwrap_or(fallback);
        let (model, name) = model_of(&r.controls, a.usage.as_ref().and_then(|u| u.model.as_deref()), sent);
        let (mut tokens, mut input, mut output, mut cached_read) = a.usage.as_ref().map(tokens_of).unwrap_or_default();
        // The vendor log is the tokens' source for these agents: a turn's own tally would count them twice
        if vendor_tokens.is_some() {
          (tokens, input, output, cached_read) = (0, 0, 0, 0);
        }
        let work_ms = match (a.responded_at.or(a.started_at), a.ended_at) {
          (Some(from), Some(to)) => {
            let ms = to - from - a.idle_ms.unwrap_or(0).max(0);
            if (0..=TURN_MAX_MS).contains(&ms) { ms as u64 } else { 0 }
          }
          _ => 0,
        };
        let (tools, added, removed) = tool_facts(&a.blocks);
        let key = a.started_at.map_or_else(|| format!("i{i}"), |t| format!("t{t}"));
        turns.push((key, TurnFact { at, model, name, tokens, input, output, cached_read, turn: true, prompt: was_prompt, work_ms, tools, added, removed }));
      }
      LiteTurn::Other => {}
    }
  }
  for sub in r.subagents.iter().flatten() {
    for (i, turn) in sub.turns.iter().enumerate() {
      let LiteTurn::Agent(a) = turn else { continue };
      let reported = a.usage.as_ref().and_then(|u| u.model.as_deref()).or(sub.model.as_deref());
      let (model, name) = match reported {
        Some(m) => model_of(&r.controls, Some(m), None),
        None => (None, None),
      };
      let (tokens, input, output, cached_read) = a.usage.as_ref().map(tokens_of).unwrap_or_default();
      let (tools, added, removed) = tool_facts(&a.blocks);
      if tokens == 0 && tools == 0 {
        continue;
      }
      let at = a.started_at.or(a.ended_at).unwrap_or(fallback);
      let key = format!("s{}:{}", sub.id, a.started_at.map_or_else(|| format!("i{i}"), |t| t.to_string()));
      turns.push((key, TurnFact { at, model, name, tokens, input, output, cached_read, tools, added, removed, ..Default::default() }));
    }
  }
  turns.extend(vendor_tokens.unwrap_or_default());
  Some(Digest { agent: r.agent, cwd: r.cwd, turns })
}

/// Kimi's and pi's tokens from their own session logs; None for other agents, or when the log is not there
fn vendor_facts(r: &LiteRecord, dirs: &VendorDirs) -> Option<Vec<(String, TurnFact)>> {
  let id = r.acp_session_id.as_deref().filter(|s| !s.is_empty())?;
  let (path, kind) = match r.agent.as_str() {
    "kimi" => (kimi_failure::wire_file(&dirs.kimi, id)?, Vendor::Kimi),
    "pi" => (pi_usage::session_file(&dirs.pi_home, &dirs.pi_agent, &r.cwd, id)?, Vendor::Pi),
    _ => return None,
  };
  let text = std::fs::read_to_string(path).ok()?;
  Some(vendor_lines(&text, kind, &r.controls))
}

#[derive(Clone, Copy)]
enum Vendor {
  Kimi,
  Pi,
}

fn vendor_lines(text: &str, kind: Vendor, controls: &SessionControls) -> Vec<(String, TurnFact)> {
  let u = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u64;
  let mut out = vec![];
  for (n, line) in text.lines().enumerate() {
    // Cheap filter before parsing: most lines are messages and tool output
    if !line.contains("\"usage") {
      continue;
    }
    let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
    let fact = match kind {
      // Kimi logs a `turn` record per model call and `session` running totals; a subagent's calls (their own agentId)
      // are the user's spend too and count alike
      Vendor::Kimi => {
        if e.get("type").and_then(Value::as_str) != Some("usage.record") || e.get("usageScope").and_then(Value::as_str) != Some("turn") {
          continue;
        }
        let Some(usage) = e.get("usage") else { continue };
        let at = e.get("time").and_then(Value::as_i64).unwrap_or(0);
        let cached_read = u(usage, "inputCacheRead");
        let input = u(usage, "inputOther") + cached_read + u(usage, "inputCacheCreation");
        let output = u(usage, "output");
        let (model, name) = model_of(controls, e.get("model").and_then(Value::as_str), None);
        TurnFact { at, model, name, tokens: input + output, input, output, cached_read, ..Default::default() }
      }
      Vendor::Pi => {
        let Some(m) = e.get("message").filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant")) else { continue };
        let Some(usage) = m.get("usage") else { continue };
        let at = m.get("timestamp").and_then(Value::as_i64).or_else(|| e.get("timestamp").and_then(Value::as_str).and_then(ms_of_iso)).unwrap_or(0);
        let cached_read = u(usage, "cacheRead");
        let input = u(usage, "input") + cached_read + u(usage, "cacheWrite");
        let output = u(usage, "output");
        let tokens = (input + output).max(u(usage, "totalTokens"));
        let reported = match (m.get("provider").and_then(Value::as_str), m.get("model").and_then(Value::as_str)) {
          (Some(p), Some(model)) => Some(format!("{p}/{model}")),
          (None, Some(model)) => Some(model.to_owned()),
          _ => None,
        };
        let (model, name) = model_of(controls, reported.as_deref(), None);
        TurnFact { at, model, name, tokens, input, output, cached_read, ..Default::default() }
      }
    };
    if fact.tokens > 0 {
      out.push((format!("v{}:{n}", fact.at), fact));
    }
  }
  out
}

fn stamp_of((time, size): (Option<SystemTime>, u64)) -> (Option<i64>, u64) {
  (time.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as i64), size)
}

/// Fold what the index holds into the page's view: one row per session, model and clock hour
fn view_of(index: &IndexFile) -> UsageView {
  let mut view = UsageView::default();
  let mut models: HashMap<(String, String), u32> = HashMap::new();
  let mut rows: BTreeMap<(i64, u32, Option<u32>), UsageRow> = BTreeMap::new();
  let mut ids: Vec<&String> = index.sessions.keys().collect();
  ids.sort();
  for id in ids {
    let entry = &index.sessions[id];
    if entry.turns.is_empty() {
      continue;
    }
    let session = view.sessions.len() as u32;
    view.sessions.push(UsageSession { agent: entry.agent.clone(), cwd: entry.cwd.clone() });
    for fact in entry.turns.values() {
      let model = fact.model.as_ref().map(|m| {
        let name = fact.name.clone().unwrap_or_else(|| m.clone());
        let next = models.len() as u32;
        *models.entry((m.clone(), name.clone())).or_insert_with(|| {
          view.models.push(UsageModel { id: m.clone(), name });
          next
        })
      });
      let at = fact.at.div_euclid(HOUR_MS) * HOUR_MS;
      let row = rows.entry((at, session, model)).or_insert_with(|| UsageRow { at, session, model, ..Default::default() });
      row.tokens += fact.tokens;
      row.input += fact.input;
      row.output += fact.output;
      row.cached_read += fact.cached_read;
      row.turns += u64::from(fact.turn);
      row.prompts += u64::from(fact.prompt);
      row.work_ms += fact.work_ms;
      row.longest_ms = row.longest_ms.max(fact.work_ms);
      row.tools += fact.tools;
      row.added += fact.added;
      row.removed += fact.removed;
    }
  }
  let mut rows: Vec<UsageRow> = rows.into_values().collect();
  rows.sort_by_key(|r| r.at);
  view.rows = rows;
  view
}

/// One per manager: the index stays in memory once loaded and is written back after a read that changed it
pub struct UsageIndex {
  path: PathBuf,
  vendor: VendorDirs,
  index: Arc<Mutex<Option<IndexFile>>>,
  refresh: tokio::sync::Mutex<()>,
}

impl UsageIndex {
  pub fn new(root: &Path, vendor: VendorDirs) -> Self {
    Self { path: root.join(INDEX_FILE), vendor, index: Default::default(), refresh: Default::default() }
  }

  /// Bring the index in line with the store and fold it for the page. Reads take turns, so the page opened twice
  /// while the backfill runs scans once
  pub async fn view(&self, store: Arc<TranscriptStore>) -> UsageView {
    let _refresh = self.refresh.lock().await;
    let sources = store.search_sources().await;
    let (index, path, vendor) = (self.index.clone(), self.path.clone(), self.vendor.clone());
    tokio::task::spawn_blocking(move || {
      let mut slot = index.lock().unwrap_or_else(|e| e.into_inner());
      let index = slot.get_or_insert_with(|| load(&path));
      let mut changed = false;
      for source in sources {
        let stamp = stamp_of(source.stamp);
        if index.sessions.get(&source.id).is_some_and(|e| e.stamp == stamp) {
          continue;
        }
        let raw = if source.in_database { store.body_text(&source.id, false) } else { store.file_text(&source.id) };
        let Some(d) = raw.and_then(|raw| digest(&raw, Some(&vendor))) else { continue };
        let entry = index.sessions.entry(source.id).or_insert_with(|| SessionEntry { stamp, agent: String::new(), cwd: String::new(), turns: BTreeMap::new() });
        entry.stamp = stamp;
        entry.agent = d.agent;
        entry.cwd = d.cwd;
        // Turns the record still holds are read again (a live turn grows); the ones it dropped stay as they were
        entry.turns.extend(d.turns);
        changed = true;
      }
      if changed {
        save(&path, index);
      }
      view_of(index)
    })
    .await
    .unwrap_or_default()
  }
}

/// The index on disk; an unreadable file or one from another version starts over (the next read scans everything)
fn load(path: &Path) -> IndexFile {
  std::fs::read(path)
    .ok()
    .and_then(|raw| serde_json::from_slice::<IndexFile>(&raw).ok())
    .filter(|f| f.version == VERSION)
    .unwrap_or(IndexFile { version: VERSION, sessions: HashMap::new() })
}

/// tmp + rename, like every file under the shared data root
fn save(path: &Path, index: &IndexFile) {
  let Ok(raw) = serde_json::to_vec(index) else { return };
  let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
  if std::fs::write(&tmp, raw).is_ok() && std::fs::rename(&tmp, path).is_err() {
    let _ = std::fs::remove_file(&tmp);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn read(raw: &str) -> Digest {
    digest(raw.as_bytes(), None).unwrap()
  }

  const CONTROLS: &str = r#"{"options":[{"id":"model","name":"Model","category":"model","value":"opus",
    "options":[{"id":"opus","name":"Opus 5.5 (High)"},{"id":"sonnet","name":"Sonnet 5.5"}]}]}"#;

  #[test]
  fn turns_carry_tokens_work_tools_and_the_model_they_were_sent_with() {
    let d = read(&format!(
      r#"{{"agent":"claude","cwd":"/p","createdAt":"2026-11-01T00:00:00.000Z","controls":{CONTROLS},"turns":[
        {{"role":"user","text":"hi","settings":{{"config":{{"model":"sonnet"}}}}}},
        {{"role":"agent","startedAt":1000,"respondedAt":3000,"endedAt":63000,"idleMs":10000,
          "usage":{{"input":100,"cachedRead":900,"output":50}},
          "blocks":[{{"type":"tool_call","id":"1","kind":"edit","verb":"Edit","status":"completed","diffStat":{{"add":5,"del":2}}}},
                    {{"type":"text","markdown":"done"}}]}},
        {{"role":"user","text":"/compact","auto":true}},
        {{"role":"agent","blocks":[]}}]}}"#
    ));
    assert_eq!(d.turns.len(), 2);
    let (key, t) = &d.turns[0];
    assert_eq!(key, "t1000");
    // claude-agent-acp reports the uncached remainder: the cache folds into the prompt
    assert_eq!((t.tokens, t.input, t.output, t.cached_read), (1050, 1000, 50, 900));
    assert_eq!((t.model.as_deref(), t.name.as_deref()), (Some("sonnet"), Some("Sonnet 5.5")));
    assert!(t.prompt && t.turn);
    assert_eq!(t.work_ms, 50_000);
    assert_eq!((t.tools, t.added, t.removed), (1, 5, 2));
    // No clock: the session's own time and an index key; the session's current pick names its model, family only
    let (key, t) = &d.turns[1];
    assert_eq!(key, "i3");
    assert_eq!(t.at, ms_of_iso("2026-11-01T00:00:00.000Z").unwrap());
    assert_eq!(t.name.as_deref(), Some("Opus 5.5"));
    assert!(!t.prompt);
  }

  #[test]
  fn a_fork_leaves_its_copied_prefix_to_the_source() {
    let d = read(
      r#"{"agent":"claude","cwd":"/p","forkedFrom":{"sessionId":"s","turnIndex":1},"turns":[
        {"role":"user","text":"a"},{"role":"agent","startedAt":1,"blocks":[]},
        {"role":"user","text":"b"},{"role":"agent","startedAt":2,"blocks":[]}]}"#,
    );
    assert_eq!(d.turns.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["t2"]);
  }

  #[test]
  fn subagent_tokens_count_under_the_parent_without_a_turn() {
    let d = read(
      r#"{"agent":"claude","cwd":"/p","turns":[],"subagents":[{"id":"c1","model":"haiku","turns":[
        {"role":"agent","startedAt":5,"usage":{"input":10,"output":5},"blocks":[]}]}]}"#,
    );
    let (key, t) = &d.turns[0];
    assert_eq!(key, "sc1:5");
    assert_eq!((t.tokens, t.turn, t.name.as_deref()), (15, false, Some("haiku")));
  }

  #[test]
  fn kimi_and_pi_logs_give_their_tokens() {
    let controls: SessionControls = serde_json::from_str(
      r#"{"options":[{"id":"model","name":"Model","category":"model","value":"kimi-code/k3",
        "options":[{"id":"kimi-code/k3","name":"K3"},{"id":"asgard/gpt-6-astra","name":"6 Astra"}]}]}"#,
    )
    .unwrap();
    let kimi = [
      r#"{"type":"usage.record","agentId":"main","model":"kimi-code/k3","usage":{"inputOther":100,"output":20,"inputCacheRead":900,"inputCacheCreation":0},"usageScope":"turn","time":7000}"#,
      r#"{"type":"usage.record","agentId":"main","model":"kimi-code/k3","usage":{"inputOther":100,"output":20,"inputCacheRead":900,"inputCacheCreation":0},"usageScope":"session","time":7001}"#,
      r#"{"type":"message","text":"no usage here"}"#,
    ]
    .join("\n");
    let facts = vendor_lines(&kimi, Vendor::Kimi, &controls);
    assert_eq!(facts.len(), 1);
    assert_eq!((facts[0].1.tokens, facts[0].1.cached_read, facts[0].1.name.as_deref()), (1020, 900, Some("K3")));
    let pi = r#"{"type":"message","timestamp":"2026-11-20T07:37:36.886Z","message":{"role":"assistant","provider":"asgard","model":"gpt-6-astra","usage":{"input":4004,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":4014},"timestamp":1795160253230}}"#;
    let facts = vendor_lines(pi, Vendor::Pi, &controls);
    assert_eq!((facts[0].1.tokens, facts[0].1.at, facts[0].1.name.as_deref()), (4014, 1795160253230, Some("6 Astra")));
  }

  #[test]
  fn the_view_folds_hours_per_session_and_model() {
    let mut index = IndexFile { version: VERSION, sessions: HashMap::new() };
    let fact = |at: i64, tokens: u64, work_ms: u64| TurnFact { at, model: Some("m".into()), name: Some("M".into()), tokens, turn: true, work_ms, ..Default::default() };
    let turns = BTreeMap::from([("a".to_owned(), fact(HOUR_MS + 5, 10, 100)), ("b".to_owned(), fact(HOUR_MS + 9, 5, 300)), ("c".to_owned(), fact(3 * HOUR_MS, 1, 0))]);
    index.sessions.insert("s".into(), SessionEntry { stamp: (None, 0), agent: "claude".into(), cwd: "/p".into(), turns });
    let view = view_of(&index);
    assert_eq!(view.models, vec![UsageModel { id: "m".into(), name: "M".into() }]);
    assert_eq!(view.rows.len(), 2);
    assert_eq!((view.rows[0].at, view.rows[0].tokens, view.rows[0].turns, view.rows[0].work_ms, view.rows[0].longest_ms), (HOUR_MS, 15, 2, 400, 300));
  }
}
