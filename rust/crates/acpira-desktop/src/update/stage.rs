//! Where a new bundle is assembled and the checks it passes before it may replace the installed one: the manifest's
//! file list (`delta::verify_tree`), then the Apple code signature with our identifier and team, then Gatekeeper's
//! assessment (notarized). `codesign` / `spctl` run as child processes rather than through Security.framework bindings.
//! A Windows exe passes the file list only: it is not Authenticode-signed yet, so the Ed25519-signed manifest and its
//! per-file digests are the whole check there. Once it is signed, a `WinVerifyTrust` check of the staged exe against the
//! running one's signer belongs beside `verify_signature`, run from the same place in `Updater::run`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The bundle identifier a staged bundle must carry (tauri.conf.json)
pub const IDENTIFIER: &str = "cc.hotic.acpira.desktop";

/// Whether two existing paths are on one volume: the swap is a rename, which cannot cross volumes
pub fn same_volume(a: &Path, b: &Path) -> bool {
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    matches!((std::fs::metadata(a), std::fs::metadata(b)), (Ok(x), Ok(y)) if x.dev() == y.dev())
  }
  #[cfg(windows)]
  {
    matches!((volume_of(a), volume_of(b)), (Some(x), Some(y)) if x.eq_ignore_ascii_case(&y))
  }
  #[cfg(not(any(unix, windows)))]
  {
    let _ = (a, b);
    true
  }
}

/// The mount point a path lives under (`C:\`, or a folder another volume is mounted on), from GetVolumePathNameW
#[cfg(windows)]
fn volume_of(p: &Path) -> Option<String> {
  use std::os::windows::ffi::OsStrExt;
  use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
  let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
  let mut buf = vec![0u16; w.len().max(261) + 1];
  // SAFETY: a NUL-terminated input and an output buffer of the length passed
  let ok = unsafe { GetVolumePathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } != 0;
  let n = buf.iter().position(|&c| c == 0)?;
  ok.then(|| String::from_utf16_lossy(&buf[..n]))
}

/// The folder version `version` is assembled in: under the cache when the cache shares the install's volume, else
/// beside the installed bundle or exe (a hidden folder in its parent), so putting it in place is a rename
pub fn staging_dir(installed: &Path, cache: &Path, version: &str) -> PathBuf {
  let parent = installed.parent().unwrap_or(Path::new("/"));
  let _ = std::fs::create_dir_all(cache);
  if same_volume(cache, parent) { cache.join(version) } else { parent.join(format!(".Acpira-{version}.staging")) }
}

/// The code requirement of a release bundle: an Apple-anchored Developer ID certificate of `team`, our identifier
pub fn requirement(team: &str) -> String {
  format!("anchor apple generic and identifier \"{IDENTIFIER}\" and certificate leaf[subject.OU] = \"{team}\"")
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
  let out = Command::new(program).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{program}: {e}"))?;
  if out.status.success() {
    return Ok(());
  }
  let msg = String::from_utf8_lossy(&out.stderr);
  Err(format!("{} failed: {}", Path::new(program).file_name().and_then(|n| n.to_str()).unwrap_or(program), msg.trim()))
}

/// `codesign --verify --deep --strict` with the requirement as text (`-R=<text>`; without `=` codesign reads a file)
pub fn verify_signature(app: &Path, requirement: &str) -> Result<(), String> {
  let app = app.to_str().ok_or("the staged bundle's path is not UTF-8")?;
  run("/usr/bin/codesign", &["--verify", "--deep", "--strict", &format!("-R={requirement}"), app])
}

/// Gatekeeper accepts the bundle as an app (Developer ID signed and notarized)
pub fn verify_notarized(app: &Path) -> Result<(), String> {
  let app = app.to_str().ok_or("the staged bundle's path is not UTF-8")?;
  run("/usr/sbin/spctl", &["--assess", "--type", "execute", app])
}

/// The Team ID a bundle is signed with; None for an ad hoc or unsigned bundle (a development build)
pub fn signing_team(app: &Path) -> Option<String> {
  let out = Command::new("/usr/bin/codesign").args(["-dv", "--verbose=2"]).arg(app).stdin(Stdio::null()).output().ok()?;
  // codesign prints the details on stderr
  team_of(&String::from_utf8_lossy(&out.stderr))
}

fn team_of(details: &str) -> Option<String> {
  details
    .lines()
    .find_map(|l| l.strip_prefix("TeamIdentifier="))
    .map(str::trim)
    .filter(|t| !t.is_empty() && *t != "not set")
    .map(str::to_owned)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_team_comes_from_codesigns_details() {
    assert_eq!(team_of("Executable=/A.app/Contents/MacOS/A\nTeamIdentifier=ABCDE12345\nSealed Resources"), Some("ABCDE12345".into()));
    assert_eq!(team_of("Signature=adhoc\nTeamIdentifier=not set\n"), None);
    assert_eq!(team_of("code object is not signed at all"), None);
  }

  #[test]
  fn the_requirement_pins_identifier_and_team() {
    let r = requirement("ABCDE12345");
    assert!(r.starts_with("anchor apple generic and identifier \"cc.hotic.acpira.desktop\""));
    assert!(r.ends_with("certificate leaf[subject.OU] = \"ABCDE12345\""));
  }

  #[test]
  fn staging_stays_on_the_installs_volume() {
    let d = tempfile::tempdir().unwrap();
    let installed = d.path().join("Applications/Acpira.app");
    std::fs::create_dir_all(&installed).unwrap();
    let cache = d.path().join("Caches/updates");
    assert_eq!(staging_dir(&installed, &cache, "1.11.0"), cache.join("1.11.0"));
    assert!(same_volume(&cache, d.path()));
  }

  // The real tools on a bundle signed here: ad hoc, so the identifier holds and a team requirement fails
  #[cfg(target_os = "macos")]
  #[test]
  fn codesign_checks_an_ad_hoc_bundle_by_its_identifier() {
    let d = tempfile::tempdir().unwrap();
    let app = d.path().join("T.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/T")).unwrap();
    std::fs::write(
      app.join("Contents/Info.plist"),
      format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>T</string><key>CFBundleIdentifier</key><string>{IDENTIFIER}</string></dict></plist>"
      ),
    )
    .unwrap();
    let signed = Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&app).output().unwrap();
    if !signed.status.success() {
      eprintln!("codesign unavailable: {}", String::from_utf8_lossy(&signed.stderr));
      return;
    }
    verify_signature(&app, &format!("identifier \"{IDENTIFIER}\"")).unwrap();
    assert!(verify_signature(&app, &requirement("ABCDE12345")).is_err());
    assert_eq!(signing_team(&app), None);
    // A byte changed after signing breaks the seal
    std::fs::write(app.join("Contents/Info.plist"), "<plist/>").unwrap();
    assert!(verify_signature(&app, &format!("identifier \"{IDENTIFIER}\"")).is_err());
  }
}
