//! The build's version. Release builds carry the workspace version (`X.Y.Z`); local packages set `ACPIRA_VERSION` at
//! build time to a prerelease of the next patch (`X.Y.Z-dev.<stamp>`, scripts/release/build-sidecar.mjs), so a local
//! build never shares a version with a published one and never counts as newer than the release it precedes

/// What `acpira --version`, hellos and the engine directory name report
pub const VERSION: &str = match option_env!("ACPIRA_VERSION") {
  Some(v) => v,
  None => env!("CARGO_PKG_VERSION"),
};
