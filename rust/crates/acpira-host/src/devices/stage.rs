//! The engine binary remote access's login service starts: a copy at one path, `<home>/devices/service/acpira`, since an
//! extension upgrade deletes the folder an engine runs from, and macOS and Windows remember a program by its path
//! (privacy grants, Login Items, firewall rules). On macOS and Linux the engine started from it moves into its landed copy
//! at once (`cli::run`), so this copy may be replaced or removed under it (on macOS the launcher above both keeps the
//! privacy grants, machine_engine/launcher.rs); Windows runs from it, so a copy in use is
//! moved aside first. `gateway.json` records the build it was made from (`engine`, `engineBuilt`), as it did for the
//! copies of builds up to 2.0.6 (`devices/bin/`, removed once a newer engine installs this one), which those builds still
//! read to leave a newer install alone.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The copy's directory; the Windows task's records sit beside it (schtask.rs, autostart.rs)
pub fn dir(root: &Path) -> PathBuf {
  root.join("devices").join("service")
}

/// The copy the login service starts
pub fn exe(root: &Path) -> PathBuf {
  dir(root).join(crate::machine_engine::stage::EXE)
}

/// The copies and the gateway of builds up to 2.0.6
fn legacy(root: &Path) -> PathBuf {
  root.join("devices").join("bin")
}

/// `gateway.json`'s `engine` for this version: a path whose folder names it, `<version>-service`, as
/// `installed_version` reads it (so do builds up to 2.0.6); nothing runs from it
pub fn record(root: &Path, version: &str) -> PathBuf {
  dir(root).join(format!("{version}-service")).join(crate::machine_engine::stage::EXE)
}

/// The version an install was made from (`<version>-<suffix>` of the folder `gateway.json`'s `engine` names)
pub fn installed_version(engine: &Path) -> Option<String> {
  let dir = engine.parent()?.file_name()?.to_str()?;
  dir.rsplit_once('-').map(|(v, _)| v.to_owned())
}

/// Whether `a` is a later version than `b` (numeric dot parts; a pre-release suffix is ignored)
pub fn newer(a: &str, b: &str) -> bool {
  let parts = |s: &str| -> Vec<u64> { s.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
  parts(a) > parts(b)
}

/// When a binary was built or installed: its modification time, in ms since the epoch. Builds of one version carry no
/// other order, and the latest one installed is the one meant to run
pub fn built(exe: &Path) -> Option<u64> {
  let t = std::fs::metadata(exe).and_then(|m| m.modified()).ok()?;
  t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

/// Whether `exe` is the service's copy: its modification time is the copy's, never a build's
pub fn is_copy(root: &Path, exe: &Path) -> bool {
  let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
  canon(exe) == canon(&self::exe(root))
}

/// Whether an engine of `version` built at `mine` leaves the copy installed from `installed` (built at `theirs`) in
/// place. An older version is replaced and a newer one kept. Within one version only a strictly later build takes over,
/// so the earlier engine never reinstalls it back. An install from before the build time was recorded is replaced once.
/// An engine that cannot tell its own build time keeps what is there
pub fn keeps_installed(version: &str, mine: Option<u64>, installed: Option<&str>, theirs: Option<u64>) -> bool {
  let Some(v) = installed else { return false };
  if newer(version, v) {
    return false;
  }
  if newer(v, version) {
    return true;
  }
  !mine.is_some_and(|m| theirs.is_none_or(|t| m > t))
}

/// Copy `from` to the service's path, unless it holds the same bytes already (or is that path). A copy in use on
/// Windows moves aside first and goes with a later `remove_aside`
pub async fn stage(root: &Path, from: &Path) -> Result<PathBuf> {
  let to = exe(root);
  if is_copy(root, from) || same_file(from, &to).await {
    return Ok(to);
  }
  let d = dir(root);
  tokio::fs::create_dir_all(&d).await?;
  let name = to.file_name().context("no file name")?.to_string_lossy().into_owned();
  let tmp = d.join(format!(".{name}.{}.tmp", std::process::id()));
  tokio::fs::copy(from, &tmp).await.with_context(|| format!("copy {}", from.display()))?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
  }
  // A running exe can be renamed but not replaced on Windows
  if cfg!(windows) && to.exists() {
    let aside = d.join(format!("{name}.{}.old", std::process::id()));
    tokio::fs::rename(&to, &aside).await.with_context(|| format!("move {} aside", to.display()))?;
  }
  tokio::fs::rename(&tmp, &to).await?;
  Ok(to)
}

/// Same length and contents
async fn same_file(a: &Path, b: &Path) -> bool {
  let (a, b) = (a.to_owned(), b.to_owned());
  tokio::task::spawn_blocking(move || {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(&a), std::fs::metadata(&b)) else { return false };
    ma.len() == mb.len() && std::fs::read(&a).ok() == std::fs::read(&b).ok()
  })
  .await
  .unwrap_or(false)
}

