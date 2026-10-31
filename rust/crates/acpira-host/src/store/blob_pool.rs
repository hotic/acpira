//! Attachments shared across sessions. A blob is named by its content (`transcript_store::blob_name`), so the same image
//! pasted into, forked into or imported into several sessions is one file on disk: each session folder holds a hard
//! link to it, and `<sessions>/.blobs/` holds one more link per name, the index a new save looks the name up in.
//!
//! Links keep every reader (older engines included) reading `<sessions>/<id>/<name>` as before, and deleting or
//! trashing a session only drops its own links. An index entry no session folder names any more is dropped by the
//! upkeep (`prune`); losing an entry only loses the sharing, never a file. Where links are refused (FAT, some network
//! volumes) every step falls back to a plain copy.
//!
//! Blocking std::fs throughout: callers run it on the blocking pool

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// The index folder, inside the sessions folder. Not a session id (a leading dot), so no reader takes it for a
/// session's blob folder
pub const POOL_DIR: &str = ".blobs";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A temp name next to `target` that `file_lock::temp_writer_pid` recognizes (`<base>.<pid>.tmp`), unique within the
/// process so two saves of one name never share it
fn temp_beside(target: &Path) -> PathBuf {
  let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  let seq = SEQ.fetch_add(1, Ordering::Relaxed);
  target.with_file_name(format!("{name}-{seq}.{}.tmp", std::process::id()))
}

fn is_file_of(path: &Path, len: u64) -> bool {
  fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == len)
}

/// Whether `path` holds exactly `bytes`. Names are a 64-bit hash prefix, so the bytes are compared before a file is
/// taken for another
fn holds(path: &Path, bytes: &[u8]) -> bool {
  is_file_of(path, bytes.len() as u64) && fs::read(path).is_ok_and(|b| b == bytes)
}

fn same_bytes(a: &Path, b: &Path) -> bool {
  use std::io::Read;
  let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
  let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
  loop {
    let (Ok(na), Ok(nb)) = (fa.read(&mut ba), fb.read(&mut bb)) else { return false };
    // Regular files fill the buffer until their end, so equal files return equal lengths
    if na != nb || ba[..na] != bb[..nb] {
      return false;
    }
    if na == 0 {
      return true;
    }
  }
}

/// Replace `target` with a link to `source` through a temp link, so a reader sees one whole file or the other
fn link_into(source: &Path, target: &Path) -> io::Result<()> {
  let tmp = temp_beside(target);
  fs::hard_link(source, &tmp)?;
  fs::rename(&tmp, target).inspect_err(|_| {
    let _ = fs::remove_file(&tmp);
  })
}

/// Enter `file` in the index under `name`, unless the index already has that name
fn enter(pool: &Path, file: &Path, name: &str) {
  if fs::create_dir_all(pool).is_ok() {
    let _ = fs::hard_link(file, pool.join(name));
  }
}

/// Store `bytes` as `dir/name`. A file already there with the same length is the same content (the name is its hash)
/// and is left alone, so a shared file is never rewritten in place; otherwise the index's copy is linked in, or the
/// bytes are written (tmp + rename) and entered in the index
pub fn place(pool: &Path, dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
  let target = dir.join(name);
  if is_file_of(&target, bytes.len() as u64) {
    return Ok(target);
  }
  let pooled = pool.join(name);
  if holds(&pooled, bytes) && link_into(&pooled, &target).is_ok() {
    // The shared file keeps its first save's mtime; a draft's attachment linked to it now must not look like an
    // old leftover to the Storage page (`storage::FRESH`), and every other link only looks fresher
    if let Ok(f) = fs::File::options().write(true).open(&target) {
      let _ = f.set_modified(SystemTime::now());
    }
    return Ok(target);
  }
  let tmp = temp_beside(&target);
  fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, &target)).inspect_err(|_| {
    let _ = fs::remove_file(&tmp);
  })?;
  enter(pool, &target, name);
  Ok(target)
}

