//! Plugin schedules (docs/dev/plugins.md, Schedules): `contributes.schedules[]` names a cron time, the engine starts the
//! run and the plugin's runtime answers it (`schedule/<id>`). A run is the engine's, not a timer in the plugin: every
//! engine on the machine runs the plugin's process, but `schedules.json` under the file lock lets exactly one of them
//! start each occurrence, a run missed while no engine ran is started once when one runs again (`catchUp`), and a run
//! never overlaps itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

/// The record of every schedule's runs, shared by the engines (`~/.acpira/plugins/schedules.json`)
pub const FILE: &str = "schedules.json";
/// How far back a missed run is still caught up
pub const CATCH_UP_DAYS: i64 = 7;
/// Failed runs in a row before a schedule pauses until the plugin changes
pub const PAUSE_AFTER: u32 = 5;

/// A five-field cron time: minute, hour, day of month, month, day of week (0 or 7 is Sunday). Each field is `*`, a
/// number, a range `a-b`, a list `a,b`, or any of these with a step `/n`; months and days also take three-letter names.
/// As in cron, when both day fields are restricted a day matching either runs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
  minutes: u64,
  hours: u32,
  days: u32,
  months: u16,
  weekdays: u8,
  any_day: bool,
  any_weekday: bool,
}

const MONTHS: &[&str] = &["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: &[&str] = &["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl Cron {
  pub fn parse(text: &str) -> Result<Cron, String> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    let [min, hour, dom, mon, dow] = fields.as_slice() else {
      return Err(format!("`{text}` is not five fields (minute hour day-of-month month day-of-week)"));
    };
    let minutes = field(min, 0, 59, &[]).map_err(|e| format!("minute: {e}"))?;
    let hours = field(hour, 0, 23, &[]).map_err(|e| format!("hour: {e}"))?;
    let days = field(dom, 1, 31, &[]).map_err(|e| format!("day of month: {e}"))?;
    let months = field(mon, 1, 12, MONTHS).map_err(|e| format!("month: {e}"))?;
    let mut weekdays = field(dow, 0, 7, WEEKDAYS).map_err(|e| format!("day of week: {e}"))?;
    // 7 is Sunday too
    if weekdays & (1 << 7) != 0 {
      weekdays |= 1;
    }
    Ok(Cron {
      minutes,
      hours: hours as u32,
      days: days as u32,
      months: months as u16,
      weekdays: (weekdays & 0x7f) as u8,
      // Only a bare `*` leaves the field open: `*/2` is a restriction like any list
      any_day: *dom == "*",
      any_weekday: *dow == "*",
    })
  }

  fn day_matches(&self, d: NaiveDate) -> bool {
    let dom = self.days & (1 << d.day()) != 0;
    let dow = self.weekdays & (1 << d.weekday().num_days_from_sunday()) != 0;
    match (self.any_day, self.any_weekday) {
      (true, true) => true,
      (false, true) => dom,
      (true, false) => dow,
      (false, false) => dom || dow,
    }
  }

  fn matches_naive(&self, t: NaiveDateTime) -> bool {
    self.months & (1 << t.month()) != 0 && self.day_matches(t.date()) && self.hours & (1 << t.hour()) != 0 && self.minutes & (1 << t.minute()) != 0
  }

  /// The first time strictly after `after` the schedule names, in the machine's zone. A wall time a daylight-saving
  /// jump skips runs at the first minute after the jump; a wall time that happens twice runs the first time only
  pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
    self.next_in(after, &Local)
  }

  fn next_in<Z: TimeZone>(&self, after: DateTime<Z>, zone: &Z) -> Option<DateTime<Z>> {
    let start = after.naive_local() + Duration::minutes(1);
    let mut t = start.date().and_hms_opt(start.hour(), start.minute(), 0)?;
    // Five years of minutes bound a search for a date that never comes (`0 0 31 2 *`)
    let limit = t + Duration::days(366 * 5);
    while t < limit {
      if self.months & (1 << t.month()) == 0 || !self.day_matches(t.date()) {
        t = (t.date() + Duration::days(1)).and_hms_opt(0, 0, 0)?;
        continue;
      }
      if self.hours & (1 << t.hour()) == 0 {
        t = t.date().and_hms_opt(t.hour(), 0, 0)? + Duration::hours(1);
        continue;
      }
      if self.minutes & (1 << t.minute()) == 0 {
        t += Duration::minutes(1);
        continue;
      }
      match zone.from_local_datetime(&t) {
        LocalResult::Single(at) => {
          if at > after {
            return Some(at);
          }
        }
        // Twice this wall time: the earlier instant (chrono does not promise which comes first), unless already past
        LocalResult::Ambiguous(a, b) => {
          let first = if a <= b { a } else { b };
          if first > after {
            return Some(first);
          }
        }
        // Skipped by a jump forward: the first minute that exists after it
        LocalResult::None => {
          let mut later = t;
          for _ in 0..180 {
            later += Duration::minutes(1);
            if let Some(at) = zone.from_local_datetime(&later).earliest() {
              if at > after {
                return Some(at);
              }
              break;
            }
          }
        }
      }
      t += Duration::minutes(1);
    }
    None
  }

  /// The last time at or before `now` the schedule names, looking back at most `days` days
  pub fn last_at_or_before(&self, now: DateTime<Local>, days: i64) -> Option<DateTime<Local>> {
    // Walk forward from the window's start: the last match found is the latest one
    let mut cursor = now - Duration::days(days) - Duration::minutes(1);
    let mut last = None;
    while let Some(at) = self.next_after(cursor) {
      if at > now {
        break;
      }
      last = Some(at);
      cursor = at;
    }
    last
  }

  /// Whether `t` (a local wall time) is one the schedule names
  pub fn matches(&self, t: DateTime<Local>) -> bool {
    self.matches_naive(t.naive_local())
  }
}

