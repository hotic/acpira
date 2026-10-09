//! Turn failures a CLI keeps out of ACP. Kimi (`kimi_failure`) and pi-acp (`pi_failure`) answer a turn whose model call
//! failed with a plain `end_turn` and no output; the cause is only in the CLI's own session log. An empty `end_turn` from
//! one of them reads the log back, so the error card shows the cause instead of the generic empty-response text

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

/// Only the end of a long session's log matters: the failed turn's lines are the last few
const TAIL_BYTES: u64 = 256 * 1024;
/// The log line and the ACP answer are written independently: give the line a moment to land
const POLL_ATTEMPTS: u32 = 10;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What the CLI recorded about a failed turn
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
  pub message: String,
  /// The CLI's own error code (Kimi's `provider.api_error`, …)
  pub code: Option<String>,
  pub retryable: Option<bool>,
}

/// How the turn the prompt opened ended, according to the log
#[derive(Debug, PartialEq)]
pub enum LastTurn {
  /// Nothing written at or after the prompt yet (not flushed, or the log is missing)
  Pending,
  Ended(Option<Failure>),
}

/// Polls `probe` (one read of the log; None = no log yet) until it shows how the turn ended. Blocking: run it off the
/// async threads
pub fn poll(mut probe: impl FnMut() -> Option<LastTurn>) -> Option<Failure> {
  for attempt in 0..POLL_ATTEMPTS {
    if attempt > 0 {
      std::thread::sleep(POLL_INTERVAL);
    }
    if let Some(LastTurn::Ended(failure)) = probe() {
      return failure;
    }
  }
  None
}

/// The last `TAIL_BYTES` of the file; a line cut at the start fails to parse and is skipped
pub fn read_tail(path: &Path) -> Option<String> {
  let mut file = std::fs::File::open(path).ok()?;
  let len = file.metadata().ok()?.len();
  file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).ok()?;
  let mut bytes = Vec::new();
  file.read_to_end(&mut bytes).ok()?;
  Some(String::from_utf8_lossy(&bytes).into_owned())
}
