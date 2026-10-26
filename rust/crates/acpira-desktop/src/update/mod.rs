//! The desktop app updating itself (docs/dev/desktop.md "Updates"): a signed manifest on the latest GitHub release,
//! a per-file zstd delta from the installed version (or the full package), assembled on the install's volume, checked
//! file by file and (macOS) by its code signature, and put in place when the app quits: a macOS bundle swapped whole,
//! Windows' single exe renamed aside. Window-free; apps/desktop schedules it, shows its state and restarts. The release
//! workflow builds the packages with `acpira-update-pack`

pub mod delta;
pub mod fetch;
pub mod manifest;
pub mod stage;
pub mod state;
pub mod swap;

pub use state::{Config, Env, Phase, Staged, UpdateState, Updater};
pub use swap::ManualReason;

/// The manifest of the latest stable release (drafts and pre-releases never become `latest`)
pub const MANIFEST_URL: &str = "https://github.com/hotic/acpira/releases/latest/download/desktop-update.json";
/// Where an install that cannot update itself sends the user
pub const RELEASE_URL: &str = "https://github.com/hotic/acpira/releases/latest";
/// The Ed25519 public keys (hex) that may sign the manifest; two can be listed while one is rotated out. Empty until
/// the release key exists (`acpira-update-pack keygen`, the seed in the `DESKTOP_UPDATE_KEY` secret): a build without
/// a key never checks
pub const PUBLIC_KEYS: &[&str] = &[];
/// The Developer ID team the running and the staged macOS bundle must be signed by; empty until the certificate exists.
/// Windows does not use it (stage.rs)
pub const TEAM_ID: &str = "";

impl Config {
  /// A release build's configuration
  pub fn release() -> Config {
    Config {
      manifest_url: MANIFEST_URL.into(),
      keys: PUBLIC_KEYS.iter().filter_map(|k| manifest::key_of_hex(k)).collect(),
      policy: fetch::Policy::release(),
      requirement: (cfg!(target_os = "macos") && !TEAM_ID.is_empty()).then(|| stage::requirement(TEAM_ID)),
      notarized: cfg!(target_os = "macos"),
      release_url: RELEASE_URL.into(),
    }
  }
}

/// Why the running bundle cannot update itself, before any check: not a bundle (or packaged exe), a location it cannot
/// be replaced in, or not signed by the release team (a development build). `team` reads the running bundle's signing
/// team (a codesign run, skipped when nothing depends on it). A Windows exe has no signer to compare yet (stage.rs): it
/// is held back only by where it runs from, and a build without an update key by `Updater::new`
pub fn install_blocker(installed: Option<&std::path::Path>, team: impl FnOnce(&std::path::Path) -> Option<String>) -> Option<ManualReason> {
  let Some(app) = installed else { return Some(ManualReason::NotBundled) };
  if swap::Layout::of(app) == swap::Layout::Exe {
    return swap::extracted_blocker(app, &std::env::temp_dir()).or_else(|| swap::location_blocker(app));
  }
  if TEAM_ID.is_empty() {
    return Some(ManualReason::Unconfigured);
  }
  if team(app).as_deref() != Some(TEAM_ID) {
    return Some(ManualReason::Unsigned);
  }
  swap::location_blocker(app)
}

/// The system's version (`26.0.1`), for the manifest's minimum
pub fn os_version() -> Option<String> {
  #[cfg(target_os = "macos")]
  {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    // SAFETY: the name is NUL-terminated and the buffer's length is passed with it
    let ok =
      unsafe { libc::sysctlbyname(c"kern.osproductversion".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    let s = ok.then(|| String::from_utf8_lossy(&buf[..len]).trim_end_matches('\0').trim().to_owned())?;
    (!s.is_empty()).then_some(s)
  }
  #[cfg(not(target_os = "macos"))]
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_release_configuration_is_consistent() {
    for k in PUBLIC_KEYS {
      assert!(manifest::key_of_hex(k).is_some(), "{k} is not a 32-byte hex key");
    }
    let c = Config::release();
    assert_eq!(c.keys.len(), PUBLIC_KEYS.len());
    assert!(c.policy.check(MANIFEST_URL).is_ok());
    assert!(!c.policy.loopback_http);
    assert_eq!(c.requirement.is_some(), cfg!(target_os = "macos") && !TEAM_ID.is_empty());
    assert_eq!(c.notarized, cfg!(target_os = "macos"));
    assert_eq!(install_blocker(None, |_| None), Some(ManualReason::NotBundled));
  }

  #[test]
  fn an_exe_needs_no_team_but_a_place_it_can_be_replaced_in() {
    let d = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let exe = d.path().join("Acpira.exe");
    std::fs::write(&exe, b"MZ").unwrap();
    // Never asks for a signing team, whatever TEAM_ID holds
    assert_eq!(install_blocker(Some(&exe), |_| panic!("no codesign for an exe")), None);
    let extracted = tempfile::tempdir().unwrap();
    let exe = extracted.path().join("Acpira.exe");
    std::fs::write(&exe, b"MZ").unwrap();
    assert_eq!(install_blocker(Some(&exe), |_| None), Some(ManualReason::Extracted));
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn the_system_version_reads_as_a_version() {
    let v = os_version().unwrap();
    assert!(manifest::parse_version(&v).is_some(), "{v}");
  }
}
