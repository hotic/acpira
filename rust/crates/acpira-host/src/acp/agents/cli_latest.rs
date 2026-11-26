//! Latest published versions of built-in non-npm CLIs (Antigravity, Grok, Devin, Kimi, OpenCode, Droid, Hermes).
//!
//! Mirrors `npm_latest`: a probe pass only reads the cache and starts a background HTTP lookup when it is stale, so
//! neither engine startup nor session launch ever waits on the network. Requests honour `net_proxy`. When a lookup
//! finds a newer version it bumps the shared watch channel so the engine reprobes and pushes the update offer at once.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;

use crate::platform::command::Os;

/// How long a looked-up version stays fresh, and how soon a failed lookup (offline, rate limit) is tried again
const FRESH: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY: Duration = Duration::from_secs(30 * 60);
const TIMEOUT: Duration = Duration::from_secs(15);
/// Cap response bodies: even the largest vendor install script (Factory Droid) is a few KB
const MAX_BODY_BYTES: u64 = 256 * 1024;

/// Where a built-in native CLI publishes its latest release version
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseSource {
  Antigravity,
  Grok,
  Devin,
  Kimi,
  OpenCode,
  Droid,
  Hermes,
}

#[derive(Default)]
struct Entry {
  /// When the last lookup ended; None before the first
  at: Option<Instant>,
  version: Option<semver::Version>,
  /// A lookup is running: no second one starts
  busy: bool,
}

static CACHE: LazyLock<Mutex<HashMap<ReleaseSource, Entry>>> = LazyLock::new(Default::default);

use std::sync::LazyLock;

/// The last version looked up for `source`, however old
pub fn cached(source: ReleaseSource) -> Option<semver::Version> {
  CACHE.lock().get(&source).and_then(|e| e.version.clone())
}

/// Start a background lookup for `source` unless the cached answer is still fresh or one is already running.
/// A failure keeps the last known version.
pub fn kick(source: ReleaseSource, os: Os) {
  if cfg!(test) {
    return;
  }
  {
    let mut cache = CACHE.lock();
    let e = cache.entry(source).or_default();
    let keep = if e.version.is_some() { FRESH } else { RETRY };
    if e.busy || e.at.is_some_and(|t| t.elapsed() < keep) {
      return;
    }
    e.busy = true;
  }
  tokio::spawn(async move {
    let found = tokio::task::spawn_blocking(move || lookup(source, os)).await.ok().flatten();
    let newer = {
      let mut cache = CACHE.lock();
      let e = cache.entry(source).or_default();
      let newer = found.as_ref().is_some_and(|f| e.version.as_ref().is_none_or(|v| f > v));
      e.version = found.or(e.version.take());
      e.at = Some(Instant::now());
      e.busy = false;
      newer
    };
    if newer {
      super::npm_latest::notify_newer();
    }
  });
}

fn lookup(source: ReleaseSource, os: Os) -> Option<semver::Version> {
  let agent: ureq::Agent =
    crate::net_proxy::ureq_config(ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false)).build().into();
  match source {
    ReleaseSource::Antigravity => {
      let platform = antigravity_platform(std::env::consts::OS, std::env::consts::ARCH, is_linux_musl())?;
      let url = format!("https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/{platform}.json");
      let body = fetch(&agent, &url, true)?;
      parse_release(source, os, &body)
    }
    ReleaseSource::Grok => {
      // Mirrors x.ai/cli/install.sh: try Cloudflare-fronted x.ai first, then direct GCS
      let body = fetch(&agent, "https://x.ai/cli/stable", false)
        .or_else(|| fetch(&agent, "https://storage.googleapis.com/grok-build-public-artifacts/cli/stable", false))?;
      parse_release(source, os, &body)
    }
    ReleaseSource::Devin => {
      let body = fetch(&agent, "https://static.devin.ai/cli/current/manifest.json", true)?;
      parse_release(source, os, &body)
    }
    ReleaseSource::Kimi => {
      let body = fetch(&agent, "https://code.kimi.com/kimi-code/latest", false)?;
      parse_release(source, os, &body)
    }
    ReleaseSource::OpenCode => {
      let body = fetch(&agent, "https://registry.npmjs.org/opencode-ai/latest", true)?;
      parse_release(source, os, &body)
    }
    ReleaseSource::Droid => {
      let url = match os {
        Os::Windows => "https://app.factory.ai/cli/windows",
        Os::Posix => "https://app.factory.ai/cli",
      };
      let body = fetch(&agent, url, false)?;
      parse_release(source, os, &body)
    }
    ReleaseSource::Hermes => {
      let body = fetch(&agent, "https://api.github.com/repos/NousResearch/hermes-agent/releases/latest", true)?;
      parse_release(source, os, &body)
    }
  }
}

fn fetch(agent: &ureq::Agent, url: &str, json: bool) -> Option<String> {
  let mut req = agent.get(url).header("user-agent", "acpira");
  if json {
    req = req.header("accept", "application/json");
  }
  let mut res = req.call().ok()?;
  let status = res.status().as_u16();
  if !(200..300).contains(&status) {
    return None;
  }
  res.body_mut().with_config().limit(MAX_BODY_BYTES).read_to_string().ok()
}

fn is_linux_musl() -> bool {
  cfg!(target_os = "linux")
    && (std::path::Path::new("/lib/libc.musl-x86_64.so.1").exists() || std::path::Path::new("/lib/libc.musl-aarch64.so.1").exists())
}

