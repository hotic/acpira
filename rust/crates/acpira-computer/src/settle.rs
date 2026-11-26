//! Knows when an app has finished reacting to an action.
//!
//! Each system's driver listens to the app's own change notifications and stamps the time of the latest one. After an
//! action the caller waits until the notifications stop, instead of sleeping a fixed time: a native control answers
//! in a few milliseconds, a web view takes as long as it takes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The count and the time of an app's notifications. Written by the listener, read by the waiter.
pub struct Stamp {
  /// Nanoseconds since `epoch` of the latest notification.
  last: AtomicU64,
  count: AtomicU64,
  epoch: Instant,
}

/// How long a wait took and what it saw.
pub struct Settled {
  pub events: u64,
  pub waited: Duration,
  pub timed_out: bool,
}

impl Default for Stamp {
  fn default() -> Self {
    Stamp { last: AtomicU64::new(0), count: AtomicU64::new(0), epoch: Instant::now() }
  }
}

impl Stamp {
  /// One notification arrived.
  pub fn hit(&self) {
    self.last.store(self.epoch.elapsed().as_nanos() as u64, Ordering::Release);
    self.count.fetch_add(1, Ordering::Release);
  }

  pub fn mark(&self) -> u64 {
    self.count.load(Ordering::Acquire)
  }

  /// Waits until the app is quiet. `mark` is the event count taken before the action. Returns as soon as events have
  /// been seen and none arrived for `quiet`; when nothing arrives at all, after `first`; never later than `max`.
  pub fn settle(&self, mark: u64, quiet: Duration, first: Duration, max: Duration) -> Settled {
    let started = Instant::now();
    loop {
      let waited = started.elapsed();
      let events = self.count.load(Ordering::Acquire).saturating_sub(mark);
      if events > 0 {
        let last = Duration::from_nanos(self.last.load(Ordering::Acquire));
        if self.epoch.elapsed().saturating_sub(last) >= quiet {
          return Settled { events, waited, timed_out: false };
        }
      } else if waited >= first {
        return Settled { events, waited, timed_out: false };
      }
      if waited >= max {
        return Settled { events, waited, timed_out: true };
      }
      std::thread::sleep(Duration::from_millis(1));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_wait_ends_when_nothing_comes_or_when_it_stops_coming() {
    let stamp = Stamp::default();
    let nothing = stamp.settle(stamp.mark(), Duration::from_millis(5), Duration::from_millis(10), Duration::from_millis(200));
    assert!(nothing.events == 0 && !nothing.timed_out && nothing.waited >= Duration::from_millis(10));
    let mark = stamp.mark();
    stamp.hit();
    let quiet = stamp.settle(mark, Duration::from_millis(5), Duration::from_millis(100), Duration::from_millis(200));
    assert!(quiet.events == 1 && !quiet.timed_out && quiet.waited < Duration::from_millis(100));
  }
}
