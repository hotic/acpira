//! Which build serves the machine. One engine per machine (per user and data root): a newer build takes the endpoint
//! over, an older one connects to what runs. Ordered by semver, so a local package (`X.Y.Z-dev.<stamp>`,
//! `acpira_shared::version`) sorts after the release before it and before the release it leads up to.
//!
//! A local build replaces a release only when it was installed on this machine on purpose (`allow_dev`): an IDE or the
//! desktop app carrying it. One that arrives over SSH from another machine's work in progress never replaces the host's
//! engine; it connects to it like any older client.

use semver::Version;

/// Whether a build of `mine` takes the endpoint from a running engine of `theirs`. Equal versions never do (two builds
/// of one version would take it from each other in turns); an unparsable version takes nothing and is taken from by
/// nothing
pub fn takes_over(mine: &str, theirs: &str, allow_dev: bool) -> bool {
  let (Ok(mine), Ok(theirs)) = (Version::parse(mine), Version::parse(theirs)) else { return false };
  mine > theirs && (mine.pre.is_empty() || allow_dev)
}

/// A local build (a prerelease version)
pub fn is_dev(version: &str) -> bool {
  Version::parse(version).is_ok_and(|v| !v.pre.is_empty())
}

/// A local package (`X.Y.Z-dev.<stamp>`, `scripts/release/local-version.mjs`): never published, so nothing can be
/// downloaded for it
pub fn is_local_package(version: &str) -> bool {
  Version::parse(version).is_ok_and(|v| v.pre.as_str().starts_with("dev."))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn newer_releases_win_and_local_builds_only_where_installed_on_purpose() {
    assert!(takes_over("2.0.7", "2.0.6", false));
    assert!(!takes_over("2.0.6", "2.0.7", true));
    assert!(!takes_over("2.0.6", "2.0.6", true));
    // A local build leads up to the next release: it beats the last one, the next one beats it
    assert!(takes_over("2.0.7-dev.20261103120000", "2.0.6", true));
    assert!(!takes_over("2.0.7-dev.20261103120000", "2.0.6", false));
    assert!(takes_over("2.0.7", "2.0.7-dev.20261103120000", false));
    // Stamps of different lengths still order by value
    assert!(takes_over("2.0.7-dev.20261103120000", "2.0.7-dev.9", true));
    assert!(!takes_over("garbage", "2.0.6", true));
    assert!(!takes_over("2.0.7", "garbage", true));
  }
}