/// One field as a bit set over `lo..=hi`
fn field(text: &str, lo: u32, hi: u32, names: &[&str]) -> Result<u64, String> {
  let mut bits = 0u64;
  for part in text.split(',') {
    let (range, step) = match part.split_once('/') {
      Some((r, s)) => (r, s.parse::<u32>().map_err(|_| format!("`{s}` is not a step"))?),
      None => (part, 1),
    };
    if step == 0 {
      return Err("a step of 0".into());
    }
    let (a, b) = if range == "*" {
      (lo, hi)
    } else if let Some((a, b)) = range.split_once('-') {
      (value(a, names, lo)?, value(b, names, lo)?)
    } else {
      let v = value(range, names, lo)?;
      // `5/15` runs from 5 to the end, as cron reads it
      (v, if part.contains('/') { hi } else { v })
    };
    if a < lo || b > hi || a > b {
      return Err(format!("`{part}` is outside {lo}-{hi}"));
    }
    let mut v = a;
    while v <= b {
      bits |= 1 << v;
      v += step;
    }
  }
  if bits == 0 {
    return Err(format!("`{text}` names nothing"));
  }
  Ok(bits)
}

fn value(text: &str, names: &[&str], lo: u32) -> Result<u32, String> {
  if let Ok(v) = text.parse::<u32>() {
    return Ok(v);
  }
  let lower = text.to_ascii_lowercase();
  names.iter().position(|n| *n == lower).map(|i| i as u32 + lo).ok_or_else(|| format!("`{text}` is not a number"))
}

/// What the engines remember of one schedule (`<plugin>/<id>` in `schedules.json`)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
  /// The occurrence last started (milliseconds since the epoch), whoever started it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_due: Option<i64>,
  /// When that run started and ended
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub started_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ended_at: Option<i64>,
  /// A run under way: until when another engine leaves it alone (its timeout from the start)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub running_until: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  #[serde(default, skip_serializing_if = "is_zero")]
  pub failures: u32,
  /// The snapshot the failures belong to: a new version starts with none
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snapshot: Option<String>,
  /// Who holds the run under way: only that run's end may release it or count its outcome (a run that outlived its
  /// lease must not release the one another engine started since)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub claim: Option<String>,
}

