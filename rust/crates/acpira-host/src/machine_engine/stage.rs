//! Where the machine's engine runs from: `<ACPIRA_HOME>/engine/<version>-<digest16>/`, never from inside an IDE
//! extension or an app bundle. An update deletes those under a running engine (VS Code removes the old extension
//! folder), after which the engine could no longer start its built-in agent or Acpira's MCP server.
//!
//! A build lands here once (copied to a temporary folder, then renamed into place), together with what the engine looks
//! for beside itself: the `remote/<platform>/` helpers. The digest is the engine
//! binary's sha256, the same naming SSH hosts get (`remote/deploy.rs`). Every process started from a landed copy holds
//! a shared lock on its `.in-use` file for its whole life, so `prune` removes only copies nothing runs from.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Under ACPIRA_HOME
pub const DIR: &str = "engine";
const IN_USE: &str = ".in-use";
const DIGESTS: &str = ".digests.json";
/// The path the copy was landed from: a checkout's build finds the other platforms' helpers through it
/// (`remote::deploy::local_binary`)
const SOURCE: &str = ".source";
/// The landed binary's name, whatever the source was called (the desktop app ships it as `acpira-engine`)
pub const EXE: &str = if cfg!(windows) { "acpira.exe" } else { "acpira" };
/// Digest characters in a folder name
pub const DIGEST_LEN: usize = 16;
/// Copies kept beyond those in use: the newest few, so going back to a recent build needs no copy
const KEEP_RECENT: usize = 2;

pub fn root(home: &Path) -> PathBuf {
  home.join(DIR)
}

/// The landed copy of `source` for `version`: lands it first unless it is there already. A source already inside the
/// engine folder is used as it is
pub fn stage(home: &Path, source: &Path, version: &str) -> Result<PathBuf> {
  let root = root(home);
  if source.starts_with(&root) {
    return Ok(source.to_path_buf());
  }
  let digest = digest_of(&root, source)?;
  let name = format!("{version}-{}", digest.get(..DIGEST_LEN).unwrap_or(&digest));
  let dir = root.join(&name);
  let exe = dir.join(EXE);
  if exe.is_file() {
    complete(source, &dir);
    return Ok(exe);
  }
  std::fs::create_dir_all(&root).with_context(|| root.display().to_string())?;
  let tmp = root.join(format!(".{name}.{}", crate::util::random_hex(6)));
  let landed = (|| -> Result<()> {
    std::fs::create_dir_all(&tmp)?;
    copy_exe(source, &tmp.join(EXE))?;
    let beside = source.parent().map(Path::to_path_buf).unwrap_or_default();
    let helpers = beside.join("remote");
    if helpers.is_dir() {
      copy_tree(&helpers, &tmp.join("remote"))?;
    }
    File::create(tmp.join(IN_USE))?;
    std::fs::write(tmp.join(SOURCE), source.as_os_str().as_encoded_bytes())?;
    // Checked after the copy: what was read must be what was meant
    let copied = crate::util::sha256_hex(&std::fs::read(tmp.join(EXE))?);
    anyhow::ensure!(copied == digest, "{} changed while it was copied", source.display());
    Ok(())
  })();
  if let Err(e) = landed {
    let _ = std::fs::remove_dir_all(&tmp);
    return Err(e.context(format!("landing {} in {}", source.display(), root.display())));
  }
  match std::fs::rename(&tmp, &dir) {
    Ok(()) => Ok(exe),
    // Another start landed the same build first: theirs is the same bytes
    Err(_) if exe.is_file() => {
      let _ = std::fs::remove_dir_all(&tmp);
      Ok(exe)
    }
    Err(e) => {
      let _ = std::fs::remove_dir_all(&tmp);
      Err(anyhow::Error::new(e).context(format!("landing {}", dir.display())))
    }
  }
}

/// A copy an SSH deploy landed holds the engine alone (`remote/deploy.rs` uploads one binary); a full install of the
/// same build adds what sits beside it. Each piece goes in under a temporary name
fn complete(source: &Path, dir: &Path) {
  let beside = source.parent().map(Path::to_path_buf).unwrap_or_default();
  let suffix = crate::util::random_hex(6);
  if source_of(dir).is_none_or(|s| !s.is_file()) {
    let tmp = dir.join(format!(".source.{suffix}"));
    if std::fs::write(&tmp, source.as_os_str().as_encoded_bytes()).is_err() || std::fs::rename(&tmp, dir.join(SOURCE)).is_err() {
      let _ = std::fs::remove_file(&tmp);
    }
  }
  let helpers = beside.join("remote");
  if helpers.is_dir() && !dir.join("remote").is_dir() {
    let tmp = dir.join(format!(".remote.{suffix}"));
    if copy_tree(&helpers, &tmp).is_err() || std::fs::rename(&tmp, dir.join("remote")).is_err() {
      let _ = std::fs::remove_dir_all(&tmp);
    }
  }
}