/// Platform slug used by Antigravity's manifest server (`manifests/<platform>.json`)
fn antigravity_platform(os: &str, arch: &str, musl: bool) -> Option<&'static str> {
  match (os, arch, musl) {
    ("macos", "aarch64", _) => Some("darwin_arm64"),
    ("macos", "x86_64", _) => Some("darwin_amd64"),
    ("windows", "aarch64", _) => Some("windows_arm64"),
    ("windows", "x86_64", _) => Some("windows_amd64"),
    ("linux", "aarch64", true) => Some("linux_arm64_musl"),
    ("linux", "x86_64", true) => Some("linux_amd64_musl"),
    ("linux", "aarch64", false) => Some("linux_arm64"),
    ("linux", "x86_64", false) => Some("linux_amd64"),
    _ => None,
  }
}

fn parse_semver(raw: &str) -> Option<semver::Version> {
  let trimmed = raw.trim();
  semver::Version::parse(trimmed.strip_prefix('v').unwrap_or(trimmed)).ok()
}

/// Parse the latest version from a release endpoint's response body
fn parse_release(source: ReleaseSource, os: Os, body: &str) -> Option<semver::Version> {
  match source {
    ReleaseSource::Antigravity | ReleaseSource::Devin => {
      let json: Value = serde_json::from_str(body).ok()?;
      parse_semver(json.get("version")?.as_str()?)
    }
    ReleaseSource::Grok | ReleaseSource::Kimi => body.lines().find(|l| !l.trim().is_empty()).and_then(parse_semver),
    ReleaseSource::OpenCode => {
      // opencode.ai/install only installs OpenCode 1.x and refuses 2.x: never offer a major the installer cannot install
      let json: Value = serde_json::from_str(body).ok()?;
      let v = parse_semver(json.get("version")?.as_str()?)?;
      (v.major == 1).then_some(v)
    }
    ReleaseSource::Droid => parse_droid_script(os, body),
    ReleaseSource::Hermes => {
      let json: Value = serde_json::from_str(body).ok()?;
      parse_semver(json.get("tag_name")?.as_str()?)
    }
  }
}

fn parse_droid_script(os: Os, body: &str) -> Option<semver::Version> {
  for line in body.lines() {
    let trimmed = line.trim();
    let quoted = match os {
      Os::Posix => trimmed.strip_prefix("VER="),
      Os::Windows => trimmed.strip_prefix("$version").and_then(|rest| rest.trim_start().strip_prefix('=')),
    };
    if let Some(val) = quoted {
      let unquoted = val.trim().trim_matches('"').trim_matches('\'');
      if let Some(v) = parse_semver(unquoted) {
        return Some(v);
      }
    }
  }
  None
}

#[cfg(test)]
pub fn seed(source: ReleaseSource, version: &str) {
  CACHE
    .lock()
    .insert(source, Entry { at: Some(Instant::now()), version: semver::Version::parse(version).ok(), busy: false });
}

#[cfg(test)]
pub fn clear(source: ReleaseSource) {
  CACHE.lock().remove(&source);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_each_vendor_release_format() {
    assert_eq!(
      parse_release(ReleaseSource::Antigravity, Os::Posix, r#"{"version":"1.5.16","url":"https://example.com"}"#)
        .unwrap()
        .to_string(),
      "1.5.16"
    );
    assert_eq!(parse_release(ReleaseSource::Grok, Os::Posix, "1.0.92\n").unwrap().to_string(), "1.0.92");
    assert_eq!(
      parse_release(ReleaseSource::Devin, Os::Posix, r#"{"version":"3000.14.30821","platforms":{}}"#).unwrap().to_string(),
      "3000.14.30821"
    );
    assert_eq!(parse_release(ReleaseSource::Kimi, Os::Posix, "2.7.3\n").unwrap().to_string(), "2.7.3");
    assert_eq!(parse_release(ReleaseSource::OpenCode, Os::Posix, r#"{"version":"1.19.6"}"#).unwrap().to_string(), "1.19.6");
    assert_eq!(parse_release(ReleaseSource::OpenCode, Os::Posix, r#"{"version":"2.4.1"}"#), None, "v1 installer refuses v2");
    assert_eq!(
      parse_release(ReleaseSource::Droid, Os::Posix, "#!/bin/sh\nVER=\"1.19.0\"\nBASE_URL=x\n").unwrap().to_string(),
      "1.19.0"
    );
    assert_eq!(
      parse_release(ReleaseSource::Droid, Os::Windows, "$baseUrl = 'x'\n$version = \"1.19.0\"\n").unwrap().to_string(),
      "1.19.0"
    );
    assert_eq!(parse_release(ReleaseSource::Hermes, Os::Posix, r#"{"tag_name":"v0.23.4"}"#).unwrap().to_string(), "0.23.4");
  }

  #[test]
  fn maps_antigravity_platform_slugs() {
    assert_eq!(antigravity_platform("macos", "aarch64", false), Some("darwin_arm64"));
    assert_eq!(antigravity_platform("macos", "x86_64", false), Some("darwin_amd64"));
    assert_eq!(antigravity_platform("windows", "x86_64", false), Some("windows_amd64"));
    assert_eq!(antigravity_platform("linux", "x86_64", false), Some("linux_amd64"));
    assert_eq!(antigravity_platform("linux", "aarch64", true), Some("linux_arm64_musl"));
    assert_eq!(antigravity_platform("freebsd", "x86_64", false), None);
  }
}