fn is_zero(n: &u32) -> bool {
  *n == 0
}

pub fn key(plugin: &str, id: &str) -> String {
  format!("{plugin}/{id}")
}

/// `schedules.json`, read and written under the plugins folder's lock
pub struct Records {
  path: PathBuf,
}

impl Records {
  pub fn new(root: &Path) -> Records {
    Records { path: root.join(FILE) }
  }

  pub fn read(&self) -> BTreeMap<String, Record> {
    std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
  }

  /// Change the records under the file lock, written only when `f` changed them
  pub async fn update<T: Send + 'static>(&self, f: impl FnOnce(&mut BTreeMap<String, Record>) -> T + Send + 'static) -> anyhow::Result<T> {
    let path = self.path.clone();
    crate::store::file_lock::with_file_lock(&self.path.clone(), move || async move {
      let mut all: BTreeMap<String, Record> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
      let before = all.clone();
      let out = f(&mut all);
      if all != before {
        crate::store::file_lock::write_atomic(&path, &serde_json::to_vec_pretty(&all)?, None).await?;
      }
      Ok(out)
    })
    .await
  }
}

/// Which occurrence to start now, if any: the latest one due within the catch-up window that nobody started yet. A
/// `skip` schedule only starts an occurrence of the current minute
pub fn due(cron: &Cron, catch_up: bool, record: Option<&Record>, now: DateTime<Local>) -> Option<DateTime<Local>> {
  let window = if catch_up { CATCH_UP_DAYS } else { 0 };
  let at = cron.last_at_or_before(now, window.max(1))?;
  if !catch_up && now - at >= Duration::minutes(1) {
    return None;
  }
  match record.and_then(|r| r.last_due) {
    Some(last) if last >= at.timestamp_millis() => None,
    _ => Some(at),
  }
}

/// A run the engine just claimed
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
  pub plugin: String,
  pub id: String,
  pub due: i64,
  /// The snapshot it runs on
  pub hash: String,
  /// `Record::claim`
  pub token: String,
}