/// Where the landed copy in `dir` came from
pub fn source_of(dir: &Path) -> Option<PathBuf> {
  let bytes = std::fs::read(dir.join(SOURCE)).ok()?;
  #[cfg(unix)]
  {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
  }
  #[cfg(not(unix))]
  {
    String::from_utf8(bytes).ok().map(PathBuf::from)
  }
}

fn copy_exe(from: &Path, to: &Path) -> Result<()> {
  std::fs::copy(from, to).with_context(|| format!("{} → {}", from.display(), to.display()))?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))?;
  }
  Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
  std::fs::create_dir_all(to)?;
  for entry in std::fs::read_dir(from)? {
    let entry = entry?;
    let (src, dst) = (entry.path(), to.join(entry.file_name()));
    if entry.file_type()?.is_dir() {
      copy_tree(&src, &dst)?;
    } else {
      copy_exe(&src, &dst)?;
    }
  }
  Ok(())
}

/// The sha256 of `source`, remembered per (path, size, mtime) in the engine folder: a debug build is large, and every
/// window start asks
fn digest_of(root: &Path, source: &Path) -> Result<String> {
  let meta = std::fs::metadata(source).with_context(|| format!("no engine binary at {}", source.display()))?;
  let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
  let key = format!("{}|{}|{mtime}", source.display(), meta.len());
  let cache_path = root.join(DIGESTS);
  let mut cache: serde_json::Map<String, serde_json::Value> =
    std::fs::read(&cache_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
  if let Some(d) = cache.get(&key).and_then(|v| v.as_str()) {
    return Ok(d.to_owned());
  }
  let digest = crate::util::sha256_hex(&std::fs::read(source).with_context(|| source.display().to_string())?);
  // Only sources that still exist are worth remembering
  cache.retain(|k, _| k.split('|').next().is_some_and(|p| Path::new(p).exists()));
  cache.insert(key, digest.clone().into());
  if std::fs::create_dir_all(root).is_ok() {
    let tmp = root.join(format!(".digests.{}", crate::util::random_hex(6)));
    if std::fs::write(&tmp, serde_json::to_vec(&cache).unwrap_or_default()).is_ok() && std::fs::rename(&tmp, &cache_path).is_err() {
      let _ = std::fs::remove_file(&tmp);
    }
  }
  Ok(digest)
}

/// The newest landed release (never a local build: one that arrived over SSH from another machine is not this
/// machine's to start), with its version
pub fn newest_release(home: &Path) -> Option<(String, PathBuf)> {
  let entries = std::fs::read_dir(root(home)).ok()?;
  entries
    .flatten()
    .filter_map(|e| {
      let name = e.file_name().to_string_lossy().into_owned();
      // `<version>-<digest>`: the digest is hex, so the last dash splits them
      let (version, _) = name.rsplit_once('-')?;
      let parsed = semver::Version::parse(version).ok().filter(|v| v.pre.is_empty())?;
      let exe = e.path().join(EXE);
      exe.is_file().then_some((parsed, version.to_owned(), exe))
    })
    .max_by(|a, b| a.0.cmp(&b.0))
    .map(|(_, v, exe)| (v, exe))
}

/// Called at the start of every `acpira` process: one running from a landed copy keeps it from `prune` until it exits
pub fn hold_in_use() {
  static HELD: std::sync::OnceLock<Option<File>> = std::sync::OnceLock::new();
  HELD.get_or_init(|| {
    let exe = std::env::current_exe().ok()?;
    let marker = exe.parent()?.join(IN_USE);
    if !marker.is_file() {
      return None;
    }
    let f = File::open(&marker).ok()?;
    f.lock_shared().ok()?;
    Some(f)
  });
}

/// Holds `exe`'s landed copy for as long as the returned file lives: from picking it until the engine started from it
/// holds it itself (`hold_in_use`), a prune elsewhere cannot take it
pub fn pin(exe: &Path) -> Option<File> {
  let f = File::open(exe.parent()?.join(IN_USE)).ok()?;
  f.lock_shared().ok()?;
  Some(f)
}

/// Removes landed copies nothing runs from, beyond `keep`, the newest release (what a start falls back to) and the most
/// recently landed few. A copy in use on Windows (its exe cannot be deleted) is left for a later prune
pub fn prune(home: &Path, keep: &[PathBuf]) {
  let root = root(home);
  let Ok(entries) = std::fs::read_dir(&root) else { return };
  let newest_release = newest_release(home).map(|(_, exe)| exe);
  let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = vec![];
  for e in entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
    let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    // A copy still landing or one that failed half way (`.<name>.<random>`): debris once older than a minute, whatever
    // its marker says (the one landing has not taken it yet)
    if e.file_name().to_string_lossy().starts_with('.') {
      if modified.elapsed().is_ok_and(|age| age.as_secs() > 60) {
        let _ = std::fs::remove_dir_all(e.path());
      }
      continue;
    }
    dirs.push((modified, e.path()));
  }
  dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
  for (i, (_, dir)) in dirs.into_iter().enumerate() {
    let kept = keep.iter().chain(&newest_release).any(|k| k.starts_with(&dir));
    if i < KEEP_RECENT || kept {
      continue;
    }
    // No marker: not a landed copy
    let Ok(marker) = File::open(dir.join(IN_USE)) else { continue };
    if marker.try_lock().is_ok() {
      let _ = std::fs::remove_dir_all(&dir);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_build_lands_once_with_what_sits_beside_it_and_unused_copies_go() {
    let home = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let bin = src.path().join("acpira-engine");
    std::fs::write(&bin, b"engine one").unwrap();
    std::fs::create_dir_all(src.path().join("remote/linux-x86_64")).unwrap();
    std::fs::write(src.path().join("remote/linux-x86_64/acpira"), b"helper").unwrap();
    let a = stage(home.path(), &bin, "2.0.6").unwrap();
    assert!(a.starts_with(root(home.path())) && a.ends_with(EXE));
    assert_eq!(std::fs::read(&a).unwrap(), b"engine one");
    assert!(a.with_file_name("remote").join("linux-x86_64/acpira").is_file());
    // Landed once: the same source lands at the same place
    assert_eq!(stage(home.path(), &bin, "2.0.6").unwrap(), a);
    // A landed copy is used where it is
    assert_eq!(stage(home.path(), &a, "2.0.6").unwrap(), a);

    // Three local builds: the two newest and the one in use stay
    let held = File::open(a.with_file_name(IN_USE)).unwrap();
    held.lock_shared().unwrap();
    let mut later = vec![];
    for n in 0..3 {
      std::thread::sleep(std::time::Duration::from_millis(20));
      std::fs::write(&bin, format!("engine {n}")).unwrap();
      later.push(stage(home.path(), &bin, &format!("2.0.8-dev.{n}")).unwrap());
    }
    prune(home.path(), &[]);
    assert!(a.is_file(), "in use");
    assert!(!later[0].exists(), "unused and not recent");
    assert!(later[1].is_file() && later[2].is_file());
    drop(held);
    prune(home.path(), &[]);
    assert!(a.is_file(), "the newest release stays for a start to fall back to");
    // A newer release lands: the old one is neither recent nor the newest release any more
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&bin, b"release").unwrap();
    stage(home.path(), &bin, "2.0.7").unwrap();
    // A child another test forks meanwhile shares the locked descriptor until it execs, so a prune may find it held once
    for _ in 0..50 {
      prune(home.path(), &[]);
      if !a.exists() {
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!a.exists());
    // Only releases count as the newest landed copy
    assert_eq!(newest_release(home.path()).map(|(v, _)| v).as_deref(), Some("2.0.7"));
  }

  #[test]
  fn a_copy_landed_bare_gets_what_a_full_install_carries() {
    let home = tempfile::tempdir().unwrap();
    let (bare, full) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for d in [&bare, &full] {
      std::fs::write(d.path().join(EXE), b"same engine").unwrap();
    }
    std::fs::create_dir_all(full.path().join("remote/linux-x86_64")).unwrap();
    std::fs::write(full.path().join("remote/linux-x86_64/acpira"), b"helper").unwrap();
    let a = stage(home.path(), &bare.path().join(EXE), "2.0.6").unwrap();
    assert!(!a.with_file_name("remote").exists());
    assert_eq!(stage(home.path(), &full.path().join(EXE), "2.0.6").unwrap(), a);
    assert!(a.with_file_name("remote").join("linux-x86_64/acpira").is_file());
  }
}
