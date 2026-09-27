//! Provider retries reported as prose. pi-acp 0.0.33 forwards pi's `auto_retry_start` / `auto_retry_end` events as
//! whole `agent_message_chunk`s; normalize turns them into one notice row per retry run instead of reply text.
//! Like `compaction_text`, only a chunk that is exactly an adapter sentence counts.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
  /// `(attempt, max attempts, wait in seconds)`; None when the adapter fell back to a bare "Retrying..."
  Attempt(Option<(u32, u32, u32)>),
  /// pi-acp says "resuming" whether or not the last attempt succeeded
  Finished,
}

static PI_ATTEMPT: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^Retrying(?: \(attempt (\d+)/(\d+), waiting (\d+)s\))?(?:\.{3}|…)$").unwrap());
static PI_FINISHED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Retry finished, resuming\.$").unwrap());

pub fn marker(agent: &str, chunk: &str) -> Option<Marker> {
  if agent != "pi" {
    return None;
  }
  let chunk = chunk.trim();
  if PI_FINISHED.is_match(chunk) {
    return Some(Marker::Finished);
  }
  let c = PI_ATTEMPT.captures(chunk)?;
  let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse().ok());
  Some(Marker::Attempt(num(1).zip(num(2)).zip(num(3)).map(|((a, m), w)| (a, m, w))))
}

#[cfg(test)]
mod tests {
  use super::Marker::*;
  use super::*;

  #[test]
  fn pi_retry_sentences_are_markers() {
    assert_eq!(marker("pi", "Retrying (attempt 1/3, waiting 2s)..."), Some(Attempt(Some((1, 3, 2)))));
    assert_eq!(marker("pi", "Retrying..."), Some(Attempt(None)));
    assert_eq!(marker("pi", "\nRetry finished, resuming.\n"), Some(Finished));
  }

  #[test]
  fn prose_and_other_agents_are_not_markers() {
    assert_eq!(marker("pi", "Retrying the build with a clean cache..."), None);
    assert_eq!(marker("pi", "Retry finished, resuming. Next I will"), None);
    assert_eq!(marker("devin", "Retrying (attempt 1/3, waiting 2s)..."), None);
  }
}