/// Copies moved aside, and the copies and gateway of builds up to 2.0.6. What is still running on Windows stays for a
/// later call
pub async fn remove_aside(root: &Path) {
  let _ = tokio::fs::remove_dir_all(legacy(root)).await;
  let Ok(mut rd) = tokio::fs::read_dir(dir(root)).await else { return };
  while let Ok(Some(e)) = rd.next_entry().await {
    let name = e.file_name().to_string_lossy().into_owned();
    if name.ends_with(".old") || name.ends_with(".tmp") {
      let _ = tokio::fs::remove_file(e.path()).await;
    }
  }
}

/// Remote access turned off: the copy goes too (on Windows not while it runs)
pub async fn remove(root: &Path) {
  remove_aside(root).await;
  let _ = tokio::fs::remove_file(exe(root)).await;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn versions() {
    assert_eq!(installed_version(Path::new("/r/devices/bin/0.9.3-ab12cd/acpira")).as_deref(), Some("0.9.3"));
    assert_eq!(installed_version(&record(Path::new("/r"), "2.0.7-dev.20261103")).as_deref(), Some("2.0.7-dev.20261103"));
    assert!(newer("0.10.0", "0.9.3"));
    assert!(!newer("0.9.3", "0.9.3"));
    assert!(!newer("0.9.3-dev", "0.9.3"));
  }

  #[test]
  fn two_builds_of_one_version_settle_on_the_later() {
    // The VS Code engine (built at 10) and the desktop app's (built at 20) both start with remote access on
    assert!(!keeps_installed("2.0.0", Some(10), Some("1.11.0"), Some(5)), "an older version is replaced");
    assert!(!keeps_installed("2.0.0", Some(10), Some("2.0.0"), None), "an install without a build time is replaced once");
    assert!(!keeps_installed("2.0.0", Some(20), Some("2.0.0"), Some(10)), "the later build takes over");
    assert!(keeps_installed("2.0.0", Some(10), Some("2.0.0"), Some(20)), "the earlier build leaves it alone");
    assert!(keeps_installed("2.0.0", Some(20), Some("2.0.0"), Some(20)), "the same build changes nothing");
    assert!(keeps_installed("2.0.0", None, Some("2.0.0"), Some(20)), "the service's own engine keeps its copy");
    assert!(keeps_installed("2.0.0", Some(99), Some("2.1.0"), Some(1)), "a newer version is never replaced");
    assert!(!keeps_installed("2.0.0", Some(1), None, None), "no install recorded: installed from this engine");
  }

  #[tokio::test]
  async fn the_copy_keeps_its_path_and_goes_with_the_legacy_ones() {
    let src = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let engine = src.path().join("acpira-engine");
    std::fs::write(&engine, "engine v1").unwrap();
    let first = stage(root.path(), &engine).await.unwrap();
    assert_eq!(first, exe(root.path()));
    assert!(is_copy(root.path(), &first));
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "engine v1");
    std::fs::write(&engine, "engine v2").unwrap();
    assert_eq!(stage(root.path(), &engine).await.unwrap(), first, "one path for every build");
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "engine v2");
    // The copy staged from itself is left as it is
    assert_eq!(stage(root.path(), &first).await.unwrap(), first);

    std::fs::create_dir_all(legacy(root.path()).join("gateway")).unwrap();
    remove_aside(root.path()).await;
    assert!(!legacy(root.path()).exists() && first.is_file());
    remove(root.path()).await;
    assert!(!first.exists());
  }
}
