//! The newest published version of an npm adapter package. A new model reaches an adapter CLI's model list only through
//! a newer adapter (the list is the bundled engine's own), so the update offer follows the registry's `latest` tag, not
//! only the version the install line pins.
//!
//! Asked through the machine's own `npm view`, so a configured registry mirror, auth and npm proxy apply as they do to
//! the install itself. A probe pass only reads the cache and starts a background lookup when it is stale, so neither
//! the engine's start nor a session's waits for npm; the next pass (a view opening, a window regaining focus) offers
//! what it found.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::platform::command::Os;

/// How long a looked-up version stays good, and how soon a failed lookup (offline, no npm) is tried again
const FRESH: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY: Duration = Duration::from_secs(30 * 60);
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Entry {
  /// When the last lookup ended; None before the first
  at: Option<Instant>,
  version: Option<semver::Version>,
  /// A lookup is running: no second one starts
  busy: bool,
}

static CACHE: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// Bumped whenever a lookup finds a newer version than the cache held, so the engine probes again and offers it
static FOUND: LazyLock<tokio::sync::watch::Sender<u64>> = LazyLock::new(|| tokio::sync::watch::channel(0).0);

pub fn found_newer() -> tokio::sync::watch::Receiver<u64> {
  FOUND.subscribe()
}

/// The last version looked up for `package`, however old
pub fn cached(package: &str) -> Option<semver::Version> {
  CACHE.lock().get(package).and_then(|e| e.version.clone())
}

/// Start a background lookup of `package` unless the cached answer is still fresh or one is already running. A failure
/// keeps the last known version
pub fn kick(package: &str, os: Os) {
  if cfg!(test) {
    return;
  }
  {
    let mut cache = CACHE.lock();
    let e = cache.entry(package.to_owned()).or_default();
    let keep = if e.version.is_some() { FRESH } else { RETRY };
    if e.busy || e.at.is_some_and(|t| t.elapsed() < keep) {
      return;
    }
    e.busy = true;
  }
  let package = package.to_owned();
  tokio::spawn(async move {
    let found = lookup(&package, os).await;
    let newer = {
      let mut cache = CACHE.lock();
      let e = cache.entry(package).or_default();
      let newer = found.as_ref().is_some_and(|f| e.version.as_ref().is_none_or(|v| f > v));
      e.version = found.or(e.version.take());
      e.at = Some(Instant::now());
      e.busy = false;
      newer
    };
    if newer {
      FOUND.send_modify(|n| *n += 1);
    }
  });
}

async fn lookup(package: &str, os: Os) -> Option<semver::Version> {
  let npm = super::registry::resolve_command("npm", &[], os, &super::launch::ProcessEnv).await?;
  let mut cmd = super::launch::command(&npm, &["view".into(), format!("{package}@latest"), "version".into()]);
  cmd.env("npm_config_update_notifier", "false").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
  let out = tokio::time::timeout(TIMEOUT, cmd.output()).await.ok()?.ok()?;
  if !out.status.success() {
    return None;
  }
  semver::Version::parse(String::from_utf8_lossy(&out.stdout).trim()).ok()
}

#[cfg(test)]
pub fn seed(package: &str, version: &str) {
  CACHE.lock().insert(package.to_owned(), Entry { at: Some(Instant::now()), version: semver::Version::parse(version).ok(), busy: false });
}
