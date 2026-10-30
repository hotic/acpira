//! The binaries a login service runs, copied under `<home>/devices/bin/` first: an extension upgrade deletes the folder
//! the engine runs from, and a service pointing there would fail at the next login. The engine goes into
//! `<version>-<digest>/`: the copy the gateway starts its engine from (an engine of its own, `shell: remote`, which reaches
//! the windows' engines over the push bus) may be running a device's turn while a newer one is installed, so a copy is
//! never overwritten. The gateway goes beside it, except on Windows: there it keeps one path, `gateway/`, because the
//! firewall remembers a program by its path and would ask again after every upgrade.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// The copied binaries' directory
pub fn bin_root(root: &Path) -> PathBuf {
  root.join("devices").join("bin")
}

/// Windows: the gateway's fixed directory under `bin_root`
const GATEWAY_DIR: &str = "gateway";

/// The version an installed copy was made from (`<version>-<digest>` of the directory the engine path names)
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

/// Whether `exe` is one of the copies under `bin_root`: the gateway's own engine, which is what is installed. Its
/// modification time is the copy's, never a build's
pub fn is_staged(root: &Path, exe: &Path) -> bool {
  let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
  canon(exe).starts_with(canon(&bin_root(root)))
}

/// Whether an engine of `version` built at `mine` leaves the copies installed from `installed` (built at `theirs`) in
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

/// Where `stage` puts the copies of `engine` and `gateway` (the same file for the Windows desktop exe), without copying:
/// what an install compares the installed service with
pub struct Staged {
  pub engine: PathBuf,
  pub gateway: PathBuf,
}

pub async fn plan(root: &Path, version: &str, engine: &Path, gateway: &Path) -> Result<Staged> {
  let (e, g) = (engine.to_owned(), gateway.to_owned());
  let digest = tokio::task::spawn_blocking(move || -> Result<String> {
    let mut h = Sha256::new();
    for p in if e == g { vec![&e] } else { vec![&e, &g] } {
      h.update(std::fs::read(p).with_context(|| format!("read {}", p.display()))?);
    }
    Ok(h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect())
  })
  .await??;
  let dir = bin_root(root).join(format!("{version}-{digest}"));
  let gateway_dir = if cfg!(windows) { bin_root(root).join(GATEWAY_DIR) } else { dir.clone() };
  Ok(Staged { engine: dir.join(file_name(engine)?), gateway: gateway_dir.join(file_name(gateway)?) })
}

