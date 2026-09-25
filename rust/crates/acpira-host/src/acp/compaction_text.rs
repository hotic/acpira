//! Compaction reported as prose. Agents without `compaction_update` announce compaction with fixed adapter strings sent
//! as whole `agent_message_chunk`s; normalize turns a chunk made only of these markers into the same `compaction` block a
//! structured update produces, wherever it lands in the turn (a /compact reply or an automatic compaction mid-turn).
//! Model prose streams as token deltas, so a chunk that is exactly an adapter sentence is the adapter speaking.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
  Start,
  Done,
  Failed(String),
  Cancelled,
  /// Statistics printed under a completion line in a chunk of their own (Kimi / Pi)
  Detail,
}

type Table = &'static [(&'static LazyLock<Regex>, fn(&str) -> Marker)];

macro_rules! re {
  ($name:ident, $src:expr) => {
    static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new(concat!("^(?:", $src, ")")).unwrap());
  };
}

// Devin 3000.6.x
re!(DEVIN_START, r"Compacting context(?:\.{3}|…)?");
re!(DEVIN_DONE, r"Context compacted\.?|Nothing to compact\.");
// Kimi 0.41.0
re!(KIMI_START, r"Context compaction started\b[^\n]*|Compaction started\b[^\n]*");
// Kimi 0.41.0 and pi-acp 0.0.33 /compact: statistics and Pi's summary follow the headline inside the same chunk
re!(COMPLETED, r"Compaction completed\.[\s\S]*|Nothing to compact\.");
// pi-acp 0.0.33 auto_compaction_start / auto_compaction_end
re!(PI_START, r"Context nearing limit, running automatic compaction(?:\.{3}|…)?");
re!(PI_DONE, r"Automatic compaction finished; context was summarized to continue the session\.");
re!(FAILED, r"(?:Force compaction|Compaction) failed:[^\n]*|/compact failed:[^\n]*|Compaction is blocked by the current turn;[^\n]*");
re!(CANCELLED, r"Compaction cancel(?:ed|led)\.");
re!(DETAIL, r"(?:\s*-?\s*(?:Messages compacted|Tokens (?:before|after)):\s*[\d,]+)+");

fn start(_: &str) -> Marker {
  Marker::Start
}
fn done(_: &str) -> Marker {
  Marker::Done
}
fn failed(m: &str) -> Marker {
  Marker::Failed(m.trim().to_owned())
}
fn cancelled(_: &str) -> Marker {
  Marker::Cancelled
}
fn detail(_: &str) -> Marker {
  Marker::Detail
}

static DEVIN: Table = &[(&DEVIN_START, start), (&DEVIN_DONE, done), (&FAILED, failed), (&CANCELLED, cancelled)];
static KIMI: Table = &[(&KIMI_START, start), (&COMPLETED, done), (&FAILED, failed), (&CANCELLED, cancelled), (&DETAIL, detail)];
static PI: Table = &[(&PI_START, start), (&PI_DONE, done), (&COMPLETED, done), (&FAILED, failed), (&DETAIL, detail)];

fn table(agent: &str) -> Option<Table> {
  match agent {
    "devin" => Some(DEVIN),
    "kimi" => Some(KIMI),
    "pi" => Some(PI),
    _ => None,
  }
}

/// The markers a whole chunk consists of, in order; None when any part of it is something else
pub fn markers(agent: &str, chunk: &str) -> Option<Vec<Marker>> {
  let table = table(agent)?;
  let mut rest = chunk.trim();
  let mut out = vec![];
  while !rest.is_empty() {
    let (len, marker) = table.iter().find_map(|(re, make)| re.find(rest).filter(|m| !m.is_empty()).map(|m| (m.end(), make(m.as_str()))))?;
    out.push(marker);
    rest = rest[len..].trim_start();
  }
  (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
  use super::Marker::*;
  use super::*;

  #[test]
  fn whole_adapter_chunks_are_markers() {
    assert_eq!(markers("devin", "Compacting context…"), Some(vec![Start]));
    assert_eq!(markers("devin", "Compacting context...Context compacted"), Some(vec![Start, Done]));
    assert_eq!(
      markers("kimi", "Context compaction started — it runs in the background and the compacted context applies once it finishes."),
      Some(vec![Start])
    );
    assert_eq!(markers("kimi", "Compaction completed.\n- Messages compacted: 3\n- Tokens after: 1,200"), Some(vec![Done]));
    assert_eq!(markers("kimi", "\n- Messages compacted: 3"), Some(vec![Detail]));
    assert_eq!(markers("pi", "Context nearing limit, running automatic compaction..."), Some(vec![Start]));
    assert_eq!(markers("pi", "Automatic compaction finished; context was summarized to continue the session."), Some(vec![Done]));
    assert_eq!(markers("pi", "Compaction completed.\nTokens before: 91000\n\n## Goal\nship it"), Some(vec![Done]));
    assert_eq!(markers("devin", "Compaction failed: window too small"), Some(vec![Failed("Compaction failed: window too small".into())]));
    assert_eq!(markers("kimi", "Compaction canceled."), Some(vec![Cancelled]));
  }

  #[test]
  fn prose_and_other_agents_are_not_markers() {
    assert_eq!(markers("devin", "Context compacted, so I will now continue"), None);
    assert_eq!(markers("devin", "Compacting"), None);
    assert_eq!(markers("devin", ""), None);
    assert_eq!(markers("claude", "Context compacted"), None);
    assert_eq!(markers("kimi", "Context compacted"), None);
  }
}