/// A token no other run on the machine holds: this process, the time, a counter
fn claim_token(now_ms: i64) -> String {
  static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
  format!("{}-{now_ms}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// One pass: claim, under the lock, every occurrence due of the schedules whose plugin runs here, and start each. A
/// schedule seen for the first time starts from now (no run of the past week replays at its install); one under way
/// anywhere, or paused after failures of this snapshot, is left alone
pub async fn tick(rt: &std::sync::Arc<crate::plugins::runtime::Runtimes>, records: &std::sync::Arc<Records>, now: DateTime<Local>, log: &crate::store::transcript_store::LogFn) -> Vec<tokio::task::JoinHandle<()>> {
  // Every plugin wanted here, started or still starting (a run waits for its process); not one whose process failed
  let wanted: Vec<_> = rt.schedules().into_iter().filter(|(plugin, _, _)| rt.error(plugin).is_none()).collect();
  if wanted.is_empty() {
    return vec![];
  }
  let now_ms = now.timestamp_millis();
  let jobs = wanted.clone();
  let claims = records
    .update(move |all| {
      let mut claims = vec![];
      for (plugin, hash, def) in &jobs {
        let Ok(cron) = Cron::parse(&def.cron) else { continue };
        let r = all.entry(key(plugin, &def.id)).or_default();
        if r.snapshot.as_deref() != Some(hash.as_str()) {
          // A new version: its failures start from none
          r.snapshot = Some(hash.clone());
          r.failures = 0;
          r.error = None;
        }
        if r.last_due.is_none() && r.started_at.is_none() {
          // First seen: what came before now is not owed
          r.last_due = cron.last_at_or_before(now - Duration::minutes(1), CATCH_UP_DAYS).map(|t| t.timestamp_millis()).or(Some(0));
        }
        if r.failures >= PAUSE_AFTER || r.running_until.is_some_and(|u| u > now_ms) {
          continue;
        }
        let Some(at) = due(&cron, def.catches_up(), Some(&*r), now) else { continue };
        let due_ms = at.timestamp_millis();
        r.last_due = Some(due_ms);
        r.started_at = Some(now_ms);
        r.running_until = Some(now_ms + def.timeout().as_millis() as i64);
        let token = claim_token(now_ms);
        r.claim = Some(token.clone());
        claims.push(Claim { plugin: plugin.clone(), id: def.id.clone(), due: due_ms, hash: hash.clone(), token });
      }
      claims
    })
    .await;
  let claims = match claims {
    Ok(c) => c,
    Err(e) => {
      log(&format!("plugins: schedules: {e:#}"));
      return vec![];
    }
  };
  let mut handles = vec![];
  for c in claims {
    let (rt, records, log) = (rt.clone(), records.clone(), log.clone());
    handles.push(tokio::spawn(async move {
      let result = rt.run_schedule(&c.plugin, &c.id, c.due).await;
      if let Err(e) = &result {
        log(&format!("plugin {}: schedule {}: {e}", c.plugin, c.id));
      }
      // Stopped under it (switched off, a new version, the engine closing): not the schedule's failure
      let interrupted = result.is_err() && !rt.schedules().iter().any(|(p, h, j)| *p == c.plugin && *h == c.hash && j.id == c.id);
      let k = key(&c.plugin, &c.id);
      let ended = chrono::Utc::now().timestamp_millis();
      let _ = records
        .update(move |all| {
          let Some(r) = all.get_mut(&k).filter(|r| r.claim.as_deref() == Some(c.token.as_str())) else { return };
          r.claim = None;
          r.ended_at = Some(ended);
          r.running_until = None;
          match result {
            Ok(_) => {
              r.error = None;
              r.failures = 0;
            }
            Err(_) if interrupted => {}
            Err(e) => {
              r.error = Some(e);
              r.failures += 1;
            }
          }
        })
        .await;
    }));
  }
  handles
}

/// The engine's loop: a pass at start (runs missed while no engine ran), then one just after each minute begins.
/// Holds the runtimes weakly, so it ends with the engine
pub fn spawn(rt: std::sync::Weak<crate::plugins::runtime::Runtimes>, root: PathBuf, log: crate::store::transcript_store::LogFn) {
  let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
  let records = std::sync::Arc::new(Records::new(&root));
  handle.spawn(async move {
    loop {
      let Some(runtimes) = rt.upgrade() else { return };
      if runtimes.enabled() {
        let _ = tick(&runtimes, &records, Local::now(), &log).await;
      }
      drop(runtimes);
      let now = Local::now();
      let into_minute = now.second() as u64 * 1000 + now.timestamp_subsec_millis() as u64;
      tokio::time::sleep(std::time::Duration::from_millis(61_000u64.saturating_sub(into_minute).max(1_000))).await;
    }
  });
}

/// What the plugin page shows of one schedule
pub fn view(def: &crate::plugins::manifest::ScheduleDef, record: Option<&Record>, now: DateTime<Local>) -> acpira_shared::plugins::PluginScheduleInfo {
  let next = Cron::parse(&def.cron).ok().and_then(|c| c.next_after(now)).map(|t| t.timestamp_millis());
  acpira_shared::plugins::PluginScheduleInfo {
    id: def.id.clone(),
    cron: def.cron.clone(),
    description: (!def.description.is_empty()).then(|| def.description.clone()),
    next,
    last: record.and_then(|r| r.started_at),
    running: record.and_then(|r| r.running_until).is_some_and(|u| u > now.timestamp_millis()),
    error: record.and_then(|r| r.error.clone()),
    paused: record.is_some_and(|r| r.failures >= PAUSE_AFTER),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::FixedOffset;

  fn at(s: &str) -> DateTime<Local> {
    Local.from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()).earliest().unwrap()
  }

  #[test]
  fn parses_fields_and_rejects_nonsense() {
    assert!(Cron::parse("*/15 9-17 * * mon-fri").is_ok());
    assert!(Cron::parse("0 0 1,15 jan,jul *").is_ok());
    assert!(Cron::parse("0 0 * * 7").is_ok());
    assert!(Cron::parse("0 0 * *").unwrap_err().contains("five fields"));
    assert!(Cron::parse("60 0 * * *").unwrap_err().starts_with("minute"));
    assert!(Cron::parse("0 0 * * */0").is_err());
    assert!(Cron::parse("0 0 * * funday").is_err());
  }

  #[test]
  fn finds_the_next_time() {
    let c = Cron::parse("30 9 * * mon-fri").unwrap();
    // 2026-10-31 is a Saturday: the next run is Monday 09:30
    assert_eq!(c.next_after(at("2026-10-31 10:00")).unwrap(), at("2026-11-02 09:30"));
    assert_eq!(c.next_after(at("2026-11-02 09:29")).unwrap(), at("2026-11-02 09:30"));
    assert_eq!(c.next_after(at("2026-11-02 09:30")).unwrap(), at("2026-11-03 09:30"));
    let every = Cron::parse("*/20 * * * *").unwrap();
    assert_eq!(every.next_after(at("2026-11-02 09:41")).unwrap(), at("2026-11-02 10:00"));
    assert!(Cron::parse("0 0 31 2 *").unwrap().next_after(at("2026-01-01 00:00")).is_none());
  }

  #[test]
  fn either_day_field_matches_when_both_are_set() {
    // The 1st of the month or any Monday
    let c = Cron::parse("0 8 1 * mon").unwrap();
    assert_eq!(c.next_after(at("2026-10-31 09:00")).unwrap(), at("2026-11-01 08:00"));
    assert_eq!(c.next_after(at("2026-11-01 09:00")).unwrap(), at("2026-11-02 08:00"));
  }

  #[test]
  fn a_step_in_a_day_field_restricts_it() {
    // Odd days of the month only, not every day
    let c = Cron::parse("0 9 */2 * *").unwrap();
    assert_eq!(c.next_after(at("2026-11-01 10:00")).unwrap(), at("2026-11-03 09:00"));
    // Every other weekday from Sunday: Sun, Tue, Thu, Sat
    let w = Cron::parse("0 9 * * */2").unwrap();
    assert_eq!(w.next_after(at("2026-11-02 10:00")).unwrap(), at("2026-11-03 09:00"));
  }

  #[test]
  fn searches_in_any_zone() {
    let z = FixedOffset::east_opt(3600).unwrap();
    let c = Cron::parse("30 2 * * *").unwrap();
    let after = z.with_ymd_and_hms(2026, 3, 29, 1, 0, 0).unwrap();
    assert_eq!(c.next_in(after, &z).unwrap(), z.with_ymd_and_hms(2026, 3, 29, 2, 30, 0).unwrap());
  }

  #[test]
  fn due_catches_up_once_and_skip_waits() {
    let c = Cron::parse("0 9 * * *").unwrap();
    let now = at("2026-11-02 14:00");
    // Never run: today's 09:00 is due (missed while closed)
    assert_eq!(due(&c, true, None, now).unwrap(), at("2026-11-02 09:00"));
    // Already started by some engine
    let r = Record { last_due: Some(at("2026-11-02 09:00").timestamp_millis()), ..Default::default() };
    assert!(due(&c, true, Some(&r), now).is_none());
    // `skip` only starts the current minute's occurrence
    assert!(due(&c, false, None, now).is_none());
    assert_eq!(due(&c, false, None, at("2026-11-02 09:00")).unwrap(), at("2026-11-02 09:00"));
  }
}