/// Copy the binaries where `plan` says; a copy already there is kept. The gateway must not be running (Windows replaces
/// its fixed copy, moving a copy still in use aside first)
pub async fn stage(root: &Path, engine: &Path, gateway: &Path, to: &Staged) -> Result<()> {
  let dir = to.engine.parent().context("staged engine without a directory")?.to_owned();
  let same_dir = to.gateway.parent() == Some(dir.as_path());
  let names = (file_name(&to.engine)?, file_name(&to.gateway)?);
  if !to.engine.is_file() || (same_dir && !to.gateway.is_file()) {
    // Staged beside it and renamed into place: a half-copied directory is never the one a service names
    let tmp = bin_root(root).join(format!(".{}.{}.tmp", file_name(&dir)?, std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    tokio::fs::copy(engine, tmp.join(&names.0)).await.with_context(|| format!("copy {}", engine.display()))?;
    if same_dir {
      tokio::fs::copy(gateway, tmp.join(&names.1)).await.with_context(|| format!("copy {}", gateway.display()))?;
    }
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::rename(&tmp, &dir).await?;
  }
  if !same_dir && !same_file(gateway, &to.gateway).await {
    let gdir = to.gateway.parent().context("staged gateway without a directory")?;
    tokio::fs::create_dir_all(gdir).await?;
    let tmp = gdir.join(format!(".{}.{}.tmp", names.1, std::process::id()));
    tokio::fs::copy(gateway, &tmp).await.with_context(|| format!("copy {}", gateway.display()))?;
    // A running exe can be renamed but not replaced on Windows: the old one moves aside, and goes with the next prune
    if to.gateway.exists() {
      let aside = gdir.join(format!("{}.{}.old", names.1, std::process::id()));
      tokio::fs::rename(&to.gateway, &aside).await.with_context(|| format!("move {} aside", to.gateway.display()))?;
    }
    tokio::fs::rename(&tmp, &to.gateway).await?;
  }
  Ok(())
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

fn file_name(p: &Path) -> Result<String> {
  Ok(p.file_name().context("binary without a file name")?.to_string_lossy().into_owned())
}

/// Remove the copies other than `keep` (older versions a previous install left). The gateway's fixed directory stays,
/// short of the copies moved aside; a copy still running (an older engine finishing a device's turn on Windows) cannot be
/// removed and stays for the next prune
pub async fn prune(root: &Path, keep: Option<&Path>) {
  let Ok(mut rd) = tokio::fs::read_dir(bin_root(root)).await else { return };
  while let Ok(Some(e)) = rd.next_entry().await {
    if e.file_name() == GATEWAY_DIR {
      prune_aside(&e.path(), keep.is_none()).await;
    } else if keep != Some(e.path().as_path()) {
      let _ = tokio::fs::remove_dir_all(e.path()).await;
    }
  }
}

/// The gateway directory's moved-aside copies, or all of it when nothing is kept (remote access turned off)
async fn prune_aside(dir: &Path, all: bool) {
  if all {
    let _ = tokio::fs::remove_dir_all(dir).await;
    return;
  }
  let Ok(mut rd) = tokio::fs::read_dir(dir).await else { return };
  while let Ok(Some(e)) = rd.next_entry().await {
    if e.file_name().to_string_lossy().ends_with(".old") {
      let _ = tokio::fs::remove_file(e.path()).await;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn versions() {
    assert_eq!(installed_version(Path::new("/r/devices/bin/0.9.3-ab12cd/acpira")).as_deref(), Some("0.9.3"));
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
    assert!(keeps_installed("2.0.0", None, Some("2.0.0"), Some(20)), "the gateway's own engine keeps its copy");
    assert!(keeps_installed("2.0.0", Some(99), Some("2.1.0"), Some(1)), "a newer version is never replaced");
    assert!(!keeps_installed("2.0.0", Some(1), None, None), "no install recorded: installed from this engine");
  }

  #[test]
  fn the_gateways_own_engine_is_a_staged_copy() {
    let root = tempfile::tempdir().unwrap();
    let copy = bin_root(root.path()).join("2.0.0-ab12cd").join("acpira");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::write(&copy, "engine").unwrap();
    assert!(is_staged(root.path(), &copy));
    assert!(!is_staged(root.path(), &root.path().join("acpira")));
    assert!(built(&copy).is_some());
  }

  #[tokio::test]
  async fn copies_are_staged_once_and_pruned_around_the_kept_one() {
    let src = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let (engine, gateway) = (src.path().join("acpira"), src.path().join("acpira-gateway"));
    std::fs::write(&engine, "engine v1").unwrap();
    std::fs::write(&gateway, "gateway v1").unwrap();
    let first = plan(root.path(), "1.0.0", &engine, &gateway).await.unwrap();
    stage(root.path(), &engine, &gateway, &first).await.unwrap();
    assert_eq!(std::fs::read_to_string(&first.engine).unwrap(), "engine v1");
    assert_eq!(std::fs::read_to_string(&first.gateway).unwrap(), "gateway v1");
    assert_eq!(installed_version(&first.engine).as_deref(), Some("1.0.0"));

    std::fs::write(&engine, "engine v2").unwrap();
    std::fs::write(&gateway, "gateway v2").unwrap();
    let second = plan(root.path(), "1.0.1", &engine, &gateway).await.unwrap();
    assert_ne!(first.engine, second.engine, "a newer engine never overwrites a copy that may be running");
    stage(root.path(), &engine, &gateway, &second).await.unwrap();
    assert_eq!(std::fs::read_to_string(&second.gateway).unwrap(), "gateway v2");
    assert_eq!(first.gateway == second.gateway, cfg!(windows), "Windows keeps the gateway at one path");

    prune(root.path(), second.engine.parent()).await;
    assert!(!first.engine.exists());
    assert!(second.engine.is_file() && second.gateway.is_file());
    prune(root.path(), None).await;
    assert!(!second.gateway.exists());
  }

  #[tokio::test]
  async fn one_exe_with_both_roles_is_hashed_and_copied_as_such() {
    let src = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let exe = src.path().join("Acpira.exe");
    std::fs::write(&exe, "app").unwrap();
    let to = plan(root.path(), "2.0.0", &exe, &exe).await.unwrap();
    stage(root.path(), &exe, &exe, &to).await.unwrap();
    assert!(to.engine.is_file() && to.gateway.is_file());
  }
}