/// Upkeep for one blob already in a session folder: enter it in the index, or, when the index holds the same content
/// as another file, replace this one with a link to it. Returns the bytes that sharing freed
pub fn share(pool: &Path, file: &Path, name: &str) -> u64 {
  let pooled = pool.join(name);
  let Ok(pm) = fs::symlink_metadata(&pooled) else {
    enter(pool, file, name);
    return 0;
  };
  let Ok(fm) = fs::symlink_metadata(file) else { return 0 };
  if !pm.is_file() || !fm.is_file() || pm.len() != fm.len() || same_file::is_same_file(&pooled, file).unwrap_or(true) {
    return 0;
  }
  if same_bytes(&pooled, file) && link_into(&pooled, file).is_ok() { fm.len() } else { 0 }
}

/// Drop the index entries whose name no session folder holds any more (`live`); the files themselves are gone with
/// their last session link
pub fn prune(pool: &Path, live: &HashSet<String>) -> usize {
  let Ok(rd) = fs::read_dir(pool) else { return 0 };
  let mut removed = 0;
  for e in rd.flatten() {
    let name = e.file_name().to_string_lossy().into_owned();
    if !live.contains(&name) && e.file_type().is_ok_and(|t| t.is_file()) && fs::remove_file(e.path()).is_ok() {
      removed += 1;
    }
  }
  removed
}

/// A key equal for two links to one file, so a size total counts a shared blob once. None where the platform gives
/// no cheap key (Windows): those files are counted once per link
pub fn file_key(meta: &fs::Metadata) -> Option<(u64, u64)> {
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
  }
  #[cfg(not(unix))]
  {
    let _ = meta;
    None
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn one_content_is_one_file_across_sessions() {
    let root = tempfile::tempdir().unwrap();
    let pool = root.path().join(POOL_DIR);
    let (a, b) = (root.path().join("a"), root.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let pa = place(&pool, &a, "x.png", b"img").unwrap();
    let pb = place(&pool, &b, "x.png", b"img").unwrap();
    assert!(same_file::is_same_file(&pa, &pb).unwrap());
    assert_eq!(fs::read(&pb).unwrap(), b"img");
    // A save of a name the folder already has leaves the shared file alone
    place(&pool, &a, "x.png", b"img").unwrap();
    assert!(same_file::is_same_file(&pa, &pb).unwrap());
    // Another content under a colliding name is never linked to the first
    let pc = place(&pool, &root.path().join("b"), "y.png", b"one").unwrap();
    fs::create_dir_all(root.path().join("c")).unwrap();
    fs::write(pool.join("z.png"), b"two").unwrap();
    let pz = place(&pool, &root.path().join("c"), "z.png", b"own").unwrap();
    assert_eq!(fs::read(&pz).unwrap(), b"own");
    assert!(!same_file::is_same_file(&pz, pool.join("z.png")).unwrap());
    assert!(pc.exists());
    // Deleting a session leaves the other's copy; pruning drops only entries nobody names
    fs::remove_dir_all(&a).unwrap();
    assert_eq!(fs::read(&pb).unwrap(), b"img");
    let live: HashSet<String> = ["x.png".to_owned()].into();
    assert_eq!(prune(&pool, &live), 2);
    assert!(pool.join("x.png").exists() && !pool.join("y.png").exists());
    assert_eq!(fs::read_dir(root.path().join("b")).unwrap().count(), 2, "no temp file is left behind");
  }

  #[test]
  fn upkeep_shares_copies_written_before_the_index() {
    let root = tempfile::tempdir().unwrap();
    let pool = root.path().join(POOL_DIR);
    let (a, b) = (root.path().join("a"), root.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.png"), b"same").unwrap();
    fs::write(b.join("x.png"), b"same").unwrap();
    fs::write(b.join("y.png"), b"diff").unwrap();
    assert_eq!(share(&pool, &a.join("x.png"), "x.png"), 0);
    assert_eq!(share(&pool, &b.join("x.png"), "x.png"), 4);
    assert!(same_file::is_same_file(a.join("x.png"), b.join("x.png")).unwrap());
    assert_eq!(share(&pool, &b.join("x.png"), "x.png"), 0, "already shared");
    // Same name, other bytes: left as it is
    fs::write(pool.join("y.png"), b"else").unwrap();
    assert_eq!(share(&pool, &b.join("y.png"), "y.png"), 0);
    assert_eq!(fs::read(b.join("y.png")).unwrap(), b"diff");
  }
}
