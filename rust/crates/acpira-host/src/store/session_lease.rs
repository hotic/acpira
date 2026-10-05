//! Cross-engine session leases: which Acpira engine is driving a session right now.
//!
//! Every engine on a machine (one per VS Code workspace, an IntelliJ project sidecar, an older version still draining
//! after an upgrade) shares `~/.acpira/sessions`. An engine holds an OS file lock on `run/leases/<session id>.lock` while
//! one of its sessions runs a turn, and on every session it keeps alive while no window is connected (a draining
//! engine). Another engine that wants to open the session sees the lock and waits instead of starting a second agent on
//! the same native session (which would mark the running turn interrupted and race the record on disk).
//!
//! A turn starts only after its engine took the lease (`claim`): two engines that both have the session open (an idle agent
//! each) can never run turns on it at the same time. Loading a session and editing its record (rename, pin, delete, move)
//! claim it too, for as long as they read and write. Every claim is a pin with its own number, released by that number
//! only: a turn's pin stays until the turn's final record is on disk, so the next engine never reads a record that still
//! lacks the end of the turn, and a later claim on the same session is never cut short by an earlier one's release.
//!
//! An engine holds the lease of every session it has open (live), not only while a turn runs: a session is driven from one
//! place at a time, and every other engine shows it read-only. Taking it over is a request file next to the lease
//! (`<id>.takeover`, naming the requesting engine): the holder sees it, stops its turn, lets go and turns read-only itself,
//! and the requester takes the lease.
//!
//! The lock is `File::try_lock` (flock / LockFileEx): the OS drops it when the holder exits or crashes, so a stale
//! lease cannot outlive its engine. Lease files are never deleted, so two engines can never lock different inodes.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::transcript_store::is_session_id;

// A probe holds a lease file for microseconds; three short retries tell a probe from a real holder
const CLAIM_RETRIES: usize = 3;
const CLAIM_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);
// A takeover request older than this is left over from a requester that gave up or died, and is ignored
const TAKEOVER_FRESH: std::time::Duration = std::time::Duration::from_secs(15);

/// What `claim` got
#[derive(Debug, PartialEq, Eq)]
pub enum Claim {
  /// The lease is this engine's, pinned under this number until `release`
  Held(u64),
  /// Another engine holds it
  Elsewhere,
  /// The lease could not be taken at all (the lease file could not be opened or locked): not safe to proceed either
  Failed(String),
}

#[derive(Default)]
struct Leases {
  /// The leases this engine holds, by session id; dropping the file releases the lock
  files: HashMap<String, File>,
  /// Live claims per session: a lease with any is kept whatever `sync` wants
  pins: HashMap<String, HashSet<u64>>,
  /// What the last `sync` wanted: a lease whose last pin goes is dropped right away unless it is wanted
  wanted: HashSet<String>,
}

pub struct SessionLeases {
  dir: PathBuf,
  /// One lock over files, pins and wants, so a claim can never fall between a sync's look at the pins and its release
  state: parking_lot::Mutex<Leases>,
  next_pin: AtomicU64,
  /// Names this engine in takeover requests (engines in one process, as in tests, still tell each other apart)
  token: String,
  log: crate::store::transcript_store::LogFn,
}

impl SessionLeases {
  /// `root`: ACPIRA_HOME; the leases live under `run/leases/`
  pub fn new(root: &Path, log: crate::store::transcript_store::LogFn) -> Self {
    SessionLeases {
      dir: root.join("run").join("leases"),
      state: Default::default(),
      next_pin: AtomicU64::new(1),
      token: crate::util::random_uuid(),
      log,
    }
  }

  fn path(&self, id: &str) -> Option<PathBuf> {
    is_session_id(id).then(|| self.dir.join(format!("{id}.lock")))
  }

  /// Lock the session's lease file into `st` unless it is there already
  fn take(&self, st: &mut Leases, id: &str) -> Claim {
    if st.files.contains_key(id) {
      return Claim::Held(0);
    }
    let Some(path) = self.path(id) else { return Claim::Failed(format!("{id} is not a session id")) };
    if let Err(e) = std::fs::create_dir_all(&self.dir) {
      return Claim::Failed(format!("{}: {e}", self.dir.display()));
    }
    let f = match OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path) {
      Ok(f) => f,
      Err(e) => return Claim::Failed(format!("{}: {e}", path.display())),
    };
    match f.try_lock() {
      Ok(()) => {
        st.files.insert(id.to_owned(), f);
        Claim::Held(0)
      }
      Err(TryLockError::WouldBlock) => Claim::Elsewhere,
      Err(TryLockError::Error(e)) => Claim::Failed(format!("{}: {e}", path.display())),
    }
  }

  /// Hold the leases in `wanted` plus every pinned one: newly wanted ones are taken (a lease another engine holds is
  /// logged and skipped, the conflict predates this call), the rest are released
  pub fn sync(&self, wanted: &HashSet<String>) {
    let mut guard = self.state.lock();
    let st = &mut *guard;
    st.wanted = wanted.clone();
    st.files.retain(|id, _| wanted.contains(id) || st.pins.contains_key(id));
    for id in wanted {
      match self.take(st, id) {
        Claim::Held(_) => {}
        Claim::Elsewhere => (self.log)(&format!("session {id}: lease held by another engine")),
        Claim::Failed(e) => (self.log)(&format!("session {id}: lease failed: {e}")),
      }
    }
  }

  /// Take the lease and pin it under a fresh number. Ids that are not session ids have no lease and always succeed.
  /// Another engine's `held_elsewhere` probe locks the file for an instant: a busy lease is tried a few more times
  /// before it counts as held elsewhere
  pub fn claim(&self, id: &str) -> Claim {
    if !is_session_id(id) {
      return Claim::Held(0);
    }
    let mut st = self.state.lock();
    let mut taken = self.take(&mut st, id);
    for _ in 0..CLAIM_RETRIES {
      if taken != Claim::Elsewhere {
        break;
      }
      std::thread::sleep(CLAIM_RETRY_PAUSE);
      taken = self.take(&mut st, id);
    }
    match taken {
      Claim::Held(_) => {
        let pin = self.next_pin.fetch_add(1, Ordering::Relaxed);
        st.pins.entry(id.to_owned()).or_default().insert(pin);
        Claim::Held(pin)
      }
      other => other,
    }
  }

  /// One claim is over; the lease goes with its last pin unless the last `sync` wanted it
  pub fn release(&self, id: &str, pin: u64) {
    let mut st = self.state.lock();
    let Some(pins) = st.pins.get_mut(id) else { return };
    pins.remove(&pin);
    if pins.is_empty() {
      st.pins.remove(id);
      if !st.wanted.contains(id) {
        st.files.remove(id);
      }
    }
  }

  /// Whether another process holds the session's lease (never true for a lease this engine holds itself)
  pub fn held_elsewhere(&self, id: &str) -> bool {
    if self.state.lock().files.contains_key(id) {
      return false;
    }
    let Some(path) = self.path(id) else { return false };
    // A session that never had a lease file was never leased: no need to create one just to look
    let Ok(f) = OpenOptions::new().read(true).write(true).open(&path) else { return false };
    // The probe lock is released when `f` drops at the end of this call
    matches!(f.try_lock(), Err(TryLockError::WouldBlock))
  }

  fn takeover_path(&self, id: &str) -> Option<PathBuf> {
    is_session_id(id).then(|| self.dir.join(format!("{id}.takeover")))
  }

  /// Ask whichever engine holds the session to let go of it (it notices within a second)
  pub fn request_takeover(&self, id: &str) -> std::io::Result<()> {
    let Some(path) = self.takeover_path(id) else { return Ok(()) };
    std::fs::create_dir_all(&self.dir)?;
    let tmp = path.with_extension(format!("takeover.{}", self.token));
    std::fs::write(&tmp, &self.token)?;
    std::fs::rename(&tmp, &path)
  }

  /// A fresh takeover request for the session from another engine
  pub fn takeover_requested(&self, id: &str) -> bool {
    let Some(path) = self.takeover_path(id) else { return false };
    let fresh = std::fs::metadata(&path)
      .and_then(|m| m.modified())
      .is_ok_and(|t| t.elapsed().map_or(true, |age| age < TAKEOVER_FRESH));
    fresh && std::fs::read_to_string(&path).is_ok_and(|who| who != self.token)
  }

  /// This engine's own request is done (taken over, or given up)
  pub fn clear_takeover(&self, id: &str) {
    let Some(path) = self.takeover_path(id) else { return };
    if std::fs::read_to_string(&path).is_ok_and(|who| who == self.token) {
      let _ = std::fs::remove_file(&path);
    }
  }

  /// Ids this engine holds, sorted (tests, logs)
  pub fn held(&self) -> Vec<String> {
    let mut ids: Vec<String> = self.state.lock().files.keys().cloned().collect();
    ids.sort();
    ids
  }

  pub fn release_all(&self) {
    let mut st = self.state.lock();
    st.pins.clear();
    st.wanted.clear();
    st.files.clear();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Arc;

  const ID: &str = "0f9c2a64-5b3e-4c51-9d7e-2b6f8a1c3e5d";

  fn leases(root: &Path) -> SessionLeases {
    SessionLeases::new(root, Arc::new(|_: &str| {}))
  }

  fn pin(c: Claim) -> u64 {
    match c {
      Claim::Held(p) => p,
      other => panic!("expected the lease, got {other:?}"),
    }
  }

  #[test]
  fn a_lease_is_seen_by_another_holder_until_released() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (leases(dir.path()), leases(dir.path()));
    assert!(!b.held_elsewhere(ID), "no lease file yet");
    a.sync(&HashSet::from([ID.to_owned()]));
    assert_eq!(a.held(), vec![ID.to_owned()]);
    assert!(!a.held_elsewhere(ID), "its own lease is not elsewhere");
    assert!(b.held_elsewhere(ID));
    // A second engine asking for the same lease does not get it while the first holds it
    b.sync(&HashSet::from([ID.to_owned()]));
    assert!(b.held().is_empty());
    a.sync(&HashSet::new());
    assert!(!b.held_elsewhere(ID));
    b.sync(&HashSet::from([ID.to_owned()]));
    assert!(a.held_elsewhere(ID));
    b.release_all();
    assert!(!a.held_elsewhere(ID));
  }

  #[test]
  fn a_claim_fails_while_another_engine_holds_the_lease_and_a_pin_outlives_sync() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (leases(dir.path()), leases(dir.path()));
    let turn = pin(a.claim(ID));
    assert_eq!(b.claim(ID), Claim::Elsewhere, "a turn must not start while another engine holds the session");
    // Pinned: a sync that no longer wants it (the turn ended, the record is still being written) keeps it
    a.sync(&HashSet::new());
    assert!(b.held_elsewhere(ID));
    a.release(ID, turn);
    assert!(!b.held_elsewhere(ID), "the last pin going releases a lease no sync wants");
    pin(b.claim(ID));
  }

  #[test]
  fn releasing_one_claim_leaves_a_later_claim_on_the_same_session_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (leases(dir.path()), leases(dir.path()));
    let first = pin(a.claim(ID));
    let second = pin(a.claim(ID));
    assert_ne!(first, second);
    // The first turn's late flush must not end the second turn's hold
    a.release(ID, first);
    a.release(ID, first);
    a.sync(&HashSet::new());
    assert!(b.held_elsewhere(ID));
    a.release(ID, second);
    assert!(!b.held_elsewhere(ID));
  }

  #[test]
  fn a_wanted_lease_stays_after_its_pins_go_until_a_sync_lets_go() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (leases(dir.path()), leases(dir.path()));
    a.sync(&HashSet::from([ID.to_owned()]));
    let p = pin(a.claim(ID));
    a.release(ID, p);
    assert!(b.held_elsewhere(ID));
    a.sync(&HashSet::new());
    assert!(!b.held_elsewhere(ID));
  }

  #[test]
  fn a_lease_that_cannot_be_taken_is_a_failure_not_a_pass() {
    let dir = tempfile::tempdir().unwrap();
    // The lease directory's place is taken by a file: nothing can be created under it
    std::fs::create_dir_all(dir.path().join("run")).unwrap();
    std::fs::write(dir.path().join("run").join("leases"), "").unwrap();
    assert!(matches!(leases(dir.path()).claim(ID), Claim::Failed(_)));
  }

  #[test]
  fn a_takeover_request_is_seen_by_the_other_engine_only_and_cleared_by_its_requester() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (leases(dir.path()), leases(dir.path()));
    assert!(!a.takeover_requested(ID));
    b.request_takeover(ID).unwrap();
    assert!(a.takeover_requested(ID));
    assert!(!b.takeover_requested(ID), "a request is not addressed to its own engine");
    a.clear_takeover(ID);
    assert!(a.takeover_requested(ID), "only the requester clears it");
    b.clear_takeover(ID);
    assert!(!a.takeover_requested(ID));
  }

  #[test]
  fn ids_that_are_not_session_ids_are_never_leased() {
    let dir = tempfile::tempdir().unwrap();
    let a = leases(dir.path());
    a.sync(&HashSet::from(["../escape".to_owned()]));
    assert!(a.held().is_empty());
    assert!(!a.held_elsewhere("../escape"));
    assert_eq!(a.claim("../escape"), Claim::Held(0));
    assert!(a.held().is_empty());
  }
}
